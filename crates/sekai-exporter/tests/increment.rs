//! `sekai-exporter update` of a `nuverse` region from a local CDN whose current version changes
//! between runs: in the second version a bundle has moved, and in the third another declares
//! other content, while a third bundle stays the same.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use sekai_testkit::nuverse::{Bundle, Cdn};
use sekai_testkit::s3::Server;
use serde_json::json;

const VIEWS: [&str; 2] = ["canonical", "storage"];

/// `pointer` without its version history and changes, which follow the publications before it.
fn contents(mut pointer: serde_json::Value) -> serde_json::Value {
    let fields = pointer.as_object_mut().unwrap();
    assert!(fields.remove("history").is_some(), "{fields:?}");
    fields.remove("changes");
    pointer
}

/// Where the tests publish in a bucket.
const PREFIX: &str = "out/";

fn fixture(compression: &str, name: &str) -> Vec<u8> {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sekai-unity/tests/fixtures");
    std::fs::read(fixtures.join(compression).join(name)).unwrap()
}

/// The uncompressed data fixture with its file and the file it refers to renamed, so that it
/// depends on no other bundle of the versions.
fn lone() -> Vec<u8> {
    let mut bytes = fixture("uncompressed", "fixture_data");
    let renames = [
        (
            "2e6074037da702b660ee3dbd01c9ea62",
            "00000000000000000000000000000001",
        ),
        (
            "9076adc37246e4c243c45fbc50ed10e1",
            "00000000000000000000000000000002",
        ),
    ];
    for (from, to) in renames {
        let (from, to) = (from.as_bytes(), to.as_bytes());
        while let Some(at) = bytes.windows(from.len()).position(|window| window == from) {
            bytes[at..at + to.len()].copy_from_slice(to);
        }
    }
    bytes
}

/// The bundles of each version, by the value of the version document.
fn versions() -> BTreeMap<String, Vec<Bundle>> {
    let bundle = |name: &str, crc, bytes| Bundle {
        name: name.to_owned(),
        crc,
        bytes,
    };
    let version = |data_crc, scene: &str| {
        vec![
            bundle("a/data", data_crc, fixture("lzma", "fixture_data")),
            bundle(scene, 21, fixture("lzma", "fixture_scene")),
            bundle("e/lone", 41, lone()),
        ]
    };
    BTreeMap::from([
        ("1".to_owned(), version(11, "b/scene")),
        ("2".to_owned(), version(11, "c/scene")),
        ("3".to_owned(), version(12, "c/scene")),
    ])
}

/// A scratch directory with the secrets of the region `r` and of buckets, and its CDN.
struct Scratch {
    root: PathBuf,
    cdn: Cdn,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
        let _ = std::fs::remove_dir_all(&root);
        Cdn::write_secrets(&root.join("secrets")).unwrap();
        Server::write_secrets(&root.join("secrets")).unwrap();
        Self {
            root,
            cdn: Cdn::start(versions(), "1"),
        }
    }

    /// Runs `update` of the region into the output directory `output`, publishing to `bucket`
    /// when there is one, and returns what it printed.
    fn update(&self, output: &str, bucket: Option<&Server>) -> String {
        let mut config = json!({ "regions": { "r": self.cdn.region() } });
        if let Some(bucket) = bucket {
            config["publish"] = json!({ "s3": bucket.section(PREFIX) });
        }
        let config_file = self.root.join(format!("{output}.json"));
        std::fs::write(&config_file, config.to_string()).unwrap();
        let run = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
            .arg("update")
            .arg("r")
            .arg(self.root.join(output))
            .arg("--config")
            .arg(config_file)
            .arg("--secrets")
            .arg(self.root.join("secrets"))
            .args(["--threads", "2"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{stdout}\n{stderr}");
        stdout
    }

    /// The current pointer of `view` in the output directory `output`.
    fn pointer(&self, output: &str, view: &str) -> serde_json::Value {
        let path = format!("published/r/{view}/current.json");
        let bytes = std::fs::read(self.root.join(output).join(path)).unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[test]
fn each_update_publishes_what_an_update_from_nothing_publishes() {
    let scratch = Scratch::new("increment-same");
    scratch.update("earlier", None);
    for value in ["2", "3"] {
        scratch.cdn.serve_version(value);
        let increment = scratch.update("earlier", None);
        // The lone bundle is the same, so its unit keeps what the last run gave.
        assert!(increment.contains("units 3 reused 1"), "{increment}");
        assert!(increment.contains(" kept 1 "), "{increment}");
        let fresh = format!("fresh-{value}");
        scratch.update(&fresh, None);
        for view in VIEWS {
            let increment = scratch.pointer("earlier", view);
            // A new version records what it changed since the one before.
            assert!(increment.get("changes").is_some(), "{value} {view}");
            assert_eq!(
                contents(increment),
                contents(scratch.pointer(&fresh, view)),
                "{value} {view}"
            );
        }
    }
}

/// Updates that publish to a bucket.
#[cfg(feature = "s3")]
mod bucket {
    use std::collections::BTreeSet;

    use sekai_testkit::s3::{Logged, Operation};

    use super::*;

    // The parts of the bucket under PREFIX.
    /// The objects.
    const STORE: &str = "out/store/";
    /// The state of the exporter.
    const STATE: &str = "out/state/";
    /// The pointers of the views.
    const POINTERS: &str = "out/published/";

    impl Scratch {
        /// How many bundles the output directory `output` downloaded.
        fn downloaded(&self, output: &str) -> usize {
            let inputs = self.root.join(output).join("inputs");
            let directories = std::fs::read_dir(inputs)
                .unwrap()
                .map(|entry| entry.unwrap());
            let directories = directories.filter(|entry| entry.file_type().unwrap().is_dir());
            directories
                .map(|directory| std::fs::read_dir(directory.path()).unwrap().count())
                .sum()
        }
    }

    /// The current pointer of `view` in `bucket`.
    fn pointer(bucket: &Server, view: &str) -> Option<serde_json::Value> {
        let bytes = bucket.object(&format!("{POINTERS}r/{view}/current.json"))?;
        Some(serde_json::from_slice(&bytes).unwrap())
    }

    /// What `run` gave, and the requests `bucket` answered while it ran.
    fn answered<T>(bucket: &Server, run: impl FnOnce() -> T) -> (T, Vec<Logged>) {
        let before = bucket.requests().len();
        let given = run();
        (given, bucket.requests().split_off(before))
    }

    /// The names of the objects `bucket` holds under `prefix`.
    fn held(bucket: &Server, prefix: &str) -> BTreeSet<String> {
        bucket.names(prefix).into_iter().collect()
    }

    /// The names that `requests` uploaded with success, in order.
    fn uploaded(requests: &[Logged]) -> Vec<&str> {
        let puts = requests.iter().filter(|r| r.operation == Operation::Put);
        puts.filter(|r| r.status < 300)
            .map(|r| r.name.as_str())
            .collect()
    }

    #[test]
    fn a_machine_with_an_empty_output_continues_from_the_bucket() {
        let scratch = Scratch::new("increment-bucket");
        let bucket = Server::start();
        scratch.update("first", Some(&bucket));
        scratch.cdn.serve_version("2");
        let (second, requests) = answered(&bucket, || scratch.update("second", Some(&bucket)));
        assert!(second.contains("units 3 reused 1"), "{second}");
        assert!(second.contains(" kept 1 "), "{second}");
        // The lone bundle is neither downloaded nor read: the state knows its files.
        assert_eq!(scratch.downloaded("second"), 2);
        let asked = |operation| requests.iter().any(|logged| logged.operation == operation);
        assert!(
            !asked(Operation::Head) && !asked(Operation::List),
            "{requests:?}"
        );

        let fresh = Server::start();
        scratch.update("fresh", Some(&fresh));
        for view in VIEWS {
            let (updated, fresh) = (pointer(&bucket, view), pointer(&fresh, view));
            assert!(updated.is_some(), "{view}");
            assert_eq!(updated.map(contents), fresh.map(contents), "{view}");
        }

        // A machine that finds the version unchanged reads only the last version and the pointers
        // of the two views from the bucket.
        let (third, requests) = answered(&bucket, || scratch.update("third", Some(&bucket)));
        assert!(third.contains("unchanged"), "{third}");
        assert_eq!(requests.len(), 3, "{requests:?}");
    }

    #[test]
    fn an_update_uploads_the_objects_the_bucket_lacks_and_otherwise_only_state_and_pointers() {
        let scratch = Scratch::new("increment-uploads");
        let bucket = Server::start();
        scratch.update("first", Some(&bucket));
        let earlier = held(&bucket, STORE);

        // The second version moves the scene bundle, which changes the unit of the data and the
        // scene and leaves the lone bundle as it was.
        scratch.cdn.serve_version("2");
        let (_, requests) = answered(&bucket, || scratch.update("second", Some(&bucket)));
        let later = held(&bucket, STORE);
        let new: BTreeSet<&str> = later.difference(&earlier).map(String::as_str).collect();
        assert!(!new.is_empty(), "the changed unit gives new objects");

        let (objects, others): (Vec<&str>, Vec<&str>) = uploaded(&requests)
            .into_iter()
            .partition(|name| name.starts_with(STORE));
        // Every new object is uploaded once, and no object the bucket held is uploaded again.
        let sent: BTreeSet<&str> = objects.iter().copied().collect();
        assert_eq!(sent.len(), objects.len(), "uploaded twice: {objects:?}");
        assert_eq!(sent, new);
        let elsewhere: Vec<&str> = others
            .into_iter()
            .filter(|name| !name.starts_with(STATE) && !name.starts_with(POINTERS))
            .collect();
        assert!(elsewhere.is_empty(), "{elsewhere:?}");
        // Nothing is looked up, listed or uploaded in parts.
        let other_operations: Vec<&Logged> = requests
            .iter()
            .filter(|r| !matches!(r.operation, Operation::Get | Operation::Put))
            .collect();
        assert!(other_operations.is_empty(), "{other_operations:?}");
    }
}
