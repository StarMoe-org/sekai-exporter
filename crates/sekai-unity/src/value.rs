//! A tree of one object's field values, for code that reads fields by name.
//!
//! The tree follows the value model of the object document specification: structures keep
//! their fields in layout order, maps keep their pairs in stored order, arrays of 8-bit integers
//! are bytes, and references are kept as stored.

use crate::endian::Endian;
use crate::error::{Error, Result};
use crate::index::PPtr;
use crate::layout::Layout;
use crate::read::{Field, ReferenceTypes, Scalar, Visitor, read_object};

/// One field value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A `bool` stored as 0 or 1. Any other stored byte is an [`UInt`](Value::UInt).
    Bool(bool),
    /// A signed integer of any width.
    Int(i64),
    /// An unsigned integer of any width, and `char`.
    UInt(u64),
    /// A `float` or `double`.
    Float(f64),
    /// A string that is valid UTF-8.
    String(String),
    /// A string that is not valid UTF-8, an array of 8-bit integers, or `TypelessData`.
    Bytes(Vec<u8>),
    /// An array.
    Array(Vec<Value>),
    /// A map, as its pairs in stored order.
    Map(Vec<(Value, Value)>),
    /// A structure, as its fields in layout order.
    Struct(Vec<(String, Value)>),
    /// A reference, as stored.
    PPtr(PPtr),
}

impl Value {
    /// The field `name` of a structure.
    pub fn get(&self, name: &str) -> Option<&Value> {
        match self {
            Self::Struct(fields) => fields.iter().find(|(n, _)| n == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// A boolean.
    pub fn as_bool(&self) -> Option<bool> {
        match *self {
            Self::Bool(v) => Some(v),
            _ => None,
        }
    }

    /// An integer that fits in `i64`.
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Int(v) => Some(v),
            Self::UInt(v) => i64::try_from(v).ok(),
            _ => None,
        }
    }

    /// An integer that fits in `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        match *self {
            Self::Int(v) => u64::try_from(v).ok(),
            Self::UInt(v) => Some(v),
            _ => None,
        }
    }

    /// A floating-point number.
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Float(v) => Some(v),
            _ => None,
        }
    }

    /// A string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(v) => Some(v),
            _ => None,
        }
    }

    /// Bytes, or the bytes of a string.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Bytes(v) => Some(v),
            Self::String(v) => Some(v.as_bytes()),
            _ => None,
        }
    }

    /// The elements of an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(v) => Some(v),
            _ => None,
        }
    }

    /// The pairs of a map.
    pub fn as_map(&self) -> Option<&[(Value, Value)]> {
        match self {
            Self::Map(v) => Some(v),
            _ => None,
        }
    }

    /// A reference.
    pub fn as_pptr(&self) -> Option<PPtr> {
        match *self {
            Self::PPtr(v) => Some(v),
            _ => None,
        }
    }
}

/// Reads an object into a value tree. The root is a [`Value::Struct`].
pub fn read_value(
    layout: &Layout,
    data: &[u8],
    endian: Endian,
    references: &dyn ReferenceTypes,
) -> Result<Value> {
    let mut builder = Builder::default();
    read_object(layout, data, endian, references, &mut builder)?;
    builder
        .root
        .ok_or_else(|| Error::invalid_layout("the layout has no root structure"))
}

enum Node {
    Struct(Vec<(String, Value)>),
    Array(Vec<Value>),
    Map(Vec<(Value, Value)>),
    Pair(Vec<Value>),
}

#[derive(Default)]
struct Builder {
    /// Each open container, with the name of the field that holds it.
    stack: Vec<(String, Node)>,
    root: Option<Value>,
}

impl Builder {
    fn push(&mut self, name: &str, value: Value) -> Result<()> {
        match self.stack.last_mut() {
            Some((_, Node::Struct(fields))) => fields.push((name.to_owned(), value)),
            Some((_, Node::Array(items))) => items.push(value),
            Some((_, Node::Pair(items))) => items.push(value),
            Some((_, Node::Map(_))) => {
                return Err(Error::invalid_layout("a map element is not a pair"));
            }
            None => self.root = Some(value),
        }
        Ok(())
    }

    fn close(&mut self) -> Result<()> {
        let Some((name, node)) = self.stack.pop() else {
            return Err(Error::invalid_layout(
                "a container closes that was not opened",
            ));
        };
        let value = match node {
            Node::Struct(fields) => Value::Struct(fields),
            Node::Array(items) => Value::Array(items),
            Node::Map(pairs) => Value::Map(pairs),
            Node::Pair(items) => {
                let [key, value]: [Value; 2] = items
                    .try_into()
                    .map_err(|_| Error::invalid_layout("a map pair has no two fields"))?;
                match self.stack.last_mut() {
                    Some((_, Node::Map(pairs))) => pairs.push((key, value)),
                    _ => return Err(Error::invalid_layout("a map pair outside a map")),
                }
                return Ok(());
            }
        };
        self.push(&name, value)
    }
}

impl Visitor for Builder {
    fn begin_struct(&mut self, field: &Field<'_>) -> Result<()> {
        let node = match self.stack.last() {
            Some((_, Node::Map(_))) => Node::Pair(Vec::with_capacity(2)),
            _ => Node::Struct(Vec::new()),
        };
        self.stack.push((field.name.to_owned(), node));
        Ok(())
    }

    fn end_struct(&mut self) -> Result<()> {
        self.close()
    }

    fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> Result<()> {
        let value = match value {
            Scalar::Bool(0) => Value::Bool(false),
            Scalar::Bool(1) => Value::Bool(true),
            Scalar::Bool(v) | Scalar::U8(v) => Value::UInt(v.into()),
            Scalar::I8(v) => Value::Int(v.into()),
            Scalar::I16(v) => Value::Int(v.into()),
            Scalar::U16(v) => Value::UInt(v.into()),
            Scalar::I32(v) => Value::Int(v.into()),
            Scalar::U32(v) => Value::UInt(v.into()),
            Scalar::I64(v) => Value::Int(v),
            Scalar::U64(v) => Value::UInt(v),
            Scalar::F32(v) => Value::Float(v.into()),
            Scalar::F64(v) => Value::Float(v),
        };
        self.push(field.name, value)
    }

    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        let value = match std::str::from_utf8(bytes) {
            Ok(s) => Value::String(s.to_owned()),
            Err(_) => Value::Bytes(bytes.to_vec()),
        };
        self.push(field.name, value)
    }

    fn bytes(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        self.push(field.name, Value::Bytes(bytes.to_vec()))
    }

    fn begin_array(&mut self, field: &Field<'_>, len: usize, map: bool) -> Result<bool> {
        let node = if map {
            Node::Map(Vec::with_capacity(len))
        } else {
            Node::Array(Vec::with_capacity(len))
        };
        self.stack.push((field.name.to_owned(), node));
        Ok(true)
    }

    fn end_array(&mut self) -> Result<()> {
        self.close()
    }

    fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
        self.push(field.name, Value::PPtr(PPtr { file_id, path_id }))
    }
}
