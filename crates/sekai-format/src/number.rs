//! Numbers in JSON documents, including the values JSON cannot hold.

use serde::{Serialize, Serializer};

/// A 32-bit floating-point value, written as the shortest decimal that reads back to it, or as
/// the string `"NaN"`, `"Infinity"` or `"-Infinity"`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Number(pub f32);

impl Serialize for Number {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match non_finite(f64::from(self.0)) {
            Some(name) => serializer.serialize_str(name),
            None => serializer.serialize_f32(self.0),
        }
    }
}

/// Writes a stored `float` or `double`: as a 32-bit value when it is one exactly, so that it gets
/// the shortest decimal of that width.
pub(crate) fn serialize_stored<S: Serializer>(
    value: f64,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    if let Some(name) = non_finite(value) {
        return serializer.serialize_str(name);
    }
    // `value` is finite, so the conversion either is exact or loses precision; it never fails.
    let single = value as f32;
    if f64::from(single) == value {
        serializer.serialize_f32(single)
    } else {
        serializer.serialize_f64(value)
    }
}

/// The name of a value JSON cannot hold.
fn non_finite(value: f64) -> Option<&'static str> {
    if value.is_nan() {
        Some("NaN")
    } else if value == f64::INFINITY {
        Some("Infinity")
    } else if value == f64::NEG_INFINITY {
        Some("-Infinity")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_shortest_and_name_the_values_json_cannot_hold() {
        let text = |n: f32| serde_json::to_string(&Number(n)).unwrap();
        assert_eq!(text(0.1), "0.1");
        assert_eq!(text(f32::NAN), "\"NaN\"");
        assert_eq!(text(f32::NEG_INFINITY), "\"-Infinity\"");
    }

    #[test]
    fn stored_doubles_keep_their_precision() {
        let text = |v: f64| {
            let mut out = Vec::new();
            serialize_stored(v, &mut serde_json::Serializer::new(&mut out)).unwrap();
            String::from_utf8(out).unwrap()
        };
        assert_eq!(text(f64::from(0.1f32)), "0.1");
        assert_eq!(text(0.1), "0.1");
        assert_eq!(text(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(text(f64::INFINITY), "\"Infinity\"");
    }
}
