//! `sekai-exporter objects` on the fixture bundles of `sekai-unity`: the output does not depend
//! on the thread count and matches the expected documents.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures")
}

/// Every file under `root`, by relative path.
fn tree(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let bytes = std::fs::read(&path).unwrap();
                files.insert(path.strip_prefix(root).unwrap().to_path_buf(), bytes);
            }
        }
    }
    files
}

fn run(threads: usize) -> BTreeMap<PathBuf, Vec<u8>> {
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("objects-{threads}"));
    let _ = std::fs::remove_dir_all(&output);
    let status = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
        .arg("objects")
        .arg(fixtures().join("lzma"))
        .arg(&output)
        .arg("--threads")
        .arg(threads.to_string())
        .status()
        .unwrap();
    assert!(status.success());
    tree(&output)
}

#[test]
fn output_does_not_depend_on_the_thread_count() {
    let one = run(1);
    assert_eq!(one, run(4));
    for name in ["fixture_data", "fixture_scene"] {
        let expected = std::fs::read(fixtures().join(format!("expected/{name}.json"))).unwrap();
        assert!(
            one[&Path::new(name).join("_objects.json")] == expected,
            "{name}"
        );
    }
    // The 64 x 64 texture is the one payload above the inline limit.
    assert_eq!(one.keys().filter(|p| p.starts_with("_payloads")).count(), 1);
}
