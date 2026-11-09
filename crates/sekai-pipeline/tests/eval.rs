//! Evaluating the Unity-built fixtures: outputs, determinism and reuse.

use std::path::Path;
use std::sync::Arc;

use sekai_catalog::LocalStore;
use sekai_convert::image::Texture;
use sekai_convert::{Asset, Context, ConvertError, Converter, ObjectId, ObjectIndex};
use sekai_format::AssetDocument;
use sekai_pipeline::metrics::NoRecorder;
use sekai_pipeline::{BundleInput, Evaluation, MemoryDerivations, Version, evaluate};
use sekai_unity::NoBlockCache;
use sha2::{Digest, Sha256};

fn input(file: &str) -> BundleInput {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../sekai-unity/tests/fixtures/lz4")
        .join(file);
    let bytes = std::fs::read(path).unwrap();
    BundleInput {
        name: file.to_owned(),
        sha256: Sha256::digest(&bytes).into(),
        bytes: Arc::from(bytes),
    }
}

fn run(root: &Path, threads: usize, derivations: &MemoryDerivations) -> Evaluation {
    run_with(Box::new(Texture), root, threads, derivations)
}

fn run_with(
    converter: Box<dyn Converter>,
    root: &Path,
    threads: usize,
    derivations: &MemoryDerivations,
) -> Evaluation {
    let (version, errors) = Version::open(
        vec![input("fixture_scene"), input("fixture_data")],
        &NoBlockCache,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let store = LocalStore::open(root).unwrap();
    evaluate(
        &version,
        &[converter],
        &store,
        derivations,
        &NoRecorder,
        threads,
    )
    .unwrap()
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("sekai-eval-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn every_named_texture_gets_a_document_and_files() {
    let root = scratch("documents");
    let evaluation = run(&root, 2, &MemoryDerivations::default());
    assert!(!evaluation.outputs.is_empty());
    assert!(evaluation.unnamed.is_empty());
    let store = LocalStore::open(&root).unwrap();
    for output in &evaluation.outputs {
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        assert_eq!(document.name, output.name);
        assert_eq!(document.form, "texture");
        assert_eq!(document.sources[0].class, "Texture2D");
        assert_eq!(document.members[0].role, "image");
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn outputs_do_not_depend_on_the_thread_count() {
    let (a, b) = (scratch("one"), scratch("many"));
    let one = run(&a, 1, &MemoryDerivations::default());
    let many = run(&b, 8, &MemoryDerivations::default());
    assert_eq!(one, many);
    std::fs::remove_dir_all(a).unwrap();
    std::fs::remove_dir_all(b).unwrap();
}

#[test]
fn a_second_run_reuses_every_derivation() {
    let root = scratch("reuse");
    let derivations = MemoryDerivations::default();
    let first = run(&root, 2, &derivations);
    assert_eq!(first.reused, 0);
    let second = run(&root, 2, &derivations);
    assert_eq!(second.converted, 0);
    assert_eq!(second.reused, first.converted);
    assert_eq!(first.outputs, second.outputs);
    std::fs::remove_dir_all(root).unwrap();
}

struct Broken;

impl Converter for Broken {
    fn name(&self) -> &'static str {
        "broken"
    }

    fn version(&self) -> u32 {
        1
    }

    fn is_root(&self, object: &ObjectIndex, _: &[String]) -> bool {
        object.class == "Texture2D"
    }

    fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
        Err(ConvertError::new("unsupported", "always"))
    }
}

#[test]
fn a_failed_asset_is_recorded_and_evaluated_again() {
    let root = scratch("failed");
    let derivations = MemoryDerivations::default();
    let first = run_with(Box::new(Broken), &root, 2, &derivations);
    assert!(!first.outputs.is_empty());
    let store = LocalStore::open(&root).unwrap();
    for output in &first.outputs {
        assert_eq!(output.error.as_deref(), Some("unsupported"));
        let document = AssetDocument::from_bytes(&store.read(&output.document).unwrap()).unwrap();
        assert!(document.members.is_empty());
    }
    let second = run_with(Box::new(Broken), &root, 2, &derivations);
    assert_eq!(second.reused, 0);
    assert_eq!(second.converted, first.converted);
    std::fs::remove_dir_all(root).unwrap();
}
