//! Publishing assets: determinism, verification and the order of a tree update.

use std::path::{Path, PathBuf};

use sekai_catalog::{Key, LocalStore};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::publication::Pointer;
use sekai_format::{AssetDocument, AssetError, MemberFile};
use sekai_publish::{Binding, Plan, index, plan, tree};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sekai-pub-{}-{name}", std::process::id()));
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

fn hex(key: &Key) -> String {
    key.sha256.iter().map(|b| format!("{b:02x}")).collect()
}

fn asset(store: &LocalStore, name: &str, members: &[(&str, &[u8])]) -> Binding {
    let members = members
        .iter()
        .map(|(role, bytes)| {
            let key = store.put(bytes, "webp").unwrap();
            MemberFile {
                role: (*role).to_owned(),
                sha256: hex(&key),
                size: bytes.len() as u64,
                extension: "webp".to_owned(),
                media_type: "image/webp".to_owned(),
            }
        })
        .collect();
    document(store, name, members, None)
}

fn failed(store: &LocalStore, name: &str) -> Binding {
    let error = AssetError {
        code: "unsupported".to_owned(),
        message: "x".to_owned(),
    };
    document(store, name, Vec::new(), Some(error))
}

fn document(
    store: &LocalStore,
    name: &str,
    members: Vec<MemberFile>,
    error: Option<AssetError>,
) -> Binding {
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
        error,
    };
    Binding {
        name: name.to_owned(),
        document: store.put(&document.to_bytes().unwrap(), "json").unwrap(),
    }
}

fn paths(plan: &Plan) -> Vec<&str> {
    plan.files.iter().map(|f| f.path.as_str()).collect()
}

fn read_tree(root: &Path, path: &str) -> Vec<u8> {
    std::fs::read(root.join(path)).unwrap()
}

#[test]
fn members_get_paths_from_the_asset_name() {
    let s = Scratch::new("paths");
    let store = s.store();
    let bindings = [
        asset(&store, "a/one", &[("image", b"1")]),
        asset(&store, "a/two", &[("base", b"2"), ("mask", b"3")]),
        failed(&store, "b/bad"),
    ];
    let plan = plan("v1", &bindings, &store, 2).unwrap();
    assert_eq!(
        paths(&plan),
        [
            "a/one.asset.json",
            "a/one.webp",
            "a/two.asset.json",
            "a/two/base.webp",
            "a/two/mask.webp",
            "b/bad.asset.json",
        ]
    );
    assert_eq!(plan.failures[0].code, "unsupported");
    assert_eq!(plan.listings["texture"].len(), 2);
}

#[test]
fn the_revision_does_not_depend_on_the_order_of_bindings() {
    let s = Scratch::new("revision");
    let store = s.store();
    let a = asset(&store, "a", &[("image", b"1")]);
    let b = asset(&store, "b", &[("image", b"2")]);
    let first = plan("v1", &[a.clone(), b.clone()], &store, 2).unwrap();
    let second = plan("v1", &[b, a], &store, 2).unwrap();
    assert_eq!(first, second);
    let other = plan("v2", &[asset(&store, "a", &[("image", b"1")])], &store, 2).unwrap();
    assert_ne!(first.revision, other.revision);
}

#[test]
fn an_index_publication_is_reproducible() {
    let s = Scratch::new("index");
    let store = s.store();
    let bindings = [asset(&store, "a/one", &[("image", b"1")])];
    let plan = plan("v1", &bindings, &store, 2).unwrap();
    let root = s.0.join("published");
    let first = index::publish(&store, &root, "jp/canonical", &plan, 2).unwrap();
    let bytes = std::fs::read(root.join("jp/canonical/current.json")).unwrap();
    let second = index::publish(&store, &root, "jp/canonical", &plan, 2).unwrap();
    assert_eq!(first, second);
    assert_eq!(
        bytes,
        std::fs::read(root.join("jp/canonical/current.json")).unwrap()
    );
    let read: Pointer = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read.revision, plan.revision);
}

#[test]
fn a_missing_object_stops_the_pointer() {
    let s = Scratch::new("verify");
    let store = s.store();
    let root = s.0.join("published");
    let good = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store, 2).unwrap();
    index::publish(&store, &root, "t", &good, 2).unwrap();
    let before = std::fs::read(root.join("t/current.json")).unwrap();

    let binding = asset(&store, "b", &[("image", b"gone")]);
    let plan = plan("v2", &[binding], &store, 2).unwrap();
    let member = plan.files.iter().find(|f| f.role == "image").unwrap();
    std::fs::remove_file(store.path(&member.key)).unwrap();
    let error = index::publish(&store, &root, "t", &plan, 2).unwrap_err();
    assert_eq!(error.stage, "verify");
    assert_eq!(before, std::fs::read(root.join("t/current.json")).unwrap());
}

#[test]
fn a_removed_file_leaves_the_tree_one_publication_later() {
    let s = Scratch::new("tree");
    let store = s.store();
    let root = s.0.join("tree");
    let a = asset(&store, "a", &[("image", b"1")]);
    let b = asset(&store, "b", &[("image", b"2")]);

    let one = plan("v1", &[a.clone(), b], &store, 2).unwrap();
    assert_eq!(tree::publish(&store, &root, &one, 2).unwrap().written, 4);

    let two = plan("v2", std::slice::from_ref(&a), &store, 2).unwrap();
    let report = tree::publish(&store, &root, &two, 2).unwrap();
    assert_eq!((report.written, report.deleted), (0, 0));
    assert_eq!(read_tree(&root, "b.webp"), b"2");

    let three = plan("v3", &[a], &store, 2).unwrap();
    assert_eq!(tree::publish(&store, &root, &three, 2).unwrap().deleted, 2);
    assert!(!root.join("b.webp").exists());
    assert_eq!(read_tree(&root, "a.webp"), b"1");
}

#[test]
fn a_failed_asset_keeps_its_previous_files() {
    let s = Scratch::new("failed");
    let store = s.store();
    let root = s.0.join("tree");
    let good = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store, 2).unwrap();
    tree::publish(&store, &root, &good, 2).unwrap();
    let document = read_tree(&root, "a.asset.json");

    for version in ["v2", "v3"] {
        let bad = plan(version, &[failed(&store, "a")], &store, 2).unwrap();
        let report = tree::publish(&store, &root, &bad, 2).unwrap();
        assert_eq!(report.deleted, 0);
        assert_eq!(read_tree(&root, "a.webp"), b"1");
        assert_eq!(read_tree(&root, "a.asset.json"), document);
    }
}

#[test]
fn publishing_again_changes_nothing() {
    let s = Scratch::new("again");
    let store = s.store();
    let root = s.0.join("tree");
    let plan = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store, 2).unwrap();
    tree::publish(&store, &root, &plan, 2).unwrap();
    let listing = read_tree(&root, "_listing.json");
    let report = tree::publish(&store, &root, &plan, 2).unwrap();
    assert_eq!(
        (report.written, report.unchanged, report.deleted),
        (0, 2, 0)
    );
    assert_eq!(listing, read_tree(&root, "_listing.json"));
}

#[test]
fn sqlite_index_is_deterministic_packs_composite_forms_and_validates() {
    use sekai_format::asset::SourceObject;
    use sekai_publish::sqlite::{self, SqliteOptions};
    use sekai_publish::tar;

    let s = Scratch::new("sqlite");
    let store = s.store();
    let root = s.0.join("published");

    let mut bindings = Vec::new();
    // Multi-member composite Live2D model (packed into .tar under default options).
    let l2d_members: Vec<MemberFile> = [
        (
            "model",
            "json",
            "application/json",
            br#"{"version":3}"#.as_slice(),
        ),
        (
            "moc3",
            "moc3",
            "application/octet-stream",
            b"MOC3_BINARY_DATA".as_slice(),
        ),
        (
            "textures/00",
            "webp",
            "image/webp",
            b"RIFF_WEBP_00".as_slice(),
        ),
    ]
    .into_iter()
    .map(|(role, ext, mime, data)| {
        let key = store.put(data, ext).unwrap();
        MemberFile {
            role: role.to_owned(),
            sha256: hex(&key),
            size: data.len() as u64,
            extension: ext.to_owned(),
            media_type: mime.to_owned(),
        }
    })
    .collect();
    let l2d_doc = AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: "chara/model_a".to_owned(),
        form: "live2d-model".to_owned(),
        form_version: 2,
        fingerprint: "11".repeat(32),
        sources: vec![SourceObject {
            bundle: "chara_bundle".to_owned(),
            path_id: 10,
            class: "MonoBehaviour".to_owned(),
        }],
        members: l2d_members,
        resources: Vec::new(),
        references: Vec::new(),
        error: None,
    };
    bindings.push(Binding {
        name: "chara/model_a".to_owned(),
        document: store.put(&l2d_doc.to_bytes().unwrap(), "json").unwrap(),
    });

    // Enough standalone textures + an overflow-sized source list to exercise multi-page B-trees
    // and SQLite payload overflow chains.
    for idx in 0..220u32 {
        let name = format!("tex/group_{:02}/img_{idx:03}", idx % 7);
        let payload = format!("webp-bytes-{idx}").into_bytes();
        let mkey = store.put(&payload, "webp").unwrap();
        let sources = if idx == 0 {
            (0..120)
                .map(|sidx| SourceObject {
                    bundle: format!("tex_bundle_long_identifier_{sidx:03}"),
                    path_id: i64::from(sidx + 1),
                    class: "Texture2D".to_owned(),
                })
                .collect()
        } else {
            Vec::new()
        };
        let doc = AssetDocument {
            format: FORMAT.to_owned(),
            version: VERSION,
            name: name.clone(),
            form: "texture".to_owned(),
            form_version: 1,
            fingerprint: "44".repeat(32),
            sources,
            members: vec![MemberFile {
                role: "image".to_owned(),
                sha256: hex(&mkey),
                size: payload.len() as u64,
                extension: "webp".to_owned(),
                media_type: "image/webp".to_owned(),
            }],
            resources: Vec::new(),
            references: Vec::new(),
            error: None,
        };
        bindings.push(Binding {
            name,
            document: store.put(&doc.to_bytes().unwrap(), "json").unwrap(),
        });
    }
    bindings.push(failed(&store, "broken/item"));

    let forward = plan("v1", &bindings, &store, 2).unwrap();
    let mut rev_bindings = bindings;
    rev_bindings.reverse();
    let backward = plan("v1", &rev_bindings, &store, 2).unwrap();

    let built_a = sqlite::build(&store, &forward, &SqliteOptions::default(), 2).unwrap();
    let built_b = sqlite::build(&store, &backward, &SqliteOptions::default(), 2).unwrap();
    assert_eq!(built_a, built_b);
    assert_eq!(built_a.archives.len(), 1);
    assert_eq!(built_a.direct_files.len(), 220);

    // Verify `.tar` archive member offsets match raw bytes (in sorted member path order).
    let tar_bytes = store.read(&built_a.archives[0].0).unwrap();
    let tar_entries = tar::entries(&tar_bytes).unwrap();
    assert_eq!(tar_entries.len(), 3);
    assert_eq!(tar_entries[0].path, "moc3.moc3");
    let start = tar_entries[0].offset as usize;
    let end = start + tar_entries[0].size as usize;
    assert_eq!(&tar_bytes[start..end], b"MOC3_BINARY_DATA");
    assert_eq!(tar_entries[1].path, "model.json");

    // Verify pure-Rust `referenced_keys` extracts the 1 `.tar` + 220 `.webp` keys.
    let db_bytes = store.read(&built_a.sqlite).unwrap();
    let refs = sqlite::referenced_keys(&db_bytes).unwrap();
    assert_eq!(refs.len(), 221);
    assert!(refs.contains(&built_a.archives[0].0));

    // Verify local `sqlite::publish` populates `pointer.sqlite`.
    let pointer = sqlite::publish(
        &store,
        &root,
        "jp/canonical",
        &forward,
        &SqliteOptions::unpacked(),
        2,
    )
    .unwrap();
    assert!(pointer.sqlite.is_some());

    // When Python with `sqlite3` is present on the host, run `PRAGMA integrity_check` as well.
    let has_python_sqlite = std::process::Command::new("python3")
        .args(["-c", "import sqlite3"])
        .status()
        .is_ok_and(|s| s.success());
    if has_python_sqlite {
        let db_path = store.path(&built_a.sqlite);
        let output = std::process::Command::new("python3")
            .args([
                "-c",
                "import sqlite3, sys; c = sqlite3.connect(sys.argv[1]); \
                 assert c.execute('PRAGMA integrity_check').fetchall() == [('ok',)]; \
                 assert c.execute('SELECT COUNT(*) FROM assets').fetchone() == (222,); \
                 assert c.execute('SELECT COUNT(*) FROM files').fetchone() == (223,)",
            ])
            .arg(&db_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "sqlite3 integrity check failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
