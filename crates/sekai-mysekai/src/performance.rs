//! The channel writes of performance scripts: motion, eyes, mouth, emoticons and waits.

use sekai_convert::ConvertError;
use serde::Serialize;

use crate::constants::{Argument, Constants, Literal};
use crate::lua::{Call, Number};
use crate::scripts::malformed;

const BLEND_TIME: &str = "animation_blend_time";

/// The segment of a motion that the numbers of `MotionType` select.
const PHASES: [&str; 4] = ["S", "L", "E", "O"];

/// One write to a channel.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub(crate) enum Action {
    Eye {
        pattern: Literal,
        alias: Option<String>,
    },
    Mouth {
        pattern: Literal,
        alias: Option<String>,
    },
    Animation {
        motion: Literal,
        alias: Option<String>,
        speed: Literal,
        playback_speed: f64,
        play_end_motion: bool,
        blend: Literal,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase: Option<&'static str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        phase_source: Option<String>,
    },
    Emoticon {
        name: Literal,
        time: Literal,
        not_play_se: Literal,
        host_not_play_se: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        show_seconds: Option<Literal>,
    },
    HideEmoticon,
    Wait {
        seconds: Literal,
    },
}

/// A write at its nominal time: the sum of the waits before it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct Step {
    t: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    who: Option<i64>,
    #[serde(flatten)]
    action: Action,
}

impl Step {
    /// Whether the step waits for a positive time.
    pub(crate) fn waits(&self) -> bool {
        matches!(&self.action, Action::Wait { seconds: Literal::Number(n) } if n.value() > 0.0)
    }
}

/// The steps of one sequence of calls.
#[derive(Debug, Default)]
pub(crate) struct Timeline {
    elapsed: f64,
    steps: Vec<Step>,
}

impl Timeline {
    /// Appends `actions` at the current time. A wait moves the time on and has no actor.
    pub(crate) fn play(
        &mut self,
        who: Option<i64>,
        actions: Vec<Action>,
    ) -> Result<(), ConvertError> {
        for action in actions {
            let seconds = match &action {
                Action::Wait { seconds } => Some(wait_seconds(seconds)?),
                _ => None,
            };
            self.steps.push(Step {
                t: self.elapsed,
                who: who.filter(|_| seconds.is_none()),
                action,
            });
            self.elapsed += seconds.unwrap_or(0.0);
        }
        Ok(())
    }

    pub(crate) fn into_steps(self) -> Vec<Step> {
        self.steps
    }
}

fn wait_seconds(seconds: &Literal) -> Result<f64, ConvertError> {
    match seconds {
        Literal::Number(number) if number.value() >= 0.0 => Ok(number.value()),
        other => Err(malformed(format!("{other:?} is not a time to wait"))),
    }
}

/// The arguments of one call, with their values.
struct Arguments(Vec<Argument>);

impl Arguments {
    fn required(&self, index: usize) -> Result<&Argument, ConvertError> {
        self.0
            .get(index)
            .ok_or_else(|| malformed(format!("a call has no argument {index}")))
    }

    /// The value at `index`, nil when the script leaves it out.
    fn value(&self, index: usize) -> Literal {
        self.0
            .get(index)
            .map_or(Literal::Nil, |argument| argument.value.clone())
    }
}

/// Reads the channel writes of the scripts of one package.
#[derive(Debug)]
pub(crate) struct Performance<'a> {
    constants: &'a Constants,
    default_blend: Literal,
}

impl<'a> Performance<'a> {
    /// Reads scripts that name `constants`, with the defaults of `library`.
    pub(crate) fn new(constants: &'a Constants, library: &Constants) -> Result<Self, ConvertError> {
        let default_blend = library
            .scalar(BLEND_TIME)
            .ok_or_else(|| malformed(format!("the library does not define {BLEND_TIME}")))?
            .clone();
        Ok(Self {
            constants,
            default_blend,
        })
    }

    /// The value of the first argument of `call`, the character it acts on.
    pub(crate) fn actor(&self, call: &Call) -> Result<i64, ConvertError> {
        match self.arguments(call)?.required(0)?.value {
            Literal::Number(Number::Int(unit)) => Ok(unit),
            ref other => Err(malformed(format!("{other:?} is not a character"))),
        }
    }

    fn arguments(&self, call: &Call) -> Result<Arguments, ConvertError> {
        call.arguments
            .iter()
            .map(|argument| self.constants.resolve(argument).map_err(malformed))
            .collect::<Result<_, _>>()
            .map(Arguments)
    }

    /// The writes that `call` makes, or `None` if it writes to no channel.
    pub(crate) fn actions(&self, call: &Call) -> Result<Option<Vec<Action>>, ConvertError> {
        let Some(name) = call.name() else {
            return Ok(None);
        };
        let arguments = self.arguments(call);
        let action = match name {
            "change_npc_eye" => {
                let (pattern, alias) = named(&arguments?, 1)?;
                Action::Eye { pattern, alias }
            }
            "change_npc_mouth" => {
                let (pattern, alias) = named(&arguments?, 1)?;
                Action::Mouth { pattern, alias }
            }
            "change_animation" => self.animation(&arguments?)?,
            "emoticon" => return emoticon(&arguments?).map(Some),
            "hide_emoticon" => Action::HideEmoticon,
            "wait_time" => Action::Wait {
                seconds: arguments?.required(0)?.value.clone(),
            },
            "alone_wait_time" => Action::Wait {
                seconds: arguments?.required(1)?.value.clone(),
            },
            _ => return Ok(None),
        };
        Ok(Some(vec![action]))
    }

    /// `change_animation(who, motion, speed, delay, play_end_motion, blend, phase)`. The game
    /// never reads `delay`, and plays a speed of zero at the normal rate.
    fn animation(&self, arguments: &Arguments) -> Result<Action, ConvertError> {
        let (motion, alias) = named(arguments, 1)?;
        let speed = arguments.value(2);
        let playback_speed = match &speed {
            Literal::Number(number) if number.value() != 0.0 => number.value(),
            _ => 1.0,
        };
        let blend = match arguments.value(5) {
            Literal::Nil => self.default_blend.clone(),
            blend => blend,
        };
        let phase = arguments.0.get(6).filter(|a| a.value != Literal::Nil);
        Ok(Action::Animation {
            motion,
            alias,
            speed,
            playback_speed,
            play_end_motion: arguments.value(4).is_truthy(),
            blend,
            phase: phase.map(phase_suffix).transpose()?,
            phase_source: phase.and_then(|phase| phase.alias.clone()),
        })
    }
}

fn named(arguments: &Arguments, index: usize) -> Result<(Literal, Option<String>), ConvertError> {
    let argument = arguments.required(index)?;
    Ok((argument.value.clone(), argument.alias.clone()))
}

fn phase_suffix(phase: &Argument) -> Result<&'static str, ConvertError> {
    let index = match phase.value {
        Literal::Number(Number::Int(index)) => usize::try_from(index).ok(),
        _ => None,
    };
    index
        .and_then(|index| PHASES.get(index).copied())
        .ok_or_else(|| malformed(format!("{:?} is not a motion phase", phase.value)))
}

/// `emoticon(who, name, time, show_seconds, not_play_se)`. The game takes the truth of `time` as
/// its choice to mute the emoticon, and a shown time hides the emoticon at once.
fn emoticon(arguments: &Arguments) -> Result<Vec<Action>, ConvertError> {
    let show_seconds = arguments.0.get(3).map(|argument| argument.value.clone());
    let hides = show_seconds
        .as_ref()
        .is_some_and(|seconds| *seconds != Literal::Nil);
    let time = arguments.value(2);
    let mut actions = vec![Action::Emoticon {
        name: arguments.required(1)?.value.clone(),
        host_not_play_se: time.is_truthy(),
        time,
        not_play_se: arguments.value(4),
        show_seconds,
    }];
    if hides {
        actions.push(Action::HideEmoticon);
    }
    Ok(actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lua::{Stat, parse};

    fn constants() -> Constants {
        Constants::read(
            &parse(
                "MotionType = { _s = 0, _l = 1 }\nL = MotionType._l\nCharacters = { Ichika = 1 }\n\
                 Motions = { nod = \"mov_nod\" }\nEyePresets = { normal = \"normal\" }\n",
            )
            .unwrap(),
        )
    }

    fn library() -> Constants {
        Constants::read(&parse("local animation_blend_time = 0.5").unwrap())
    }

    fn actions(source: &str) -> Vec<Action> {
        let constants = constants();
        let performance = Performance::new(&constants, &library()).unwrap();
        let block = parse(source).unwrap();
        let [Stat::Call(call)] = block.as_slice() else {
            panic!("{block:?}");
        };
        performance.actions(call).unwrap().unwrap()
    }

    fn json(action: &Action) -> serde_json::Value {
        serde_json::to_value(action).unwrap()
    }

    #[test]
    fn a_speed_of_zero_plays_at_the_normal_rate() {
        let value = json(&actions("change_animation(Characters.Ichika, Motions.nod, 0)")[0]);
        assert_eq!(value["op"], "animation");
        assert_eq!(value["motion"], "mov_nod");
        assert_eq!(value["alias"], "Motions.nod");
        assert_eq!(value["speed"], 0);
        assert_eq!(value["playbackSpeed"], 1.0);
        assert_eq!(value["playEndMotion"], false);
        assert_eq!(value["blend"], 0.5);
        assert!(value.get("phase").is_none());
    }

    #[test]
    fn a_left_out_speed_is_null() {
        let value = json(&actions("change_animation(1, Motions.nod)")[0]);
        assert!(value["speed"].is_null());
        assert_eq!(value["playbackSpeed"], 1.0);
    }

    #[test]
    fn a_phase_names_its_segment_and_constant() {
        let value = json(&actions("change_animation(1, Motions.nod, 2, 0, true, 0.25, L)")[0]);
        assert_eq!(value["playbackSpeed"], 2.0);
        assert_eq!(value["playEndMotion"], true);
        assert_eq!(value["blend"], 0.25);
        assert_eq!(value["phase"], "L");
        assert_eq!(value["phaseSource"], "L");
    }

    #[test]
    fn a_shown_time_hides_the_emoticon_at_once() {
        let shown = actions("emoticon(1, \"fx\", 0, 5, false)");
        assert_eq!(shown.len(), 2);
        assert_eq!(shown[1], Action::HideEmoticon);
        let value = json(&shown[0]);
        assert_eq!(value["hostNotPlaySe"], true);
        assert_eq!(value["showSeconds"], 5);
        assert_eq!(actions("emoticon(1, \"fx\")").len(), 1);
    }

    #[test]
    fn a_wait_moves_the_time_and_has_no_actor() {
        let mut timeline = Timeline::default();
        timeline
            .play(Some(1), actions("change_npc_eye(1, EyePresets.normal)"))
            .unwrap();
        timeline.play(Some(1), actions("wait_time(1.5)")).unwrap();
        timeline
            .play(Some(1), actions("change_npc_eye(1, EyePresets.normal)"))
            .unwrap();
        let steps = timeline.into_steps();
        let times: Vec<f64> = steps.iter().map(|step| step.t).collect();
        assert_eq!(times, [0.0, 0.0, 1.5]);
        assert_eq!(steps[1].who, None);
        assert_eq!(steps[2].who, Some(1));
        assert!(steps[1].waits());
    }

    #[test]
    fn a_wait_needs_a_time() {
        let mut timeline = Timeline::default();
        assert!(timeline.play(None, actions("wait_time(-1)")).is_err());
        assert!(timeline.play(None, actions("wait_time(someone)")).is_err());
    }

    #[test]
    fn other_calls_write_to_no_channel() {
        let constants = constants();
        let performance = Performance::new(&constants, &library()).unwrap();
        let block = parse("look_at_body(1)").unwrap();
        let [Stat::Call(call)] = block.as_slice() else {
            panic!("{block:?}");
        };
        assert_eq!(performance.actions(call).unwrap(), None);
    }
}
