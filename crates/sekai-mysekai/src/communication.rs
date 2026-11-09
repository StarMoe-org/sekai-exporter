//! The performances characters play while they are together.
//!
//! [`CommunicationActions`] produces the `communication-actions` form, specified in
//! `docs/spec/communication-actions.md`.

use std::collections::BTreeMap;

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::lua::{BinaryOp, Block, Call, Expr, Stat};
use crate::performance::{Performance, Step, Timeline};
use crate::scripts::{
    NoAssets, OneAsset, RANDOM_RANGE, RandomRange, Scripts, document, is_call, malformed,
    random_below, random_draw, rows,
};

const NAME: &str = "mysekai-runtime/communication-actions";
const BUNDLE: &str = "mysekai/some_character_communication";
const TABLE: &str = "mysekaiCharacterTalkTogetherCommunications";

const LIFETIME: &str = "enable_communication";
const PARTICIPATES: &str = "is_communicating";
const WAIT: &str = "wait_time";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Row {
    game_character_unit_id: i64,
    lua: String,
}

#[derive(Serialize)]
struct Document {
    version: u32,
    units: BTreeMap<String, Unit>,
}

#[derive(Serialize)]
struct Unit {
    program: Program,
}

#[derive(Debug, Serialize)]
struct Program {
    kind: &'static str,
    random: Option<RandomRange>,
    branches: Vec<Branch>,
    steps: Vec<Step>,
}

#[derive(Debug, Serialize)]
struct Branch {
    condition: Condition,
    steps: Vec<Step>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Condition {
    participant: Option<i64>,
    random_below: i64,
}

/// The performances of the characters.
#[derive(Debug, Clone, Copy, Default)]
pub struct CommunicationActions;

impl TableConverter for CommunicationActions {
    fn name(&self) -> &'static str {
        "communication-actions"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let Some(scripts) = Scripts::open(cx, BUNDLE) else {
            return Ok(Box::new(NoAssets));
        };
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(scripts, &rows::<Row>(cx, TABLE)?)
        })))
    }
}

fn build(scripts: Scripts<'_>, table: &[Row]) -> Result<Asset, ConvertError> {
    let constants = scripts.defines()?;
    let library = scripts.library()?;
    let performance = Performance::new(&constants, &library)?;
    let reader = ProgramReader {
        performance: &performance,
    };
    let mut units = BTreeMap::new();
    for row in table {
        let unit = Unit {
            program: reader.program(&scripts.parse(&row.lua)?)?,
        };
        let key = row.game_character_unit_id.to_string();
        if units.insert(key, unit).is_some() {
            return Err(malformed(format!(
                "character {} has two programs",
                row.game_character_unit_id
            )));
        }
    }
    document(&Document { version: 1, units })
}

/// The draw and the branches that open a loop, and the statements after them.
struct Opening<'s> {
    random: Option<RandomRange>,
    branches: Vec<Branch>,
    tail: &'s [Stat],
}

struct ProgramReader<'a> {
    performance: &'a Performance<'a>,
}

impl ProgramReader<'_> {
    /// The program of a script that is one `while enable_communication() do ... end`.
    fn program(&self, script: &[Stat]) -> Result<Program, ConvertError> {
        let [Stat::While(condition, body)] = script else {
            return Err(malformed("a script is not one lifetime loop"));
        };
        if !is_call(condition, LIFETIME) {
            return Err(malformed("a script is not one lifetime loop"));
        }
        let opening = match body.split_first().map(|(s, r)| (random_draw(s), r)) {
            Some((Some(range), rest)) => self.random_opening(range, rest)?,
            _ => Opening {
                random: None,
                branches: Vec::new(),
                tail: body,
            },
        };
        let steps = self.steps(opening.tail)?;
        if !steps.iter().any(Step::waits) {
            return Err(malformed("a lifetime loop has no positive wait"));
        }
        Ok(Program {
            kind: "whileCommunication",
            random: opening.random,
            branches: opening.branches,
            steps,
        })
    }

    /// The branches that follow the draw of a random number, and the statements after them.
    fn random_opening<'s>(
        &self,
        range: RandomRange,
        statements: &'s [Stat],
    ) -> Result<Opening<'s>, ConvertError> {
        if range != RANDOM_RANGE {
            return Err(malformed(format!("an unknown random range {range:?}")));
        }
        let Some((Stat::If(arms, None), tail)) = statements.split_first() else {
            return Err(malformed("a random draw has no branch"));
        };
        let branches = arms
            .iter()
            .map(|(condition, body)| self.branch(condition, body))
            .collect::<Result<_, _>>()?;
        Ok(Opening {
            random: Some(range),
            branches,
            tail,
        })
    }

    fn branch(&self, condition: &Expr, body: &Block) -> Result<Branch, ConvertError> {
        Ok(Branch {
            condition: self.condition(condition)?,
            steps: self.steps(body)?,
        })
    }

    /// `is_communicating(character) and rand < n`, or `rand < n`.
    fn condition(&self, condition: &Expr) -> Result<Condition, ConvertError> {
        let unknown = || malformed("an unknown branch condition");
        if let Some(random_below) = random_below(condition) {
            return Ok(Condition {
                participant: None,
                random_below,
            });
        }
        let Expr::Binary(BinaryOp::And, participant, random) = condition else {
            return Err(unknown());
        };
        let Expr::Call(call) = participant.as_ref() else {
            return Err(unknown());
        };
        if call.name() != Some(PARTICIPATES) {
            return Err(unknown());
        }
        Ok(Condition {
            participant: Some(self.performance.actor(call)?),
            random_below: random_below(random).ok_or_else(unknown)?,
        })
    }

    /// The steps of statements that are all calls writing to a channel.
    fn steps(&self, statements: &[Stat]) -> Result<Vec<Step>, ConvertError> {
        let mut timeline = Timeline::default();
        for statement in statements {
            let Stat::Call(call) = statement else {
                return Err(malformed("control flow among the steps of a program"));
            };
            self.play(call, &mut timeline)?;
        }
        Ok(timeline.into_steps())
    }

    fn play(&self, call: &Call, timeline: &mut Timeline) -> Result<(), ConvertError> {
        let actions = self
            .performance
            .actions(call)?
            .ok_or_else(|| malformed(format!("an unknown operation {:?}", call.name())))?;
        let who = match call.name() {
            Some(WAIT) => None,
            _ => Some(self.performance.actor(call)?),
        };
        timeline.play(who, actions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::Constants;
    use crate::lua::parse;

    const DEFINES: &str = "Characters = { Ichika = 1, Saki = 2 }\n\
                           EyePresets = { normal = \"normal\" }\n";

    fn program(source: &str) -> Result<serde_json::Value, ConvertError> {
        let constants = Constants::read(&parse(DEFINES).unwrap());
        let library = Constants::read(&parse("local animation_blend_time = 0.5").unwrap());
        let performance = Performance::new(&constants, &library).unwrap();
        let reader = ProgramReader {
            performance: &performance,
        };
        let program = reader.program(&parse(source).unwrap())?;
        Ok(serde_json::to_value(program).unwrap())
    }

    #[test]
    fn a_plain_loop_has_steps_and_no_branches() {
        let value = program(
            "while enable_communication() do\n\
             change_npc_eye(Characters.Ichika, EyePresets.normal)\nwait_time(3)\nend",
        )
        .unwrap();
        assert_eq!(value["kind"], "whileCommunication");
        assert!(value["random"].is_null());
        assert_eq!(value["branches"].as_array().unwrap().len(), 0);
        let steps = value["steps"].as_array().unwrap();
        assert_eq!(steps[0]["who"], 1);
        assert!(steps[1].get("who").is_none());
        assert_eq!(steps[1]["seconds"], 3);
    }

    #[test]
    fn a_random_loop_has_its_two_branches_and_a_tail() {
        let value = program(
            "while enable_communication() do\nrand = math.random(0, 99)\n\
             if is_communicating(Characters.Saki) and rand < 1 then wait_time(1)\n\
             elseif (rand < 100) then wait_time(2) end\nwait_time(3)\nend",
        )
        .unwrap();
        assert_eq!(value["random"], serde_json::json!({"min": 0, "max": 99}));
        let branches = value["branches"].as_array().unwrap();
        assert_eq!(branches[0]["condition"]["participant"], 2);
        assert_eq!(branches[0]["condition"]["randomBelow"], 1);
        assert!(branches[1]["condition"]["participant"].is_null());
        assert_eq!(branches[1]["condition"]["randomBelow"], 100);
        assert_eq!(branches[1]["steps"][0]["seconds"], 2);
        assert_eq!(value["steps"][0]["seconds"], 3);
    }

    #[test]
    fn what_the_form_does_not_know_is_an_error() {
        for source in [
            "wait_time(1)",
            "while other() do wait_time(1) end",
            "while enable_communication() do change_npc_eye(1, EyePresets.normal) end",
            "while enable_communication() do unknown(1) wait_time(1) end",
            "while enable_communication() do if x then wait_time(1) end wait_time(1) end",
            "while enable_communication() do rand = math.random(1, 9)\n\
             if rand < 1 then wait_time(1) end wait_time(1) end",
        ] {
            assert!(program(source).is_err(), "{source}");
        }
    }
}
