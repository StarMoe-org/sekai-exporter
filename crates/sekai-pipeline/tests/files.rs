//! Assets whose roots are files of a bundle that are not serialized files.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_convert::{
    Asset, Content, Context, ConvertError, Converter, FileConverter, Member, ObjectId, ObjectIndex,
};
use sekai_format::{AssetDocument, SourceResource};
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::stream::{self, Entry, InBundle, Inputs, Segment, Stream, Streamed};
use sekai_pipeline::{BundleInput, MemoryDerivations, NoBundleIndexes, Options};
use sekai_unity::{NoBlockCache, NoClasses, StoredFile, store_files};
use sha2::{Digest, Sha256};

/// The bundle of files the tests evaluate.
const BUNDLE: &str = "files";

/// A form whose assets are the text files of a bundle, as they are.
struct Text;

impl Converter for Text {
    fn name(&self) -> &'static str {
        "text-file"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, _: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
        false
    }

    fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        Ok(None)
    }

    fn files(&self) -> Option<&dyn FileConverter> {
        Some(self)
    }
}

impl FileConverter for Text {
    fn is_file_root(&self, path: &str) -> bool {
        path.ends_with(".txt")
    }

    fn convert_file(&self, bytes: &[u8]) -> Result<Option<Asset>, ConvertError> {
        let member = Member {
            role: "text".to_owned(),
            content: Content::File {
                extension: "txt".to_owned(),
                bytes: bytes.to_vec(),
            },
        };
        Ok(Some(Asset {
            members: vec![member],
        }))
    }
}

/// The one bundle [`BUNDLE`], which holds a text file with the bytes `text` and a file of another
/// kind.
struct Files(Vec<u8>);

impl Files {
    fn new(text: &[u8]) -> Self {
        let file = |path: &str, bytes: &[u8]| StoredFile {
            path: path.to_owned(),
            bytes: bytes.to_vec(),
            serialized: false,
        };
        let files = [file("notes/a.txt", text), file("notes/b.bin", b"other")];
        Self(store_files(&files).unwrap())
    }

    fn manifest(&self) -> Vec<Entry> {
        vec![Entry {
            name: BUNDLE.to_owned(),
            size: self.0.len() as u64,
            content: None,
            dependencies: Vec::new(),
        }]
    }
}

impl Inputs for Files {
    fn open(&self, name: &str) -> io::Result<BundleInput> {
        assert_eq!(name, BUNDLE);
        Ok(BundleInput {
            name: name.to_owned(),
            sha256: Sha256::digest(&self.0).into(),
            source: Arc::new(Arc::<[u8]>::from(self.0.as_slice())),
        })
    }
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sekai-files-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The evaluation of [`Files`] holding `text`, with the ledger of its bundle, into the store at
/// `root`.
fn evaluate(root: &Path, text: &[u8]) -> (Streamed, LocalStore) {
    let store = LocalStore::open(root).unwrap();
    let converters: Vec<Box<dyn Converter>> = vec![Box::new(Text)];
    let derivations = MemoryDerivations::default();
    let stream = Stream {
        converters: &converters,
        naming: &InBundle,
        store: &store,
        derivations: &derivations,
        indexes: &NoBundleIndexes,
        cache: &NoBlockCache,
        classes: &NoClasses,
        recorder: &NoRecorder,
        fusion: None,
        options: Options::threads(1),
        segment: Segment::default(),
        ledger: Some(&[]),
        limit: None,
    };
    let files = Files::new(text);
    let streamed = stream::evaluate(&files.manifest(), &files, &stream).unwrap();
    (streamed, store)
}

fn document(store: &LocalStore, streamed: &Streamed) -> AssetDocument {
    let output = &streamed.evaluation.outputs[0];
    AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap()
}

#[test]
fn a_file_of_a_bundle_is_the_root_of_an_asset_named_by_its_path() {
    let (streamed, store) = evaluate(&scratch("named"), b"hello");
    assert!(streamed.errors.is_empty(), "{:?}", streamed.errors);
    let names: Vec<_> = streamed
        .evaluation
        .outputs
        .iter()
        .map(|o| &o.name)
        .collect();
    assert_eq!(names, ["files/notes/a.txt"]);
    assert_eq!(streamed.evaluation.outputs[0].bundle, BUNDLE);

    let document = document(&store, &streamed);
    assert!(document.sources.is_empty());
    let resource = SourceResource {
        bundle: BUNDLE.to_owned(),
        file: "notes/a.txt".to_owned(),
    };
    assert_eq!(document.resources, [resource]);
    assert_eq!(document.bundle(), Some(BUNDLE));
    let member = &document.members[0];
    let hello: String = Sha256::digest(b"hello")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!((member.sha256.as_str(), member.size), (hello.as_str(), 5));
}

#[test]
fn the_ledger_counts_the_file_an_asset_read() {
    let (streamed, _) = evaluate(&scratch("ledger"), b"hello");
    let ledger = &streamed.ledgers[0];
    assert_eq!((ledger.resources, ledger.resources_read), (2, 1));
}

#[test]
fn the_fingerprint_follows_the_bytes_of_the_file() {
    let fingerprint = |name: &str, text: &[u8]| {
        let (streamed, store) = evaluate(&scratch(name), text);
        document(&store, &streamed).fingerprint
    };
    assert_eq!(
        fingerprint("same-a", b"hello"),
        fingerprint("same-b", b"hello")
    );
    assert_ne!(
        fingerprint("same-a", b"hello"),
        fingerprint("other", b"world")
    );
}
