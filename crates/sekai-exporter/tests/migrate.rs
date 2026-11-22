//! `sekai-exporter migrate joint-stereo` of a site whose records name two audio documents: one of
//! a stereo MP3 with frames in stereo mode, and one of a mono MP3.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use sekai_catalog::Key;
use sekai_catalog::store::hex;
use sekai_codec::{Pcm, Sound};
use sekai_encode::mp3;
use sekai_format::AssetDocument;
use sekai_format::asset::{FORMAT, MemberFile, VERSION};
use sekai_pipeline::records::{DERIVATION_RECORDS, Entry, Pack, RECORD_PACKS, UNIT_RECORDS};
use serde_json::json;

/// A stereo MP3 whose frames coded as left and right are in stereo mode.
const STEREO_MODES: &[u8] =
    include_bytes!("../../sekai-encode/tests/fixtures/stereo-modes-mixed.mp3");

/// The files of a site, by their names under it.
trait Files {
    fn write(&self, name: &str, bytes: &[u8]);
    fn read(&self, name: &str) -> Option<Vec<u8>>;
    /// The names of the files under `directory`.
    fn list(&self, directory: &str) -> BTreeSet<String>;
}

/// An output directory as the site.
struct Local(PathBuf);

impl Files for Local {
    fn write(&self, name: &str, bytes: &[u8]) {
        let path = self.0.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    fn read(&self, name: &str) -> Option<Vec<u8>> {
        std::fs::read(self.0.join(name)).ok()
    }

    fn list(&self, directory: &str) -> BTreeSet<String> {
        fn walk(root: &Path, directory: &str, names: &mut BTreeSet<String>) {
            let Ok(entries) = std::fs::read_dir(root.join(directory)) else {
                return;
            };
            for entry in entries {
                let entry = entry.unwrap();
                let name = format!("{directory}/{}", entry.file_name().to_string_lossy());
                if entry.file_type().unwrap().is_dir() {
                    walk(root, &name, names);
                } else {
                    names.insert(name);
                }
            }
        }
        let mut names = BTreeSet::new();
        walk(&self.0, directory, &mut names);
        names
    }
}

/// The name of the object `key` under the site.
fn object(key: &Key) -> String {
    format!("store/{}", key.relative_path())
}

/// A second of a mono tone.
fn mono() -> Vec<u8> {
    let samples = (0..44_100)
        .map(|i| ((i % 100) * 300 - 15_000) as i16)
        .collect();
    let pcm = Pcm::new(44_100, 1, samples).unwrap();
    mp3::encode(&Sound::Pcm(pcm)).unwrap()
}

/// The audio document `name` whose one member is the MP3 `bytes`.
fn document(name: &str, bytes: &[u8]) -> AssetDocument {
    AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: name.to_owned(),
        form: "audio".to_owned(),
        form_version: 1,
        fingerprint: hex(&[7; 32]),
        sources: Vec::new(),
        resources: Vec::new(),
        members: vec![MemberFile {
            role: "audio".to_owned(),
            sha256: hex(&Key::of(bytes, "mp3").sha256),
            size: bytes.len() as u64,
            extension: "mp3".to_owned(),
            media_type: "audio/mpeg".to_owned(),
        }],
        references: Vec::new(),
        error: None,
    }
}

/// Puts `bytes` in the store of `site` and returns their key.
fn store(site: &dyn Files, bytes: &[u8], extension: &str) -> Key {
    let key = Key::of(bytes, extension);
    site.write(&object(&key), bytes);
    key
}

/// The record of a derivation that gave `document`, of `size` bytes.
fn derivation(document: &Key, size: usize) -> Vec<u8> {
    json!({ "document": { "key": document.to_string(), "size": size } })
        .to_string()
        .into_bytes()
}

/// What [`plant`] put at a site.
struct Planted {
    stereo: AssetDocument,
    stereo_key: Key,
    mono_key: Key,
    /// The record pack of the derivation of the mono document.
    pack: String,
}

/// Puts at `site` the two documents with their MP3 files, the record of a unit that gave both and
/// the record of the derivation of the stereo one, each kept in a file of its own, and a record
/// pack that holds the record of the derivation of the mono one.
fn plant(site: &dyn Files) -> Planted {
    let stereo = document("a/stereo", STEREO_MODES);
    let mono = mono();
    store(site, STEREO_MODES, "mp3");
    store(site, &mono, "mp3");
    let stereo_bytes = stereo.to_bytes().unwrap();
    let stereo_key = store(site, &stereo_bytes, "json");
    let mono_bytes = document("a/mono", &mono).to_bytes().unwrap();
    let mono_key = store(site, &mono_bytes, "json");

    let asset = |name: &str, document: &Key, path_id: i64| {
        json!({
            "name": name, "form": "audio", "document": document.to_string(), "error": null,
            "logical": name, "root": { "path_id": path_id },
        })
    };
    let unit = json!({
        "scanned": ["audio"], "forms": ["audio 1"],
        "assets": [asset("a/stereo", &stereo_key, 1), asset("a/mono", &mono_key, 2)],
        "unnamed": [], "outside": [],
    });
    let single =
        |directory: &str, byte: &str| format!("{directory}/{byte}/{}.json", byte.repeat(32));
    site.write(
        &format!("state/{}", single(UNIT_RECORDS, "09")),
        unit.to_string().as_bytes(),
    );
    let stereo_derivation = derivation(&stereo_key, stereo_bytes.len());
    site.write(
        &format!("state/{}", single(DERIVATION_RECORDS, "05")),
        &stereo_derivation,
    );
    let mono_derivation = derivation(&mono_key, mono_bytes.len());
    let entry = Entry::of_file(&single(DERIVATION_RECORDS, "06"), &mono_derivation).unwrap();
    let pack = Pack::of(vec![entry]).unwrap().remove(0);
    site.write(&format!("state/{}", pack.name), &pack.bytes);
    Planted {
        stereo,
        stereo_key,
        mono_key,
        pack: format!("state/{}", pack.name),
    }
}

/// The objects that each record of the packs of `site` names, by subject.
fn records(site: &dyn Files) -> BTreeMap<String, Vec<String>> {
    let mut records = BTreeMap::new();
    for name in site.list(&format!("state/{RECORD_PACKS}")) {
        for entry in Pack::read(&name, &site.read(&name).unwrap()).unwrap() {
            let objects = entry
                .objects()
                .unwrap()
                .iter()
                .map(Key::to_string)
                .collect();
            records.insert(format!("{:?}", entry.subject), objects);
        }
    }
    records
}

/// Runs `migrate joint-stereo` on the output directory `output` with `options`, and returns what
/// it printed.
fn migrate(output: &Path, options: &[&str]) -> String {
    let run = Command::new(env!("CARGO_BIN_EXE_sekai-exporter"))
        .args(["migrate", "joint-stereo"])
        .arg(output)
        .args(["--threads", "2"])
        .args(options)
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "{stdout}\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    stdout
}

/// Migrates the site `site`, which [`plant`] filled, with `run`, and checks what it holds after.
fn migrates(site: &dyn Files, planted: &Planted, run: impl Fn() -> String) {
    let printed = run();
    assert!(
        printed.contains("documents 2/2: 1 rewritten, 1 MP3 files"),
        "{printed}"
    );

    let mut joint = STEREO_MODES.to_vec();
    assert!(mp3::joint_stereo(&mut joint).unwrap());
    let rewritten = Key::of(&joint, "mp3");
    assert_eq!(site.read(&object(&rewritten)), Some(joint.clone()));
    let mut expected = planted.stereo.clone();
    expected.members[0].sha256 = hex(&rewritten.sha256);
    expected.members[0].size = joint.len() as u64;
    let expected = expected.to_bytes().unwrap();
    let new_key = Key::of(&expected, "json");
    assert_eq!(site.read(&object(&new_key)), Some(expected.clone()));

    // The records kept one file each are in a new pack, which names the new document; the pack
    // whose record names the mono document stays as it is.
    assert!(site.list(&format!("state/{UNIT_RECORDS}")).is_empty());
    assert!(site.list(&format!("state/{DERIVATION_RECORDS}")).is_empty());
    let packs = site.list(&format!("state/{RECORD_PACKS}"));
    assert_eq!(packs.len(), 2, "{packs:?}");
    assert!(packs.contains(&planted.pack), "{packs:?}");
    let named = |byte: u8| format!("{:?}", [byte; 32]);
    let records = records(site);
    let (new, mono) = (new_key.to_string(), planted.mono_key.to_string());
    assert_eq!(
        records[&format!("Unit(UnitKey({}))", named(9))],
        [new.clone(), mono.clone()]
    );
    assert_eq!(records[&format!("Derivation({})", named(5))], [new]);
    assert_eq!(records[&format!("Derivation({})", named(6))], [mono]);
    assert!(
        !records
            .values()
            .flatten()
            .any(|key| *key == planted.stereo_key.to_string())
    );

    // A second migration finds nothing to rewrite.
    let again = run();
    assert!(again.contains("documents 2/2: 0 rewritten"), "{again}");
    assert_eq!(site.list(&format!("state/{RECORD_PACKS}")), packs);
}

fn scratch(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&root);
    root
}

#[test]
fn a_migration_rewrites_the_stereo_mp3_files_of_an_output_and_the_records_that_name_them() {
    let output = scratch("migrate-local");
    let site = Local(output.clone());
    let planted = plant(&site);
    migrates(&site, &planted, || migrate(&output, &[]));
}

/// Migrating the site of a bucket.
#[cfg(feature = "s3")]
mod bucket {
    use sekai_testkit::s3::Server;

    use super::*;

    /// Where the test keeps the site in the bucket.
    const PREFIX: &str = "out/";

    impl Files for Server {
        fn write(&self, name: &str, bytes: &[u8]) {
            self.insert(&format!("{PREFIX}{name}"), bytes.to_vec());
        }

        fn read(&self, name: &str) -> Option<Vec<u8>> {
            self.object(&format!("{PREFIX}{name}"))
        }

        fn list(&self, directory: &str) -> BTreeSet<String> {
            let names = self.names(&format!("{PREFIX}{directory}/")).into_iter();
            names
                .map(|name| name.strip_prefix(PREFIX).unwrap().to_owned())
                .collect()
        }
    }

    #[test]
    fn a_migration_of_a_bucket_keeps_no_copy_of_the_new_files_in_the_output() {
        let root = scratch("migrate-bucket");
        let bucket = Server::start();
        Server::write_secrets(&root.join("secrets")).unwrap();
        let config = root.join("config.json");
        let section = json!({ "publish": { "s3": bucket.section(PREFIX) } });
        std::fs::write(&config, section.to_string()).unwrap();
        let output = root.join("output");
        let planted = plant(&bucket);
        let (config, secrets) = (config.to_str().unwrap(), root.join("secrets"));
        let options = ["--config", config, "--secrets", secrets.to_str().unwrap()];
        migrates(&bucket, &planted, || migrate(&output, &options));
        assert!(Local(output).list("store").is_empty());
    }
}
