//! The stored form of what indexing a bundle produced.
//!
//! A record depends only on the bytes of the bundle, so it is kept by their SHA-256. [`FORMAT`]
//! changes whenever the meaning or the layout of a record changes; a record of another format, or
//! one that cannot be read, is ignored and the bundle is indexed again.

use sekai_unity::index::{FileIndex, ObjectIndex, PPtr};
use sekai_unity::serialized::External;

const FORMAT: u32 = 2;

/// The streamed `.resS`/`.resource` slice referenced by one object, and the SHA-256 of its bytes
/// when that slice is readable in the object's bundle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StreamDigest {
    /// The object's path ID, in ascending order within the file.
    pub(crate) path_id: i64,
    /// The resource entry name inside the bundle.
    pub(crate) file: String,
    /// The byte offset inside the resource entry.
    pub(crate) offset: u64,
    /// The byte length inside the resource entry.
    pub(crate) size: u64,
    /// The SHA-256 of `entry[offset..offset + size]`, or `None` if missing or out of bounds.
    pub(crate) sha256: Option<[u8; 32]>,
}

/// What indexing a bundle produced.
#[derive(Debug)]
pub(crate) struct IndexRecord {
    /// The index of each serialized file, in directory order.
    pub(crate) files: Vec<FileIndex>,
    /// The SHA-256 of each resource entry that can be read, in directory order.
    pub(crate) resources: Vec<[u8; 32]>,
    /// Streamed resource slices for each serialized file, sorted by `path_id`.
    pub(crate) streams: Vec<Vec<StreamDigest>>,
}

fn put_u32(out: &mut Vec<u8>, value: usize) {
    out.extend_from_slice(
        &u32::try_from(value)
            .expect("a count fits 32 bits")
            .to_le_bytes(),
    );
}

fn put_str(out: &mut Vec<u8>, text: &str) {
    put_u32(out, text.len());
    out.extend_from_slice(text.as_bytes());
}

fn put_option_str(out: &mut Vec<u8>, text: Option<&str>) {
    out.push(u8::from(text.is_some()));
    if let Some(text) = text {
        put_str(out, text);
    }
}

fn put_pptr(out: &mut Vec<u8>, p: PPtr) {
    out.extend_from_slice(&p.file_id.to_le_bytes());
    out.extend_from_slice(&p.path_id.to_le_bytes());
}

fn put_object(out: &mut Vec<u8>, object: &ObjectIndex) {
    out.extend_from_slice(&object.path_id.to_le_bytes());
    out.extend_from_slice(&object.class_id.to_le_bytes());
    put_str(out, &object.class);
    put_option_str(out, object.name.as_deref());
    out.push(u8::from(object.script.is_some()));
    if let Some(script) = object.script {
        put_pptr(out, script);
    }
    put_option_str(out, object.script_name.as_deref());
    put_u32(out, object.references.len());
    for reference in &object.references {
        put_pptr(out, *reference);
    }
    out.extend_from_slice(&object.size.to_le_bytes());
    out.extend_from_slice(&object.sha256);
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn bytes(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, tail) = self.0.split_at_checked(n)?;
        self.0 = tail;
        Some(head)
    }

    fn array<const N: usize>(&mut self) -> Option<[u8; N]> {
        self.bytes(N)?.try_into().ok()
    }

    fn u32(&mut self) -> Option<u32> {
        self.array().map(u32::from_le_bytes)
    }

    fn i32(&mut self) -> Option<i32> {
        self.array().map(i32::from_le_bytes)
    }

    fn u64(&mut self) -> Option<u64> {
        self.array().map(u64::from_le_bytes)
    }

    fn i64(&mut self) -> Option<i64> {
        self.array().map(i64::from_le_bytes)
    }

    fn count(&mut self) -> Option<usize> {
        usize::try_from(self.u32()?).ok()
    }

    fn flag(&mut self) -> Option<bool> {
        match self.array::<1>()? {
            [0] => Some(false),
            [1] => Some(true),
            _ => None,
        }
    }

    fn string(&mut self) -> Option<String> {
        let n = self.count()?;
        String::from_utf8(self.bytes(n)?.to_vec()).ok()
    }

    fn option_string(&mut self) -> Option<Option<String>> {
        if self.flag()? {
            self.string().map(Some)
        } else {
            Some(None)
        }
    }

    fn pptr(&mut self) -> Option<PPtr> {
        Some(PPtr {
            file_id: self.i32()?,
            path_id: self.i64()?,
        })
    }

    /// `n` items read by `item`. The capacity is not taken from `n`, which may be damaged.
    fn list<T>(&mut self, mut item: impl FnMut(&mut Self) -> Option<T>) -> Option<Vec<T>> {
        let n = self.count()?;
        let mut items = Vec::new();
        for _ in 0..n {
            items.push(item(self)?);
        }
        Some(items)
    }

    fn object(&mut self) -> Option<ObjectIndex> {
        Some(ObjectIndex {
            path_id: self.i64()?,
            class_id: self.i32()?,
            class: self.string()?,
            name: self.option_string()?,
            script: if self.flag()? {
                Some(self.pptr()?)
            } else {
                None
            },
            script_name: self.option_string()?,
            references: self.list(Self::pptr)?,
            size: self.u32()?,
            sha256: self.array()?,
            error: None,
        })
    }

    fn file(&mut self) -> Option<FileIndex> {
        Some(FileIndex {
            path: self.string()?,
            externals: self.list(|r| {
                Some(External {
                    path: r.string()?,
                    guid: r.array()?,
                    kind: r.i32()?,
                })
            })?,
            objects: self.list(Self::object)?,
            container: self.list(|r| Some((r.string()?, r.pptr()?)))?,
        })
    }

    fn stream_digest(&mut self) -> Option<StreamDigest> {
        Some(StreamDigest {
            path_id: self.i64()?,
            file: self.string()?,
            offset: self.u64()?,
            size: self.u64()?,
            sha256: if self.flag()? {
                Some(self.array()?)
            } else {
                None
            },
        })
    }
}

impl IndexRecord {
    /// The stored form of the record. A bundle with an object that could not be read is indexed
    /// again each time, so that the error comes from the reader that is running.
    pub(crate) fn encode(&self) -> Option<Vec<u8>> {
        let objects = || self.files.iter().flat_map(|file| &file.objects);
        if objects().any(|object| object.error.is_some()) {
            return None;
        }
        let mut out = Vec::new();
        out.extend_from_slice(&FORMAT.to_le_bytes());
        put_u32(&mut out, self.resources.len());
        for resource in &self.resources {
            out.extend_from_slice(resource);
        }
        put_u32(&mut out, self.files.len());
        for file in &self.files {
            put_str(&mut out, &file.path);
            put_u32(&mut out, file.externals.len());
            for external in &file.externals {
                put_str(&mut out, &external.path);
                out.extend_from_slice(&external.guid);
                out.extend_from_slice(&external.kind.to_le_bytes());
            }
            put_u32(&mut out, file.objects.len());
            for object in &file.objects {
                put_object(&mut out, object);
            }
            put_u32(&mut out, file.container.len());
            for (path, reference) in &file.container {
                put_str(&mut out, path);
                put_pptr(&mut out, *reference);
            }
        }
        put_u32(&mut out, self.streams.len());
        for list in &self.streams {
            put_u32(&mut out, list.len());
            for s in list {
                out.extend_from_slice(&s.path_id.to_le_bytes());
                put_str(&mut out, &s.file);
                out.extend_from_slice(&s.offset.to_le_bytes());
                out.extend_from_slice(&s.size.to_le_bytes());
                out.push(u8::from(s.sha256.is_some()));
                if let Some(sha) = s.sha256 {
                    out.extend_from_slice(&sha);
                }
            }
        }
        Some(out)
    }

    /// Reads a stored record, or `None` if it is of another format or damaged.
    pub(crate) fn decode(bytes: &[u8]) -> Option<Self> {
        let mut reader = Reader(bytes);
        if reader.u32()? != FORMAT {
            return None;
        }
        let resources = reader.list(Reader::array)?;
        let files = reader.list(Reader::file)?;
        let streams = reader.list(|r| r.list(Reader::stream_digest))?;
        if streams.len() != files.len() {
            return None;
        }
        let record = Self {
            resources,
            files,
            streams,
        };
        reader.0.is_empty().then_some(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sekai_unity::Error;

    fn object(error: Option<Error>) -> ObjectIndex {
        ObjectIndex {
            path_id: -7,
            class_id: 28,
            class: "Texture2D".to_owned(),
            name: Some("名前".to_owned()),
            script: Some(PPtr {
                file_id: 1,
                path_id: -2,
            }),
            script_name: None,
            references: vec![PPtr {
                file_id: 0,
                path_id: 9,
            }],
            size: 12,
            sha256: [3; 32],
            error,
        }
    }

    fn record(error: Option<Error>) -> IndexRecord {
        IndexRecord {
            resources: vec![[5; 32]],
            files: vec![FileIndex {
                path: "CAB-a".to_owned(),
                externals: vec![External {
                    path: "archive:/CAB-b/CAB-b".to_owned(),
                    guid: [1; 16],
                    kind: 2,
                }],
                objects: vec![object(error)],
                container: vec![(
                    "assets/a.png".to_owned(),
                    PPtr {
                        file_id: 0,
                        path_id: -7,
                    },
                )],
            }],
            streams: vec![vec![StreamDigest {
                path_id: -7,
                file: "CAB-a.resS".to_owned(),
                offset: 64,
                size: 128,
                sha256: Some([9; 32]),
            }]],
        }
    }

    #[test]
    fn a_record_survives_its_stored_form() {
        let original = record(None);
        let decoded = IndexRecord::decode(&original.encode().unwrap()).unwrap();
        assert_eq!(format!("{decoded:?}"), format!("{original:?}"));
    }

    #[test]
    fn damaged_records_are_not_read() {
        let bytes = record(None).encode().unwrap();
        assert!(IndexRecord::decode(&bytes[..bytes.len() - 1]).is_none());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(IndexRecord::decode(&longer).is_none());
        let mut other_format = bytes;
        other_format[0] ^= 0xff;
        assert!(IndexRecord::decode(&other_format).is_none());
    }

    #[test]
    fn a_bundle_with_an_unreadable_object_is_not_stored() {
        let error = Error::Truncated {
            what: "a test",
            offset: 0,
            needed: 1,
            available: 0,
        };
        assert!(record(Some(error)).encode().is_none());
    }
}
