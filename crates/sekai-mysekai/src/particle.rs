//! Particle systems and their renderers as the documents a runtime simulates them from.
//!
//! Every value of a particle system is stored in a shape that depends on a state flag, such as a
//! constant, a random range or a pair of curves. This module writes each one as an explicit JSON
//! object, so that a runtime need not know which stored field is live in which state. The layouts
//! below say which stored field becomes which key.

use sekai_convert::{ConvertError, PPtr, Value};
use serde_json::{Value as Json, json};

use crate::scripts::malformed;
use crate::stored::{
    Code, Codes, Color, Curve, Flag, Kind, Layout, List, Object, Vector, XY, XYZ, array, field,
    fields, is_set, read,
};

const SIMULATION_SPACES: Codes = &[(0, "Local"), (1, "World"), (2, "Custom")];
const SHAPE_TYPES: Codes = &[
    (0, "Sphere"),
    (1, "SphereShell"),
    (2, "Hemisphere"),
    (3, "HemisphereShell"),
    (4, "Cone"),
    (5, "Box"),
    (6, "Mesh"),
    (7, "ConeShell"),
    (8, "ConeVolume"),
    (9, "ConeVolumeShell"),
    (10, "Circle"),
    (11, "CircleEdge"),
    (12, "SingleSidedEdge"),
    (15, "BoxShell"),
    (16, "BoxEdge"),
    (17, "Donut"),
    (18, "Rectangle"),
    (19, "Sprite"),
];
const RENDER_MODES: Codes = &[
    (0, "Billboard"),
    (1, "Stretch"),
    (2, "HorizontalBillboard"),
    (3, "VerticalBillboard"),
    (4, "Mesh"),
    (5, "None"),
];
const SUB_EMITTER_TYPES: Codes = &[
    (0, "birth"),
    (1, "collision"),
    (2, "death"),
    (3, "trigger"),
    (4, "manual"),
];

const SYSTEM: Layout = &[
    ("duration", "lengthInSec", Kind::Number),
    ("looping", "looping", Flag),
    ("simulationSpeed", "simulationSpeed", Kind::Number),
    (
        "simulationSpace",
        "moveWithTransform",
        Code(SIMULATION_SPACES),
    ),
    ("scalingMode", "scalingMode", Kind::Number),
    (
        "maxParticles",
        "InitialModule.maxNumParticles",
        Kind::Number,
    ),
];

const START: Module = Module {
    key: "start",
    source: "InitialModule",
    fields: &[
        ("lifetime", "startLifetime", Curve),
        ("speed", "startSpeed", Curve),
        ("size", "startSize", Curve),
        ("rotation", "startRotation", Curve),
        ("color", "startColor", Color),
        ("gravityModifier", "gravityModifier", Curve),
        ("size3D", "size3D", Flag),
        ("rotation3D", "rotation3D", Flag),
    ],
    gated: &[
        (
            "size3D",
            &[
                ("sizeY", "startSizeY", Curve),
                ("sizeZ", "startSizeZ", Curve),
            ],
        ),
        (
            "rotation3D",
            &[
                ("rotationX", "startRotationX", Curve),
                ("rotationY", "startRotationY", Curve),
            ],
        ),
    ],
};

const BURST: Layout = &[
    ("time", "time", Kind::Number),
    ("count", "countCurve", Curve),
    ("cycleCount", "cycleCount", Kind::Number),
    ("repeatInterval", "repeatInterval", Kind::Number),
    ("probability", "probability", Kind::Number),
];

/// The modules written only when they are enabled.
const MODULES: &[Module] = &[
    Module {
        key: "emission",
        source: "EmissionModule",
        fields: &[
            ("rateOverTime", "rateOverTime", Curve),
            ("bursts", "m_Bursts", List(BURST)),
        ],
        gated: &[],
    },
    Module {
        key: "shape",
        source: "ShapeModule",
        fields: &[
            ("type", "type", Code(SHAPE_TYPES)),
            ("radius", "radius.value", Kind::Number),
            ("radiusThickness", "radiusThickness", Kind::Number),
            ("arc", "arc.value", Kind::Number),
            ("angle", "angle", Kind::Number),
            ("position", "m_Position", Vector(XYZ)),
            ("rotation", "m_Rotation", Vector(XYZ)),
            ("scale", "m_Scale", Vector(XYZ)),
        ],
        gated: &[],
    },
    Module {
        key: "sizeOverLifetime",
        source: "SizeModule",
        fields: &[
            ("separateAxes", "separateAxes", Flag),
            ("curve", "curve", Curve),
        ],
        gated: &[("separateAxes", &[("y", "y", Curve), ("z", "z", Curve)])],
    },
    Module {
        key: "rotationOverLifetime",
        source: "RotationModule",
        fields: &[
            ("separateAxes", "separateAxes", Flag),
            ("curve", "curve", Curve),
        ],
        gated: &[("separateAxes", &[("x", "x", Curve), ("y", "y", Curve)])],
    },
    Module {
        key: "velocityOverLifetime",
        source: "VelocityModule",
        fields: &[
            ("x", "x", Curve),
            ("y", "y", Curve),
            ("z", "z", Curve),
            ("speedModifier", "speedModifier", Curve),
        ],
        gated: &[],
    },
    Module {
        key: "limitVelocity",
        source: "ClampVelocityModule",
        fields: &[
            ("separateAxis", "separateAxis", Flag),
            ("magnitude", "magnitude", Curve),
            ("dampen", "dampen", Kind::Number),
            ("drag", "drag", Curve),
            ("multiplyDragBySize", "multiplyDragByParticleSize", Flag),
            (
                "multiplyDragByVelocity",
                "multiplyDragByParticleVelocity",
                Flag,
            ),
        ],
        gated: &[],
    },
    Module {
        key: "textureSheet",
        source: "UVModule",
        fields: &[
            ("mode", "mode", Kind::Number),
            ("tilesX", "tilesX", Kind::Number),
            ("tilesY", "tilesY", Kind::Number),
            ("animationType", "animationType", Kind::Number),
            ("timeMode", "timeMode", Kind::Number),
            ("fps", "fps", Kind::Number),
            ("cycles", "cycles", Kind::Number),
            ("rowIndex", "rowIndex", Kind::Number),
            ("rowMode", "rowMode", Kind::Number),
            ("startFrame", "startFrame", Curve),
            ("frameOverTime", "frameOverTime", Curve),
            ("speedRange", "speedRange", Vector(XY)),
            ("uvChannelMask", "uvChannelMask", Kind::Number),
            ("flipU", "flipU", Kind::Number),
            ("flipV", "flipV", Kind::Number),
        ],
        gated: &[],
    },
];

const COLOR_MODULE: &str = "ColorModule";
const SUB_MODULE: &str = "SubModule";

/// The bits of the properties of a sub-emitter that say what it inherits.
const SUB_EMITTER_INHERIT: &[(&str, u64)] = &[
    ("color", 1),
    ("size", 2),
    ("rotation", 4),
    ("lifetime", 8),
    ("duration", 16),
];

const NOT_MODELLED: &str = "particle module not modelled";
const UNRESOLVED_SUB_EMITTER: &str = "sub-emitter is not in this package";

const RENDERER: Layout = &[
    ("enabled", "m_Enabled", Flag),
    ("renderMode", "m_RenderMode", Code(RENDER_MODES)),
    ("normalDirection", "m_NormalDirection", Kind::Number),
    ("flip", "m_Flip", Vector(XYZ)),
    ("pivot", "m_Pivot", Vector(XYZ)),
    ("allowRoll", "m_AllowRoll", Flag),
    ("alignment", "m_RenderAlignment", Kind::Number),
    ("minParticleSize", "m_MinParticleSize", Kind::Number),
    ("maxParticleSize", "m_MaxParticleSize", Kind::Number),
    ("lengthScale", "m_LengthScale", Kind::Number),
    ("velocityScale", "m_VelocityScale", Kind::Number),
    ("cameraVelocityScale", "m_CameraVelocityScale", Kind::Number),
    ("freeformStretching", "m_FreeformStretching", Flag),
];

/// A section of a particle system.
struct Module {
    key: &'static str,
    source: &'static str,
    fields: Layout,
    /// Fields that are live only when the named flag of the section is set.
    gated: &'static [(&'static str, Layout)],
}

impl Module {
    fn read(&self, section: &Value) -> Result<Object, ConvertError> {
        let mut out = fields(section, self.fields)?;
        for (flag_name, layout) in self.gated {
            if is_set(section, flag_name)? {
                out.extend(fields(section, layout)?);
            }
        }
        Ok(out)
    }
}

/// A particle system as a document.
pub(crate) struct System {
    /// The parameters of the emitter.
    pub(crate) document: Json,
    /// What the document leaves out: an object with `module` and `reason` for each.
    pub(crate) unsupported: Vec<Json>,
}

/// Gives the path of the prefab node that a reference names, if it is in the same package.
pub(crate) type NodePath<'a> = &'a dyn Fn(PPtr) -> Result<Option<String>, ConvertError>;

/// Reads the `ParticleSystem` `stored`.
pub(crate) fn system(stored: &Value, node_path: NodePath<'_>) -> Result<System, ConvertError> {
    let mut document = fields(stored, SYSTEM)?;
    let initial = field(stored, START.source)?;
    document.insert(START.key.to_owned(), Json::Object(START.read(initial)?));
    for module in MODULES {
        let section = field(stored, module.source)?;
        if is_set(section, "enabled")? {
            document.insert(module.key.to_owned(), Json::Object(module.read(section)?));
        }
    }
    let color = field(stored, COLOR_MODULE)?;
    if is_set(color, "enabled")? {
        let gradient = read(field(color, "gradient")?, Color, "gradient")?;
        document.insert("colorOverLifetime".to_owned(), gradient);
    }
    let mut unsupported = not_modelled(stored)?;
    let sub = field(stored, SUB_MODULE)?;
    if is_set(sub, "enabled")? {
        let emitters = sub_emitters(sub, node_path, &mut unsupported)?;
        document.insert("subEmitters".to_owned(), Json::Array(emitters));
    }
    Ok(System {
        document: Json::Object(document),
        unsupported,
    })
}

/// Reads the draw settings of the `ParticleSystemRenderer` `stored`.
pub(crate) fn renderer(stored: &Value) -> Result<Object, ConvertError> {
    fields(stored, RENDERER)
}

/// A record of each enabled module this form does not write, in name order.
fn not_modelled(stored: &Value) -> Result<Vec<Json>, ConvertError> {
    let Value::Struct(sections) = stored else {
        return Err(malformed("the particle system is not a structure"));
    };
    let mut names = Vec::new();
    for (name, section) in sections {
        let modelled = name == START.source
            || name == COLOR_MODULE
            || name == SUB_MODULE
            || MODULES.iter().any(|module| module.source == name);
        if !modelled && section.get("enabled").is_some() && is_set(section, "enabled")? {
            names.push(name.as_str());
        }
    }
    names.sort_unstable();
    Ok(names
        .into_iter()
        .map(|name| gap(name, NOT_MODELLED))
        .collect())
}

fn gap(module: &str, reason: &str) -> Json {
    json!({ "module": module, "reason": reason })
}

fn sub_emitters(
    sub: &Value,
    node_path: NodePath<'_>,
    unsupported: &mut Vec<Json>,
) -> Result<Vec<Json>, ConvertError> {
    let entries = array(field(sub, "subEmitters")?, "subEmitters")?;
    entries
        .iter()
        .map(|entry| {
            let reference = field(entry, "emitter")?
                .as_pptr()
                .ok_or_else(|| malformed("emitter is not a reference"))?;
            let emitter = node_path(reference)?;
            if emitter.is_none() && reference.path_id != 0 {
                unsupported.push(gap(SUB_MODULE, UNRESOLVED_SUB_EMITTER));
            }
            let properties = field(entry, "properties")?
                .as_u64()
                .ok_or_else(|| malformed("properties is not a count"))?;
            let inherit: Object = SUB_EMITTER_INHERIT
                .iter()
                .map(|(name, bit)| ((*name).to_owned(), Json::Bool(properties & bit != 0)))
                .collect();
            Ok(json!({
                "emitter": emitter,
                "type": read(field(entry, "type")?, Code(SUB_EMITTER_TYPES), "type")?,
                "inherit": inherit,
                "emitProbability":
                    read(field(entry, "emitProbability")?, Kind::Number, "emitProbability")?,
            }))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stored::{STATE, gradient};

    fn structure(fields: Vec<(&str, Value)>) -> Value {
        Value::Struct(
            fields
                .into_iter()
                .map(|(name, value)| (name.to_owned(), value))
                .collect(),
        )
    }

    fn constant(value: f64) -> Value {
        structure(vec![
            (STATE, Value::Int(0)),
            ("scalar", Value::Float(value)),
        ])
    }

    fn key(time: f64, in_slope: f64) -> Value {
        structure(vec![
            ("time", Value::Float(time)),
            ("value", Value::Float(1.0)),
            ("inSlope", Value::Float(in_slope)),
            ("outSlope", Value::Float(0.0)),
            ("weightedMode", Value::Int(1)),
            ("inWeight", Value::Float(0.5)),
            ("outWeight", Value::Float(0.25)),
        ])
    }

    fn curve_lane(keys: Vec<Value>) -> Value {
        structure(vec![
            ("m_Curve", Value::Array(keys)),
            ("m_PreInfinity", Value::Int(2)),
            ("m_PostInfinity", Value::Int(4)),
        ])
    }

    fn disabled() -> Value {
        structure(vec![("enabled", Value::Bool(false))])
    }

    fn enabled(mut parts: Vec<(&str, Value)>) -> Value {
        parts.push(("enabled", Value::Bool(true)));
        structure(parts)
    }

    fn start_color() -> Value {
        structure(vec![
            (STATE, Value::Int(0)),
            (
                "maxColor",
                structure(vec![
                    ("r", Value::Float(1.0)),
                    ("g", Value::Float(0.5)),
                    ("b", Value::Float(0.25)),
                    ("a", Value::Float(0.0)),
                ]),
            ),
        ])
    }

    /// A particle system whose section `name` is `module`, and whose other modules are off.
    fn system_with(name: &str, module: Value) -> Value {
        let initial = structure(vec![
            ("maxNumParticles", Value::Int(100)),
            ("startLifetime", constant(1.0)),
            ("startSpeed", constant(2.0)),
            ("startSize", constant(3.0)),
            ("startRotation", constant(4.0)),
            ("startColor", start_color()),
            ("gravityModifier", constant(0.0)),
            ("size3D", Value::Bool(false)),
            ("rotation3D", Value::Bool(false)),
        ]);
        let mut parts = vec![
            ("lengthInSec", Value::Float(5.0)),
            ("looping", Value::Bool(true)),
            ("simulationSpeed", Value::Float(1.0)),
            ("moveWithTransform", Value::Int(1)),
            ("scalingMode", Value::Int(1)),
            ("InitialModule", initial),
            (COLOR_MODULE, disabled()),
            (SUB_MODULE, disabled()),
        ];
        parts.extend(MODULES.iter().map(|module| (module.source, disabled())));
        parts.retain(|(part, _)| *part != name);
        parts.push((name, module));
        structure(parts)
    }

    fn no_nodes(_: PPtr) -> Result<Option<String>, ConvertError> {
        Ok(None)
    }

    fn sub_module(file_id: i32) -> Value {
        let entry = structure(vec![
            (
                "emitter",
                Value::PPtr(PPtr {
                    file_id,
                    path_id: 7,
                }),
            ),
            ("type", Value::Int(2)),
            ("properties", Value::Int(5)),
            ("emitProbability", Value::Float(1.0)),
        ]);
        enabled(vec![("subEmitters", Value::Array(vec![entry]))])
    }

    #[test]
    fn a_constant_curve_is_written_by_its_state() {
        let written = read(&constant(1.5), Curve, "a curve").unwrap();
        assert_eq!(written, json!({ "mode": "constant", "value": 1.5 }));
    }

    #[test]
    fn a_curve_carries_its_keys_and_wrap_modes() {
        let stored = structure(vec![
            (STATE, Value::Int(1)),
            ("scalar", Value::Float(2.0)),
            ("maxCurve", curve_lane(vec![key(0.25, f64::INFINITY)])),
        ]);
        let written = read(&stored, Curve, "a curve").unwrap();
        assert_eq!(
            written,
            json!({
                "mode": "curve",
                "multiplier": 2.0,
                "preInfinity": 2,
                "postInfinity": 4,
                "keys": [{
                    "time": 0.25,
                    "value": 1.0,
                    "inSlope": "Infinity",
                    "outSlope": 0.0,
                    "weightedMode": 1,
                    "inWeight": 0.5,
                    "outWeight": 0.25
                }]
            })
        );
    }

    #[test]
    fn an_unknown_state_is_unsupported() {
        let stored = structure(vec![(STATE, Value::Int(9))]);
        let error = read(&stored, Curve, "a curve").unwrap_err();
        assert_eq!(error.code, "unsupported");
    }

    #[test]
    fn a_gradient_scales_its_stored_times() {
        let color = |r: f64| {
            structure(vec![
                ("r", Value::Float(r)),
                ("g", Value::Float(0.0)),
                ("b", Value::Float(0.0)),
                ("a", Value::Float(0.5)),
            ])
        };
        let stored = structure(vec![
            ("m_NumColorKeys", Value::UInt(1)),
            ("m_NumAlphaKeys", Value::UInt(2)),
            ("key0", color(1.0)),
            ("key1", color(0.0)),
            ("ctime0", Value::UInt(65535)),
            ("atime0", Value::UInt(0)),
            ("atime1", Value::UInt(65535)),
        ]);
        assert_eq!(
            gradient(&stored).unwrap(),
            json!({
                "colorKeys": [{ "time": 1.0, "color": [1.0, 0.0, 0.0] }],
                "alphaKeys": [
                    { "time": 0.0, "alpha": 0.5 },
                    { "time": 1.0, "alpha": 0.5 }
                ]
            })
        );
    }

    #[test]
    fn a_disabled_module_is_not_written() {
        let written = system(&system_with("Unused", disabled()), &no_nodes).unwrap();
        assert!(written.document.get("sizeOverLifetime").is_none());
        assert_eq!(written.document["simulationSpace"], "World");
        assert_eq!(written.document["maxParticles"], 100);
        assert_eq!(written.document["start"]["speed"]["value"], 2.0);
        assert!(written.unsupported.is_empty());
    }

    #[test]
    fn extra_axes_are_written_only_when_separate() {
        let size = |separate| {
            enabled(vec![
                ("separateAxes", Value::Bool(separate)),
                ("curve", constant(1.0)),
                ("y", constant(2.0)),
                ("z", constant(3.0)),
            ])
        };
        let joined = system(&system_with("SizeModule", size(false)), &no_nodes).unwrap();
        assert!(joined.document["sizeOverLifetime"].get("y").is_none());
        let separate = system(&system_with("SizeModule", size(true)), &no_nodes).unwrap();
        assert_eq!(separate.document["sizeOverLifetime"]["y"]["value"], 2.0);
    }

    #[test]
    fn an_enabled_module_without_a_layout_is_reported() {
        let written = system(&system_with("NoiseModule", enabled(vec![])), &no_nodes).unwrap();
        assert_eq!(
            written.unsupported,
            vec![json!({ "module": "NoiseModule", "reason": NOT_MODELLED })]
        );
    }

    #[test]
    fn a_sub_emitter_names_its_node_and_what_it_inherits() {
        let resolve = |reference: PPtr| Ok(Some(format!("node{}", reference.path_id)));
        let written = system(&system_with(SUB_MODULE, sub_module(0)), &resolve).unwrap();
        assert_eq!(
            written.document["subEmitters"],
            json!([{
                "emitter": "node7",
                "type": "death",
                "emitProbability": 1.0,
                "inherit": {
                    "color": true,
                    "size": false,
                    "rotation": true,
                    "lifetime": false,
                    "duration": false
                }
            }])
        );
        assert!(written.unsupported.is_empty());
    }

    #[test]
    fn a_sub_emitter_outside_the_package_is_reported() {
        let written = system(&system_with(SUB_MODULE, sub_module(1)), &no_nodes).unwrap();
        assert_eq!(written.document["subEmitters"][0]["emitter"], Json::Null);
        assert_eq!(
            written.unsupported,
            vec![json!({ "module": SUB_MODULE, "reason": UNRESOLVED_SUB_EMITTER })]
        );
    }

    #[test]
    fn a_missing_field_is_malformed() {
        assert_eq!(renderer(&disabled()).unwrap_err().code, "malformed");
    }
}
