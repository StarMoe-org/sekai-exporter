//! What a character does while it is on its own.
//!
//! [`AloneActions`] produces the `alone-actions` form, specified in
//! `docs/spec/alone-actions.md`.

use std::collections::BTreeMap;

use sekai_convert::tables::{TableContext, TableConverter, TablePlan};
use sekai_convert::{Asset, ConvertError};
use serde::{Deserialize, Serialize};

use crate::constants::{Constants, Literal};
use crate::lua::{self, BinaryOp, Block, Expr, Number, Stat};
use crate::performance::{Performance, Step, Timeline};
use crate::scripts::{
    NoAssets, OneAsset, RANDOM_RANGE, Scripts, document, is_call, malformed, random_below,
    random_draw, rows,
};

const NAME: &str = "mysekai-runtime/alone-actions";
const BUNDLE: &str = "mysekai/character_alone_action";
const TABLE: &str = "mysekaiCharacterTalkSoloActions";
const DOCUMENT_VERSION: u32 = 3;

const LIFETIME: &str = "is_end";
const TIME_WINDOW: &str = "hasTimeElapsed";
const PROBABILITY: &str = "shouldExecuteWithProbability";
const SLOT_MEMORY: &str = "canSelectMotion";
const MEMORY_DURATION: &str = "memoryDuration";
const REMEMBERED: &str = "lastSelectedTimes";
const LOOP_TIME: &str = "nowTime";

/// The number of values a draw of [`RANDOM_RANGE`] can take.
const DRAW_OUTCOMES: f64 = 100.0;

/// The statement that opens the loop of a time-gated script, next to the draw.
const LOOP_TIME_CAPTURE: &str = "local nowTime = os.time()";

/// The definitions the gates of a script evaluate. The document describes the behaviour of
/// exactly these, so a script must hold them as they are.
const GATE_DEFINITIONS: &str = "\
local startTime = os.time()
local lastSelectedTimes = {}
local function hasTimeElapsed(limit)
    if not startTime then
        return false
    end
    local currentTime = os.time()
    local elapsedTime = currentTime - startTime
    return elapsedTime < limit
end
local function shouldExecuteWithProbability(probability)
    local chance = math.random(0, 99)
    return chance < probability * 100
end
local function canSelectMotion(id, nowTime)
    if not lastSelectedTimes[id] then
        return true
    end
    return (nowTime - lastSelectedTimes[id]) > memoryDuration
end
";

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

#[derive(Debug, Serialize)]
struct Unit {
    asset: String,
    scenarios: Vec<Scenario>,
    tail: Tail,
    program: Program,
}

#[derive(Debug, Serialize)]
struct Tail {
    steps: Vec<Step>,
}

#[derive(Debug, Serialize)]
struct Scenario {
    id: String,
    trigger: Trigger,
    steps: Vec<Step>,
    #[serde(skip)]
    remember_slot: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum Trigger {
    TimeGated {
        time_limit_name: String,
        time_limit_seconds: Number,
        probability_name: String,
        probability: Number,
        motion_slot_value: Option<i64>,
        slot_memory_seconds: Number,
    },
    RandomBranch {
        low: i64,
        high: i64,
        weight: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
enum ProgramKind {
    SequentialIfs,
    RandomBranch,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Program {
    kind: ProgramKind,
    clock: &'static str,
    loop_random_min: i64,
    loop_random_max: i64,
    capture_loop_time: bool,
    blocks: Vec<ProgramBlock>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProgramBlock {
    scenario: usize,
    remember_slot: Option<i64>,
}

impl Program {
    /// The program that runs `scenarios` in order.
    fn new(kind: ProgramKind, scenarios: &[Scenario]) -> Self {
        Self {
            kind,
            clock: "wallSeconds",
            loop_random_min: RANDOM_RANGE.min,
            loop_random_max: RANDOM_RANGE.max,
            capture_loop_time: kind == ProgramKind::SequentialIfs,
            blocks: scenarios
                .iter()
                .enumerate()
                .map(|(index, scenario)| ProgramBlock {
                    scenario: index,
                    remember_slot: scenario.remember_slot,
                })
                .collect(),
        }
    }
}

/// What each character does on its own.
#[derive(Debug, Clone, Copy, Default)]
pub struct AloneActions;

impl TableConverter for AloneActions {
    fn name(&self) -> &'static str {
        "alone-actions"
    }

    fn version(&self) -> u32 {
        1
    }

    fn plan<'a>(&self, cx: &'a dyn TableContext) -> Result<Box<dyn TablePlan + 'a>, ConvertError> {
        let Some(scripts) = Scripts::open(cx, BUNDLE) else {
            return Ok(Box::new(NoAssets));
        };
        let table: Vec<Row> = rows(cx, TABLE)?;
        Ok(Box::new(OneAsset::new(NAME, move || {
            build(scripts, &table)
        })))
    }
}

fn build(scripts: Scripts<'_>, table: &[Row]) -> Result<Asset, ConvertError> {
    let constants = scripts.defines()?;
    let library = scripts.library()?;
    let performance = Performance::new(&constants, &library)?;
    let shapes = Shapes::parse()?;
    let mut units = BTreeMap::new();
    for row in table {
        let reader = ScriptReader {
            performance: &performance,
            shapes: &shapes,
            character: row.game_character_unit_id,
        };
        let unit = reader.unit(format!("{}.lua", row.lua), &scripts.parse(&row.lua)?)?;
        let key = row.game_character_unit_id.to_string();
        if units.insert(key, unit).is_some() {
            return Err(malformed(format!(
                "character {} has two programs",
                row.game_character_unit_id
            )));
        }
    }
    document(&Document {
        version: DOCUMENT_VERSION,
        units,
    })
}

/// The statements a script must hold word for word.
struct Shapes {
    loop_time_capture: Block,
    gate_definitions: Block,
}

impl Shapes {
    fn parse() -> Result<Self, ConvertError> {
        Ok(Self {
            loop_time_capture: lua::parse(LOOP_TIME_CAPTURE).map_err(malformed)?,
            gate_definitions: lua::parse(GATE_DEFINITIONS).map_err(malformed)?,
        })
    }
}

/// The condition `is_end(character) == false`.
fn is_lifetime(condition: &Expr) -> bool {
    matches!(condition, Expr::Binary(BinaryOp::Equal, left, right)
        if is_call(left, LIFETIME) && **right == Expr::Bool(false))
}

fn is_definition(statement: &Stat) -> bool {
    matches!(statement, Stat::Local(..) | Stat::Function(..))
}

/// The operands of a chain of `and`, in order.
fn conjuncts(expr: &Expr) -> Vec<&Expr> {
    match expr {
        Expr::Binary(BinaryOp::And, left, right) => {
            let mut operands = conjuncts(left);
            operands.push(right);
            operands
        }
        other => vec![other],
    }
}

/// The arguments of the call `function(...)`, if `expr` is one that passes only names.
fn names_passed<'e>(expr: &'e Expr, function: &str) -> Option<Vec<&'e str>> {
    let Expr::Call(call) = expr else {
        return None;
    };
    if call.name() != Some(function) {
        return None;
    }
    call.arguments
        .iter()
        .map(|argument| match argument {
            Expr::Name(name) => Some(name.as_str()),
            _ => None,
        })
        .collect()
}

/// The constants a gate names:
/// `hasTimeElapsed(limit) and shouldExecuteWithProbability(probability)`, optionally followed by
/// `and canSelectMotion(slot, nowTime)`.
struct Gate<'a> {
    limit: &'a str,
    probability: &'a str,
    slot: Option<&'a str>,
}

impl<'a> Gate<'a> {
    fn read(condition: &'a Expr) -> Option<Self> {
        let operands = conjuncts(condition);
        let (limit, probability, memory) = match operands.as_slice() {
            [limit, probability] => (limit, probability, None),
            [limit, probability, memory] => (limit, probability, Some(memory)),
            _ => return None,
        };
        let [limit] = names_passed(limit, TIME_WINDOW)?[..] else {
            return None;
        };
        let [probability] = names_passed(probability, PROBABILITY)?[..] else {
            return None;
        };
        let slot = match memory {
            Some(memory) => match names_passed(memory, SLOT_MEMORY)?[..] {
                [slot, LOOP_TIME] => Some(slot),
                _ => return None,
            },
            None => None,
        };
        Some(Self {
            limit,
            probability,
            slot,
        })
    }
}

/// The slot of the statement `lastSelectedTimes[slot] = nowTime`.
fn remembered_slot(statement: &Stat) -> Option<&str> {
    let Stat::Assign(targets, values) = statement else {
        return None;
    };
    let ([Expr::Index(table, key)], [Expr::Name(value)]) = (targets.as_slice(), values.as_slice())
    else {
        return None;
    };
    match (table.as_ref(), key.as_ref()) {
        (Expr::Name(table), Expr::Name(slot)) if table == REMEMBERED && value == LOOP_TIME => {
            Some(slot)
        }
        _ => None,
    }
}

fn number(locals: &Constants, name: &str) -> Result<Number, ConvertError> {
    match locals.scalar(name) {
        Some(Literal::Number(value)) => Ok(*value),
        _ => Err(malformed(format!("{name} is not a number"))),
    }
}

fn slot_value(locals: &Constants, slot: &str) -> Result<i64, ConvertError> {
    match number(locals, slot)? {
        Number::Int(value) => Ok(value),
        other => Err(malformed(format!("{other:?} is not a motion slot"))),
    }
}

/// The blocks of a loop and the statements that run after them.
struct Flow<'s> {
    kind: ProgramKind,
    scenarios: Vec<Scenario>,
    tail: &'s [Stat],
}

struct ScriptReader<'a> {
    performance: &'a Performance<'a>,
    shapes: &'a Shapes,
    character: i64,
}

impl ScriptReader<'_> {
    /// The unit of a script that is definitions and one lifetime loop.
    fn unit(&self, asset: String, script: &[Stat]) -> Result<Unit, ConvertError> {
        let [definitions @ .., Stat::While(condition, body)] = script else {
            return Err(malformed("a script does not end with its lifetime loop"));
        };
        if !is_lifetime(condition) || !definitions.iter().all(is_definition) {
            return Err(malformed("a script is not definitions and a lifetime loop"));
        }
        let first_block = body
            .iter()
            .position(|statement| matches!(statement, Stat::If(..)))
            .unwrap_or(body.len());
        let (opening, blocks) = body.split_at(first_block);
        let draws = |statement| random_draw(statement) == Some(RANDOM_RANGE);
        let captures = |statement| self.shapes.loop_time_capture.contains(statement);
        let flow = match opening {
            [draw] if draws(draw) => self.random_branches(blocks)?,
            [draw, capture] | [capture, draw] if draws(draw) && captures(capture) => {
                self.time_gated(definitions, blocks)?
            }
            _ => return Err(malformed("a lifetime loop does not open with its draw")),
        };
        let tail = Tail {
            steps: self.steps(flow.tail)?,
        };
        if !tail.steps.iter().any(Step::waits) {
            return Err(malformed("a lifetime loop has no positive wait"));
        }
        Ok(Unit {
            asset,
            program: Program::new(flow.kind, &flow.scenarios),
            scenarios: flow.scenarios,
            tail,
        })
    }

    /// The `if`s of a loop that tries each gate in turn.
    fn time_gated<'s>(
        &self,
        definitions: &[Stat],
        statements: &'s [Stat],
    ) -> Result<Flow<'s>, ConvertError> {
        let expected = &self.shapes.gate_definitions;
        if !expected.iter().all(|shape| definitions.contains(shape)) {
            return Err(malformed("a script changes the definitions of its gates"));
        }
        let locals = Constants::read(definitions);
        let mut scenarios = Vec::new();
        let mut tail = statements;
        while let [Stat::If(arms, None), rest @ ..] = tail {
            scenarios.push(self.gated_scenario(&locals, scenarios.len(), arms)?);
            tail = rest;
        }
        Ok(Flow {
            kind: ProgramKind::SequentialIfs,
            scenarios,
            tail,
        })
    }

    fn gated_scenario(
        &self,
        locals: &Constants,
        index: usize,
        arms: &[(Expr, Block)],
    ) -> Result<Scenario, ConvertError> {
        let [(condition, body)] = arms else {
            return Err(malformed("a gate has more than one arm"));
        };
        let gate = Gate::read(condition).ok_or_else(|| malformed("an unknown gate"))?;
        let (calls, written) = body
            .split_last()
            .and_then(|(last, calls)| Some((calls, Some(remembered_slot(last)?))))
            .unwrap_or((body.as_slice(), None));
        if written.is_some() && written != gate.slot {
            return Err(malformed("a gate remembers a slot it does not test"));
        }
        let motion_slot_value = gate.slot.map(|slot| slot_value(locals, slot)).transpose()?;
        Ok(Scenario {
            id: gate
                .slot
                .map_or_else(|| format!("gate@{index}"), str::to_owned),
            trigger: Trigger::TimeGated {
                time_limit_name: gate.limit.to_owned(),
                time_limit_seconds: number(locals, gate.limit)?,
                probability_name: gate.probability.to_owned(),
                probability: number(locals, gate.probability)?,
                motion_slot_value,
                slot_memory_seconds: number(locals, MEMORY_DURATION)?,
            },
            steps: self.steps(calls)?,
            remember_slot: motion_slot_value.filter(|_| written.is_some()),
        })
    }

    /// The arms of the one `if` of a loop that picks a branch by its draw.
    fn random_branches<'s>(&self, statements: &'s [Stat]) -> Result<Flow<'s>, ConvertError> {
        let [Stat::If(arms, None), tail @ ..] = statements else {
            return Err(malformed("a random loop has no branches"));
        };
        let mut low = RANDOM_RANGE.min;
        let mut scenarios = Vec::with_capacity(arms.len());
        for (condition, body) in arms {
            let high =
                random_below(condition).ok_or_else(|| malformed("an unknown branch condition"))?;
            scenarios.push(Scenario {
                id: format!("rand<{high}"),
                trigger: Trigger::RandomBranch {
                    low,
                    high,
                    weight: (high - low) as f64 / DRAW_OUTCOMES,
                },
                steps: self.steps(body)?,
                remember_slot: None,
            });
            low = high;
        }
        Ok(Flow {
            kind: ProgramKind::RandomBranch,
            scenarios,
            tail,
        })
    }

    /// The steps of statements that are all calls on the character of the script.
    fn steps(&self, statements: &[Stat]) -> Result<Vec<Step>, ConvertError> {
        let mut timeline = Timeline::default();
        for statement in statements {
            let Stat::Call(call) = statement else {
                return Err(malformed("control flow among the steps of a block"));
            };
            let actions = self
                .performance
                .actions(call)?
                .ok_or_else(|| malformed(format!("an unknown operation {:?}", call.name())))?;
            if self.performance.actor(call)? != self.character {
                return Err(malformed("a script acts on another character"));
            }
            timeline.play(None, actions)?;
        }
        Ok(timeline.into_steps())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::parse;

    const DEFINES: &str = "Characters = { Ichika = 1, Saki = 2 }\n\
                           EyePresets = { normal = \"normal\" }\n";

    fn unit(source: &str) -> Result<serde_json::Value, ConvertError> {
        let constants = Constants::read(&parse(DEFINES).unwrap());
        let library = Constants::read(&parse("local animation_blend_time = 0.5").unwrap());
        let reader = ScriptReader {
            performance: &Performance::new(&constants, &library).unwrap(),
            shapes: &Shapes::parse().unwrap(),
            character: 1,
        };
        let unit = reader.unit("script.lua".to_owned(), &parse(source).unwrap())?;
        Ok(serde_json::to_value(unit).unwrap())
    }

    /// A script of gates with `definitions` before its loop and `body` after the capture.
    fn gated_with(definitions: &str, body: &str) -> String {
        format!(
            "local timeLimit_1 = 5\nlocal probability_1 = 0.25\nlocal MOTION_1 = 7\n\
             local memoryDuration = 15\n{definitions}\n\
             while (is_end(Characters.Ichika) == false) do\nrand = math.random(0, 99)\n\
             local nowTime = os.time()\n{body}\nend"
        )
    }

    fn gated(body: &str) -> String {
        gated_with(GATE_DEFINITIONS, body)
    }

    const REST: &str = "alone_wait_time(Characters.Ichika, 3)";
    const GATE: &str =
        "hasTimeElapsed(timeLimit_1) and shouldExecuteWithProbability(probability_1)";
    const SLOT_GATE: &str = "hasTimeElapsed(timeLimit_1) and \
        shouldExecuteWithProbability(probability_1) and canSelectMotion(MOTION_1, nowTime)";

    fn random(body: &str) -> String {
        format!(
            "while (is_end(Characters.Ichika) == false) do\nrand = math.random(0, 99)\n{body}\nend"
        )
    }

    #[test]
    fn a_gate_names_its_constants_and_their_values() {
        let value = unit(&gated(&format!("if {GATE} then {REST} end\n{REST}"))).unwrap();
        assert_eq!(value["asset"], "script.lua");
        assert_eq!(value["program"]["kind"], "sequentialIfs");
        assert_eq!(value["program"]["captureLoopTime"], true);
        assert_eq!(value["program"]["clock"], "wallSeconds");
        assert_eq!(value["program"]["loopRandomMax"], 99);
        let scenario = &value["scenarios"][0];
        assert_eq!(scenario["id"], "gate@0");
        assert_eq!(
            scenario["trigger"],
            serde_json::json!({
                "kind": "timeGated",
                "timeLimitName": "timeLimit_1",
                "timeLimitSeconds": 5,
                "probabilityName": "probability_1",
                "probability": 0.25,
                "motionSlotValue": null,
                "slotMemorySeconds": 15,
            })
        );
        assert_eq!(scenario["steps"][0]["seconds"], 3);
        assert!(scenario["steps"][0].get("who").is_none());
        assert_eq!(value["tail"]["steps"][0]["seconds"], 3);
        assert!(value["program"]["blocks"][0]["rememberSlot"].is_null());
    }

    #[test]
    fn a_gate_that_writes_its_slot_remembers_it() {
        let body = format!(
            "if {SLOT_GATE} then {REST}\nlastSelectedTimes[MOTION_1] = nowTime end\n{REST}"
        );
        let value = unit(&gated(&body)).unwrap();
        assert_eq!(value["scenarios"][0]["id"], "MOTION_1");
        assert_eq!(value["scenarios"][0]["trigger"]["motionSlotValue"], 7);
        assert_eq!(value["scenarios"][0]["steps"].as_array().unwrap().len(), 1);
        assert_eq!(value["program"]["blocks"][0]["rememberSlot"], 7);
    }

    #[test]
    fn a_gate_may_test_a_slot_without_writing_it() {
        let body = format!("if {SLOT_GATE} then {REST} end\n{REST}");
        let value = unit(&gated(&body)).unwrap();
        assert_eq!(value["scenarios"][0]["trigger"]["motionSlotValue"], 7);
        assert!(value["program"]["blocks"][0]["rememberSlot"].is_null());
    }

    #[test]
    fn random_branches_take_the_range_each_bound_leaves() {
        let value = unit(&random(&format!(
            "if (rand < 20) then {REST} elseif (rand < 100) then {REST} end\n{REST}"
        )))
        .unwrap();
        assert_eq!(value["program"]["kind"], "randomBranch");
        assert_eq!(value["program"]["captureLoopTime"], false);
        let triggers: Vec<_> = (0..2).map(|i| &value["scenarios"][i]["trigger"]).collect();
        assert_eq!(triggers[0]["low"], 0);
        assert_eq!(triggers[0]["high"], 20);
        assert_eq!(triggers[0]["weight"], 0.2);
        assert_eq!(triggers[1]["low"], 20);
        assert_eq!(triggers[1]["weight"], 0.8);
        assert_eq!(value["scenarios"][1]["id"], "rand<100");
        assert_eq!(value["program"]["blocks"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn a_loop_may_capture_the_time_before_its_draw() {
        let definitions = gated_with(GATE_DEFINITIONS, "");
        let (definitions, _) = definitions.split_once("while").unwrap();
        let source = format!(
            "{definitions}while (is_end(Characters.Ichika) == false) do\n\
             local nowTime = os.time()\nlocal rand = math.random(0, 99)\n\
             if {GATE} then {REST} end\n{REST}\nend"
        );
        assert_eq!(unit(&source).unwrap()["program"]["kind"], "sequentialIfs");
    }

    #[test]
    fn what_the_form_does_not_know_is_an_error() {
        let wrong_memory = format!(
            "if {SLOT_GATE} then {REST}\nlastSelectedTimes[timeLimit_1] = nowTime end\n{REST}"
        );
        let other_slot_argument =
            format!("if {GATE} and canSelectMotion(MOTION_1, 1) then {REST} end\n{REST}");
        let one_gate = format!("if {GATE} then {REST} end\n{REST}");
        let two_arms = format!("if {GATE} then {REST} elseif {GATE} then {REST} end\n{REST}");
        let scripts = [
            REST.to_owned(),
            gated(&format!("if {GATE} then {REST} end")),
            gated(&format!("if rand < 5 then {REST} end\n{REST}")),
            gated(&format!(
                "if {GATE} then for i = 1, 2 do {REST} end end\n{REST}"
            )),
            gated(&format!("if {GATE} then {REST} end\nunknown(1)\n{REST}")),
            gated(&format!(
                "if {GATE} then {REST} end\nif {GATE} then {REST} else {REST} end"
            )),
            gated(&format!(
                "if {GATE} then alone_wait_time(Characters.Saki, 3) end\n{REST}"
            )),
            gated(&wrong_memory),
            gated(&other_slot_argument),
            gated(&two_arms),
            gated_with("", &one_gate),
            gated_with(
                &GATE_DEFINITIONS.replace("elapsedTime < limit", "elapsedTime > limit"),
                &one_gate,
            ),
            gated(REST),
            random(&format!("if (rand < n) then {REST} end\n{REST}")),
            random(REST),
            format!(
                "{REST}\n{}",
                random(&format!("if (rand < 9) then {REST} end\n{REST}"))
            ),
            format!(
                "while enable() do rand = math.random(0, 99)\n\
                 if (rand < 9) then {REST} end\n{REST} end"
            ),
            format!(
                "while (is_end(Characters.Ichika) == false) do rand = math.random(1, 9)\n\
                 if (rand < 9) then {REST} end\n{REST} end"
            ),
        ];
        for source in scripts {
            assert!(unit(&source).is_err(), "{source}");
        }
    }
}
