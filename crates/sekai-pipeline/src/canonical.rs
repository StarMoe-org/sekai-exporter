//! Canonical files: the encoding of the members of an asset, and the records under which content
//! that was encoded before keeps its file.
//!
//! An encoding has a fingerprint of its own: the encoder and what the content decodes to. A
//! conversion that runs again and decodes the same content finds the file of that content and
//! does not encode it, and members of one asset with the same content are encoded once. The
//! version of an encoder is part of the fingerprint only when the encoder loses information;
//! the files of a lossless encoder all decode to the same content, so the file of any version
//! stands for the others.

use std::borrow::Cow;
use std::collections::HashMap;
use std::io;

use sekai_catalog::{Derivations, Derived, Key, Store};
use sekai_convert::{Asset, Content, Converter, Encoders, Member, Pcm};
use sekai_encode::{EncodeError, exr, mp3, mp4, png};
use sekai_format::{AssetError, MemberFile};
use sha2::{Digest, Sha256};

use crate::eval::hex;
use crate::metrics::Recorder;

/// Content written as a canonical file.
pub(crate) struct Encoded<'a> {
    pub(crate) bytes: Cow<'a, [u8]>,
    pub(crate) extension: &'a str,
    pub(crate) media_type: &'static str,
}

pub(crate) fn encode(content: &Content) -> Result<Encoded<'_>, EncodeError> {
    let (bytes, extension): (Cow<'_, [u8]>, &str) = match content {
        Content::Image(surface) => (png::encode(surface)?.into(), "png"),
        Content::HdrImage(surface) => (exr::encode(surface)?.into(), "exr"),
        Content::Audio(sound) => (mp3::encode(sound)?.into(), "mp3"),
        Content::Video(movie) => (mp4::encode(movie)?.into(), "mp4"),
        Content::File { extension, bytes } => (bytes.into(), extension),
    };
    Ok(Encoded {
        bytes,
        extension,
        media_type: media_type(extension),
    })
}

fn media_type(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "exr" => "image/x-exr",
        "mp3" => "audio/mpeg",
        "mp4" => "video/mp4",
        "json" => "application/json",
        "txt" => "text/plain",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "glb" => "model/gltf-binary",
        _ => "application/octet-stream",
    }
}

/// Feeds `values` to `hash` as the bytes `bytes` gives each.
fn update<T: Copy, const N: usize>(hash: &mut Sha256, values: &[T], bytes: fn(T) -> [u8; N]) {
    for chunk in values.chunks(4096) {
        let buffer: Vec<u8> = chunk.iter().flat_map(|&value| bytes(value)).collect();
        hash.update(&buffer);
    }
}

fn update_audio(hash: &mut Sha256, pcm: &Pcm) {
    hash.update(format!("audio {} {}\n", pcm.sample_rate(), pcm.channels()));
    update(hash, pcm.samples(), i16::to_le_bytes);
}

/// The fingerprint of the canonical file of `content`: its encoder and what it decodes to.
/// `None` for a finished file, which is stored as it is.
fn encoding(content: &Content) -> Option<[u8; 32]> {
    let mut hash = Sha256::new();
    hash.update("encoding 1\n");
    match content {
        Content::Image(surface) => {
            hash.update(format!("encoder {}\n", png::NAME));
            hash.update(format!("image {} {}\n", surface.width(), surface.height()));
            hash.update(surface.pixels());
        }
        Content::HdrImage(surface) => {
            hash.update(format!("encoder {}\n", exr::NAME));
            hash.update(format!("image {} {}\n", surface.width(), surface.height()));
            update(&mut hash, surface.pixels(), u16::to_le_bytes);
        }
        Content::Audio(sound) => {
            hash.update(format!("encoder {} {}\n", mp3::NAME, mp3::VERSION));
            sound.identify(|bytes| hash.update(bytes));
        }
        Content::Video(movie) => {
            hash.update(format!("encoder {} {}\n", mp4::NAME, mp4::VERSION));
            hash.update(format!(
                "video {} {} {}/{} {}\n",
                movie.width,
                movie.height,
                movie.framerate_n,
                movie.framerate_d,
                movie.stream.len()
            ));
            hash.update(&movie.stream);
            if let Some(pcm) = &movie.audio {
                update_audio(&mut hash, pcm);
            }
        }
        Content::File { .. } => return None,
    }
    Some(hash.finalize().into())
}

/// The encoder of `content`, when `declared` does not list it.
fn undeclared(declared: Encoders, content: &Content) -> Option<&'static str> {
    match content {
        Content::Image(_) if !declared.image => Some(png::NAME),
        Content::HdrImage(_) if !declared.hdr_image => Some(exr::NAME),
        Content::Audio(_) if !declared.audio => Some(mp3::NAME),
        Content::Video(_) if !declared.video => Some(mp4::NAME),
        _ => None,
    }
}

/// The canonical file of a member.
pub(crate) enum Canonical<'a> {
    /// Encoded now, and recorded under the fingerprint `record` once it is stored.
    New {
        file: Encoded<'a>,
        record: Option<[u8; 32]>,
    },
    /// The file that an earlier encoding of the same content stored.
    Stored { key: Key, size: u64 },
    /// The file of the member at this earlier position of the same asset, whose content is the
    /// same.
    Same(usize),
}

/// Where canonical files are stored, recorded and reported.
#[derive(Clone, Copy)]
pub(crate) struct Files<'a> {
    pub(crate) store: &'a dyn Store,
    /// The records of encodings. Without them every member is encoded and nothing is recorded.
    pub(crate) records: Option<&'a dyn Derivations>,
    pub(crate) recorder: &'a dyn Recorder,
}

impl Files<'_> {
    /// The canonical file of each member of `asset`, with its role, in order, or the error of the
    /// first member that has none. A member whose encoder the converter does not declare fails
    /// the asset before anything is encoded. `name` is the name of the asset, for the reports.
    pub(crate) fn encode<'a>(
        &self,
        converter: &dyn Converter,
        name: &str,
        asset: &'a Asset,
    ) -> io::Result<Result<Vec<(&'a str, Canonical<'a>)>, AssetError>> {
        let declared = converter.encoders();
        for member in &asset.members {
            if let Some(encoder) = undeclared(declared, &member.content) {
                return Ok(Err(AssetError {
                    code: "undeclared_encoder".to_owned(),
                    message: format!(
                        "the form {} encodes the member {} with {encoder}, which it does not declare",
                        converter.name(),
                        member.role
                    ),
                }));
            }
        }
        let mut files = Vec::with_capacity(asset.members.len());
        let mut first = HashMap::new();
        for (position, member) in asset.members.iter().enumerate() {
            let fingerprint = encoding(&member.content);
            let earlier = fingerprint.and_then(|fingerprint| first.get(&fingerprint));
            let file = match earlier {
                Some(&earlier) => Canonical::Same(earlier),
                None => match self.canonical(converter, name, member, fingerprint)? {
                    Ok(file) => file,
                    Err(error) => return Ok(Err(error)),
                },
            };
            if let Some(fingerprint) = fingerprint {
                first.entry(fingerprint).or_insert(position);
            }
            files.push((member.role.as_str(), file));
        }
        Ok(Ok(files))
    }

    /// The canonical file of `member`, whose encoding has the fingerprint `fingerprint`.
    fn canonical<'a>(
        &self,
        converter: &dyn Converter,
        name: &str,
        member: &'a Member,
        fingerprint: Option<[u8; 32]>,
    ) -> io::Result<Result<Canonical<'a>, AssetError>> {
        let record = self.records.and(fingerprint);
        if let Some(stored) = self.recorded(record.as_ref())? {
            return Ok(Ok(stored));
        }
        let mut span = self.recorder.span("encode", converter.name(), name);
        Ok(match encode(&member.content) {
            Ok(file) => {
                span.bytes(0, file.bytes.len() as u64);
                Ok(Canonical::New { file, record })
            }
            Err(e) => Err(AssetError {
                code: match e {
                    EncodeError::Unavailable { .. } => "encoder_unavailable",
                    _ => "encode_failed",
                }
                .to_owned(),
                message: e.to_string(),
            }),
        })
    }

    /// The file recorded for the encoding `record`, when the store still has it.
    fn recorded<'a>(&self, record: Option<&[u8; 32]>) -> io::Result<Option<Canonical<'a>>> {
        let (Some(records), Some(record)) = (self.records, record) else {
            return Ok(None);
        };
        let Some(Derived::Document(key)) = records.get(record)? else {
            return Ok(None);
        };
        let size = self.store.size(&key)?;
        Ok(size.map(|size| Canonical::Stored { key, size }))
    }

    /// Stores the new files among `files` and returns the member files, in order. `name` is the
    /// name of the asset, for the reports.
    pub(crate) fn store(
        &self,
        name: &str,
        files: Vec<(&str, Canonical<'_>)>,
    ) -> io::Result<Vec<MemberFile>> {
        let mut stored: Vec<MemberFile> = Vec::with_capacity(files.len());
        for (role, file) in files {
            let role = role.to_owned();
            let member = match file {
                Canonical::New { file, record } => {
                    let key = self.put(name, &file, record)?;
                    member_file(role, key, file.bytes.len() as u64)
                }
                Canonical::Stored { key, size } => member_file(role, key, size),
                Canonical::Same(earlier) => MemberFile {
                    role,
                    ..stored[earlier].clone()
                },
            };
            stored.push(member);
        }
        Ok(stored)
    }

    fn put(&self, name: &str, file: &Encoded<'_>, record: Option<[u8; 32]>) -> io::Result<Key> {
        let mut span = self.recorder.span("store", file.extension, name);
        span.bytes(0, file.bytes.len() as u64);
        let key = self.store.put(&file.bytes, file.extension)?;
        if let (Some(records), Some(record)) = (self.records, record) {
            records.put(record, Derived::Document(key.clone()))?;
        }
        Ok(key)
    }
}

/// The member `role` of the file `key`, of `size` bytes.
fn member_file(role: String, key: Key, size: u64) -> MemberFile {
    MemberFile {
        role,
        sha256: hex(&key.sha256),
        size,
        media_type: media_type(&key.extension).to_owned(),
        extension: key.extension,
    }
}

#[cfg(test)]
mod tests {
    use sekai_catalog::{MemoryDerivations, MemoryStore};
    use sekai_convert::{Context, ConvertError, Movie, ObjectId, ObjectIndex, Surface};

    use super::*;
    use crate::metrics::NoRecorder;

    struct Images;

    impl Converter for Images {
        fn name(&self) -> &'static str {
            "images"
        }

        fn version(&self) -> u32 {
            1
        }

        fn encoders(&self) -> Encoders {
            Encoders::IMAGE
        }

        fn is_root(&self, _: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
            false
        }

        fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
            Ok(None)
        }
    }

    fn asset(content: Content) -> Asset {
        Asset {
            members: vec![Member {
                role: "image".to_owned(),
                content,
            }],
        }
    }

    fn image(shade: u8) -> Content {
        Content::Image(Surface::new(2, 1, vec![shade; 8]).unwrap())
    }

    #[test]
    fn content_that_was_encoded_keeps_its_file() {
        let (store, records) = (MemoryStore::default(), MemoryDerivations::default());
        let files = Files {
            store: &store,
            records: Some(&records),
            recorder: &NoRecorder,
        };
        let member_files = |content| {
            let asset = asset(content);
            let encoded = files.encode(&Images, "name", &asset).unwrap().unwrap();
            let again = matches!(encoded[0].1, Canonical::Stored { .. });
            (files.store("name", encoded).unwrap(), again)
        };
        let (first, again) = member_files(image(1));
        assert!(!again);
        let (second, again) = member_files(image(1));
        assert!(again);
        assert_eq!(first, second);
        let (other, again) = member_files(image(2));
        assert!(!again);
        assert_ne!(first, other);
    }

    #[test]
    fn members_with_the_same_content_are_encoded_once() {
        let store = MemoryStore::default();
        let files = Files {
            store: &store,
            records: None,
            recorder: &NoRecorder,
        };
        let member = |role: &str, content| Member {
            role: role.to_owned(),
            content,
        };
        let asset = Asset {
            members: vec![
                member("a", image(1)),
                member("b", image(2)),
                member("c", image(1)),
            ],
        };
        let encoded = files.encode(&Images, "name", &asset).unwrap().unwrap();
        assert!(matches!(encoded[1].1, Canonical::New { .. }));
        assert!(matches!(encoded[2].1, Canonical::Same(0)));
        let stored = files.store("name", encoded).unwrap();
        let roles: Vec<&str> = stored.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["a", "b", "c"]);
        assert_eq!(stored[2].sha256, stored[0].sha256);
        assert_ne!(stored[1].sha256, stored[0].sha256);
    }

    #[test]
    fn a_recorded_file_that_the_store_lost_is_encoded_again() {
        let (store, records) = (MemoryStore::default(), MemoryDerivations::default());
        let files = Files {
            store: &store,
            records: Some(&records),
            recorder: &NoRecorder,
        };
        let asset = asset(image(1));
        let encoded = files.encode(&Images, "name", &asset).unwrap().unwrap();
        files.store("name", encoded).unwrap();
        store.drain();
        let encoded = files.encode(&Images, "name", &asset).unwrap().unwrap();
        assert!(matches!(encoded[0].1, Canonical::New { .. }));
    }

    #[test]
    fn the_fingerprint_of_an_encoding_follows_the_content() {
        let of = |content| encoding(&content).unwrap();
        assert_eq!(of(image(1)), of(image(1)));
        assert_ne!(of(image(1)), of(image(2)));
        let wide = Content::Image(Surface::new(1, 2, vec![1; 8]).unwrap());
        assert_ne!(of(image(1)), of(wide));
        let file = Content::File {
            extension: "json".to_owned(),
            bytes: Vec::new(),
        };
        assert_eq!(encoding(&file), None);
    }

    #[test]
    fn a_member_whose_encoder_is_not_declared_fails_the_asset() {
        let store = MemoryStore::default();
        let files = Files {
            store: &store,
            records: None,
            recorder: &NoRecorder,
        };
        let movie = Movie {
            stream: Vec::new(),
            width: 2,
            height: 2,
            framerate_n: 30,
            framerate_d: 1,
            audio: None,
        };
        let asset = asset(Content::Video(movie));
        let error = files
            .encode(&Images, "name", &asset)
            .unwrap()
            .err()
            .unwrap();
        assert_eq!(error.code, "undeclared_encoder");
    }
}
