//! `sekai-exporter update` from a `dir` region holding a copy of the fixture bundles of
//! `sekai-unity`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures/lzma")
}

/// A scratch directory with its own copy of the fixture bundles and a configuration naming them
/// the region `local`.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        let source = root.join("source");
        std::fs::create_dir_all(&source).unwrap();
        for entry in std::fs::read_dir(fixtures()).unwrap() {
            let path = entry.unwrap().path();
            std::fs::copy(&path, source.join(path.file_name().unwrap())).unwrap();
        }
        let region = serde_json::json!({
            "regions": { "local": { "kind": "dir", "directory": source } }
        });
        std::fs::write(root.join("config.json"), region.to_string()).unwrap();
        Self(root)
    }

    fn output(&self) -> PathBuf {
        self.0.join("output")
    }

    fn update(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
            .arg("update")
            .arg("local")
            .arg(self.output())
            .arg("--config")
            .arg(self.0.join("config.json"))
            .args(extra)
            .output()
            .unwrap()
    }

    fn bundles(&self) -> Vec<PathBuf> {
        std::fs::read_dir(self.0.join("source"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect()
    }

    fn pointer(&self) -> serde_json::Value {
        let path = self.output().join("published/local/canonical/current.json");
        serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
    }
}

fn succeeded(output: &Output) -> String {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

/// The output of a check that found a change, which exits with 1.
fn changed(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout.clone()).unwrap()
}

#[test]
fn an_update_publishes_the_version_and_a_second_one_finds_it_unchanged() {
    let scratch = Scratch::new("update-unchanged");
    succeeded(&scratch.update(&[]));
    let pointer = scratch.pointer();
    let version = pointer["published_version"].as_str().unwrap();
    assert!(pointer["paths"].is_string(), "{pointer}");
    let pinned = scratch
        .output()
        .join(format!("published/local/canonical/versions/{version}.json"));
    assert!(pinned.is_file(), "{}", pinned.display());

    let again = succeeded(&scratch.update(&[]));
    assert!(again.contains("unchanged"), "{again}");
    assert_eq!(scratch.pointer(), pointer);
}

#[test]
fn a_check_of_an_output_without_a_catalog_reports_a_change_and_writes_nothing() {
    let scratch = Scratch::new("update-check-fresh");
    let checked = changed(&scratch.update(&["--check"]));
    assert!(checked.contains(" changed"), "{checked}");
    assert!(!checked.contains("unchanged"), "{checked}");
    assert!(!scratch.output().exists(), "{}", scratch.output().display());
}

#[test]
fn a_check_after_an_update_finds_the_same_version() {
    let scratch = Scratch::new("update-check-unchanged");
    succeeded(&scratch.update(&[]));
    let checked = succeeded(&scratch.update(&["--check"]));
    assert!(checked.ends_with(" unchanged\n"), "{checked}");
    assert_eq!(checked.lines().count(), 1, "{checked}");
}

#[test]
fn a_check_names_the_recorded_version_when_the_source_has_changed() {
    let scratch = Scratch::new("update-check-changed");
    succeeded(&scratch.update(&[]));
    let recorded = scratch.pointer()["published_version"]
        .as_str()
        .unwrap()
        .to_owned();
    for bundle in scratch.bundles() {
        std::fs::write(bundle, b"other bytes").unwrap();
    }
    let checked = changed(&scratch.update(&["--check"]));
    assert!(
        checked.ends_with(&format!(" changed from {recorded}\n")),
        "{checked}"
    );
}

#[test]
fn a_check_refuses_to_force_an_update() {
    let scratch = Scratch::new("update-check-forced");
    let output = scratch.update(&["--check", "--force"]);
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("--check"), "{message}");
    assert!(!scratch.output().exists(), "{}", scratch.output().display());
}

#[test]
fn a_forced_update_publishes_the_same_pointer() {
    let scratch = Scratch::new("update-forced");
    succeeded(&scratch.update(&[]));
    let pointer = scratch.pointer();
    succeeded(&scratch.update(&["--force"]));
    assert_eq!(scratch.pointer(), pointer);
}

#[test]
fn a_selection_needs_an_output_without_a_complete_run() {
    let scratch = Scratch::new("update-selection");
    succeeded(&scratch.update(&[]));
    let output = scratch.update(&["--only", "a"]);
    assert!(!output.status.success());
    let message = String::from_utf8_lossy(&output.stderr);
    assert!(message.contains("--only"), "{message}");
}
