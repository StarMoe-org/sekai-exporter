//! Dependencies that a manifest does not list.
//!
//! The game finds an object of another bundle by the name of the serialized file that holds it,
//! among the bundles it has loaded, and it loads some bundles, such as shared shaders, that no
//! manifest entry lists as a dependency. A bundle therefore also depends on each bundle that holds
//! a file its serialized files name as an external file. Which files a bundle holds and names
//! follows from its bytes, so the catalog keeps them by their SHA-256, and a later run finds them
//! there without fetching the bundle.

use std::collections::{HashMap, HashSet};
use std::io;
use std::sync::PoisonError;

use sekai_catalog::store::parse_hex;
use sekai_game::cdn::BundleEntry;
use sekai_pipeline::stream::{self, Entry};
use sekai_pipeline::version::{BundleFiles, bundle_files, indexed_files};
use sekai_pipeline::{BundleIndexes, parallel};
use sekai_unity::BlockCache;

use super::{Fetcher, declared_content, open_input};

/// The serialized files of a bundle, under the SHA-256 of its bytes.
struct Files {
    sha256: [u8; 32],
    files: BundleFiles,
    /// Whether the catalog already records them.
    recorded: bool,
}

impl Fetcher<'_> {
    /// Adds to the dependencies of each entry of `manifest` the bundles that hold the external
    /// files it names and that it does not list. Which files a bundle holds and names is read from
    /// the catalog, from `indexes`, or else from the bundle, which is fetched from the entries of
    /// `all`, `concurrency` at a time, when `fetchable` accepts its name. A bundle whose files stay
    /// unknown neither gains dependencies nor holds files for others. What the run reads is
    /// recorded in the catalog.
    pub(super) fn add_file_dependencies(
        &self,
        all: &[BundleEntry],
        manifest: &mut [Entry],
        fetchable: impl Fn(&str) -> bool,
        cache: &dyn BlockCache,
        indexes: &dyn BundleIndexes,
        concurrency: usize,
    ) -> Result<(), String> {
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
        self.record_files(known.iter().flatten().filter(|files| !files.recorded))?;

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
        Ok(())
    }

    /// The files of each entry of `manifest` that the catalog or `indexes` know, read
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

    /// The files of the bundle of `entry`, when the catalog or `indexes` know its bytes.
    fn known(&self, entry: &Entry, indexes: &dyn BundleIndexes) -> io::Result<Option<Files>> {
        let Some(sha256) = self.known_sha256(entry)? else {
            return Ok(None);
        };
        let recorded = self.catalog.bundle_files(&sha256)?;
        if let Some(files) = recorded.and_then(|bytes| serde_json::from_slice(&bytes).ok()) {
            return Ok(Some(Files {
                sha256,
                files,
                recorded: true,
            }));
        }
        let indexed = indexed_files(&sha256, indexes)?;
        Ok(indexed.map(|files| Files {
            sha256,
            files,
            recorded: false,
        }))
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
        let declared = row.filter(|row| {
            !row.declared.is_empty() && declared_content(&row.declared) == entry.content
        });
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
                sha256: key.sha256,
                files,
                recorded: false,
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

    /// Records `files` in the catalog.
    fn record_files<'f>(&self, files: impl Iterator<Item = &'f Files>) -> Result<(), String> {
        let write = || -> io::Result<()> {
            let mut batch = self.catalog.write()?;
            for files in files {
                let bytes = serde_json::to_vec(&files.files).map_err(io::Error::other)?;
                batch.put_bundle_files(&files.sha256, &bytes)?;
            }
            batch.commit()
        };
        write().map_err(|e| format!("catalog: {e}"))
    }
}
