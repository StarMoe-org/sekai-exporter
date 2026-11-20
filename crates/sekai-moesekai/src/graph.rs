//! The objects a root reaches, as one JSON document of their stored fields in which a reference
//! is the position of its object in the document.
//!
//! The `scenario-prefab` and `virtual-live-timeline` forms are such documents.

use std::collections::{HashMap, HashSet};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use sekai_convert::fields::string;
use sekai_convert::{
    Asset, Content, Context, ConvertError, Member, Object, ObjectId, PPtr, Value, script_name,
};
use sha2::{Digest, Sha256};

const SHADER: i32 = 48;
const MONO_SCRIPT: i32 = 115;

/// The largest byte payload written inline.
const INLINE_BYTES: usize = 4096;
/// The largest integer a JSON reader holds exactly.
const EXACT_INTEGER: u64 = 1 << 53;
/// The top-level fields that name bytes an object stores in a resource file of its bundle, with
/// the names of their offset, size and path: the `StreamingInfo` of textures and meshes and the
/// `StreamedResource` of audio and video clips.
const STREAMS: &[(&str, [&str; 3])] = &[
    ("m_StreamData", ["offset", "size", "path"]),
    ("m_Resource", ["m_Offset", "m_Size", "m_Source"]),
];

/// The document a form writes of the objects its root reaches.
pub(crate) struct GraphDocument {
    /// The value of the `format` key.
    pub format: &'static str,
    /// The value of the `version` key.
    pub version: u32,
    /// The role of the member that holds the document.
    pub role: &'static str,
    /// What the form makes of each object beside its fields.
    pub media: Media,
}

/// What a form makes of object `n` of its document beside the object's fields.
pub(crate) type Media =
    fn(n: usize, object: &Object, cx: &dyn Context) -> Result<Medium, ConvertError>;

/// A member that holds an object of the document in another form.
pub(crate) struct Medium {
    /// The member, named in the object by its role.
    pub member: Option<Member>,
    /// The fields that the member holds, which the document leaves out.
    pub held: &'static [&'static str],
}

/// The [`Media`] of a document whose objects are their fields alone.
pub(crate) fn fields_only(_: usize, _: &Object, _: &dyn Context) -> Result<Medium, ConvertError> {
    Ok(Medium {
        member: None,
        held: &[],
    })
}

impl GraphDocument {
    /// The asset whose root is `root`: the document, then the members of its objects in object
    /// order, then the byte payloads in the order the document first names them.
    pub(crate) fn asset(&self, root: ObjectId, cx: &dyn Context) -> Result<Asset, ConvertError> {
        let objects = reachable(root, cx)?;
        let positions = objects
            .iter()
            .enumerate()
            .map(|(n, object)| (object.id, n))
            .collect();
        let mut writer = Writer {
            cx,
            media: self.media,
            positions,
            out: String::new(),
            members: Vec::new(),
            payloads: Vec::new(),
            named: HashSet::new(),
        };
        writer.document(self, &objects)?;
        let Writer {
            out,
            members,
            payloads,
            ..
        } = writer;
        let document = file_member(self.role.into(), "json", out.into_bytes());
        let members = std::iter::once(document).chain(members).chain(payloads);
        Ok(Asset {
            members: members.collect(),
        })
    }
}

/// The objects the walk reaches from `root`: the root first, then each object in the order its
/// first reference is met, reading the objects in that order and their fields in stored order.
pub(crate) fn reachable(root: ObjectId, cx: &dyn Context) -> Result<Vec<Object>, ConvertError> {
    let mut order = vec![root];
    let mut listed = HashSet::from([root]);
    let mut objects = Vec::new();
    while let Some(&id) = order.get(objects.len()) {
        let object = cx.object(id)?;
        if !is_listed_by_name(object.class_id) {
            let mut references = Vec::new();
            collect_references(&object.fields, &mut references);
            for reference in references {
                if cx.engine_object(id, reference)?.is_some() {
                    continue;
                }
                if let Some(target) = cx.resolve(id, reference)?
                    && listed.insert(target)
                {
                    order.push(target);
                }
            }
        }
        objects.push(object);
    }
    Ok(objects)
}

/// Whether an object of class `class_id` is listed by its name alone, without its fields and
/// without following its references: a shader or a script.
fn is_listed_by_name(class_id: i32) -> bool {
    matches!(class_id, SHADER | MONO_SCRIPT)
}

/// Appends every reference `value` holds to `references`, in stored order.
fn collect_references(value: &Value, references: &mut Vec<PPtr>) {
    match value {
        Value::PPtr(reference) => references.push(*reference),
        Value::Array(items) => items
            .iter()
            .for_each(|item| collect_references(item, references)),
        Value::Map(pairs) => pairs.iter().for_each(|(key, item)| {
            collect_references(key, references);
            collect_references(item, references);
        }),
        Value::Struct(fields) => fields
            .iter()
            .for_each(|(_, item)| collect_references(item, references)),
        _ => {}
    }
}

/// The document being written, with the members it names.
struct Writer<'c> {
    cx: &'c dyn Context,
    media: Media,
    /// The position of each object in the document.
    positions: HashMap<ObjectId, usize>,
    out: String,
    /// The members of the objects, in object order.
    members: Vec<Member>,
    /// The byte payloads too large to write inline, in the order the document first names them.
    payloads: Vec<Member>,
    /// The hashes of `payloads`.
    named: HashSet<[u8; 32]>,
}

impl Writer<'_> {
    fn document(
        &mut self,
        document: &GraphDocument,
        objects: &[Object],
    ) -> Result<(), ConvertError> {
        self.out.push_str(r#"{"format":"#);
        string(&mut self.out, document.format);
        self.out
            .push_str(&format!(r#","version":{},"objects":["#, document.version));
        for (n, object) in objects.iter().enumerate() {
            if n > 0 {
                self.out.push(',');
            }
            self.object(n, object)?;
        }
        self.out.push_str("]}");
        Ok(())
    }

    /// Writes object `n`.
    fn object(&mut self, n: usize, object: &Object) -> Result<(), ConvertError> {
        self.header(object)?;
        if !is_listed_by_name(object.class_id) {
            let Medium { member, held } = (self.media)(n, object, self.cx)?;
            if let Some(member) = member {
                self.key("member");
                string(&mut self.out, &member.role);
                self.members.push(member);
            }
            self.key("fields");
            self.fields(object, held)?;
        }
        self.out.push('}');
        Ok(())
    }

    /// Writes the keys that identify `object`: its file, ID, class, script and name.
    fn header(&mut self, object: &Object) -> Result<(), ConvertError> {
        let id = object.id;
        self.out.push_str(r#"{"file":"#);
        string(&mut self.out, self.cx.file_name(id)?);
        self.out
            .push_str(&format!(r#","id":"{}","class":"#, id.path_id));
        string(&mut self.out, &object.class);
        self.out
            .push_str(&format!(r#","class_id":{}"#, object.class_id));
        if let Some(script) = &object.script {
            self.key("script");
            string(&mut self.out, script);
        }
        if let Some(name) = name(object) {
            self.key("name");
            string(&mut self.out, &name);
        }
        Ok(())
    }

    /// Writes the stored fields of `object` but `held`.
    fn fields(&mut self, object: &Object, held: &[&str]) -> Result<(), ConvertError> {
        let Value::Struct(fields) = &object.fields else {
            return Err(ConvertError::new(
                "malformed",
                "the fields are not a structure",
            ));
        };
        self.out.push('{');
        let kept = fields
            .iter()
            .filter(|(name, _)| !held.contains(&name.as_str()));
        for (i, (name, value)) in kept.enumerate() {
            if i > 0 {
                self.out.push(',');
            }
            string(&mut self.out, name);
            self.out.push(':');
            match stream(name, value) {
                Some((offset, size, path)) => {
                    let chunk = self.cx.resource(object.id, path, offset, size)?;
                    self.payload(&chunk);
                }
                None => self.value(object.id, value)?,
            }
        }
        self.out.push('}');
        Ok(())
    }

    fn key(&mut self, key: &str) {
        self.out.push(',');
        string(&mut self.out, key);
        self.out.push(':');
    }

    /// Writes `value`, stored in object `from`.
    fn value(&mut self, from: ObjectId, value: &Value) -> Result<(), ConvertError> {
        match value {
            Value::Bool(v) => self.out.push_str(if *v { "true" } else { "false" }),
            Value::Int(v) => integer(&mut self.out, (*v).into()),
            Value::UInt(v) => integer(&mut self.out, (*v).into()),
            Value::Float(v) => float(&mut self.out, *v),
            Value::String(v) => string(&mut self.out, v),
            Value::Bytes(bytes) => self.payload(bytes),
            Value::PPtr(reference) => self.reference(from, *reference)?,
            Value::Array(items) => {
                self.out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.value(from, item)?;
                }
                self.out.push(']');
            }
            Value::Map(pairs) => {
                self.out.push('[');
                for (i, (key, item)) in pairs.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    self.out.push('[');
                    self.value(from, key)?;
                    self.out.push(',');
                    self.value(from, item)?;
                    self.out.push(']');
                }
                self.out.push(']');
            }
            Value::Struct(fields) => {
                self.out.push('{');
                for (i, (name, item)) in fields.iter().enumerate() {
                    if i > 0 {
                        self.out.push(',');
                    }
                    string(&mut self.out, name);
                    self.out.push(':');
                    self.value(from, item)?;
                }
                self.out.push('}');
            }
        }
        Ok(())
    }

    /// Writes a reference stored in object `from`.
    fn reference(&mut self, from: ObjectId, reference: PPtr) -> Result<(), ConvertError> {
        if let Some(engine) = self.cx.engine_object(from, reference)? {
            self.out.push_str(r#"{"$ref":{"external":"#);
            string(&mut self.out, &engine.file);
            self.out
                .push_str(&format!(r#","id":"{}"}}}}"#, engine.path_id));
            return Ok(());
        }
        let Some(target) = self.cx.resolve(from, reference)? else {
            self.out.push_str("null");
            return Ok(());
        };
        let position = self.positions.get(&target).ok_or_else(|| {
            ConvertError::new(
                "malformed",
                format!("object {} was not reached by the walk", target.path_id),
            )
        })?;
        self.out.push_str(&format!(r#"{{"$ref":{position}}}"#));
        Ok(())
    }

    /// Writes a byte payload: inline up to [`INLINE_BYTES`], else as a member named by its hash.
    fn payload(&mut self, bytes: &[u8]) {
        let sha256: [u8; 32] = Sha256::digest(bytes).into();
        let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
        let size = bytes.len();
        self.out
            .push_str(&format!(r#"{{"$bytes":{{"size":{size},"sha256":"{hex}","#));
        if size <= INLINE_BYTES {
            let base64 = STANDARD.encode(bytes);
            self.out.push_str(&format!(r#""base64":"{base64}"}}}}"#));
            return;
        }
        let role = format!("bytes/{hex}");
        self.out.push_str(&format!(r#""member":"{role}"}}}}"#));
        if self.named.insert(sha256) {
            self.payloads.push(file_member(role, "bin", bytes.to_vec()));
        }
    }
}

/// The name an object is listed under: the parsed name of a shader, the full name of a script,
/// and otherwise a nonempty top-level `m_Name`.
fn name(object: &Object) -> Option<String> {
    let fields = &object.fields;
    let text = |value: Option<&Value>| {
        let text = value?.as_str().filter(|s| !s.is_empty())?;
        Some(text.to_owned())
    };
    match object.class_id {
        SHADER => text(fields.get("m_ParsedForm")?.get("m_Name")),
        MONO_SCRIPT => {
            let part = |name: &str| fields.get(name)?.as_str();
            let assembly = part("m_AssemblyName")?;
            Some(script_name(
                assembly,
                part("m_Namespace")?,
                part("m_ClassName")?,
            ))
        }
        _ => text(fields.get("m_Name")),
    }
}

/// The offset, size and path of the bytes the top-level field `name` names in a resource file,
/// when it is one of [`STREAMS`] with a path.
fn stream<'v>(name: &str, value: &'v Value) -> Option<(u64, u64, &'v str)> {
    let (_, [offset, size, path]) = STREAMS.iter().find(|(field, _)| *field == name)?;
    let path = value.get(path)?.as_str().filter(|path| !path.is_empty())?;
    Some((
        value.get(offset)?.as_u64()?,
        value.get(size)?.as_u64()?,
        path,
    ))
}

/// Writes an integer: a number, or a string beyond what JSON readers hold exactly.
fn integer(out: &mut String, value: i128) {
    if value.unsigned_abs() > u128::from(EXACT_INTEGER) {
        out.push_str(&format!(r#""{value}""#));
    } else {
        out.push_str(&value.to_string());
    }
}

/// Writes a floating-point number: the shortest decimal of the 32-bit float it is when it is
/// one, and the names of the values JSON cannot hold as strings.
fn float(out: &mut String, value: f64) {
    if value.is_nan() {
        out.push_str(r#""NaN""#);
    } else if value.is_infinite() {
        out.push_str(if value > 0.0 {
            r#""Infinity""#
        } else {
            r#""-Infinity""#
        });
    } else {
        let single = value as f32;
        if f64::from(single) == value {
            out.push_str(&single.to_string());
        } else {
            out.push_str(&value.to_string());
        }
    }
}

/// A member of finished bytes.
pub(crate) fn file_member(role: String, extension: &str, bytes: Vec<u8>) -> Member {
    Member {
        role,
        content: Content::File {
            extension: extension.into(),
            bytes,
        },
    }
}
