//! Reading stored fields, with the errors of the model module.

pub(super) use crate::read::{array, bytes, count, field, integer, malformed, unsupported};
use crate::{ConvertError, PPtr, Value};

pub(super) fn pairs<'a>(
    value: &'a Value,
    name: &str,
) -> Result<&'a [(Value, Value)], ConvertError> {
    field(value, name)?
        .as_map()
        .ok_or_else(|| malformed(format!("{name} is not a map")))
}

pub(super) fn string<'a>(value: &'a Value, name: &str) -> Result<&'a str, ConvertError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| malformed(format!("{name} is not a string")))
}

pub(super) fn reference(value: &Value, name: &str) -> Result<PPtr, ConvertError> {
    field(value, name)?
        .as_pptr()
        .ok_or_else(|| malformed(format!("{name} is not a reference")))
}

/// A stored 32-bit float.
pub(super) fn number(value: &Value, name: &str) -> Result<f32, ConvertError> {
    field(value, name)?
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| malformed(format!("{name} is not a number")))
}

pub(super) fn unsigned(value: &Value, name: &str) -> Result<u64, ConvertError> {
    field(value, name)?
        .as_u64()
        .ok_or_else(|| malformed(format!("{name} is not an unsigned integer")))
}

/// A boolean stored as `bool` or as a byte.
pub(super) fn flag(value: &Value, name: &str) -> Result<bool, ConvertError> {
    let stored = field(value, name)?;
    stored
        .as_bool()
        .or_else(|| stored.as_u64().map(|v| v != 0))
        .ok_or_else(|| malformed(format!("{name} is not a boolean")))
}

/// The components `names` of a stored vector, quaternion or colour.
pub(super) fn components<const N: usize>(
    vector: &Value,
    names: [&str; N],
) -> Result<[f32; N], ConvertError> {
    let mut out = [0.0; N];
    for (slot, component) in out.iter_mut().zip(names) {
        *slot = number(vector, component)?;
    }
    Ok(out)
}

pub(super) fn vector3(value: &Value, name: &str) -> Result<[f32; 3], ConvertError> {
    components(field(value, name)?, ["x", "y", "z"])
}

/// Fails when a value that the file's JSON holds as a number is not finite.
pub(super) fn finite<const N: usize>(
    values: [f32; N],
    what: &str,
) -> Result<[f32; N], ConvertError> {
    if values.iter().all(|v| v.is_finite()) {
        Ok(values)
    } else {
        Err(malformed(format!("{what} is not finite")))
    }
}
