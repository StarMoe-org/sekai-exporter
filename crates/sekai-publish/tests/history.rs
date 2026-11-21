//! Version histories: one entry per published version, newest first, and the changes of each
//! version since the version before it, also when a version is published again.

use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, LocalStore};
use sekai_format::publication::{ObjectReference, Pointer};
use sekai_format::version_history::{
    AddedFile, Cause, ChangeCounts, ChangedFile, HistoryEntry, RemovedFile, VersionChanges,
    VersionHistory,
};
use sekai_publish::history::{NoVersions, Versions};
use sekai_publish::patch::{self, Change, Edition, Previous};
use sekai_publish::{File, Layout, Plan, PublishError, documents};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "sekai-publish-history-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        Self(path)
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

/// The pointers of the versions published so far, the latest publication of each.
#[derive(Default)]
struct Published(Vec<Pointer>);

impl Versions for Published {
    fn pointer(&self, version: &str) -> Result<Option<Pointer>, PublishError> {
        let found = self.0.iter().rev().find(|p| p.published_version == version);
        Ok(found.cloned())
    }
}

/// A file of the storage view at `path` with `bytes`.
fn file(store: &LocalStore, path: &str, bytes: &[u8]) -> File {
    File {
        path: path.to_owned(),
        key: store.put(bytes, "png").unwrap(),
        size: bytes.len() as u64,
        media_type: "image/png".to_owned(),
        asset: path.to_owned(),
        role: "image".to_owned(),
    }
}

/// The first publication of the storage view, of `files`.
fn first(store: &LocalStore, files: Vec<File>) -> Pointer {
    let plan = Plan {
        version: "1".to_owned(),
        files,
        assets: Vec::new(),
        listings: Default::default(),
        failures: Vec::new(),
    };
    documents(store, "jp/storage", &plan, Layout::Paths)
        .unwrap()
        .pointer
}

/// The time `seconds` after 1970.
fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

/// Publishes `version` of the storage view at `time`, as `change` to `previous`.
fn publish(
    store: &LocalStore,
    version: &str,
    time: SystemTime,
    previous: &Previous,
    change: Change,
    versions: &dyn Versions,
) -> Pointer {
    let edition = Edition {
        view: "storage",
        version,
        layout: Layout::Paths,
        published_at: Some(time),
    };
    let publication = patch::apply(store, store, &edition, previous, change, versions, 4);
    publication.unwrap().pointer
}

fn read_gzip(store: &LocalStore, document: &ObjectReference) -> Vec<u8> {
    let key = Key {
        sha256: parse_hex(&document.sha256).unwrap(),
        extension: "json.gz".to_owned(),
    };
    let bytes = store.read(&key).unwrap();
    let mut json = Vec::new();
    GzDecoder::new(bytes.as_slice())
        .read_to_end(&mut json)
        .unwrap();
    json
}

fn history(store: &LocalStore, pointer: &Pointer) -> Vec<HistoryEntry> {
    let json = read_gzip(store, pointer.history.as_ref().unwrap());
    let history = VersionHistory::from_json(&json).unwrap();
    assert_eq!(history.view, "storage");
    history.entries
}

fn changes(store: &LocalStore, pointer: &Pointer) -> Option<VersionChanges> {
    let changes = pointer.changes.as_ref()?;
    Some(VersionChanges::from_json(&read_gzip(store, changes)).unwrap())
}

fn entry(
    pointer: &Pointer,
    time: Option<&str>,
    files: u64,
    counts: Option<[u64; 3]>,
) -> HistoryEntry {
    HistoryEntry {
        published_version: pointer.published_version.clone(),
        revision: pointer.revision.clone(),
        published_at: time.map(str::to_owned),
        files,
        counts: counts.map(|[added, removed, changed]| ChangeCounts {
            added,
            removed,
            changed,
        }),
        conversion: None,
    }
}

fn added(path: &str, size: u64) -> AddedFile {
    AddedFile {
        path: path.to_owned(),
        size,
        media_type: "image/png".to_owned(),
        cause: None,
    }
}

/// The change that replaces the files at `removed` and puts `added`, with no cause known.
fn change(removed: &[&str], added: &[File]) -> Change {
    caused(removed, added, None)
}

/// The change that replaces the files at `removed` and puts `added`, following from `cause`.
fn caused(removed: &[&str], added: &[File], cause: Option<Cause>) -> Change {
    let mut change = Change::default();
    for path in removed {
        change.remove_file(path, cause);
    }
    for file in added {
        change.add_file(file.clone(), cause).unwrap();
    }
    change
}

/// Version 1 has `a`, `b` and `c`. Version 2 removes `a`, changes `b`, adds `d`, and writes `c`
/// again with the same bytes.
fn two_versions(store: &LocalStore, published: &mut Published) -> Pointer {
    let one = first(
        store,
        vec![
            file(store, "a.png", b"a"),
            file(store, "b.png", b"b"),
            file(store, "c.png", b"c"),
        ],
    );
    published.0.push(one.clone());
    let previous = Previous::read(store, &one).unwrap();
    let edit = change(
        &["a.png", "b.png", "c.png"],
        &[
            file(store, "b.png", b"b2"),
            file(store, "c.png", b"c"),
            file(store, "d.png", b"dd"),
        ],
    );
    let two = publish(store, "2", at(86_400), &previous, edit, published);
    published.0.push(two.clone());
    two
}

#[test]
fn a_history_lists_each_version_newest_first_with_its_changes() {
    let s = Scratch::new("versions");
    let store = s.store();
    let mut published = Published::default();
    let two = two_versions(&store, &mut published);
    let one = &published.0[0];

    let expected = [
        entry(&two, Some("1970-01-02T00:00:00Z"), 3, Some([1, 1, 1])),
        entry(one, None, 3, None),
    ];
    assert_eq!(history(&store, &two), expected);
    assert_eq!(history(&store, one), expected[1..]);
    assert!(one.changes.is_none());

    let changes = changes(&store, &two).unwrap();
    assert_eq!((changes.from.as_str(), changes.to.as_str()), ("1", "2"));
    assert_eq!(changes.added, [added("d.png", 2)]);
    let removed = RemovedFile {
        path: "a.png".to_owned(),
        size: 1,
        cause: None,
    };
    assert_eq!(changes.removed, [removed]);
    let changed = ChangedFile {
        path: "b.png".to_owned(),
        size: 2,
        previous_size: 1,
        cause: None,
    };
    assert_eq!(changes.changed, [changed]);
}

#[test]
fn a_version_published_again_keeps_its_changes_since_the_version_before() {
    let s = Scratch::new("again");
    let store = s.store();
    let mut published = Published::default();
    let two = two_versions(&store, &mut published);

    // Version 2 again: `b` is back to its bytes of version 1, `d` changes, `e` is added.
    let previous = Previous::read(&store, &two).unwrap();
    let edit = change(
        &["b.png", "d.png"],
        &[
            file(&store, "b.png", b"b"),
            file(&store, "d.png", b"ddd"),
            file(&store, "e.png", b"e"),
        ],
    );
    let again = publish(&store, "2", at(2 * 86_400), &previous, edit, &published);
    assert_ne!(again.revision, two.revision);

    let entries = history(&store, &again);
    let expected = entry(&again, Some("1970-01-03T00:00:00Z"), 4, Some([2, 1, 0]));
    assert_eq!(entries[0], expected);
    assert_eq!(entries[1..], history(&store, &two)[1..]);
    let changes = changes(&store, &again).unwrap();
    assert_eq!((changes.from.as_str(), changes.to.as_str()), ("1", "2"));
    assert_eq!(changes.added, [added("d.png", 3), added("e.png", 1)]);
    assert_eq!(changes.removed.len(), 1);
    assert!(changes.changed.is_empty());

    // Without the pointer of version 1, the changes since it are not known.
    let previous = Previous::read(&store, &two).unwrap();
    let edit = change(&["d.png"], &[file(&store, "d.png", b"dddd")]);
    let unknown = publish(&store, "2", at(3 * 86_400), &previous, edit, &NoVersions);
    assert!(unknown.changes.is_none());
    let entries = history(&store, &unknown);
    assert_eq!(
        entries[0],
        entry(&unknown, Some("1970-01-04T00:00:00Z"), 3, None)
    );
    assert_eq!(entries.len(), 2);
}

#[test]
fn the_same_publication_again_keeps_its_history() {
    let s = Scratch::new("same");
    let store = s.store();
    let mut published = Published::default();
    let two = two_versions(&store, &mut published);
    let previous = Previous::read(&store, &two).unwrap();
    let again = publish(
        &store,
        "2",
        at(9 * 86_400),
        &previous,
        Change::default(),
        &published,
    );
    assert_eq!(again, two);
}

#[test]
fn a_version_that_changes_no_file_keeps_the_publication() {
    let s = Scratch::new("unchanged");
    let store = s.store();
    let mut published = Published::default();
    let two = two_versions(&store, &mut published);
    let previous = Previous::read(&store, &two).unwrap();
    // Every file is written again with the same bytes.
    let rewritten = change(
        &["b.png", "c.png", "d.png"],
        &[
            file(&store, "b.png", b"b2"),
            file(&store, "c.png", b"c"),
            file(&store, "d.png", b"dd"),
        ],
    );
    let three = publish(
        &store,
        "3",
        at(9 * 86_400),
        &previous,
        rewritten,
        &published,
    );
    assert_eq!(three, two);
    let three = publish(
        &store,
        "3",
        at(9 * 86_400),
        &previous,
        Change::default(),
        &published,
    );
    assert_eq!(three, two);
}

#[test]
fn a_view_published_anew_goes_on_with_its_history() {
    let s = Scratch::new("anew");
    let store = s.store();
    let mut published = Published::default();
    let two = two_versions(&store, &mut published);
    let previous = Previous::after(&store, &two).unwrap();
    let three = publish(
        &store,
        "3",
        at(4 * 86_400),
        &previous,
        change(&[], &[file(&store, "z.png", b"z")]),
        &published,
    );
    assert!(three.changes.is_none());
    let entries = history(&store, &three);
    assert_eq!(
        entries[0],
        entry(&three, Some("1970-01-05T00:00:00Z"), 1, None)
    );
    assert_eq!(entries[1..], history(&store, &two));
}

#[test]
fn a_publication_without_a_history_starts_one() {
    let s = Scratch::new("start");
    let store = s.store();
    let one = first(&store, vec![file(&store, "a.png", b"a")]);
    let older = Pointer {
        history: None,
        ..one
    };
    let previous = Previous::read(&store, &older).unwrap();
    let edit = change(&[], &[file(&store, "b.png", b"b")]);
    let two = publish(&store, "2", at(0), &previous, edit, &NoVersions);
    let expected = [
        entry(&two, Some("1970-01-01T00:00:00Z"), 2, Some([1, 0, 0])),
        entry(&older, None, 1, None),
    ];
    assert_eq!(history(&store, &two), expected);
}

#[test]
fn each_change_tells_what_it_follows_from() {
    let s = Scratch::new("causes");
    let store = s.store();
    let one = first(
        &store,
        vec![
            file(&store, "a.png", b"a"),
            file(&store, "b.png", b"b"),
            file(&store, "c.png", b"c"),
        ],
    );
    let previous = Previous::read(&store, &one).unwrap();
    // Only the conversion removes `a` and changes `b`; the sources of the new `c` changed.
    let b = file(&store, "b.png", b"b2");
    let mut edit = caused(&["a.png", "b.png"], &[b], Some(Cause::Conversion));
    edit.remove_file("c.png", Some(Cause::Conversion));
    let c = file(&store, "c.png", b"c2");
    edit.add_file(c, Some(Cause::Source)).unwrap();
    let two = publish(&store, "2", at(0), &previous, edit, &NoVersions);

    let changes = changes(&store, &two).unwrap();
    assert_eq!(changes.removed[0].cause, Some(Cause::Conversion));
    let changed: Vec<_> = changes
        .changed
        .iter()
        .map(|file| (file.path.as_str(), file.cause))
        .collect();
    assert_eq!(
        changed,
        [
            ("b.png", Some(Cause::Conversion)),
            ("c.png", Some(Cause::Source))
        ]
    );
    let conversion = ChangeCounts {
        added: 0,
        removed: 1,
        changed: 1,
    };
    assert_eq!(history(&store, &two)[0].conversion, Some(conversion));
}
