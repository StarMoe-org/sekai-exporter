//! Object documents of the fixture bundles, which `tools/unity-fixtures` builds with the Unity
//! editor in three variants: uncompressed, LZ4 and LZMA.
//!
//! The expected documents in `tests/fixtures/expected` were checked field by field against
//! UnityPy with `tools/compare-unitypy`. Set `UPDATE_EXPECTED=1` to rewrite them after a change
//! that is meant to change the documents, and check them against UnityPy again.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_unity::document::{EntryRecord, Payloads, write_bundle};
use sekai_unity::index::{BundleIndex, FileLayouts, OpenFile, VersionIndex, index_file};
use sekai_unity::{
    BlockCache, BoundedBlockCache, Bundle, ByteSource, NoBlockCache, SerializedFile,
};
use sha2::{Digest, Sha256};

const BUNDLES: [&str; 2] = ["fixture_data", "fixture_scene"];
const VARIANTS: [&str; 3] = ["uncompressed", "lz4", "lzma"];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

#[derive(Default)]
struct Collected(BTreeMap<[u8; 32], Vec<u8>>);

impl Payloads for Collected {
    fn store(&mut self, sha256: &[u8; 32], bytes: &[u8]) -> sekai_unity::Result<()> {
        assert_eq!(Sha256::digest(bytes).as_slice(), sha256);
        self.0.insert(*sha256, bytes.to_vec());
        Ok(())
    }
}

struct Loaded {
    bundle: Bundle,
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

impl Loaded {
    fn new(path: &Path, cache: &dyn BlockCache) -> Self {
        let bytes = std::fs::read(path).unwrap();
        let id = Sha256::digest(&bytes).into();
        let bundle = Bundle::open(Arc::new(Arc::<[u8]>::from(bytes)), id).unwrap();
        let files = (0..bundle.entries().len())
            .filter(|&i| bundle.entries()[i].is_serialized())
            .map(|i| {
                let file = SerializedFile::parse(&bundle.entry(i, cache)).unwrap();
                let layouts = FileLayouts::new(&file);
                (i, file, layouts)
            })
            .collect();
        Self { bundle, files }
    }

    /// Calls `f` with the bundle's serialized files opened for reading.
    fn with_files<R>(&self, cache: &dyn BlockCache, f: impl FnOnce(&[OpenFile<'_>]) -> R) -> R {
        let sources: Vec<_> = self
            .files
            .iter()
            .map(|(i, _, _)| self.bundle.entry(*i, cache))
            .collect();
        let open: Vec<OpenFile<'_>> = self
            .files
            .iter()
            .zip(&sources)
            .map(|((i, file, layouts), source)| OpenFile {
                path: &self.bundle.entries()[*i].path,
                file,
                source,
                layouts,
            })
            .collect();
        f(&open)
    }

    fn entries(&self, cache: &dyn BlockCache) -> Vec<EntryRecord> {
        (0..self.bundle.entries().len())
            .map(|i| {
                let entry = &self.bundle.entries()[i];
                let source = self.bundle.entry(i, cache);
                let bytes = source.read(0, entry.size as usize).unwrap();
                EntryRecord {
                    path: entry.path.clone(),
                    serialized: entry.is_serialized(),
                    size: entry.size,
                    sha256: Sha256::digest(&*bytes).into(),
                }
            })
            .collect()
    }
}

/// The documents and payloads of one variant, read through `cache`.
fn documents(variant: &str, cache: &dyn BlockCache) -> (Vec<Vec<u8>>, Collected) {
    let loaded: Vec<Loaded> = BUNDLES
        .iter()
        .map(|name| Loaded::new(&fixtures().join(variant).join(name), cache))
        .collect();
    let indexes = BUNDLES
        .iter()
        .zip(&loaded)
        .map(|(name, l)| BundleIndex {
            name: name.to_string(),
            files: l.with_files(cache, |files| {
                files.iter().map(|f| index_file(f).unwrap()).collect()
            }),
        })
        .collect();
    let version = VersionIndex::new(indexes);
    let mut payloads = Collected::default();
    let documents = loaded
        .iter()
        .enumerate()
        .map(|(b, l)| {
            let mut out = Vec::new();
            l.with_files(cache, |files| {
                write_bundle(
                    &mut out,
                    &version,
                    b,
                    &l.entries(cache),
                    files,
                    &mut payloads,
                )
            })
            .unwrap();
            out
        })
        .collect();
    (documents, payloads)
}

#[test]
fn documents_are_the_expected_ones_for_every_variant_and_cache() {
    let expected_dir = fixtures().join("expected");
    if std::env::var_os("UPDATE_EXPECTED").is_some() {
        let (documents, _) = documents("uncompressed", &NoBlockCache);
        for (name, document) in BUNDLES.iter().zip(documents) {
            std::fs::write(expected_dir.join(format!("{name}.json")), document).unwrap();
        }
    }
    let expected: Vec<Vec<u8>> = BUNDLES
        .iter()
        .map(|name| std::fs::read(expected_dir.join(format!("{name}.json"))).unwrap())
        .collect();
    for variant in VARIANTS {
        let small = BoundedBlockCache::new(16 << 10);
        let large = BoundedBlockCache::new(64 << 20);
        let caches: [(&str, &dyn BlockCache); 3] = [
            ("no cache", &NoBlockCache),
            ("small cache", &small),
            ("large cache", &large),
        ];
        for (cache_name, cache) in caches {
            let (documents, payloads) = documents(variant, cache);
            for ((name, document), expected) in BUNDLES.iter().zip(&documents).zip(&expected) {
                assert!(
                    document == expected,
                    "{variant}, {cache_name}: the document of {name} differs from the expected one"
                );
            }
            // The 64 x 64 texture is the one payload above the inline limit.
            assert_eq!(payloads.0.len(), 1, "{variant}, {cache_name}");
        }
    }
}

#[test]
fn value_trees_read_every_object() {
    use sekai_unity::Value;
    let cache = NoBlockCache;
    let mut checked = 0;
    for name in BUNDLES {
        let loaded = Loaded::new(&fixtures().join("lzma").join(name), &cache);
        loaded.with_files(&cache, |files| {
            for file in files {
                for (i, object) in file.file.objects.iter().enumerate() {
                    let value = file.value(i).unwrap();
                    assert!(matches!(value, Value::Struct(_)));
                    match object.class_id {
                        142 => {
                            let container = value.get("m_Container").unwrap().as_map().unwrap();
                            assert!(!container.is_empty());
                            assert!(container[0].0.as_str().unwrap().starts_with("assets/"));
                            checked += 1;
                        }
                        114 => {
                            assert_eq!(
                                value.get("longValue").unwrap().as_i64(),
                                Some(-9007199254740993)
                            );
                            assert_eq!(value.get("unsignedLong").unwrap(), &Value::UInt(u64::MAX));
                            assert_eq!(value.get("flag").unwrap().as_u64(), Some(1));
                            let texture = value.get("texture").unwrap().as_pptr().unwrap();
                            assert_eq!(texture.file_id, 0);
                            checked += 1;
                        }
                        49 => {
                            let bytes = value.get("m_Script").unwrap();
                            assert_eq!(
                                bytes,
                                &Value::Bytes(vec![0x66, 0x69, 0x78, 0xff, 0xfe, 0x00, 0x01])
                            );
                            checked += 1;
                        }
                        _ => {}
                    }
                }
            }
        });
    }
    // Two AssetBundle objects, the MonoBehaviour and the TextAsset.
    assert_eq!(checked, 4);
}
