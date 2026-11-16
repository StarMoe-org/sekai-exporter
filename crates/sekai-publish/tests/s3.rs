//! Publishing to an S3-compatible bucket, against a local server that checks signatures.

use std::path::PathBuf;

use sekai_catalog::{Holdings, Key, LocalStore, MemoryHoldings, NoRemote, Objects, Remote};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, MemberFile};
use sekai_publish::s3::{Bucket, Credentials, S3, publish};
use sekai_publish::{Binding, Plan, plan};
use sekai_testkit::s3::{ACCESS_KEY, Operation, REGION, SECRET_KEY, Server};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A client of the bucket of `server` under the prefix `out/`, signing with `secret`.
fn client(server: &Server, secret: &str) -> S3 {
    S3::new(
        Bucket {
            endpoint: server.endpoint().to_owned(),
            region: REGION.to_owned(),
            prefix: "out/".to_owned(),
            connections: 3,
        },
        Credentials {
            access_key_id: ACCESS_KEY.to_owned(),
            secret_access_key: secret.to_owned(),
        },
    )
    .unwrap()
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sekai-s3-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> LocalStore {
        LocalStore::open(self.0.join("store")).unwrap()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn asset(store: &LocalStore, name: &str, members: &[(&str, &[u8])]) -> Binding {
    let members = members
        .iter()
        .map(|(role, bytes)| {
            let key = store.put(bytes, "webp").unwrap();
            MemberFile {
                role: (*role).to_owned(),
                sha256: hex(&key.sha256),
                size: bytes.len() as u64,
                extension: "webp".to_owned(),
                media_type: "image/webp".to_owned(),
            }
        })
        .collect();
    let document = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: name.to_owned(),
        form: "texture".to_owned(),
        form_version: 1,
        fingerprint: "ab".to_owned(),
        sources: Vec::new(),
        members,
        resources: Vec::new(),
        references: Vec::new(),
        error: None,
    };
    Binding {
        name: name.to_owned(),
        document: store.put(&document.to_bytes().unwrap(), "json").unwrap(),
    }
}

fn sample(store: &LocalStore, version: &str, extra: &[u8]) -> Plan {
    let bindings = [
        asset(store, "a/one", &[("image", b"one")]),
        asset(store, "a/two", &[("base", b"two"), ("mask", extra)]),
        asset(store, "b/three", &[("image", b"three")]),
    ];
    plan(version, &bindings, store, 2).unwrap()
}

fn object_name(key: &Key) -> String {
    format!("out/store/{}", key.relative_path())
}

const POINTER: &str = "out/published/jp/canonical/current.json";

#[test]
fn a_second_publication_uploads_nothing() {
    let s = Scratch::new("twice");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");

    let (pointer, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    // Seven files, three shards, a chunk of directories, a chunk of one listing and the root, and
    // a path index of one chunk, the directory tree and the version history.
    assert_eq!((report.uploaded, report.present), (17, 0));
    assert!(report.pointer_written);
    for file in &plan.files {
        assert_eq!(
            server.object(&object_name(&file.key)),
            Some(store.read(&file.key).unwrap()),
            "{}",
            file.path
        );
    }
    assert_eq!(
        server.object(POINTER),
        Some(serde_json::to_vec(&pointer).unwrap())
    );
    let root = Key {
        sha256: sekai_catalog::store::parse_hex(pointer.root.as_deref().unwrap()).unwrap(),
        extension: "json".to_owned(),
    };
    assert!(server.object(&object_name(&root)).is_some());

    let puts = server.count(Operation::Put);
    let (again, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    assert_eq!(again, pointer);
    assert_eq!((report.uploaded, report.present), (0, 17));
    assert!(!report.pointer_written);
    assert_eq!(server.count(Operation::Put), puts);
}

#[test]
fn a_wrong_key_is_refused_and_writes_no_pointer() {
    let s = Scratch::new("signature");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let plan = sample(&store, "v1", b"mask");
    let e = publish(&objects, &client(&server, "wrong"), "jp/canonical", &plan).unwrap_err();
    // The first upload is refused, which stops the publication before any pointer.
    assert_eq!(e.stage, "files");
    assert!(server.object(POINTER).is_none());
    assert_eq!(server.count(Operation::Put), 0);
}

#[test]
fn an_interrupted_publication_converges() {
    let s = Scratch::new("interrupted");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");

    server.refuse_after(Some(4));
    let e = publish(&objects, &s3, "jp/canonical", &plan).unwrap_err();
    assert_eq!(e.stage, "files");
    assert!(server.object(POINTER).is_none());

    server.refuse_after(None);
    let (pointer, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    assert_eq!(report.uploaded + report.present, 17);
    assert!(report.present >= 4);
    assert_eq!(
        server.object(POINTER),
        Some(serde_json::to_vec(&pointer).unwrap())
    );
}

#[test]
fn a_failed_publication_keeps_the_previous_pointer() {
    let s = Scratch::new("keeps");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    publish(
        &objects,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
    )
    .unwrap();
    let previous = server.object(POINTER).unwrap();

    server.refuse_after(Some(0));
    let e = publish(&objects, &s3, "jp/canonical", &sample(&store, "v2", b"new")).unwrap_err();
    assert_eq!(e.stage, "files");
    assert_eq!(server.object(POINTER), Some(previous));
}

#[test]
fn an_object_the_store_does_not_have_is_taken_to_be_in_the_bucket() {
    let s = Scratch::new("missing");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let plan = sample(&store, "v1", b"mask");
    let file = plan
        .files
        .iter()
        .find(|f| f.path == "a/two/mask.webp")
        .unwrap();
    std::fs::remove_file(store.path(&file.key)).unwrap();
    let (_, report) = publish(
        &objects,
        &client(&server, SECRET_KEY),
        "jp/canonical",
        &plan,
    )
    .unwrap();
    assert!(report.pointer_written);
    assert!(server.object(&object_name(&file.key)).is_none());
    assert_eq!(held.size(&file.key).unwrap(), None);
}

#[test]
fn a_local_object_of_another_size_stops_before_the_pointer() {
    let s = Scratch::new("resized");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let plan = sample(&store, "v1", b"mask");
    std::fs::write(store.path(&plan.files[0].key), b"other bytes").unwrap();
    let e = publish(
        &objects,
        &client(&server, SECRET_KEY),
        "jp/canonical",
        &plan,
    )
    .unwrap_err();
    assert_eq!(e.stage, "verify");
    assert!(server.object(POINTER).is_none());
}

#[test]
fn a_publication_uploads_only_unrecorded_objects() {
    let s = Scratch::new("incremental");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);

    publish(
        &objects,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
    )
    .unwrap();

    // Identical publication sends 0 HEADs and 0 PUTs.
    publish(
        &objects,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
    )
    .unwrap();
    assert_eq!(server.count(Operation::Head), 0);

    // Changing one member uploads only the 11 changed objects (1 member, 1 document, the shards
    // "a" and "a/two", the chunks of the directories and of 1 listing, the root, the chunk and
    // root of the path index, the directory tree and the version history): the 6 unchanged
    // objects of v1 are recorded as held. The bucket is never asked about an object.
    let (_, report) = publish(
        &objects,
        &s3,
        "jp/canonical",
        &sample(&store, "v2", b"mask2"),
    )
    .unwrap();
    assert_eq!((report.uploaded, report.present), (11, 6));
    assert_eq!(server.count(Operation::Head), 0);
}

#[test]
fn a_listing_replaces_the_holdings_and_restores_lost_objects() {
    let s = Scratch::new("verify");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");
    publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    let lost = object_name(&plan.files[0].key);
    server.remove(&lost);

    // The holdings record the lost object, so a plain publication trusts them.
    let (_, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    assert_eq!(report.uploaded, 0);
    assert!(server.object(&lost).is_none());

    let listed = MemoryHoldings::listed(s3.holdings().unwrap());
    // One request for each empty store directory, and more where a directory fills pages.
    assert!(server.count(Operation::List) >= 256);
    let objects = Objects {
        local: &store,
        held: &listed,
        remote: &NoRemote,
    };
    let (_, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    assert_eq!((report.uploaded, report.present), (1, 16));
    assert!(!report.pointer_written);
    assert_eq!(
        server.object(&lost),
        Some(store.read(&plan.files[0].key).unwrap())
    );
    assert_eq!(server.count(Operation::Head), 0);
    assert_eq!(
        listed.size(&plan.files[0].key).unwrap(),
        Some(plan.files[0].size)
    );
}

#[test]
fn a_listing_without_an_object_the_store_lacks_is_an_error() {
    let s = Scratch::new("unlisted");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    let plan = sample(&store, "v1", b"mask");
    publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    let key = &plan.files[0].key;
    server.remove(&object_name(key));
    std::fs::remove_file(store.path(key)).unwrap();

    let listed = MemoryHoldings::listed(s3.holdings().unwrap());
    let objects = Objects {
        local: &store,
        held: &listed,
        remote: &NoRemote,
    };
    let e = publish(&objects, &s3, "jp/canonical", &plan).unwrap_err();
    assert_eq!(e.stage, "verify");
    assert!(e.to_string().contains(&key.to_string()), "{e}");
}

#[test]
fn an_object_the_bucket_holds_may_leave_the_store() {
    let s = Scratch::new("evicted");
    let store = s.store();
    let held = MemoryHoldings::default();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    publish(
        &objects,
        &s3,
        "jp/canonical",
        &sample(&store, "v1", b"mask"),
    )
    .unwrap();
    let plan = sample(&store, "v2", b"mask");
    for file in &plan.files {
        std::fs::remove_file(store.path(&file.key)).unwrap();
    }

    // The evicted files are held, so only the documents of the new version go up.
    let (_, report) = publish(&objects, &s3, "jp/canonical", &plan).unwrap();
    assert!(report.present >= plan.files.len());
    assert!(report.pointer_written);
}

#[test]
fn an_object_larger_than_ten_megabytes_is_read_back() {
    let server = Server::start();
    let bytes: Vec<u8> = (0..11 << 20).map(|i: u32| (i % 251) as u8).collect();
    let key = Key::of(&bytes, "bin");
    server.insert(&object_name(&key), bytes.clone());

    let fetched = client(&server, SECRET_KEY).fetch(&key).unwrap();
    assert_eq!(fetched.as_deref(), Some(bytes.as_slice()));
}

#[test]
fn a_response_cut_off_midway_is_asked_for_again() {
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    s3.write_state("records/a.json", &[7; 64]).unwrap();
    server.cut_gets(2);
    let read = s3.read_state("records/a.json").unwrap();
    assert_eq!(read.as_deref(), Some(&[7; 64][..]));
    assert_eq!(server.count(Operation::Get), 3);
}

#[test]
fn the_bucket_keeps_state_files_outside_its_publications() {
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    assert_eq!(s3.read_state("versions/jp.json").unwrap(), None);
    s3.write_state("versions/jp.json", b"{}").unwrap();
    let name = "out/state/versions/jp.json";
    assert_eq!(server.object(name).as_deref(), Some(&b"{}"[..]));
    let read = s3.read_state("versions/jp.json").unwrap();
    assert_eq!(read.as_deref(), Some(&b"{}"[..]));
    let cache_control = server.cache_control(name).unwrap();
    assert_eq!(cache_control, "private, no-store");
}

/// Bytes of 33 MiB and a few more, which make three parts.
fn large_state() -> Vec<u8> {
    (0..(33u32 << 20) + 5).map(|i| (i % 253) as u8).collect()
}

#[test]
fn a_large_state_file_is_written_in_parts() {
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    let bytes = large_state();

    s3.write_state("regions/jp.json", &bytes).unwrap();
    let name = "out/state/regions/jp.json";
    assert!(server.object(name) == Some(bytes), "{name}");
    let cache_control = server.cache_control(name).unwrap();
    assert_eq!(cache_control, "private, no-store");
    assert_eq!(server.count(Operation::PutPart), 3);
    assert!(server.uploads_in_progress() == 0);
}

#[test]
fn a_failed_upload_in_parts_is_aborted() {
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    server.refuse_after(Some(1));

    assert!(s3.write_state("regions/jp.json", &large_state()).is_err());
    assert_eq!(server.object("out/state/regions/jp.json"), None);
    assert!(server.uploads_in_progress() == 0);
}

#[test]
fn an_upload_the_server_timed_out_or_saw_closed_is_sent_again() {
    let server = Server::start();
    let s3 = client(&server, SECRET_KEY);
    server.fail_uploads(&[499, 408]);

    s3.write_state("records/a.json", &[7; 64]).unwrap();
    assert_eq!(server.object("out/state/records/a.json"), Some(vec![7; 64]));
    let statuses: Vec<_> = server
        .requests()
        .iter()
        .map(|logged| logged.status)
        .collect();
    assert_eq!(statuses, [499, 408, 200]);
}
