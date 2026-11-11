//! Reading objects through a layout.
//!
//! [`read_object`] walks an object's bytes by its layout and reports each value to a
//! [`Visitor`]. Writers of object documents, the reference extractor of the index and typed
//! readers are visitors, so all of them read a class the same way.

use crate::endian::{Endian, Reader};
use crate::error::{Error, Result};
use crate::layout::{Kind, Layout, Op, ScalarKind};
use crate::serialized::ReferencedName;

/// A scalar value as stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Scalar {
    /// `bool`, with the stored byte.
    Bool(u8),
    /// `SInt8`.
    I8(i8),
    /// `UInt8` and `char`.
    U8(u8),
    /// `SInt16`.
    I16(i16),
    /// `UInt16`.
    U16(u16),
    /// `SInt32`.
    I32(i32),
    /// `UInt32`.
    U32(u32),
    /// `SInt64`.
    I64(i64),
    /// `UInt64`.
    U64(u64),
    /// `float`.
    F32(f32),
    /// `double`.
    F64(f64),
}

/// The field a value belongs to.
#[derive(Debug, Clone, Copy)]
pub struct Field<'a> {
    /// The field name. Array elements carry the name of the element node.
    pub name: &'a str,
    /// The type name.
    pub type_name: &'a str,
    /// Whether the value contains a reference.
    pub has_references: bool,
}

/// Receives the values of an object in layout order.
///
/// Every method does nothing by default, so a visitor implements only what it uses.
#[allow(unused_variables)]
pub trait Visitor {
    /// A structure begins; its fields follow until [`Visitor::end_struct`].
    fn begin_struct(&mut self, field: &Field<'_>) -> Result<()> {
        Ok(())
    }
    /// The structure begun last ends.
    fn end_struct(&mut self) -> Result<()> {
        Ok(())
    }
    /// A scalar.
    fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> Result<()> {
        Ok(())
    }
    /// A string, as stored bytes.
    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        Ok(())
    }
    /// A byte payload: `TypelessData` or an array of 8-bit integers.
    fn bytes(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        Ok(())
    }
    /// An array of `len` elements begins. Returning `false` skips the elements without reporting
    /// them; [`Visitor::end_array`] is called either way.
    fn begin_array(&mut self, field: &Field<'_>, len: usize, map: bool) -> Result<bool> {
        Ok(true)
    }
    /// The array begun last ends.
    fn end_array(&mut self) -> Result<()> {
        Ok(())
    }
    /// A reference to another object.
    fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
        Ok(())
    }
}

/// A visitor that ignores everything.
#[derive(Debug, Default, Clone, Copy)]
pub struct IgnoreValues;

impl Visitor for IgnoreValues {}

/// The layouts of the types that `[SerializeReference]` entries name.
pub trait ReferenceTypes {
    /// The layout of the named type, if the file's table of referenced types contains it.
    fn layout(&self, name: &ReferencedName) -> Option<&Layout>;
}

impl ReferenceTypes for () {
    fn layout(&self, _name: &ReferencedName) -> Option<&Layout> {
        None
    }
}

/// Reads one object by its layout, reporting every value to `visitor`.
///
/// The object must be read to exactly its end: reading past the end fails with
/// [`Error::ObjectTruncated`], stopping before it with [`Error::TrailingBytes`].
pub fn read_object(
    layout: &Layout,
    data: &[u8],
    endian: Endian,
    references: &dyn ReferenceTypes,
    visitor: &mut dyn Visitor,
) -> Result<()> {
    let mut run = Run {
        r: Reader::new(data, endian, "object"),
        references,
        in_registry: false,
        in_referenced: false,
    };
    run.value(layout, 0, true, None, visitor)?;
    if run.r.pos() != data.len() {
        return Err(Error::TrailingBytes {
            consumed: run.r.pos() as u64,
            size: data.len() as u64,
        });
    }
    Ok(())
}

struct Run<'d, 'r> {
    r: Reader<'d>,
    references: &'r dyn ReferenceTypes,
    in_registry: bool,
    /// Whether the data of a referenced object is being read.
    in_referenced: bool,
}

impl<'d> Run<'d, '_> {
    /// Converts a read past the end into an error naming the field.
    fn at<T>(&self, op: &Op, result: Result<T>) -> Result<T> {
        result.map_err(|e| match e {
            Error::Truncated { offset, .. } => Error::ObjectTruncated {
                field: op.name.clone(),
                offset,
                size: self.r.len() as u64,
            },
            other => other,
        })
    }

    fn len(&mut self, op: &Op) -> Result<usize> {
        let at = self.r.pos() as u64;
        let n = self.r.i32();
        let n = self.at(op, n)?;
        usize::try_from(n).map_err(|_| Error::ObjectTruncated {
            field: op.name.clone(),
            offset: at,
            size: self.r.len() as u64,
        })
    }

    fn bytes(&mut self, op: &Op, n: usize) -> Result<&'d [u8]> {
        let b = self.r.bytes(n);
        self.at(op, b)
    }

    /// Reads op `i` of `layout`. Array elements pass `align_self = false`: the element node's
    /// own alignment flag applies once after the array, whose op carries it.
    fn value(
        &mut self,
        layout: &Layout,
        i: usize,
        align_self: bool,
        name: Option<&str>,
        v: &mut dyn Visitor,
    ) -> Result<()> {
        let op = &layout.ops[i];
        let field = Field {
            name: name.unwrap_or(&op.name),
            type_name: &op.type_name,
            has_references: op.has_references,
        };
        match op.kind {
            Kind::Scalar(kind) => {
                let value = self.scalar(kind);
                let value = self.at(op, value)?;
                v.scalar(&field, value)?;
            }
            Kind::String => {
                let n = self.len(op)?;
                let bytes = self.bytes(op, n)?;
                let aligned = self.r.align(4);
                self.at(op, aligned)?;
                v.string(&field, bytes)?;
            }
            Kind::Bytes => {
                let n = self.len(op)?;
                let bytes = self.bytes(op, n)?;
                v.bytes(&field, bytes)?;
            }
            Kind::PPtr { wide_path_id } => {
                let file_id = self.r.i32();
                let file_id = self.at(op, file_id)?;
                let path_id = if wide_path_id {
                    self.r.i64()
                } else {
                    self.r.i32().map(i64::from)
                };
                let path_id = self.at(op, path_id)?;
                v.pptr(&field, file_id, path_id)?;
            }
            Kind::Array { map } => {
                let n = self.len(op)?;
                let element = i + 1;
                if v.begin_array(&field, n, map)? {
                    for _ in 0..n {
                        self.value(layout, element, false, None, v)?;
                    }
                } else if let Some(size) = layout.ops[element].fixed_size {
                    let total = n.saturating_mul(size as usize);
                    self.bytes(op, total)?;
                } else {
                    for _ in 0..n {
                        self.value(layout, element, false, None, &mut IgnoreValues)?;
                    }
                }
                v.end_array()?;
            }
            Kind::Struct | Kind::ReferencedObjectData => {
                v.begin_struct(&field)?;
                for c in layout.children(i) {
                    self.value(layout, c, true, None, v)?;
                }
                v.end_struct()?;
            }
            Kind::Registry => {
                // A registry nested in a referenced object is not stored; only the outermost
                // one is read.
                if self.in_registry {
                    return Ok(());
                }
                self.in_registry = true;
                v.begin_struct(&field)?;
                for c in layout.children(i) {
                    self.value(layout, c, true, None, v)?;
                }
                v.end_struct()?;
                self.in_registry = false;
            }
            Kind::ReferencedObject => {
                v.begin_struct(&field)?;
                let mut name = None;
                for c in layout.children(i) {
                    let child = &layout.ops[c];
                    if child.kind == Kind::ReferencedObjectData {
                        let Some(name) = name.take() else {
                            return Err(Error::invalid_layout(
                                "a referenced object has data before its type",
                            ));
                        };
                        self.referenced_data(&name, &child.name, v)?;
                    } else if child.name == "type" {
                        let mut record = RecordStrings {
                            inner: v,
                            strings: Vec::new(),
                        };
                        self.value(layout, c, true, None, &mut record)?;
                        name = Some(record.into_name()?);
                    } else {
                        self.value(layout, c, true, None, v)?;
                    }
                }
                v.end_struct()?;
            }
        }
        if op.align && align_self {
            let aligned = self.r.align(4);
            self.at(op, aligned)?;
        }
        Ok(())
    }

    fn referenced_data(
        &mut self,
        name: &ReferencedName,
        field: &str,
        v: &mut dyn Visitor,
    ) -> Result<()> {
        // An entry with an empty class name is a null reference and stores no data.
        if name.class.is_empty() {
            return Ok(());
        }
        // The referenced objects of an object are stored side by side in its registry, so the
        // data of one never holds another. Refusing it bounds the depth of the reader.
        if self.in_referenced {
            return Err(Error::invalid_layout(
                "a referenced object is stored in the data of another",
            ));
        }
        let references = self.references;
        let layout = references
            .layout(name)
            .ok_or_else(|| Error::UnresolvedReferenceType {
                class: name.class.clone(),
                namespace: name.namespace.clone(),
                assembly: name.assembly.clone(),
            })?;
        self.in_referenced = true;
        let read = self.value(layout, 0, true, Some(field), v);
        self.in_referenced = false;
        read
    }

    fn scalar(&mut self, kind: ScalarKind) -> Result<Scalar> {
        let r = &mut self.r;
        Ok(match kind {
            ScalarKind::Bool => Scalar::Bool(r.u8()?),
            ScalarKind::I8 => Scalar::I8(r.i8()?),
            ScalarKind::U8 => Scalar::U8(r.u8()?),
            ScalarKind::I16 => Scalar::I16(r.i16()?),
            ScalarKind::U16 => Scalar::U16(r.u16()?),
            ScalarKind::I32 => Scalar::I32(r.i32()?),
            ScalarKind::U32 => Scalar::U32(r.u32()?),
            ScalarKind::I64 => Scalar::I64(r.i64()?),
            ScalarKind::U64 => Scalar::U64(r.u64()?),
            ScalarKind::F32 => Scalar::F32(r.f32()?),
            ScalarKind::F64 => Scalar::F64(r.f64()?),
        })
    }
}

/// Forwards every value and keeps the strings, to learn the type a referenced object names.
struct RecordStrings<'v> {
    inner: &'v mut dyn Visitor,
    strings: Vec<String>,
}

impl RecordStrings<'_> {
    fn into_name(self) -> Result<ReferencedName> {
        let [class, namespace, assembly]: [String; 3] = self
            .strings
            .try_into()
            .map_err(|_| Error::invalid_layout("a referenced type is not three strings"))?;
        Ok(ReferencedName {
            class,
            namespace,
            assembly,
        })
    }
}

impl Visitor for RecordStrings<'_> {
    fn begin_struct(&mut self, field: &Field<'_>) -> Result<()> {
        self.inner.begin_struct(field)
    }
    fn end_struct(&mut self) -> Result<()> {
        self.inner.end_struct()
    }
    fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> Result<()> {
        self.inner.scalar(field, value)
    }
    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        self.strings
            .push(String::from_utf8_lossy(bytes).into_owned());
        self.inner.string(field, bytes)
    }
    fn bytes(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        self.inner.bytes(field, bytes)
    }
    fn begin_array(&mut self, field: &Field<'_>, len: usize, map: bool) -> Result<bool> {
        self.inner.begin_array(field, len, map)
    }
    fn end_array(&mut self) -> Result<()> {
        self.inner.end_array()
    }
    fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
        self.inner.pptr(field, file_id, path_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tests::tree;

    #[derive(Default)]
    struct Collect(Vec<String>);

    impl Visitor for Collect {
        fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> Result<()> {
            self.0.push(format!("{}={value:?}", field.name));
            Ok(())
        }
        fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
            self.0.push(format!(
                "{}={:?}",
                field.name,
                String::from_utf8_lossy(bytes)
            ));
            Ok(())
        }
        fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
            self.0.push(format!("{}=({file_id},{path_id})", field.name));
            Ok(())
        }
    }

    fn layout() -> Layout {
        Layout::compile(&tree(&[
            (0, "Thing", "Base", false),
            (1, "string", "m_Name", false),
            (2, "Array", "Array", true),
            (3, "int", "size", false),
            (3, "char", "data", false),
            (1, "bool", "m_On", true),
            (1, "PPtr<Object>", "m_Target", false),
            (2, "int", "m_FileID", false),
            (2, "SInt64", "m_PathID", false),
        ]))
        .unwrap()
    }

    fn bytes() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&3i32.to_le_bytes());
        b.extend_from_slice(b"abc\0");
        b.extend_from_slice(&[1, 0, 0, 0]);
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&(-7i64).to_le_bytes());
        b
    }

    #[test]
    fn an_aligned_element_aligns_once_after_its_array() {
        let layout = Layout::compile(&tree(&[
            (0, "Thing", "Base", false),
            (1, "vector", "m_Flags", false),
            (2, "Array", "Array", false),
            (3, "int", "size", false),
            (3, "bool", "data", true),
            (1, "int", "m_Count", false),
        ]))
        .unwrap();
        let mut b = 3i32.to_le_bytes().to_vec();
        b.extend_from_slice(&[1, 0, 1, 0]);
        b.extend_from_slice(&9i32.to_le_bytes());
        let mut c = Collect::default();
        read_object(&layout, &b, Endian::Little, &(), &mut c).unwrap();
        assert_eq!(
            c.0,
            [
                "data=Bool(1)",
                "data=Bool(0)",
                "data=Bool(1)",
                "m_Count=I32(9)"
            ]
        );
    }

    #[test]
    fn reads_fields_with_alignment() {
        let mut c = Collect::default();
        read_object(&layout(), &bytes(), Endian::Little, &(), &mut c).unwrap();
        assert_eq!(c.0, ["m_Name=\"abc\"", "m_On=Bool(1)", "m_Target=(1,-7)"]);
    }

    #[test]
    fn reports_short_and_long_objects() {
        let b = bytes();
        let err = read_object(
            &layout(),
            &b[..b.len() - 1],
            Endian::Little,
            &(),
            &mut IgnoreValues,
        )
        .unwrap_err();
        assert_eq!(err.code(), "truncated");
        let mut long = b.clone();
        long.push(0);
        let err =
            read_object(&layout(), &long, Endian::Little, &(), &mut IgnoreValues).unwrap_err();
        assert_eq!(err.code(), "trailing_bytes");
    }
}
