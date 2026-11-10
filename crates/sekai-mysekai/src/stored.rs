//! Reading stored fields through layouts: tables that say which stored field becomes which
//! key of a document, and how each value is written.

use sekai_convert::{ConvertError, Value};
use serde_json::{Map, Number, Value as Json, json};

use crate::scripts::malformed;

pub(crate) type Object = Map<String, Json>;
/// The key, the stored field as a dotted path, and the kind of each value of a section.
pub(crate) type Layout = &'static [(&'static str, &'static str, Kind)];
/// The stored code and the name of each value of an enumeration.
pub(crate) type Codes = &'static [(i64, &'static str)];
/// The stored state, the mode it is written as, and the fields live in that state.
pub(crate) type States = &'static [(i64, &'static str, Layout)];

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    /// A number, as stored.
    Number,
    /// A curve tangent: a number, or `Infinity` or `-Infinity` for a stepped segment.
    Slope,
    Flag,
    Code(Codes),
    Vector(&'static [&'static str]),
    Curve,
    Color,
    Gradient,
    List(Layout),
}

pub(crate) use Kind::{Code, Color, Curve, Flag, Gradient, List, Slope, Vector};

pub(crate) const XY: &[&str] = &["x", "y"];
pub(crate) const XYZ: &[&str] = &["x", "y", "z"];
pub(crate) const RGB: &[&str] = &["r", "g", "b"];
pub(crate) const RGBA: &[&str] = &["r", "g", "b", "a"];

pub(crate) const STATE: &str = "minMaxState";

pub(crate) const CURVE_KEY: Layout = &[
    ("time", "time", Kind::Number),
    ("value", "value", Kind::Number),
    ("inSlope", "inSlope", Slope),
    ("outSlope", "outSlope", Slope),
    ("weightedMode", "weightedMode", Kind::Number),
    ("inWeight", "inWeight", Kind::Number),
    ("outWeight", "outWeight", Kind::Number),
];

pub(crate) const CURVE_STATES: States = &[
    (0, "constant", &[("value", "scalar", Kind::Number)]),
    (
        1,
        "curve",
        &[
            ("multiplier", "scalar", Kind::Number),
            ("keys", "maxCurve.m_Curve", List(CURVE_KEY)),
            ("preInfinity", "maxCurve.m_PreInfinity", Kind::Number),
            ("postInfinity", "maxCurve.m_PostInfinity", Kind::Number),
        ],
    ),
    (
        2,
        "twoCurves",
        &[
            ("multiplier", "scalar", Kind::Number),
            ("minKeys", "minCurve.m_Curve", List(CURVE_KEY)),
            ("minPreInfinity", "minCurve.m_PreInfinity", Kind::Number),
            ("minPostInfinity", "minCurve.m_PostInfinity", Kind::Number),
            ("maxKeys", "maxCurve.m_Curve", List(CURVE_KEY)),
            ("maxPreInfinity", "maxCurve.m_PreInfinity", Kind::Number),
            ("maxPostInfinity", "maxCurve.m_PostInfinity", Kind::Number),
        ],
    ),
    (
        3,
        "twoConstants",
        &[
            ("min", "minScalar", Kind::Number),
            ("max", "scalar", Kind::Number),
        ],
    ),
];

pub(crate) const COLOR_STATES: States = &[
    (0, "color", &[("color", "maxColor", Vector(RGBA))]),
    (1, "gradient", &[("gradient", "maxGradient", Gradient)]),
    (
        2,
        "twoColors",
        &[
            ("min", "minColor", Vector(RGBA)),
            ("max", "maxColor", Vector(RGBA)),
        ],
    ),
    (
        3,
        "twoGradients",
        &[
            ("minGradient", "minGradient", Gradient),
            ("maxGradient", "maxGradient", Gradient),
        ],
    ),
    (4, "randomColor", &[("gradient", "maxGradient", Gradient)]),
];

/// The field at the dotted path `source` of `section`.
pub(crate) fn field<'v>(section: &'v Value, source: &str) -> Result<&'v Value, ConvertError> {
    source.split('.').try_fold(section, |value, name| {
        value
            .get(name)
            .ok_or_else(|| malformed(format!("no field {source}")))
    })
}

pub(crate) fn array<'v>(value: &'v Value, source: &str) -> Result<&'v [Value], ConvertError> {
    value
        .as_array()
        .ok_or_else(|| malformed(format!("{source} is not an array")))
}

pub(crate) fn fields(section: &Value, layout: Layout) -> Result<Object, ConvertError> {
    layout
        .iter()
        .map(|(key, source, kind)| {
            let value = read(field(section, source)?, *kind, source)?;
            Ok(((*key).to_owned(), value))
        })
        .collect()
}

pub(crate) fn read(value: &Value, kind: Kind, source: &str) -> Result<Json, ConvertError> {
    match kind {
        Kind::Number => number(value, source),
        Slope => slope(value, source),
        Flag => flag(value, source).map(Json::Bool),
        Code(codes) => code(value, codes, source),
        Vector(names) => names
            .iter()
            .map(|name| number(field(value, name)?, name))
            .collect::<Result<Vec<_>, _>>()
            .map(Json::Array),
        Curve => by_state(value, CURVE_STATES, source),
        Color => by_state(value, COLOR_STATES, source),
        Gradient => gradient(value),
        List(layout) => array(value, source)?
            .iter()
            .map(|item| fields(item, layout).map(Json::Object))
            .collect::<Result<Vec<_>, _>>()
            .map(Json::Array),
    }
}

pub(crate) fn number(value: &Value, source: &str) -> Result<Json, ConvertError> {
    let not_a_number = || malformed(format!("{source} is not a finite number"));
    if let Some(float) = value.as_f64() {
        return Number::from_f64(float)
            .map(Json::Number)
            .ok_or_else(not_a_number);
    }
    value
        .as_i64()
        .map(Json::from)
        .or_else(|| value.as_u64().map(Json::from))
        .ok_or_else(not_a_number)
}

pub(crate) fn slope(value: &Value, source: &str) -> Result<Json, ConvertError> {
    match value.as_f64() {
        Some(f64::INFINITY) => Ok(Json::from("Infinity")),
        Some(f64::NEG_INFINITY) => Ok(Json::from("-Infinity")),
        _ => number(value, source),
    }
}

pub(crate) fn flag(value: &Value, source: &str) -> Result<bool, ConvertError> {
    value
        .as_bool()
        .or_else(|| value.as_u64().map(|stored| stored != 0))
        .ok_or_else(|| malformed(format!("{source} is not a flag")))
}

pub(crate) fn is_set(section: &Value, name: &str) -> Result<bool, ConvertError> {
    flag(field(section, name)?, name)
}

pub(crate) fn stored_code(value: &Value, source: &str) -> Result<i64, ConvertError> {
    value
        .as_i64()
        .ok_or_else(|| malformed(format!("{source} is not an integer")))
}

pub(crate) fn code(value: &Value, codes: Codes, source: &str) -> Result<Json, ConvertError> {
    let stored = stored_code(value, source)?;
    codes
        .iter()
        .find(|(code, _)| *code == stored)
        .map(|(_, name)| Json::from(*name))
        .ok_or_else(|| unknown(source, stored))
}

pub(crate) fn unknown(source: &str, stored: i64) -> ConvertError {
    ConvertError::new(
        "unsupported",
        format!("{source} has the unknown value {stored}"),
    )
}

/// The value in the shape that the state of `node` selects.
pub(crate) fn by_state(node: &Value, states: States, source: &str) -> Result<Json, ConvertError> {
    let stored = stored_code(field(node, STATE)?, STATE)?;
    let (_, mode, layout) = states
        .iter()
        .find(|(state, ..)| *state == stored)
        .ok_or_else(|| unknown(source, stored))?;
    let mut out = fields(node, layout)?;
    out.insert("mode".to_owned(), Json::from(*mode));
    Ok(Json::Object(out))
}

/// The keys of a gradient, whose times are stored as fractions of 65535.
pub(crate) fn gradient(node: &Value) -> Result<Json, ConvertError> {
    const TIME_SCALE: f64 = 65535.0;
    let key_time = |prefix: &str, index: u64| -> Result<f64, ConvertError> {
        let name = format!("{prefix}{index}");
        let stored = field(node, &name)?
            .as_u64()
            .ok_or_else(|| malformed(format!("{name} is not a time")))?;
        Ok(stored as f64 / TIME_SCALE)
    };
    let count = |name: &str| -> Result<u64, ConvertError> {
        field(node, name)?
            .as_u64()
            .ok_or_else(|| malformed(format!("{name} is not a count")))
    };
    let colors = (0..count("m_NumColorKeys")?)
        .map(|index| {
            let color = read(field(node, &format!("key{index}"))?, Vector(RGB), "key")?;
            Ok(json!({ "time": key_time("ctime", index)?, "color": color }))
        })
        .collect::<Result<Vec<_>, ConvertError>>()?;
    let alphas = (0..count("m_NumAlphaKeys")?)
        .map(|index| {
            let alpha = number(field(node, &format!("key{index}.a"))?, "a")?;
            Ok(json!({ "time": key_time("atime", index)?, "alpha": alpha }))
        })
        .collect::<Result<Vec<_>, ConvertError>>()?;
    Ok(json!({ "colorKeys": colors, "alphaKeys": alphas }))
}
