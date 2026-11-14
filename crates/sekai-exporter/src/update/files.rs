//! Dependencies that a manifest does not list.
//!
//! The game finds an object of another bundle by the name of the serialized file that holds it,
//! among the bundles it has loaded, and it loads some bundles, such as shared shaders, that no
//! manifest entry lists as a dependency. A bundle therefore also depends on each bundle that holds
//! a file its serialized files name as an external file. Which files a bundle holds and names
//! follows from its content, so the state of the region keeps them under the content each bundle
//! had, and the catalog keeps them by the SHA-256 of its bytes. A later run finds them there
//! without fetching the bundle.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::PoisonError;

use sekai_catalog::store::parse_hex;
use sekai_game::cdn::BundleEntry;
use sekai_pipeline::state::BundleState;
use sekai_pipeline::stream::{self, Content, Entry};
use sekai_pipeline::version::{BundleFiles, bundle_files, indexed_files};
use sekai_pipeline::{BundleIndexes, parallel};
use sekai_unity::BlockCache;

use super::fetch::{Fetcher, open_input};

/// The serialized files of a bundle.
struct Files {
    files: BundleFiles,
    /// The SHA-256 of the bundle, when the catalog does not record its files yet.
    unrecorded: Option<[u8; 32]>,
}

impl Fetcher<'_> {
    /// Adds to the dependencies of each entry of `manifest` the bundles that hold the external
    /// files it names and that it does not list. Which files a bundle holds and names is read from
    /// the state of the region, from the catalog, from `indexes`, or else from the bundle, which
    /// is fetched from the entries of `all`, `concurrency` at a time, when `fetchable` accepts its
    /// name. A bundle whose files stay unknown neither gains dependencies nor holds files for
    /// others. What the run reads is recorded in the catalog. Returns the files of each entry
    /// whose content is identified, for the state of the region, in name order.
    pub(super) fn add_file_dependencies(
        &self,
        all: &[BundleEntry],
        manifest: &mut [Entry],
        fetchable: impl Fn(&str) -> bool,
        cache: &dyn BlockCache,
        indexes: &dyn BundleIndexes,
        concurrency: usize,
    ) -> Result<Vec<BundleState>, String> {
        let mut known = self.known_files(manifest, indexes, concurrency)?;
        let unknown: HashSet<&str> = manifest
            .iter()
            .zip(&known)
            .filter(|(entry, files)| files.is_none() && fetchable(&entry.name))
            .map(|(entry, _)| entry.name.as_str())
            .collect();
        let entries: Vec<BundleEntry> = all
            .iter()
            .filter(|entry| unknown.contains(entry.name.as_str()))
            .cloned()
            .collect();
        let mut read = self.read_files(&entries, cache, indexes, concurrency)?;
        for (entry, files) in manifest.iter().zip(&mut known) {
            if files.is_none() {
                *files = read.remove(&entry.name);
            }
        }
        self.record_files(known.iter().flatten())?;
        let mut stated: Vec<BundleState> = manifest
            .iter()
            .zip(&known)
            .filter_map(|(entry, files)| {
                Some(BundleState {
                    name: entry.name.clone(),
                    content: entry.content.clone()?,
                    files: files.as_ref()?.files.clone(),
                })
            })
            .collect();
        stated.sort_by(|a, b| a.name.cmp(&b.name));

        let none = BundleFiles::new();
        let bundles: Vec<(&str, &BundleFiles)> = manifest
            .iter()
            .zip(&known)
            .map(|(entry, files)| {
                let files = files.as_ref().map_or(&none, |files| &files.files);
                (entry.name.as_str(), files)
            })
            .collect();
        let referred = stream::referred_bundles(&bundles);
        let mut extended = 0;
        for (entry, referred) in manifest.iter_mut().zip(referred) {
            let unlisted: Vec<String> = referred
                .into_iter()
                .filter(|name| !entry.dependencies.contains(name))
                .collect();
            extended += usize::from(!unlisted.is_empty());
            entry.dependencies.extend(unlisted);
        }
        println!(
            "bundles {} with known files {} read {} with unlisted dependencies {extended}",
            manifest.len(),
            known.iter().flatten().count(),
            entries.len(),
        );
        Ok(stated)
    }

    /// The files of each entry of `manifest` that the state, the catalog or `indexes` know, read
    /// `concurrency` at a time.
    fn known_files(
        &self,
        manifest: &[Entry],
        indexes: &dyn BundleIndexes,
        concurrency: usize,
    ) -> Result<Vec<Option<Files>>, String> {
        let known = parallel(manifest.len(), concurrency, |i| {
            self.known(&manifest[i], indexes)
        });
        known
            .into_iter()
            .collect::<io::Result<_>>()
            .map_err(|e| format!("catalog: {e}"))
    }

    /// The files of the bundle of `entry`: as the state of the region has them under the content
    /// of the entry, or as the catalog or `indexes` know them by its bytes.
    fn known(&self, entry: &Entry, indexes: &dyn BundleIndexes) -> io::Result<Option<Files>> {
        if let Some(files) = self.stated_files(entry) {
            return Ok(Some(Files {
                files: files.clone(),
                unrecorded: None,
            }));
        }
        let Some(sha256) = self.known_sha256(entry)? else {
            return Ok(None);
        };
        let recorded = self.catalog.bundle_files(&sha256)?;
        if let Some(files) = recorded.and_then(|bytes| serde_json::from_slice(&bytes).ok()) {
            return Ok(Some(Files {
                files,
                unrecorded: None,
            }));
        }
        let indexed = indexed_files(&sha256, indexes)?;
        Ok(indexed.map(|files| Files {
            files,
            unrecorded: Some(sha256),
        }))
    }

    /// The files of the bundle of `entry` that the state of the region has under its content.
    fn stated_files(&self, entry: &Entry) -> Option<&BundleFiles> {
        let content = entry.content.as_ref()?;
        let bundles = self.bundles;
        let found = bundles.binary_search_by(|bundle| bundle.name.as_str().cmp(&entry.name));
        let bundle = &bundles[found.ok()?];
        (bundle.content == *content).then_some(&bundle.files)
    }

    /// The SHA-256 of the bundle of `entry`: as this run fetched it, or as the catalog records it
    /// under the content the entry declares.
    fn known_sha256(&self, entry: &Entry) -> io::Result<Option<[u8; 32]>> {
        let fetched = self.fetched.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(key) = fetched.get(&entry.name) {
            return Ok(Some(key.sha256));
        }
        drop(fetched);
        let row = self.catalog.bundle(self.region, &entry.name)?;
        let declared = row.filter(
            |row| matches!(&entry.content, Some(Content::Declared(hash)) if *hash == row.declared),
        );
        Ok(declared.and_then(|row| parse_hex(&row.sha256)))
    }

    /// Fetches `entries`, `concurrency` at a time, and reads the files of each bundle that could
    /// be fetched and read, by name.
    fn read_files(
        &self,
        entries: &[BundleEntry],
        cache: &dyn BlockCache,
        indexes: &dyn BundleIndexes,
        concurrency: usize,
    ) -> Result<HashMap<String, Files>, String> {
        let keys: Vec<_> = self.fetch_keys(entries, concurrency)?.into_iter().collect();
        let read = parallel(keys.len(), concurrency, |i| -> io::Result<Option<Files>> {
            let (name, key) = &keys[i];
            let input = open_input(self.inputs, name, key)?;
            let files = bundle_files(&input, cache, indexes)?.ok();
            Ok(files.map(|files| Files {
                files,
                unrecorded: Some(key.sha256),
            }))
        });
        let mut files = HashMap::new();
        for ((name, _), read) in keys.into_iter().zip(read) {
            if let Some(read) = read.map_err(|e| format!("{name}: {e}"))? {
                files.insert(name, read);
            }
        }
        Ok(files)
    }

    /// Records in the catalog those of `files` that it does not record yet.
    fn record_files<'f>(&self, files: impl Iterator<Item = &'f Files>) -> Result<(), String> {
        let write = || -> io::Result<()> {
            let mut batch = self.catalog.write()?;
            for files in files {
                let Some(sha256) = &files.unrecorded else {
                    continue;
                };
                let bytes = serde_json::to_vec(&files.files).map_err(io::Error::other)?;
                batch.put_bundle_files(sha256, &bytes)?;
            }
            batch.commit()
        };
        write().map_err(|e| format!("catalog: {e}"))
    }
}
