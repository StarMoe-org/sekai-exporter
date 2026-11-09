//! A version opened from the Unity-built fixtures serves objects, references and textures.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;
use std::sync::{Arc, Mutex};

use sekai_convert::image::Texture;
use sekai_convert::{Content, Context, Converter, PPtr};
use sekai_pipeline::{BundleIndexes, BundleInput, NoBundleIndexes, Version};
use sekai_unity::{ByteSource, Chunk, NoBlockCache, NoClasses};
use sha2::{Digest, Sha256};

fn input(file: &str, name: &str) -> BundleInput {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    let bytes = std::fs::read(path).unwrap();
    BundleInput {
        name: name.to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        source: Arc::new(Arc::<[u8]>::from(bytes)),
    }
}

fn fixtures() -> Vec<BundleInput> {
    vec![
        input("fixture_scene", "fixture_scene"),
        input("fixture_data", "fixture_data"),
    ]
}

#[derive(Default)]
struct MemoryIndexes(Mutex<BTreeMap<[u8; 32], Vec<u8>>>);

impl BundleIndexes for MemoryIndexes {
    fn get(&self, bundle: &[u8; 32]) -> io::Result<Option<Vec<u8>>> {
        Ok(self.0.lock().unwrap().get(bundle).cloned())
    }

    fn put(&self, bundle: [u8; 32], index: Vec<u8>) -> io::Result<()> {
        self.0.lock().unwrap().insert(bundle, index);
        Ok(())
    }
}

/// A source with no bytes behind it, which shows that a bundle was not read.
struct Gone;

impl ByteSource for Gone {
    fn len(&self) -> u64 {
        0
    }

    fn read(&self, offset: u64, len: usize) -> sekai_unity::Result<Chunk<'_>> {
        Err(sekai_unity::Error::Truncated {
            what: "a bundle that must not be read",
            offset,
            needed: len as u64,
            available: 0,
        })
    }
}

/// Everything the index of a version says, in order.
fn listing(version: &Version<'_>) -> String {
    let bundles = version.index().bundles().len();
    let hashes: Vec<_> = (0..bundles).map(|b| version.resource_hashes(b)).collect();
    format!("{:?} {hashes:?}", version.index().bundles())
}

#[test]
fn a_stored_index_replaces_reading_the_bundle() {
    let indexes = MemoryIndexes::default();
    let (first, errors) =
        Version::open_indexed(fixtures(), &NoBlockCache, &NoClasses, &indexes, 4).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(indexes.0.lock().unwrap().len(), 2);
    let unread = fixtures()
        .into_iter()
        .map(|input| BundleInput {
            source: Arc::new(Gone),
            ..input
        })
        .collect();
    let (second, errors) =
        Version::open_indexed(unread, &NoBlockCache, &NoClasses, &indexes, 1).unwrap();
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(listing(&first), listing(&second));
    let id = find(&second, "small");
    let error = second.object(id).map(|_| ()).unwrap_err();
    assert_eq!(error.code, "truncated_file");
}

#[test]
fn opening_does_not_depend_on_threads() {
    let open = |threads| {
        let (version, errors) = Version::open_indexed(
            fixtures(),
            &NoBlockCache,
            &NoClasses,
            &NoBundleIndexes,
            threads,
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        listing(&version)
    };
    assert_eq!(open(1), open(8));
}

fn find(version: &Version<'_>, name: &str) -> sekai_convert::ObjectId {
    version
        .objects()
        .find(|(_, object, _)| object.name.as_deref() == Some(name))
        .map(|(id, _, _)| id)
        .unwrap_or_else(|| panic!("no object named {name}"))
}

#[test]
fn texture_rows_run_from_top_to_bottom() {
    let (version, errors) = Version::open(fixtures(), &NoBlockCache);
    assert!(errors.is_empty(), "{errors:?}");
    let small = find(&version, "small");
    let asset = Texture.convert(small, &version).unwrap().unwrap();
    let Content::Image(surface) = &asset.members[0].content else {
        panic!("not an image");
    };
    assert_eq!((surface.width(), surface.height()), (2, 2));
    // Unity stores the bottom row first: (0, 0, 0), (1, 0, 7), then (2, 0, 14), (3, 0, 21).
    assert_eq!(
        surface.pixels(),
        [2, 0, 14, 255, 3, 0, 21, 255, 0, 0, 0, 255, 1, 0, 7, 255]
    );
    let large = find(&version, "large");
    assert!(Texture.convert(large, &version).unwrap().is_some());
}

#[test]
fn references_resolve_across_bundles() {
    let (version, _) = Version::open(fixtures(), &NoBlockCache);
    let data = find(&version, "data");
    let object = version.object(data).unwrap();
    assert_eq!(
        object.script.as_deref(),
        Some("Assembly-CSharp/FixtureData")
    );
    let material = object.fields.get("material").unwrap().as_pptr().unwrap();
    let target = version.resolve(data, material).unwrap().unwrap();
    assert_eq!(
        version.index().bundles()[target.bundle].name,
        "fixture_scene"
    );
    let null = PPtr {
        file_id: 0,
        path_id: 0,
    };
    assert_eq!(version.resolve(data, null).unwrap(), None);
    let absent = PPtr {
        file_id: 0,
        path_id: 12345,
    };
    assert_eq!(
        version.resolve(data, absent).unwrap_err().code,
        "missing_object"
    );
}

#[test]
fn the_container_lists_the_objects_of_the_bundle() {
    let (version, _) = Version::open(fixtures(), &NoBlockCache);
    let small = find(&version, "small");
    let container = version.container(small).unwrap();
    assert!(!container.is_empty());
    for (path, id) in &container {
        assert_eq!(id.bundle, small.bundle);
        let paths = version
            .index()
            .container_paths(id.bundle, id.file, id.path_id);
        assert!(paths.contains(path), "{path}");
    }
}

#[test]
fn bundles_that_share_a_name_are_left_out() {
    let inputs = vec![
        input("fixture_data", "fixture_data"),
        input("fixture_data", "fixture_data"),
        input("fixture_scene", "fixture_scene"),
    ];
    let (version, errors) = Version::open(inputs, &NoBlockCache);
    let codes: Vec<_> = errors.iter().map(|e| (e.name.as_str(), e.code)).collect();
    assert_eq!(
        codes,
        [
            ("fixture_data", "duplicate_name"),
            ("fixture_data", "duplicate_name")
        ]
    );
    assert_eq!(version.index().bundles().len(), 1);
}
