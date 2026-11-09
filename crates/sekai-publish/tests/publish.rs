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
    let plan = plan("v1", &bindings, &store).unwrap();
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
    let first = plan("v1", &[a.clone(), b.clone()], &store).unwrap();
    let second = plan("v1", &[b, a], &store).unwrap();
    assert_eq!(first, second);
    let other = plan("v2", &[asset(&store, "a", &[("image", b"1")])], &store).unwrap();
    assert_ne!(first.revision, other.revision);
}

#[test]
fn an_index_publication_is_reproducible() {
    let s = Scratch::new("index");
    let store = s.store();
    let bindings = [asset(&store, "a/one", &[("image", b"1")])];
    let plan = plan("v1", &bindings, &store).unwrap();
    let root = s.0.join("published");
    let first = index::publish(&store, &root, "jp/canonical", &plan).unwrap();
    let bytes = std::fs::read(root.join("jp/canonical/current.json")).unwrap();
    let second = index::publish(&store, &root, "jp/canonical", &plan).unwrap();
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
    let good = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store).unwrap();
    index::publish(&store, &root, "t", &good).unwrap();
    let before = std::fs::read(root.join("t/current.json")).unwrap();

    let binding = asset(&store, "b", &[("image", b"gone")]);
    let plan = plan("v2", &[binding], &store).unwrap();
    let member = plan.files.iter().find(|f| f.role == "image").unwrap();
    std::fs::remove_file(store.path(&member.key)).unwrap();
    let error = index::publish(&store, &root, "t", &plan).unwrap_err();
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

    let one = plan("v1", &[a.clone(), b], &store).unwrap();
    assert_eq!(tree::publish(&store, &root, &one).unwrap().written, 4);

    let two = plan("v2", std::slice::from_ref(&a), &store).unwrap();
    let report = tree::publish(&store, &root, &two).unwrap();
    assert_eq!((report.written, report.deleted), (0, 0));
    assert_eq!(read_tree(&root, "b.webp"), b"2");

    let three = plan("v3", &[a], &store).unwrap();
    assert_eq!(tree::publish(&store, &root, &three).unwrap().deleted, 2);
    assert!(!root.join("b.webp").exists());
    assert_eq!(read_tree(&root, "a.webp"), b"1");
}

#[test]
fn a_failed_asset_keeps_its_previous_files() {
    let s = Scratch::new("failed");
    let store = s.store();
    let root = s.0.join("tree");
    let good = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store).unwrap();
    tree::publish(&store, &root, &good).unwrap();
    let document = read_tree(&root, "a.asset.json");

    for version in ["v2", "v3"] {
        let bad = plan(version, &[failed(&store, "a")], &store).unwrap();
        let report = tree::publish(&store, &root, &bad).unwrap();
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
    let plan = plan("v1", &[asset(&store, "a", &[("image", b"1")])], &store).unwrap();
    tree::publish(&store, &root, &plan).unwrap();
    let listing = read_tree(&root, "_listing.json");
    let report = tree::publish(&store, &root, &plan).unwrap();
    assert_eq!(
        (report.written, report.unchanged, report.deleted),
        (0, 2, 0)
    );
    assert_eq!(listing, read_tree(&root, "_listing.json"));
}
