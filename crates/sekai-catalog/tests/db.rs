//! The catalog database: batches, snapshots and rebuilding.
#![cfg(feature = "catalog-db")]

use std::path::PathBuf;

use sekai_catalog::db::{AssetRow, BundleRow, Catalog, PublicationRow};
use sekai_catalog::{Derivations, Derived, Key, LocalStore};
use sekai_format::asset::{FORMAT, VERSION};
use sekai_format::{AssetDocument, AssetError};

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("sekai-db-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn store(&self) -> LocalStore {
        LocalStore::open(self.0.join("store")).unwrap()
    }

    fn file(&self) -> PathBuf {
        self.0.join("catalog.redb")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key(byte: u8) -> Key {
    Key {
        sha256: [byte; 32],
        extension: "json".to_owned(),
    }
}

fn row(name: &str, error: Option<&str>) -> AssetRow {
    AssetRow {
        name: name.to_owned(),
        form: "texture".to_owned(),
        fingerprint: "ab".to_owned(),
        document: key(1),
        error: error.map(str::to_owned),
    }
}

fn document(fingerprint: &str, error: Option<AssetError>) -> AssetDocument {
    AssetDocument {
        format: FORMAT.to_owned(),
        version: VERSION,
        name: "a".to_owned(),
        form: "texture".to_owned(),
        form_version: 1,
        fingerprint: fingerprint.to_owned(),
        sources: Vec::new(),
        members: Vec::new(),
        references: Vec::new(),
        error,
    }
}

#[test]
fn rows_read_back() {
    let s = Scratch::new("rows");
    let catalog = Catalog::open_or_rebuild(&s.file(), &s.store()).unwrap();
    let bundle = BundleRow {
        declared: "d".to_owned(),
        sha256: "e".to_owned(),
    };
    let publication = PublicationRow {
        version: "v1".to_owned(),
        revision: "r".to_owned(),
    };
    let mut batch = catalog.write().unwrap();
    batch
        .put_derivation(&[7; 32], &Derived::Document(key(2)))
        .unwrap();
    batch.put_derivation(&[8; 32], &Derived::Nothing).unwrap();
    batch
        .put_asset("v1", &row("b", Some("unsupported")))
        .unwrap();
    batch.put_asset("v1", &row("a", None)).unwrap();
    batch.put_asset("v2", &row("c", None)).unwrap();
    batch.put_bundle("jp", "x", &bundle).unwrap();
    batch.put_version("jp", "v1", "{}").unwrap();
    batch.set_pointer("jp", "v1").unwrap();
    batch
        .put_publication("local", "jp/canonical", &publication)
        .unwrap();
    batch.commit().unwrap();

    assert_eq!(
        catalog.derivation(&[7; 32]).unwrap(),
        Some(Derived::Document(key(2)))
    );
    assert_eq!(
        catalog.derivation(&[8; 32]).unwrap(),
        Some(Derived::Nothing)
    );
    assert_eq!(catalog.derivation(&[9; 32]).unwrap(), None);
    let names: Vec<_> = catalog.assets("v1").unwrap();
    assert_eq!(names, [row("a", None), row("b", Some("unsupported"))]);
    assert_eq!(catalog.bundle("jp", "x").unwrap(), Some(bundle));
    assert_eq!(catalog.bundle("cn", "x").unwrap(), None);
    assert_eq!(catalog.version("jp", "v1").unwrap().as_deref(), Some("{}"));
    assert_eq!(catalog.pointer("jp").unwrap().as_deref(), Some("v1"));
    assert_eq!(
        catalog.publication("local", "jp/canonical").unwrap(),
        Some(publication)
    );
}

#[test]
fn readers_do_not_see_an_uncommitted_batch() {
    let s = Scratch::new("snapshot");
    let catalog = Catalog::open_or_rebuild(&s.file(), &s.store()).unwrap();
    let mut batch = catalog.write().unwrap();
    batch.set_pointer("jp", "v1").unwrap();
    assert_eq!(catalog.pointer("jp").unwrap(), None);
    batch.commit().unwrap();
    assert_eq!(catalog.pointer("jp").unwrap().as_deref(), Some("v1"));
}

#[test]
fn derivations_survive_reopening_after_a_flush() {
    let s = Scratch::new("flush");
    let store = s.store();
    let derivations = Catalog::open_or_rebuild(&s.file(), &store)
        .unwrap()
        .into_derivations();
    derivations.put([1; 32], Derived::Nothing).unwrap();
    assert_eq!(derivations.get(&[1; 32]).unwrap(), Some(Derived::Nothing));
    derivations.flush().unwrap();
    drop(derivations);
    let catalog = Catalog::open_or_rebuild(&s.file(), &store).unwrap();
    assert_eq!(
        catalog.derivation(&[1; 32]).unwrap(),
        Some(Derived::Nothing)
    );
}

#[test]
fn a_damaged_file_is_rebuilt_from_the_store() {
    let s = Scratch::new("damaged");
    let store = s.store();
    let good = document(&"ab".repeat(32), None);
    let good_key = store.put(&good.to_bytes().unwrap(), "json").unwrap();
    let failed = document(
        &"cd".repeat(32),
        Some(AssetError {
            code: "unsupported".to_owned(),
            message: "x".to_owned(),
        }),
    );
    store.put(&failed.to_bytes().unwrap(), "json").unwrap();
    store.put(b"{\"not\":\"a document\"}", "json").unwrap();
    std::fs::write(s.file(), vec![0x5a; 8192]).unwrap();

    let catalog = Catalog::open_or_rebuild(&s.file(), &store).unwrap();
    assert_eq!(
        catalog.derivation(&[0xab; 32]).unwrap(),
        Some(Derived::Document(good_key))
    );
    assert_eq!(catalog.derivation(&[0xcd; 32]).unwrap(), None);
    assert!(s.0.join("catalog.redb.corrupt").is_file());
}
