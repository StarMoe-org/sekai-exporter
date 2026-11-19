//! Reading stored fields, with the errors of the forms that read them.

use crate::{ConvertError, Value};

pub(crate) fn malformed(message: impl Into<String>) -> ConvertError {
    ConvertError::new("malformed", message)
}

pub(crate) fn unsupported(message: impl Into<String>) -> ConvertError {
    ConvertError::new("unsupported", message)
}

pub(crate) fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ConvertError> {
    value
        .get(name)
        .ok_or_else(|| malformed(format!("the field {name} is missing")))
}

pub(crate) fn array<'a>(value: &'a Value, name: &str) -> Result<&'a [Value], ConvertError> {
    field(value, name)?
        .as_array()
        .ok_or_else(|| malformed(format!("{name} is not an array")))
}

pub(crate) fn bytes<'a>(value: &'a Value, name: &str) -> Result<&'a [u8], ConvertError> {
    field(value, name)?
        .as_bytes()
        .ok_or_else(|| malformed(format!("{name} is not a byte array")))
}

pub(crate) fn integer(value: &Value, name: &str) -> Result<i64, ConvertError> {
    field(value, name)?
        .as_i64()
        .ok_or_else(|| malformed(format!("{name} is not an integer")))
}

/// A stored count, size or offset.
pub(crate) fn count(value: &Value, name: &str) -> Result<usize, ConvertError> {
    field(value, name)?
        .as_u64()
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| malformed(format!("{name} is not a count")))
}
