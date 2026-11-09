//! The scripts of the MySekai talks, read as steps. The forms built from the talk package share
//! this reader, so that a call means the same step in every one of them.

use std::collections::BTreeMap;

use sekai_convert::ConvertError;
use sekai_convert::tables::TableContext;
use serde::Serialize;

use crate::constants::{Argument, ConstantError, Constants, Literal};
use crate::lua::{Expr, Stat};
use crate::scripts::{Scripts, malformed};

const TALK_BUNDLE: &str = "mysekai/talk/scenario/talk";
const LIBRARY_BUNDLE: &str = "mysekai/talk/scenario/lib";

/// How one argument of a call fills the step that the call makes.
#[derive(Clone, Copy)]
enum Slot {
    /// Written when the script passes the argument.
    Passed(&'static str),
    /// Always written, null when the script leaves the argument out or passes nil.
    Nullable(&'static str),
    /// Written when the script passes anything but nil.
    Given(&'static str),
    /// Written as the truth of the argument when the script passes anything but nil.
    Truth(&'static str),
    /// The key `alias`: the constant that named the argument, or its value when none did.
    Alias,
}

use Slot::{Alias, Given, Nullable, Passed, Truth};

/// The slots of the arguments of one operation, by the place of the argument.
type Layout = &'static [(usize, Slot)];

const NPC_FACE: Layout = &[
    (0, Passed("who")),
    (1, Passed("pattern")),
    (1, Alias),
    (2, Nullable("delaySeconds")),
];
const FIXTURE_FACE: Layout = &[
    (0, Passed("fixture")),
    (1, Passed("pattern")),
    (1, Alias),
    (2, Nullable("delaySeconds")),
];
const WAIT: Layout = &[(0, Passed("seconds")), (1, Passed("auto"))];
const FIXTURE_NAMED: Layout = &[
    (0, Passed("fixture")),
    (1, Passed("name")),
    (2, Passed("value")),
];
const NONE: Layout = &[];

/// The vocabulary of the talk scripts.
const OPERATIONS: &[(&str, Layout)] = &[
    ("label", &[(0, Passed("name"))]),
    ("text", &[(0, Passed("text"))]),
    ("wait_click", NONE),
    ("show_talk_window", NONE),
    ("hide_talk_window", NONE),
    ("wait_time", WAIT),
    ("wait_time_on_auto_mode", WAIT),
    ("change_npc_eye", NPC_FACE),
    ("change_npc_mouth", NPC_FACE),
    ("change_fixture_character_eye", FIXTURE_FACE),
    ("change_fixture_character_mouth", FIXTURE_FACE),
    (
        "change_animation",
        &[
            (0, Passed("who")),
            (1, Passed("motion")),
            (1, Alias),
            (2, Nullable("delay")),
            (3, Nullable("animationSpeed")),
            (4, Truth("playEndMotion")),
            (5, Given("blend")),
            (6, Given("phase")),
        ],
    ),
    (
        "play_animation",
        &[
            (0, Passed("who")),
            (1, Passed("motion")),
            (1, Alias),
            (2, Nullable("delay")),
            (3, Nullable("animationSpeed")),
            (4, Given("blend")),
        ],
    ),
    (
        "voice",
        &[
            (0, Passed("channel")),
            (1, Passed("cue")),
            (2, Passed("who")),
        ],
    ),
    (
        "fixture_voice",
        &[(0, Passed("cue")), (1, Passed("fixture"))],
    ),
    (
        "look_at_body",
        &[
            (0, Passed("who")),
            (1, Passed("target")),
            (2, Passed("duration")),
        ],
    ),
    (
        "look_at_to_npc",
        &[
            (0, Passed("fixture")),
            (1, Passed("who")),
            (2, Passed("duration")),
        ],
    ),
    (
        "look_at_fixture",
        &[(0, Passed("fixture")), (1, Passed("who"))],
    ),
    (
        "emoticon",
        &[
            (0, Passed("who")),
            (1, Passed("name")),
            (1, Alias),
            (3, Passed("showSeconds")),
        ],
    ),
    (
        "show_fixture_emoticon",
        &[
            (0, Passed("fixture")),
            (1, Passed("name")),
            (1, Alias),
            (3, Passed("showSeconds")),
        ],
    ),
    ("hide_emoticon", &[(0, Passed("who"))]),
    ("change_fixture_timeline", FIXTURE_NAMED),
    ("play_fixture_gimmick", FIXTURE_NAMED),
    ("stop_fixture_gimmick", FIXTURE_NAMED),
];

/// One call of a talk script.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct TalkStep {
    op: &'static str,
    #[serde(flatten)]
    fields: BTreeMap<&'static str, Literal>,
}

impl TalkStep {
    fn new(op: &'static str, layout: Layout, arguments: &[Argument]) -> Self {
        let mut fields = BTreeMap::new();
        for (index, slot) in layout {
            let passed = arguments.get(*index);
            let given = passed.filter(|argument| argument.value != Literal::Nil);
            let value = |argument: &Argument| argument.value.clone();
            fields.extend(match *slot {
                Passed(key) => passed.map(|argument| (key, value(argument))),
                Nullable(key) => Some((key, given.map_or(Literal::Nil, value))),
                Given(key) => given.map(|argument| (key, value(argument))),
                Truth(key) => {
                    given.map(|argument| (key, Literal::Bool(argument.value.is_truthy())))
                }
                Alias => passed.map(|argument| ("alias", alias(argument))),
            });
        }
        Self { op, fields }
    }

    /// The voice cue that the step plays.
    fn cue(&self) -> Option<String> {
        match self.fields.get("cue") {
            Some(Literal::Text(cue)) => Some(cue.clone()),
            _ => None,
        }
    }
}

fn alias(argument: &Argument) -> Literal {
    argument
        .alias
        .clone()
        .map_or_else(|| argument.value.clone(), Literal::Text)
}

/// The voice cues and the steps of one script.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct TalkScript {
    voices: Vec<String>,
    steps: Vec<TalkStep>,
}

/// Reads the scripts of the talk package with the constants of its library.
pub(crate) struct TalkReader<'a> {
    scripts: Scripts<'a>,
    constants: Constants,
}

impl<'a> TalkReader<'a> {
    /// The reader of the version, if the version holds both packages.
    pub(crate) fn open(cx: &'a dyn TableContext) -> Result<Option<Self>, ConvertError> {
        let (Some(scripts), Some(library)) = (
            Scripts::open(cx, TALK_BUNDLE),
            Scripts::open(cx, LIBRARY_BUNDLE),
        ) else {
            return Ok(None);
        };
        Ok(Some(Self {
            scripts,
            constants: library.defines()?,
        }))
    }

    /// The script `lua`, given without its extension.
    pub(crate) fn read(&self, lua: &str) -> Result<TalkScript, ConvertError> {
        let steps = self
            .scripts
            .parse(lua)?
            .iter()
            .map(|statement| step(&self.constants, statement))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| malformed(format!("{lua}: {e}")))?;
        Ok(TalkScript {
            voices: steps.iter().filter_map(TalkStep::cue).collect(),
            steps,
        })
    }
}

/// The step of a statement that calls an operation.
fn step(constants: &Constants, statement: &Stat) -> Result<TalkStep, ConvertError> {
    let Stat::Call(call) = statement else {
        return Err(malformed("a statement that is not a call"));
    };
    let (op, layout) = OPERATIONS
        .iter()
        .find(|(op, _)| call.name() == Some(*op))
        .ok_or_else(|| malformed(format!("an unknown operation {:?}", call.name())))?;
    let arguments = call
        .arguments
        .iter()
        .map(|argument| resolved(constants, argument))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(TalkStep::new(op, layout, &arguments))
}

/// The value of an argument. A key that its table does not define stays the token the script
/// wrote: scripts ship with such keys, and a reader has to be able to see them.
fn resolved(constants: &Constants, expr: &Expr) -> Result<Argument, ConvertError> {
    match constants.resolve(expr) {
        Err(ConstantError::UndefinedKey(token)) => Ok(Argument {
            value: Literal::Text(token.clone()),
            alias: Some(token),
        }),
        other => other.map_err(malformed),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::parse;
    use serde_json::{Value, json};

    fn reader_step(source: &str) -> Value {
        let constants = Constants::read(
            &parse("EyePresets = { normal = \"open\" }\nMotions = { nod = \"mov_nod\" }").unwrap(),
        );
        let block = parse(source).unwrap();
        serde_json::to_value(step(&constants, &block[0]).unwrap()).unwrap()
    }

    #[test]
    fn a_face_names_its_pattern_and_keeps_a_nil_delay() {
        assert_eq!(
            reader_step("change_npc_eye(1, EyePresets.normal, nil)"),
            json!({
                "op": "change_npc_eye", "who": 1, "pattern": "open",
                "alias": "EyePresets.normal", "delaySeconds": null,
            })
        );
    }

    #[test]
    fn a_key_that_is_not_defined_stays_the_token() {
        let step = reader_step("change_npc_eye(1, EyePresets.noraml)");
        assert_eq!(step["pattern"], "EyePresets.noraml");
        assert_eq!(step["alias"], "EyePresets.noraml");
    }

    #[test]
    fn an_animation_writes_only_what_is_given() {
        assert_eq!(
            reader_step("change_animation(1, Motions.nod, 0)"),
            json!({
                "op": "change_animation", "who": 1, "motion": "mov_nod",
                "alias": "Motions.nod", "delay": 0, "animationSpeed": null,
            })
        );
        let full = reader_step("change_animation(1, Motions.nod, 0, 2, 0, 0.3, 1)");
        assert_eq!(full["animationSpeed"], 2);
        assert_eq!(full["playEndMotion"], true);
        assert_eq!(full["blend"], 0.3);
        assert_eq!(full["phase"], 1);
        let nil = reader_step("change_animation(1, Motions.nod, 0, 2, nil, nil, nil)");
        assert!(nil.get("playEndMotion").is_none());
        assert!(nil.get("blend").is_none());
        assert_eq!(
            reader_step("change_animation(1, Motions.nod, 0, 1, false)")["playEndMotion"],
            false
        );
    }

    #[test]
    fn a_literal_stands_for_its_own_alias() {
        let step = reader_step("play_animation(0, \"mov_pick\", 0, 1, 0.25)");
        assert_eq!(step["motion"], "mov_pick");
        assert_eq!(step["alias"], "mov_pick");
        assert_eq!(step["blend"], 0.25);
    }

    #[test]
    fn an_emoticon_skips_its_time() {
        assert_eq!(
            reader_step("emoticon(1, \"fx\", 9, 5)"),
            json!({"op": "emoticon", "who": 1, "name": "fx", "alias": "fx", "showSeconds": 5})
        );
    }

    #[test]
    fn an_operation_without_arguments_is_only_its_name() {
        assert_eq!(reader_step("wait_click()"), json!({"op": "wait_click"}));
    }
}
