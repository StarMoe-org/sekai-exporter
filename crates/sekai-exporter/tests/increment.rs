//! `sekai-exporter update` of a `nuverse` region from a local CDN whose current version changes
//! between runs: in the second version a bundle has moved, and in the third another declares
//! other content, while a third bundle stays the same. `sekai-exporter gc` of the sites these
//! updates keep.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use sekai_pipeline::records::{Entry, Pack, RECORD_PACKS};
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

/// A scratch directory with the secrets of the regions `r` and `s` and of buckets, and the CDN
/// that both regions read.
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
        self.run(&["update", "r"], output, bucket, &[])
    }

    /// Runs `gc` of the site of the output directory `output`, or of `bucket` when there is one,
    /// with `options`, and returns what it printed.
    fn gc(&self, output: &str, bucket: Option<&Server>, options: &[&str]) -> String {
        self.run(&["gc"], output, bucket, options)
    }

    /// Runs the command `command` with the output directory `output`, the configuration of the
    /// regions and of `bucket` when there is one, and `options`, and returns what it printed.
    fn run(
        &self,
        command: &[&str],
        output: &str,
        bucket: Option<&Server>,
        options: &[&str],
    ) -> String {
        let run = self.attempt(command, output, bucket, options);
        let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&run.stderr);
        assert!(run.status.success(), "{stdout}\n{stderr}");
        stdout
    }

    /// Runs the command `command` as [`Scratch::run`] does, whether or not it succeeds.
    fn attempt(
        &self,
        command: &[&str],
        output: &str,
        bucket: Option<&Server>,
        options: &[&str],
    ) -> Output {
        let regions = ["r", "s"].map(|name| (name.to_owned(), self.cdn.region()));
        let mut config = json!({ "regions": serde_json::Map::from_iter(regions) });
        if let Some(bucket) = bucket {
            config["publish"] = json!({ "s3": bucket.section(PREFIX) });
        }
        let config_file = self.root.join(format!("{output}.json"));
        std::fs::write(&config_file, config.to_string()).unwrap();
        Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
            .args(command)
            .arg(self.root.join(output))
            .arg("--config")
            .arg(config_file)
            .arg("--secrets")
            .arg(self.root.join("secrets"))
            .args(["--threads", "2"])
            .args(options)
            .output()
            .unwrap()
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

/// Files that nothing a site keeps refers to, by their names under the site: an asset document and
/// an audio file of the store, a record pack whose one record names the document, and a state chunk
/// that no region names.
fn unreferred() -> [(String, Vec<u8>); 4] {
    let name = |digits: &str| digits.repeat(32);
    let document = format!("{}.json", name("ab"));
    let record = json!({ "document": { "key": document, "size": 2 } }).to_string();
    let file = format!("records/derivations/ef/{}.json", name("ef"));
    let entry = Entry::of_file(&file, record.as_bytes()).unwrap();
    let [pack]: [Pack; 1] = Pack::of(vec![entry]).unwrap().try_into().unwrap();
    [
        (format!("store/ab/{document}"), b"{}".to_vec()),
        (format!("store/cd/{}.mp3", name("cd")), b"audio".to_vec()),
        (format!("state/{}", pack.name), pack.bytes),
        (
            format!("state/chunks/12/{}.json.gz", name("12")),
            b"chunk".to_vec(),
        ),
    ]
}

/// What the report `printed` of `gc` finds to be garbage among the files of `kind`.
fn garbage(printed: &str, kind: &str) -> Option<String> {
    let line = printed
        .lines()
        .find(|line| line.starts_with(&format!("{kind}: ")))?;
    Some(line.split_once("; ")?.1.to_owned())
}

/// The report of `files` garbage files of `bytes` bytes.
fn reported(files: usize, bytes: usize) -> Option<String> {
    Some(format!("garbage {files} files, {bytes} bytes"))
}

/// Asserts that the report `printed` of `gc` finds the files of [`unreferred`], and nothing else,
/// to be garbage.
fn finds_the_unreferred(printed: &str) {
    let sizes: Vec<usize> = unreferred().iter().map(|(_, bytes)| bytes.len()).collect();
    for kind in ["records/units", "records/derivations"] {
        assert_eq!(garbage(printed, kind), reported(0, 0), "{printed}");
    }
    let packs = garbage(printed, "records/packs");
    assert_eq!(packs, reported(1, sizes[2]), "{printed}");
    assert_eq!(
        garbage(printed, "chunks"),
        reported(1, sizes[3]),
        "{printed}"
    );
    let store = garbage(printed, "store");
    assert_eq!(store, reported(2, sizes[0] + sizes[1]), "{printed}");
    let audio = format!("  .mp3: 1 files, {} bytes", sizes[1]);
    assert!(printed.lines().any(|line| line == audio), "{printed}");
}

/// Asserts that the report `printed` of `gc` finds no garbage.
fn finds_no_garbage(printed: &str) {
    let kinds = [
        "records/packs",
        "records/units",
        "records/derivations",
        "chunks",
        "store",
        "inputs",
        "incoming",
    ];
    for kind in kinds {
        assert_eq!(garbage(printed, kind), reported(0, 0), "{printed}");
    }
}

/// The records in the record packs `packs`, by subject, each with the objects it names.
fn records_of(packs: impl IntoIterator<Item = (String, Vec<u8>)>) -> BTreeMap<String, Vec<String>> {
    let mut records = BTreeMap::new();
    for (name, bytes) in packs {
        for entry in Pack::read(&name, &bytes).unwrap() {
            let objects = entry.objects().unwrap().into_iter();
            let objects = objects.map(|object| object.to_string()).collect();
            records.insert(format!("{:?}", entry.subject), objects);
        }
    }
    records
}

/// Collecting the garbage of the output directory.
mod gc {
    use super::*;

    /// The record packs of the output directory `output`.
    fn packs(output: &Path) -> Vec<(String, Vec<u8>)> {
        let packs = files_under(output, &[&format!("state/{RECORD_PACKS}")]).into_iter();
        packs
            .map(|name| (name.clone(), std::fs::read(output.join(&name)).unwrap()))
            .collect()
    }

    #[test]
    fn gc_merges_small_packs_into_one_with_their_records() {
        let scratch = Scratch::new("gc-packs");
        scratch.update("site", None);
        scratch.cdn.serve_version("2");
        scratch.update("site", None);
        let site = scratch.root.join("site");
        let before = packs(&site);
        assert_eq!(before.len(), 2);

        let merged = scratch.gc("site", None, &["--grace", "0s", "--delete"]);
        let rewritten = garbage(&merged, "records/packs").unwrap();
        assert!(rewritten.starts_with("garbage 2 files"), "{merged}");
        assert!(merged.contains("records/packs: new 1 files"), "{merged}");
        let after = packs(&site);
        assert_eq!(after.len(), 1);
        assert_eq!(records_of(after), records_of(before));
        finds_no_garbage(&scratch.gc("site", None, &["--grace", "0s"]));
    }

    /// The files under `store/`, `state/` and `published/` of the output directory `output`.
    fn site_files(output: &Path) -> BTreeSet<String> {
        files_under(output, &["store", "state", "published"])
    }

    /// The files under `directories` of the output directory `output`.
    fn files_under(output: &Path, directories: &[&str]) -> BTreeSet<String> {
        fn walk(root: &Path, directory: &Path, files: &mut BTreeSet<String>) {
            let Ok(entries) = std::fs::read_dir(directory) else {
                return;
            };
            for entry in entries {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, files);
                } else {
                    let name = path.strip_prefix(root).unwrap().to_string_lossy();
                    files.insert(name.replace('\\', "/"));
                }
            }
        }
        let mut files = BTreeSet::new();
        for directory in directories {
            walk(output, &output.join(directory), &mut files);
        }
        files
    }

    #[test]
    fn gc_deletes_inputs_no_region_fetched_last_and_partial_writes() {
        let scratch = Scratch::new("gc-inputs");
        scratch.update("site", None);
        let site = scratch.root.join("site");
        let fetched = files_under(&site, &["inputs"]);
        assert!(!fetched.is_empty());
        finds_no_garbage(&scratch.gc("site", None, &["--grace", "0s"]));

        let unfetched = format!("inputs/ab/{}.bundle", "ab".repeat(32));
        let partial = ["inputs/incoming/1-0.tmp", "store/incoming/1-1.tmp"];
        for name in partial.iter().copied().chain([unfetched.as_str()]) {
            let path = site.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"left").unwrap();
        }
        finds_no_garbage(&scratch.gc("site", None, &[]));
        let listed = scratch.gc("site", None, &["--grace", "0s"]);
        assert_eq!(garbage(&listed, "inputs"), reported(1, 4), "{listed}");
        assert_eq!(garbage(&listed, "incoming"), reported(2, 8), "{listed}");

        scratch.gc("site", None, &["--grace", "0s", "--delete"]);
        assert_eq!(files_under(&site, &["inputs"]), fetched);
        assert!(!site.join(partial[1]).exists());
    }

    #[test]
    fn gc_deletes_only_what_nothing_kept_refers_to() {
        let scratch = Scratch::new("gc-local");
        scratch.update("site", None);
        let site = scratch.root.join("site");
        let kept = site_files(&site);
        finds_no_garbage(&scratch.gc("site", None, &["--grace", "0s"]));

        for (name, bytes) in unreferred() {
            let path = site.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
        let planted = site_files(&site);
        // Files written within the grace period are kept.
        finds_no_garbage(&scratch.gc("site", None, &[]));
        let listed = scratch.gc("site", None, &["--grace", "0s"]);
        finds_the_unreferred(&listed);
        assert!(listed.contains("nothing deleted"), "{listed}");
        assert_eq!(site_files(&site), planted);

        scratch.gc("site", None, &["--grace", "0s", "--delete"]);
        assert_eq!(site_files(&site), kept);

        // A later update takes the records and the state that gc kept.
        scratch.cdn.serve_version("2");
        let increment = scratch.update("site", None);
        assert!(increment.contains("units 3 reused 1"), "{increment}");
        scratch.update("fresh", None);
        for view in VIEWS {
            assert_eq!(
                contents(scratch.pointer("site", view)),
                contents(scratch.pointer("fresh", view)),
                "{view}"
            );
        }
    }
}

/// Updates that publish to a bucket.
#[cfg(feature = "s3")]
mod bucket {
    use sekai_pipeline::records::Subject;
    use sekai_testkit::s3::{Logged, Operation};

    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    // The parts of the bucket under PREFIX.
    /// The objects.
    const STORE: &str = "out/store/";
    /// The state of the exporter.
    const STATE: &str = "out/state/";
    /// The record packs.
    const PACKS: &str = "out/state/records/packs/";
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

    /// The current pointer of `view` of the region `r` in `bucket`.
    fn pointer(bucket: &Server, view: &str) -> Option<serde_json::Value> {
        pointer_of(bucket, "r", view)
    }

    /// The current pointer of `view` of `region` in `bucket`.
    fn pointer_of(bucket: &Server, region: &str, view: &str) -> Option<serde_json::Value> {
        let bytes = bucket.object(&format!("{POINTERS}{region}/{view}/current.json"))?;
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
        let asked = |operation| requests.iter().filter(move |r| r.operation == operation);
        assert_eq!(asked(Operation::Head).count(), 0, "{requests:?}");
        // The records of earlier runs come from their packs, after one listing of them.
        let listed: Vec<&str> = asked(Operation::List).map(|r| r.name.as_str()).collect();
        assert_eq!(listed, [PACKS], "{requests:?}");
        let records = format!("{STATE}records/");
        let read = asked(Operation::Get).filter(|r| r.name.starts_with(&records));
        let read: Vec<&Logged> = read.collect();
        assert!(!read.is_empty(), "{requests:?}");
        assert!(
            read.iter()
                .all(|r| r.status == 200 && r.name.starts_with(PACKS)),
            "{read:?}"
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
        // Nothing is looked up or uploaded in parts, and only the record packs are listed.
        let other_operations: Vec<&Logged> = requests
            .iter()
            .filter(|r| match r.operation {
                Operation::Get | Operation::Put => false,
                Operation::List => r.name != PACKS,
                _ => true,
            })
            .collect();
        assert!(other_operations.is_empty(), "{other_operations:?}");
    }

    #[test]
    fn the_regions_of_one_update_list_the_store_once_and_share_its_objects() {
        let scratch = Scratch::new("increment-regions");
        let bucket = Server::start();
        let both = ["update", "r", "s"];
        let (printed, requests) = answered(&bucket, || {
            scratch.run(&both, "both", Some(&bucket), &["--verify"])
        });
        assert!(printed.contains("bucket holds 0 objects"), "{printed}");
        for (region, view) in ["r", "s"].into_iter().flat_map(|r| VIEWS.map(|v| (r, v))) {
            assert!(
                pointer_of(&bucket, region, view).is_some(),
                "{region} {view}"
            );
        }
        // One listing of each directory of the store serves both regions.
        let listed: Vec<&str> = requests
            .iter()
            .filter(|r| r.operation == Operation::List && r.name.starts_with(STORE))
            .map(|r| r.name.as_str())
            .collect();
        let directories: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(directories.len(), 256, "{listed:?}");
        assert_eq!(listed.len(), directories.len(), "listed twice: {listed:?}");
        // The second region reads the same bundles, and finds every object the first one put.
        let objects: Vec<&str> = uploaded(&requests)
            .into_iter()
            .filter(|name| name.starts_with(STORE))
            .collect();
        let once: BTreeSet<&str> = objects.iter().copied().collect();
        assert!(!once.is_empty());
        assert_eq!(once.len(), objects.len(), "uploaded twice: {objects:?}");
    }

    /// The records in the packs of `bucket`, by subject, each with the objects it names.
    fn records(bucket: &Server) -> BTreeMap<String, Vec<String>> {
        let packs = held(bucket, PACKS).into_iter();
        records_of(packs.map(|name| {
            let bytes = bucket.object(&name).unwrap();
            (name, bytes)
        }))
    }

    /// The bytes in `bucket` of the object named `key`, as `<sha256>.<extension>`.
    fn stored(bucket: &Server, key: &str) -> Option<Vec<u8>> {
        bucket.object(&format!("{STORE}{}/{key}", &key[..2]))
    }

    #[test]
    fn a_run_stopped_after_its_records_leaves_them_to_the_next() {
        let scratch = Scratch::new("increment-stopped");
        let whole = Server::start();
        let (_, requests) = answered(&whole, || scratch.update("whole", Some(&whole)));
        let made = records(&whole);
        assert!(!made.is_empty());
        // The objects and the record packs are uploaded before the state and the pointers.
        let accepted = uploaded(&requests);
        let derivation = |name: &str| name.starts_with(STORE) || name.starts_with(PACKS);
        let derived = accepted.iter().filter(|name| derivation(name)).count();
        let first = &accepted[..derived];
        assert!(first.iter().all(|name| derivation(name)), "{accepted:?}");
        let once: BTreeSet<&str> = accepted.iter().copied().collect();
        assert_eq!(once.len(), accepted.len(), "uploaded twice: {accepted:?}");

        let bucket = Server::start();
        bucket.refuse_after(Some(derived));
        let stopped = scratch.attempt(&["update", "r"], "stopped", Some(&bucket), &[]);
        assert!(!stopped.status.success());
        assert_eq!(pointer(&bucket, "canonical"), None);
        assert_eq!(records(&bucket), made);
        // A record names only objects that the bucket holds, and so does an asset document it
        // names.
        for key in made.values().flatten() {
            let bytes = stored(&bucket, key).unwrap_or_else(|| panic!("{key}"));
            let document: Option<serde_json::Value> = serde_json::from_slice(&bytes).ok();
            let members = document.as_ref().and_then(|d| d["members"].as_array());
            for member in members.into_iter().flatten() {
                let (sha256, extension) = (&member["sha256"], &member["extension"]);
                let key = format!(
                    "{}.{}",
                    sha256.as_str().unwrap(),
                    extension.as_str().unwrap()
                );
                assert!(stored(&bucket, &key).is_some(), "{key}");
            }
        }

        bucket.refuse_after(None);
        let next = scratch.update("next", Some(&bucket));
        assert!(next.contains(" converted 0 "), "{next}");
        for view in VIEWS {
            let (next, whole) = (pointer(&bucket, view), pointer(&whole, view));
            assert!(next.is_some(), "{view}");
            assert_eq!(next.map(contents), whole.map(contents), "{view}");
        }
    }

    #[test]
    fn gc_moves_records_kept_one_file_each_into_packs() {
        let scratch = Scratch::new("gc-singles");
        let bucket = Server::start();
        scratch.update("first", Some(&bucket));
        let made = records(&bucket);
        // The records, kept one file each.
        for name in held(&bucket, PACKS) {
            for entry in Pack::read(&name, &bucket.object(&name).unwrap()).unwrap() {
                let (directory, key) = match entry.subject {
                    Subject::Unit(key) => ("units", key.to_string()),
                    Subject::Derivation(fingerprint) => ("derivations", hex(&fingerprint)),
                };
                let single = format!("{STATE}records/{directory}/{}/{key}.json", &key[..2]);
                bucket.insert(&single, entry.json().as_bytes().to_vec());
            }
            bucket.remove(&name);
        }

        let moved = scratch.gc("moving", Some(&bucket), &["--delete"]);
        assert!(moved.contains("records/packs: new 1 files"), "{moved}");
        assert!(held(&bucket, &format!("{STATE}records/units/")).is_empty());
        assert!(held(&bucket, &format!("{STATE}records/derivations/")).is_empty());
        assert_eq!(records(&bucket), made);

        // A machine with an empty output takes the records from the new pack.
        scratch.cdn.serve_version("2");
        let second = scratch.update("second", Some(&bucket));
        assert!(second.contains("units 3 reused 1"), "{second}");
    }

    #[test]
    fn gc_deletes_from_the_bucket_only_what_nothing_kept_refers_to() {
        let scratch = Scratch::new("gc-bucket");
        let bucket = Server::start();
        scratch.update("first", Some(&bucket));
        let kept = held(&bucket, PREFIX);
        for (name, bytes) in unreferred() {
            bucket.insert(&format!("{PREFIX}{name}"), bytes);
        }
        let planted = held(&bucket, PREFIX);

        // The bucket lists every object as written long ago, so the default grace period keeps
        // none of them. A machine with an empty output reads what it needs from the bucket.
        let (listed, requests) = answered(&bucket, || scratch.gc("listing", Some(&bucket), &[]));
        finds_the_unreferred(&listed);
        assert_eq!(held(&bucket, PREFIX), planted);
        let changes = requests
            .iter()
            .filter(|r| matches!(r.operation, Operation::Put | Operation::Delete));
        assert_eq!(changes.count(), 0, "{requests:?}");

        let (_, requests) = answered(&bucket, || {
            scratch.gc("deleting", Some(&bucket), &["--delete"])
        });
        assert_eq!(held(&bucket, PREFIX), kept);
        let deletes = requests.iter().filter(|r| r.operation == Operation::Delete);
        assert_eq!(deletes.count(), unreferred().len());
        finds_no_garbage(&scratch.gc("again", Some(&bucket), &[]));

        scratch.cdn.serve_version("2");
        let second = scratch.update("second", Some(&bucket));
        assert!(second.contains("units 3 reused 1"), "{second}");
        let fresh = Server::start();
        scratch.update("fresh", Some(&fresh));
        for view in VIEWS {
            let (updated, fresh) = (pointer(&bucket, view), pointer(&fresh, view));
            assert_eq!(updated.map(contents), fresh.map(contents), "{view}");
        }
    }
}
