//! Publishing assets: determinism, verification and the order of a tree update.

use std::path::{Path, PathBuf};

use sekai_catalog::{
    Holdings, Key, LocalStore, MemoryHoldings, MemoryStore, NoRemote, Objects, Store,
};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::publication::Pointer;
use sekai_format::{AssetDocument, AssetError, MemberFile};
use sekai_publish::patch::{self, Previous};
use sekai_publish::plan::change;
use sekai_publish::{Binding, Layout, Plan, documents, plan, publish, tree};

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
    let second = plan("v1", &[b, a.clone()], &store, 2).unwrap();
    assert_eq!(first, second);
    let revision = |plan: &Plan| {
        let publication = documents(&store, "jp/canonical", plan, Layout::Index).unwrap();
        publication.pointer.revision
    };
    let other = plan("v1", &[a], &store, 2).unwrap();
    assert_eq!(revision(&first), revision(&second));
    assert_ne!(revision(&first), revision(&other));
}

#[test]
fn an_index_publication_is_reproducible() {
    let s = Scratch::new("index");
    let store = s.store();
    let bindings = [asset(&store, "a/one", &[("image", b"1")])];
    let plan = plan("v1", &bindings, &store, 2).unwrap();
    let root = s.0.join("published");
    let first = publish(
        &Objects::local(&store),
        &root,
        "jp/canonical",
        &plan,
        Layout::Index,
        2,
    )
    .unwrap();
    let bytes = std::fs::read(root.join("jp/canonical/current.json")).unwrap();
    let second = publish(
        &Objects::local(&store),
        &root,
        "jp/canonical",
        &plan,
        Layout::Index,
        2,
    )
    .unwrap();
    assert_eq!(first, second);
    assert_eq!(
        bytes,
        std::fs::read(root.join("jp/canonical/current.json")).unwrap()
    );
    let read: Pointer = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(read, first);
}

#[test]
fn a_missing_object_stops_the_pointer() {
    let s = Scratch::new("verify");
    let store = s.store();
    let root = s.0.join("published");
    let good = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store, 2).unwrap();
    publish(
        &Objects::local(&store),
        &root,
        "jp/t",
        &good,
        Layout::Index,
        2,
    )
    .unwrap();
    let before = std::fs::read(root.join("jp/t/current.json")).unwrap();

    let binding = asset(&store, "b", &[("image", b"gone")]);
    let plan = plan("v2", &[binding], &store, 2).unwrap();
    let member = plan.files.iter().find(|f| f.role == "image").unwrap();
    std::fs::remove_file(store.path(&member.key)).unwrap();
    let error = publish(
        &Objects::local(&store),
        &root,
        "jp/t",
        &plan,
        Layout::Index,
        2,
    )
    .unwrap_err();
    assert_eq!(error.stage, "verify");
    assert_eq!(
        before,
        std::fs::read(root.join("jp/t/current.json")).unwrap()
    );
}

#[test]
fn an_object_the_destination_holds_may_leave_the_store() {
    let s = Scratch::new("held");
    let store = s.store();
    let root = s.0.join("published");
    let plan = plan(
        "v1",
        &[asset(&store, "a", &[("image", b"evicted")])],
        &store,
        2,
    )
    .unwrap();
    let member = plan.files.iter().find(|f| f.role == "image").unwrap();
    std::fs::remove_file(store.path(&member.key)).unwrap();
    let held = MemoryHoldings::default();
    held.hold(&[(member.key.clone(), member.size)]).unwrap();
    let objects = Objects {
        local: &store,
        held: &held,
        remote: &NoRemote,
    };

    let pointer = publish(&objects, &root, "jp/t", &plan, Layout::Index, 2).unwrap();
    assert!(pointer.root.is_some());
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
fn sqlite_index_is_deterministic_and_valid() {
    use sekai_format::asset::SourceObject;

    let s = Scratch::new("sqlite");
    let store = s.store();
    let root = s.0.join("published");

    let mut bindings = Vec::new();
    // An asset of several members.
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

    let sqlite_of = |plan: &Plan| {
        let publication = documents(&store, "jp/canonical", plan, Layout::Sqlite).unwrap();
        publication.pointer.sqlite.unwrap()
    };
    let sqlite = sqlite_of(&forward);
    assert_eq!(sqlite, sqlite_of(&backward));
    let pointer = publish(
        &Objects::local(&store),
        &root,
        "jp/canonical",
        &forward,
        Layout::Sqlite,
        2,
    )
    .unwrap();
    assert_eq!(pointer.sqlite.as_deref(), Some(sqlite.as_str()));

    // SQLite itself checks the database, through the `sqlite3` module of Python.
    let python = ["python3", "python"]
        .into_iter()
        .find(|python| {
            std::process::Command::new(python)
                .args(["-c", "import sqlite3"])
                .status()
                .is_ok_and(|s| s.success())
        })
        .expect("the integrity check needs Python with its sqlite3 module");
    let db_path = store.path(&Key {
        sha256: sekai_catalog::store::parse_hex(&sqlite).unwrap(),
        extension: "sqlite".to_owned(),
    });
    let output = std::process::Command::new(python)
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

#[test]
fn a_plan_reads_the_documents_that_only_the_destination_holds() {
    let s = Scratch::new("read-back");
    let earlier = LocalStore::open(s.0.join("earlier")).unwrap();
    let bindings = [
        asset(&earlier, "a", &[("image", b"1")]),
        failed(&earlier, "b"),
    ];
    let remote = MemoryStore::default();
    let held = MemoryHoldings::default();
    for Binding { document, .. } in &bindings {
        let bytes = earlier.read(document).unwrap();
        remote.put(&bytes, &document.extension).unwrap();
        held.hold(&[(document.clone(), bytes.len() as u64)])
            .unwrap();
    }
    let local = s.store();
    let objects = Objects {
        local: &local,
        held: &held,
        remote: &remote,
    };

    let read_back = plan("v1", &bindings, &objects, 2).unwrap();
    assert_eq!(read_back, plan("v1", &bindings, &earlier, 2).unwrap());
    assert!(bindings.iter().all(|b| local.contains(&b.document)));
}

/// `count` assets in a few directories, every seventh of them failed.
fn many(store: &LocalStore, count: usize, salt: &str) -> Vec<Binding> {
    (0..count)
        .map(|i| {
            let name = format!("group_{:02}/item_{i:05}", i % 9);
            match i % 7 {
                0 => failed(store, &name),
                3 => asset(store, &name, &[("image", b"a"), ("mask", salt.as_bytes())]),
                _ => asset(store, &name, &[("image", format!("{i}").as_bytes())]),
            }
        })
        .collect()
}

#[test]
fn a_change_gives_the_publication_of_the_new_assets() {
    let s = Scratch::new("change");
    let store = s.store();
    let old = many(&store, 6000, "x");
    let mut new = old.clone();
    new.remove(4321);
    new.remove(10);
    new[77] = failed(&store, &new[77].name);
    new[700] = asset(&store, &new[700].name, &[("image", b"changed")]);
    new[3500] = asset(&store, &new[3500].name, &[("image", b"a"), ("mask", b"y")]);
    new.push(asset(&store, "new_directory/one", &[("image", b"new")]));
    new.sort_by(|a, b| a.name.cmp(&b.name));

    for layout in [Layout::Index, Layout::Paths] {
        let before = documents(
            &store,
            "jp/canonical",
            &plan("v1", &old, &store, 2).unwrap(),
            layout,
        );
        let before = before.unwrap();
        let previous = Previous::read(&store, &before.pointer).unwrap();
        let change = change(&old, &new, &store, 2).unwrap();
        let patched =
            patch::apply(&store, &store, "canonical", "v2", layout, &previous, change).unwrap();
        let whole = plan("v2", &new, &store, 2).unwrap();
        let whole = documents(&store, "jp/canonical", &whole, layout).unwrap();
        assert_eq!(patched.pointer, whole.pointer, "{layout:?}");
        assert!(
            patched.objects.len() * 20 < whole.objects.len(),
            "{layout:?}: {} of {}",
            patched.objects.len(),
            whole.objects.len()
        );
    }
}

#[test]
fn a_change_that_changes_nothing_keeps_every_document() {
    let s = Scratch::new("unchanged");
    let store = s.store();
    let assets = many(&store, 2000, "x");
    let before = documents(
        &store,
        "jp/canonical",
        &plan("v1", &assets, &store, 2).unwrap(),
        Layout::Index,
    );
    let before = before.unwrap();
    let previous = Previous::read(&store, &before.pointer).unwrap();
    let change = change(&assets, &assets, &store, 2).unwrap();
    assert!(change.is_empty());
    let after = patch::apply(
        &store,
        &store,
        "canonical",
        "v1",
        Layout::Index,
        &previous,
        change,
    )
    .unwrap();
    assert_eq!(after.pointer, before.pointer);
}

#[test]
fn a_change_may_not_publish_two_files_at_one_path() {
    let s = Scratch::new("conflict");
    let store = s.store();
    let old = vec![asset(&store, "d/a", &[("image", b"1"), ("mask", b"2")])];
    let before = documents(
        &store,
        "jp/t",
        &plan("v1", &old, &store, 2).unwrap(),
        Layout::Index,
    );
    let previous = Previous::read(&store, &before.unwrap().pointer).unwrap();
    // The single member of d/a/image lands on the path of the image of d/a.
    let mut new = old.clone();
    new.push(asset(&store, "d/a/image", &[("image", b"3")]));
    for layout in [Layout::Index, Layout::Paths] {
        let change = change(&old, &new, &store, 2).unwrap();
        let error = patch::apply(&store, &store, "t", "v2", layout, &previous, change);
        let error = error.unwrap_err();
        assert!(error.message.contains("d/a/image.webp"), "{error}");
    }
}
