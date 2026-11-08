//! Object documents: every object of a bundle with all of its fields, as JSON.
//!
//! The format is defined in `docs/spec/object-documents.md`. Field values are written while
//! the object is read, without building a value tree.

use std::fmt::Write as _;

use sha2::{Digest, Sha256};

use crate::error::{Error, Result};
use crate::index::{OpenFile, PPtr, Target, VersionIndex};
use crate::read::{Field, Scalar, Visitor};

/// The version of the object document format.
pub const FORMAT_VERSION: u32 = 1;

/// Payloads larger than this are referenced by hash instead of written inline.
pub const INLINE_PAYLOAD_LIMIT: usize = 4096;

/// Receives the byte payloads that are too large to write inline.
pub trait Payloads {
    /// Stores `bytes` under their SHA-256.
    fn store(&mut self, sha256: &[u8; 32], bytes: &[u8]) -> Result<()>;
}

/// One entry of a bundle's container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryRecord {
    /// The entry path.
    pub path: String,
    /// Whether the entry is a serialized file.
    pub serialized: bool,
    /// The size of the entry.
    pub size: u64,
    /// The SHA-256 of the entry's bytes.
    pub sha256: [u8; 32],
}

/// Writes the object document of bundle `bundle` of `version`.
///
/// `files` are the bundle's serialized files in the order of its index. An object that cannot be
/// read exactly is written with its error; a failure to read the bundle's bytes fails the whole
/// document.
pub fn write_bundle(
    out: &mut Vec<u8>,
    version: &VersionIndex,
    bundle: usize,
    entries: &[EntryRecord],
    files: &[OpenFile<'_>],
    payloads: &mut dyn Payloads,
) -> Result<()> {
    let index = &version.bundles()[bundle];
    assert_eq!(
        index.files.len(),
        files.len(),
        "one open file per indexed file"
    );

    out.extend_from_slice(b"{\"format\":\"objects\",\"version\":");
    push_display(out, FORMAT_VERSION);
    out.extend_from_slice(b",\"bundle\":");
    push_str(out, &index.name);
    out.extend_from_slice(b",\"entries\":[");
    for (i, entry) in entries.iter().enumerate() {
        if i > 0 {
            out.push(b',');
        }
        out.extend_from_slice(b"{\"path\":");
        push_str(out, &entry.path);
        out.extend_from_slice(if entry.serialized {
            b",\"kind\":\"serialized\",\"size\":"
        } else {
            b",\"kind\":\"resource\",\"size\":"
        });
        push_display(out, entry.size);
        out.extend_from_slice(b",\"sha256\":");
        push_hash(out, &entry.sha256);
        out.push(b'}');
    }
    out.extend_from_slice(b"],\"files\":[");
    for (f, open) in files.iter().enumerate() {
        if f > 0 {
            out.push(b',');
        }
        let file_index = &index.files[f];
        out.extend_from_slice(b"{\"path\":");
        push_str(out, open.path);
        out.extend_from_slice(b",\"serialized_version\":");
        push_display(out, open.file.header.version);
        out.extend_from_slice(b",\"unity_version\":");
        push_str(out, &open.file.unity_version);
        out.extend_from_slice(b",\"platform\":");
        push_display(out, open.file.platform);
        out.extend_from_slice(b",\"type_trees\":");
        push_display(out, open.file.type_trees);
        out.extend_from_slice(b",\"externals\":[");
        for (e, external) in open.file.externals.iter().enumerate() {
            if e > 0 {
                out.push(b',');
            }
            out.extend_from_slice(b"{\"index\":");
            push_display(out, e + 1);
            out.extend_from_slice(b",\"path\":");
            push_str(out, &external.path);
            out.extend_from_slice(b",\"target\":");
            match version.external(bundle, f, e + 1) {
                Some((b, tf)) => {
                    let target = &version.bundles()[b];
                    out.extend_from_slice(b"{\"bundle\":");
                    push_str(out, &target.name);
                    out.extend_from_slice(b",\"file\":");
                    push_str(out, &target.files[tf].path);
                    out.push(b'}');
                }
                None => out.extend_from_slice(b"null"),
            }
            out.push(b'}');
        }
        out.extend_from_slice(b"],\"objects\":[");
        for (o, object) in file_index.objects.iter().enumerate() {
            if o > 0 {
                out.push(b',');
            }
            out.extend_from_slice(b"{\"id\":");
            push_quoted_display(out, object.path_id);
            out.extend_from_slice(b",\"class\":");
            push_str(out, &object.class);
            out.extend_from_slice(b",\"class_id\":");
            push_display(out, object.class_id);
            if let Some(script) = version.script(bundle, f, o) {
                out.extend_from_slice(b",\"script\":");
                match script {
                    Some(name) => push_str(out, name),
                    None => out.extend_from_slice(b"null"),
                }
            }
            if let Some(name) = &object.name {
                out.extend_from_slice(b",\"name\":");
                push_str(out, name);
            }
            let container = version.container_paths(bundle, f, object.path_id);
            if !container.is_empty() {
                out.extend_from_slice(b",\"container\":[");
                for (c, path) in container.iter().enumerate() {
                    if c > 0 {
                        out.push(b',');
                    }
                    push_str(out, path);
                }
                out.push(b']');
            }
            out.extend_from_slice(b",\"size\":");
            push_display(out, object.size);
            out.extend_from_slice(b",\"sha256\":");
            push_hash(out, &object.sha256);
            out.extend_from_slice(b",\"fields\":");

            let mut fields = Vec::new();
            let mut writer = ValueWriter {
                out: &mut fields,
                stack: Vec::new(),
                version,
                bundle,
                file: f,
                payloads: &mut *payloads,
            };
            match open.read(o, &mut writer) {
                Ok(()) => out.extend_from_slice(&fields),
                Err(e) if is_object_error(&e) => {
                    out.extend_from_slice(b"null,\"error\":{\"code\":");
                    push_str(out, e.code());
                    out.extend_from_slice(b",\"message\":");
                    push_str(out, &e.to_string());
                    out.extend_from_slice(b"},\"raw\":");
                    let info = &open.file.objects[o];
                    let data = open.file.object_bytes(open.source, info)?;
                    push_payload(out, &data, payloads)?;
                }
                Err(e) => return Err(e),
            }
            out.push(b'}');
        }
        out.extend_from_slice(b"]}");
    }
    out.extend_from_slice(b"]}\n");
    Ok(())
}

/// Whether an error concerns one object rather than the bytes of the file.
fn is_object_error(e: &Error) -> bool {
    matches!(
        e,
        Error::NoLayout { .. }
            | Error::ObjectTruncated { .. }
            | Error::TrailingBytes { .. }
            | Error::InvalidLayout { .. }
            | Error::UnresolvedReferenceType { .. }
            | Error::Malformed { .. }
    )
}

enum Frame {
    Object { first: bool },
    Array { first: bool, map: bool },
    Pair { written: u8 },
}

/// Writes field values as JSON while an object is read.
struct ValueWriter<'a> {
    out: &'a mut Vec<u8>,
    stack: Vec<Frame>,
    version: &'a VersionIndex,
    bundle: usize,
    file: usize,
    payloads: &'a mut dyn Payloads,
}

impl ValueWriter<'_> {
    /// Writes the separator and, inside a structure, the key that precede a value.
    fn before(&mut self, name: &str) {
        match self.stack.last_mut() {
            None => {}
            Some(Frame::Object { first }) => {
                if !std::mem::replace(first, false) {
                    self.out.push(b',');
                }
                push_str(self.out, name);
                self.out.push(b':');
            }
            Some(Frame::Array { first, .. }) => {
                if !std::mem::replace(first, false) {
                    self.out.push(b',');
                }
            }
            Some(Frame::Pair { written }) => {
                if *written > 0 {
                    self.out.push(b',');
                }
                *written += 1;
            }
        }
    }
}

impl Visitor for ValueWriter<'_> {
    fn begin_struct(&mut self, field: &Field<'_>) -> Result<()> {
        let pair = matches!(self.stack.last(), Some(Frame::Array { map: true, .. }));
        self.before(field.name);
        if pair {
            self.out.push(b'[');
            self.stack.push(Frame::Pair { written: 0 });
        } else {
            self.out.push(b'{');
            self.stack.push(Frame::Object { first: true });
        }
        Ok(())
    }

    fn end_struct(&mut self) -> Result<()> {
        match self.stack.pop() {
            Some(Frame::Pair { .. }) => self.out.push(b']'),
            _ => self.out.push(b'}'),
        }
        Ok(())
    }

    fn scalar(&mut self, field: &Field<'_>, value: Scalar) -> Result<()> {
        self.before(field.name);
        let out = &mut *self.out;
        match value {
            Scalar::Bool(0) => out.extend_from_slice(b"false"),
            Scalar::Bool(1) => out.extend_from_slice(b"true"),
            Scalar::Bool(other) => push_display(out, other),
            Scalar::I8(v) => push_display(out, v),
            Scalar::U8(v) => push_display(out, v),
            Scalar::I16(v) => push_display(out, v),
            Scalar::U16(v) => push_display(out, v),
            Scalar::I32(v) => push_display(out, v),
            Scalar::U32(v) => push_display(out, v),
            Scalar::I64(v) => push_quoted_display(out, v),
            Scalar::U64(v) => push_quoted_display(out, v),
            Scalar::F32(v) => push_float(out, v.is_finite(), v.is_nan(), v.is_sign_negative(), v),
            Scalar::F64(v) => push_float(out, v.is_finite(), v.is_nan(), v.is_sign_negative(), v),
        }
        Ok(())
    }

    fn string(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        self.before(field.name);
        match std::str::from_utf8(bytes) {
            Ok(s) => push_str(self.out, s),
            Err(_) => push_payload(self.out, bytes, self.payloads)?,
        }
        Ok(())
    }

    fn bytes(&mut self, field: &Field<'_>, bytes: &[u8]) -> Result<()> {
        self.before(field.name);
        push_payload(self.out, bytes, self.payloads)
    }

    fn begin_array(&mut self, field: &Field<'_>, _len: usize, map: bool) -> Result<bool> {
        self.before(field.name);
        self.out.push(b'[');
        self.stack.push(Frame::Array { first: true, map });
        Ok(true)
    }

    fn end_array(&mut self) -> Result<()> {
        self.stack.pop();
        self.out.push(b']');
        Ok(())
    }

    fn pptr(&mut self, field: &Field<'_>, file_id: i32, path_id: i64) -> Result<()> {
        let target = self
            .version
            .target(self.bundle, self.file, PPtr { file_id, path_id })?;
        self.before(field.name);
        let out = &mut *self.out;
        let here = &self.version.bundles()[self.bundle];
        match target {
            Target::Null => out.extend_from_slice(b"null"),
            Target::Object {
                bundle,
                file,
                path_id,
            } => {
                out.extend_from_slice(b"{\"$ref\":{");
                if bundle != here.name {
                    out.extend_from_slice(b"\"bundle\":");
                    push_str(out, bundle);
                    out.push(b',');
                }
                if bundle != here.name || file != here.files[self.file].path {
                    out.extend_from_slice(b"\"file\":");
                    push_str(out, file);
                    out.push(b',');
                }
                out.extend_from_slice(b"\"id\":");
                push_quoted_display(out, path_id);
                out.extend_from_slice(b"}}");
            }
            Target::External { path, path_id } => {
                out.extend_from_slice(b"{\"$ref\":{\"external\":");
                push_str(out, path);
                out.extend_from_slice(b",\"id\":");
                push_quoted_display(out, path_id);
                out.extend_from_slice(b"}}");
            }
        }
        Ok(())
    }
}

fn push_display(out: &mut Vec<u8>, v: impl std::fmt::Display) {
    let mut s = String::new();
    let _ = write!(s, "{v}");
    out.extend_from_slice(s.as_bytes());
}

fn push_quoted_display(out: &mut Vec<u8>, v: impl std::fmt::Display) {
    out.push(b'"');
    push_display(out, v);
    out.push(b'"');
}

/// Writes a float as the shortest decimal that reads back to the same value.
fn push_float(
    out: &mut Vec<u8>,
    finite: bool,
    nan: bool,
    negative: bool,
    v: impl std::fmt::Display,
) {
    if finite {
        push_display(out, v);
    } else if nan {
        out.extend_from_slice(b"\"NaN\"");
    } else if negative {
        out.extend_from_slice(b"\"-Infinity\"");
    } else {
        out.extend_from_slice(b"\"Infinity\"");
    }
}

/// Writes a JSON string.
fn push_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for c in s.chars() {
        match c {
            '"' => out.extend_from_slice(b"\\\""),
            '\\' => out.extend_from_slice(b"\\\\"),
            '\n' => out.extend_from_slice(b"\\n"),
            '\r' => out.extend_from_slice(b"\\r"),
            '\t' => out.extend_from_slice(b"\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(StrOut(out), "\\u{:04x}", c as u32);
            }
            c => {
                let mut buf = [0; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    out.push(b'"');
}

struct StrOut<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for StrOut<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn push_hash(out: &mut Vec<u8>, hash: &[u8; 32]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    out.push(b'"');
    for b in hash {
        out.push(HEX[usize::from(b >> 4)]);
        out.push(HEX[usize::from(b & 15)]);
    }
    out.push(b'"');
}

/// Writes a byte payload: inline as base64 up to the limit, by hash beyond it.
fn push_payload(out: &mut Vec<u8>, bytes: &[u8], payloads: &mut dyn Payloads) -> Result<()> {
    let sha256: [u8; 32] = Sha256::digest(bytes).into();
    out.extend_from_slice(b"{\"$bytes\":{\"size\":");
    push_display(out, bytes.len());
    out.extend_from_slice(b",\"sha256\":");
    push_hash(out, &sha256);
    if bytes.len() <= INLINE_PAYLOAD_LIMIT {
        out.extend_from_slice(b",\"base64\":\"");
        push_base64(out, bytes);
        out.push(b'"');
    } else {
        payloads.store(&sha256, bytes)?;
    }
    out.extend_from_slice(b"}}");
    Ok(())
}

fn push_base64(out: &mut Vec<u8>, bytes: &[u8]) {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize]);
            } else {
                out.push(b'=');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_standard_alphabet_and_padding() {
        let cases: [(&[u8], &str); 4] =
            [(b"", ""), (b"f", "Zg=="), (b"fo", "Zm8="), (b"foo", "Zm9v")];
        for (input, expected) in cases {
            let mut out = Vec::new();
            push_base64(&mut out, input);
            assert_eq!(out, expected.as_bytes());
        }
    }

    #[test]
    fn strings_escape_quotes_and_controls() {
        let mut out = Vec::new();
        push_str(&mut out, "a\"b\\c\n\u{1}é");
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\"a\\\"b\\\\c\\n\\u0001é\""
        );
    }
}
