//! Publishing assets: determinism, verification and the order of a tree update.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use sekai_catalog::{
    Holdings, Key, LocalStore, MemoryHoldings, MemoryStore, NoRemote, Objects, Store,
};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::publication::{POINTER_VERSION, Pointer};
use sekai_format::{AssetDocument, AssetError, MemberFile};
use sekai_publish::history::NoVersions;
use sekai_publish::patch::{self, Change, Edition, Previous};
use sekai_publish::plan::change;
use sekai_publish::{
    Binding, File, Layout, Plan, Publication, PublishError, documents, plan, publish, referred,
    tree,
};

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
fn a_publication_interrupted_after_its_listing_still_retires_removed_files() {
    let s = Scratch::new("interrupted-tree");
    let store = s.store();
    let root = s.0.join("tree");
    let a = asset(&store, "a", &[("image", b"1")]);
    let b = asset(&store, "b", &[("image", b"2")]);

    let one = plan("v1", &[a.clone(), b], &store, 2).unwrap();
    tree::publish(&store, &root, &one, 2).unwrap();
    // v2 stops at its deletions: the listing is written and the state still names `b` retiring.
    let two = plan("v2", std::slice::from_ref(&a), &store, 2).unwrap();
    tree::publish(&store, &root, &two, 2).unwrap();
    std::fs::write(
        root.join("_state.json"),
        br#"{"pending_delete":[],"retiring":["b.asset.json","b.webp"]}"#,
    )
    .unwrap();

    // Running v2 again keeps `b` for readers of v1, and the next publication deletes it.
    assert_eq!(tree::publish(&store, &root, &two, 2).unwrap().deleted, 0);
    assert!(root.join("b.webp").exists());
    let three = plan("v3", &[a], &store, 2).unwrap();
    assert_eq!(tree::publish(&store, &root, &three, 2).unwrap().deleted, 2);
    assert!(!root.join("b.webp").exists());
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

/// Applies `change` to `previous` as the publication of `version` of `view` in `layout`, which
/// records no time and finds no other published version.
fn apply(
    store: &LocalStore,
    view: &str,
    version: &str,
    layout: Layout,
    previous: &Previous,
    change: Change,
) -> Result<Publication, PublishError> {
    let edition = Edition {
        view,
        version,
        layout,
        published_at: None,
    };
    patch::apply(store, store, &edition, previous, change, &NoVersions)
}

/// `pointer` without its version history and changes, which follow the publications before it:
/// what a publication of the same files gives, whatever came before.
fn contents(pointer: Pointer) -> Pointer {
    Pointer {
        history: None,
        changes: None,
        ..pointer
    }
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
        let patch = |previous: &Previous| {
            apply(&store, "canonical", "v2", layout, previous, change.clone())
        };
        let patched = patch(&previous).unwrap();
        let whole = plan("v2", &new, &store, 2).unwrap();
        let whole = documents(&store, "jp/canonical", &whole, layout).unwrap();
        let expected = contents(whole.pointer.clone());
        assert_eq!(contents(patched.pointer.clone()), expected, "{layout:?}");
        // Without the directory tree of the previous publication, the new one is made from every
        // file of its path index, and is the same.
        let without_tree = patch(&Previous {
            tree: None,
            ..previous.clone()
        });
        let without_tree = without_tree.unwrap().pointer;
        assert_eq!(contents(without_tree), expected, "{layout:?}");
        assert!(
            patched.objects.len() * 20 < whole.objects.len(),
            "{layout:?}: {} of {}",
            patched.objects.len(),
            whole.objects.len()
        );
    }
}

#[test]
fn a_version_with_the_same_assets_keeps_the_publication() {
    let s = Scratch::new("same assets");
    let store = s.store();
    let assets = many(&store, 300, "x");
    for layout in [Layout::Index, Layout::Paths] {
        let before = documents(
            &store,
            "jp/canonical",
            &plan("v1", &assets, &store, 2).unwrap(),
            layout,
        );
        let before = before.unwrap();
        let previous = Previous::read(&store, &before.pointer).unwrap();
        let same = change(&assets, &assets, &store, 2).unwrap();
        let kept = apply(&store, "canonical", "v2", layout, &previous, same).unwrap();
        assert_eq!(kept.pointer, before.pointer, "{layout:?}");
        assert!(kept.objects.is_empty(), "{layout:?}");
    }
}

#[test]
fn a_change_to_a_publication_of_an_earlier_pointer_format_gives_the_whole_publication() {
    let s = Scratch::new("earlier-pointer");
    let store = s.store();
    let old = many(&store, 3000, "x");
    let mut new = old.clone();
    new[5] = failed(&store, &new[5].name);
    new[100] = asset(&store, &new[100].name, &[("image", b"changed")]);
    let before = documents(
        &store,
        "jp/canonical",
        &plan("v1", &old, &store, 2).unwrap(),
        Layout::Index,
    );
    // The pointer of the earlier format names the same documents, but no history or changes.
    let earlier = Pointer {
        version: POINTER_VERSION - 1,
        history: None,
        changes: None,
        ..before.unwrap().pointer
    };
    let previous = Previous::read(&store, &earlier).unwrap();
    let change = change(&old, &new, &store, 2).unwrap();
    let patched = apply(&store, "canonical", "v2", Layout::Index, &previous, change).unwrap();
    let whole = plan("v2", &new, &store, 2).unwrap();
    let whole = documents(&store, "jp/canonical", &whole, Layout::Index).unwrap();
    assert_eq!(patched.pointer.version, POINTER_VERSION);
    assert_eq!(contents(patched.pointer), contents(whole.pointer));
}

#[test]
fn a_publication_gives_back_the_documents_of_the_assets_it_names() {
    let s = Scratch::new("bindings");
    let store = s.store();
    let assets = many(&store, 6000, "x");
    let published = documents(
        &store,
        "jp/canonical",
        &plan("v1", &assets, &store, 2).unwrap(),
        Layout::Paths,
    );
    let previous = Previous::read(&store, &published.unwrap().pointer).unwrap();
    let wanted = [&assets[5000], &assets[3], &assets[4999]];
    let names = wanted
        .iter()
        .map(|b| b.name.as_str())
        .chain(["missing/one"]);
    let mut expected: Vec<Binding> = wanted.into_iter().cloned().collect();
    expected.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(previous.lookup(&store).bindings(names).unwrap(), expected);
    let nothing = Previous::default()
        .lookup(&store)
        .bindings(["group_00/item_00000"]);
    assert!(nothing.unwrap().is_empty());
}

#[test]
fn a_pointer_leads_to_every_object_its_publication_refers_to() {
    let s = Scratch::new("referred");
    let store = s.store();
    let assets = many(&store, 3000, "x");
    let plan = plan("v1", &assets, &store, 2).unwrap();
    for layout in [Layout::Index, Layout::Paths] {
        let Publication { pointer, objects } =
            documents(&store, "jp/canonical", &plan, layout).unwrap();
        let found: BTreeSet<Key> = referred(&store, &pointer, 3).unwrap().into_iter().collect();
        let expected: BTreeSet<Key> = objects.into_iter().map(|object| object.key).collect();
        assert_eq!(found, expected, "{layout:?}");
    }
}

/// Whether `path` is directly in `directory`.
fn directly_in(path: &str, directory: &str) -> bool {
    let file = path
        .strip_prefix(directory)
        .and_then(|p| p.strip_prefix('/'));
    file.is_some_and(|file| !file.contains('/'))
}

/// A plan of `files` alone, as a view without listings publishes them.
fn files_plan(version: &str, mut files: Vec<File>) -> Plan {
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Plan {
        version: version.to_owned(),
        files,
        assets: Vec::new(),
        listings: Default::default(),
        failures: Vec::new(),
    }
}

#[test]
fn a_directory_is_replaced_file_by_file() {
    let s = Scratch::new("directory");
    let store = s.store();
    let assets = plan("v1", &many(&store, 6000, "x"), &store, 2).unwrap();
    let before = documents(
        &store,
        "jp/storage",
        &files_plan("v1", assets.files.clone()),
        Layout::Paths,
    );
    let previous = Previous::read(&store, &before.unwrap().pointer).unwrap();

    let lookup = previous.lookup(&store);
    let directory = lookup.directory("group_03").unwrap();
    let paths: Vec<&str> = directory.iter().map(|f| f.path.as_str()).collect();
    let expected: Vec<&str> = assets
        .files
        .iter()
        .map(|f| f.path.as_str())
        .filter(|path| directly_in(path, "group_03"))
        .collect();
    assert!(expected.len() > 100);
    assert_eq!(paths, expected);
    assert!(lookup.directory("group_3").unwrap().is_empty());

    // The directory now holds its files but the first, and a new one.
    let added = File {
        path: "group_03/new.webp".to_owned(),
        key: store.put(b"new", "webp").unwrap(),
        size: 3,
        media_type: "image/webp".to_owned(),
        asset: "group_03/new".to_owned(),
        role: "image".to_owned(),
    };
    let mut change = Change::default();
    change.remove_file(paths[0]);
    change.add_file(added.clone()).unwrap();
    let mut files: Vec<File> = assets
        .files
        .iter()
        .filter(|f| f.path != paths[0])
        .cloned()
        .collect();
    files.push(added);
    for layout in [Layout::Paths, Layout::Index] {
        let before = documents(
            &store,
            "jp/storage",
            &files_plan("v1", assets.files.clone()),
            layout,
        );
        let previous = Previous::read(&store, &before.unwrap().pointer).unwrap();
        let patched = apply(&store, "storage", "v2", layout, &previous, change.clone());
        let whole = documents(
            &store,
            "jp/storage",
            &files_plan("v2", files.clone()),
            layout,
        );
        assert_eq!(
            contents(patched.unwrap().pointer),
            contents(whole.unwrap().pointer),
            "{layout:?}"
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
    let after = apply(&store, "canonical", "v1", Layout::Index, &previous, change).unwrap();
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
        let error = apply(&store, "t", "v2", layout, &previous, change);
        let error = error.unwrap_err();
        assert!(error.message.contains("d/a/image.webp"), "{error}");
    }
}
