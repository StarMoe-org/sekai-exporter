//! Object documents of a set of bundles.
//!
//! [`index`] indexes every bundle, which resolves the references between them, and [`document`]
//! then writes the object document of one indexed bundle. The two steps are separate so that a
//! caller can run the second for each bundle on as many threads as it has, or one after another.

use std::collections::BTreeMap;

use sekai_unity::document::{EntryRecord, Payloads, write_bundle};
use sekai_unity::index::{BundleIndex, FileLayouts, OpenFile, VersionIndex, index_file};
use sekai_unity::{BlockCache, Bundle, ByteSource, ClassLayouts, SerializedFile, open_files};
use sha2::{Digest, Sha256};

use crate::eval::parallel;
use crate::stream::Inputs;

/// What the documents of a set of bundles are written from.
pub struct Source<'a> {
    /// The bundles, by name.
    pub inputs: &'a dyn Inputs,
    /// The layouts of objects stored without one.
    pub classes: &'a dyn ClassLayouts,
    /// The block cache the bundles are read through.
    pub cache: &'a dyn BlockCache,
}

/// The bundles that were indexed, and those that could not be.
pub struct Indexed {
    /// The indexed bundles, in name order.
    pub version: VersionIndex,
    /// The bundles that could not be opened or indexed, as (name, reason), in name order.
    pub failed: Vec<(String, String)>,
}

impl Indexed {
    /// The number of objects of the indexed bundles, and the number of them with each error
    /// code.
    pub fn objects(&self) -> (usize, BTreeMap<&'static str, usize>) {
        let mut objects = 0;
        let mut errors = BTreeMap::new();
        for bundle in self.version.bundles() {
            for file in &bundle.files {
                objects += file.objects.len();
                for object in &file.objects {
                    if let Some(e) = &object.error {
                        *errors.entry(e.code()).or_default() += 1;
                    }
                }
            }
        }
        (objects, errors)
    }
}

/// A bundle with its serialized files parsed.
struct Loaded {
    bundle: Bundle,
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

fn load(source: &Source<'_>, name: &str) -> Result<Loaded, String> {
    let input = source.inputs.open(name).map_err(|e| e.to_string())?;
    let bundle = Bundle::open(input.source, input.sha256).map_err(|e| e.to_string())?;
    let files = open_files(&bundle, source.cache, source.classes).map_err(|e| e.to_string())?;
    Ok(Loaded { bundle, files })
}

/// Indexes the bundles `names` on `threads` threads. The result does not depend on `threads` or
/// on the order of `names`.
pub fn index(source: &Source<'_>, names: &[String], threads: usize) -> Indexed {
    let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
    names.sort_unstable();
    let indexed = parallel(names.len(), threads, |i| -> Result<BundleIndex, String> {
        let loaded = load(source, names[i])?;
        let files = loaded
            .files
            .iter()
            .map(|(e, file, layouts)| {
                let path = &loaded.bundle.entries()[*e].path;
                let entry = loaded.bundle.entry(*e, source.cache);
                index_file(&OpenFile {
                    path,
                    file,
                    source: &entry,
                    layouts,
                })
                .map_err(|err| format!("{path}: {err}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(BundleIndex {
            name: names[i].to_owned(),
            files,
        })
    });
    let mut failed = Vec::new();
    let mut bundles = Vec::new();
    for (name, r) in names.iter().zip(indexed) {
        match r {
            Ok(b) => bundles.push(b),
            Err(e) => failed.push(((*name).to_owned(), e)),
        }
    }
    Indexed {
        version: VersionIndex::new(bundles),
        failed,
    }
}

/// The object document of the bundle at `bundle` in `version`, with the payloads too large to
/// inline handed to `payloads`.
pub fn document(
    source: &Source<'_>,
    version: &VersionIndex,
    bundle: usize,
    payloads: &mut dyn Payloads,
) -> Result<Vec<u8>, String> {
    let loaded = load(source, &version.bundles()[bundle].name)?;
    let entries = loaded
        .bundle
        .entries()
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let entry_source = loaded.bundle.entry(i, source.cache);
            let bytes = entry_source
                .read(0, entry.size as usize)
                .map_err(|e| e.to_string())?;
            Ok(EntryRecord {
                path: entry.path.clone(),
                serialized: entry.is_serialized(),
                size: entry.size,
                sha256: Sha256::digest(&*bytes).into(),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    let sources: Vec<_> = loaded
        .files
        .iter()
        .map(|(e, _, _)| loaded.bundle.entry(*e, source.cache))
        .collect();
    let files: Vec<OpenFile<'_>> = loaded
        .files
        .iter()
        .zip(&sources)
        .map(|((e, file, layouts), entry)| OpenFile {
            path: &loaded.bundle.entries()[*e].path,
            file,
            source: entry,
            layouts,
        })
        .collect();
    let mut out = Vec::new();
    write_bundle(&mut out, version, bundle, &entries, &files, payloads)
        .map_err(|e| e.to_string())?;
    Ok(out)
}
