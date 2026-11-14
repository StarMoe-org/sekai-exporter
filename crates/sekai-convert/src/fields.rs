//! The serialized fields of a `MonoBehaviour` as a JSON document.

use crate::{Asset, Content, Context, ConvertError, Member, ObjectId, Value};

/// Fields every `MonoBehaviour` has, which say nothing about what it holds.
const ENGINE_FIELDS: &[&str] = &["m_GameObject", "m_Enabled", "m_Script", "m_Name"];

/// An asset of one member `role`: the serialized fields of `root` as a JSON document.
pub fn fields_document(
    root: ObjectId,
    cx: &dyn Context,
    role: &str,
) -> Result<Asset, ConvertError> {
    let object = cx.object(root)?;
    let Value::Struct(fields) = &object.fields else {
        return Err(ConvertError::new(
            "malformed",
            "the fields are not a structure",
        ));
    };
    let mut out = String::from("{");
    let mut first = true;
    for (name, value) in fields {
        if ENGINE_FIELDS.contains(&name.as_str()) {
            continue;
        }
        if !first {
            out.push(',');
        }
        first = false;
        string(&mut out, name);
        out.push(':');
        json(&mut out, value, name)?;
    }
    out.push('}');
    Ok(Asset {
        members: vec![Member {
            role: role.into(),
            content: Content::File {
                extension: "json".into(),
                bytes: out.into_bytes(),
            },
        }],
    })
}

/// Writes `value`, the field `path`, as JSON.
fn json(out: &mut String, value: &Value, path: &str) -> Result<(), ConvertError> {
    match value {
        Value::Bool(v) => out.push_str(if *v { "true" } else { "false" }),
        Value::Int(v) => out.push_str(&v.to_string()),
        Value::UInt(v) => out.push_str(&v.to_string()),
        Value::Float(v) => {
            if !v.is_finite() {
                return Err(ConvertError::new(
                    "malformed",
                    format!("{path} is not a finite number"),
                ));
            }
            // A value that a 32-bit float holds exactly is written as the shortest decimal that
            // reads back to that float.
            let single = *v as f32;
            if f64::from(single) == *v {
                out.push_str(&single.to_string());
            } else {
                out.push_str(&v.to_string());
            }
        }
        Value::String(v) => string(out, v),
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                json(out, item, path)?;
            }
            out.push(']');
        }
        Value::Struct(fields) => {
            out.push('{');
            for (i, (name, item)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                string(out, name);
                out.push(':');
                json(out, item, &format!("{path}.{name}"))?;
            }
            out.push('}');
        }
        Value::Bytes(_) => {
            return Err(ConvertError::new(
                "unsupported",
                format!("{path} holds bytes or a string that is not UTF-8"),
            ));
        }
        Value::Map(_) | Value::PPtr(_) => {
            return Err(ConvertError::new(
                "unsupported",
                format!("{path} holds a map or a reference"),
            ));
        }
    }
    Ok(())
}

/// Writes `s` as a JSON string.
pub fn string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}
