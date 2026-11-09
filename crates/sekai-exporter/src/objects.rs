//! `sekai-exporter objects`: object documents for a directory of bundles.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sekai_game::obfuscation::deobfuscate;
use sekai_pipeline::parallel;
use sekai_unity::document::{EntryRecord, Payloads, write_bundle};
use sekai_unity::index::{BundleIndex, FileLayouts, OpenFile, VersionIndex, index_file};
use sekai_unity::{BlockCache, BoundedBlockCache, Bundle, ByteSource, SerializedFile};
use sha2::{Digest, Sha256};

const BLOCK_CACHE_BYTES: usize = 256 << 20;

struct Options {
    input: PathBuf,
    output: PathBuf,
    threads: usize,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut positional = Vec::new();
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    while let Some(arg) = parser.next()? {
        match arg {
            Long("threads") => threads = parser.value()?.parse()?,
            Value(v) => positional.push(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    let [input, output]: [PathBuf; 2] = positional
        .try_into()
        .map_err(|_| lexopt::Error::from("expected <input> and <output>"))?;
    Ok(Options {
        input,
        output,
        threads: threads.max(1),
    })
}

/// A bundle file read from disk and deobfuscated.
struct Loaded {
    bundle: Bundle,
    files: Vec<(usize, SerializedFile, FileLayouts)>,
}

fn load(path: &Path, cache: &dyn BlockCache) -> Result<Loaded, String> {
    let mut bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    deobfuscate(&mut bytes).map_err(|e| e.to_string())?;
    let id: [u8; 32] = Sha256::digest(&bytes).into();
    let source: Arc<dyn ByteSource> = Arc::new(Arc::<[u8]>::from(bytes));
    let bundle = Bundle::open(source, id).map_err(|e| e.to_string())?;
    let mut files = Vec::new();
    for (i, entry) in bundle.entries().iter().enumerate() {
        if entry.is_serialized() {
            let file = SerializedFile::parse(&bundle.entry(i, cache))
                .map_err(|e| format!("{}: {e}", entry.path))?;
            let layouts = FileLayouts::new(&file);
            files.push((i, file, layouts));
        }
    }
    Ok(Loaded { bundle, files })
}

struct DiskPayloads<'a> {
    root: &'a Path,
}

impl Payloads for DiskPayloads<'_> {
    fn store(&mut self, sha256: &[u8; 32], bytes: &[u8]) -> sekai_unity::Result<()> {
        let hex: String = sha256.iter().map(|b| format!("{b:02x}")).collect();
        let dir = self.root.join(&hex[..2]);
        let path = dir.join(format!("{hex}.bin"));
        if path.exists() {
            return Ok(());
        }
        let io = |e: std::io::Error| sekai_unity::Error::Visitor {
            message: format!("writing {}: {e}", path.display()),
        };
        std::fs::create_dir_all(&dir).map_err(io)?;
        // Write to a temporary name first, so a payload is either absent or complete.
        let tmp = dir.join(format!("{hex}.{:?}.tmp", std::thread::current().id()));
        std::fs::write(&tmp, bytes).map_err(io)?;
        std::fs::rename(&tmp, &path).map_err(io)
    }
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let options = parse(args)?;
    let mut paths: Vec<PathBuf> = match std::fs::read_dir(&options.input) {
        Ok(dir) => dir
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file())
            .collect(),
        Err(e) => return Err(format!("{}: {e}", options.input.display()).into()),
    };
    paths.sort();
    let names: Vec<String> = paths
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);

    // Pass 1: index every bundle.
    let indexed = parallel(
        paths.len(),
        options.threads,
        |i| -> Result<BundleIndex, String> {
            let loaded = load(&paths[i], &cache)?;
            let files = loaded
                .files
                .iter()
                .map(|(e, file, layouts)| {
                    let source = loaded.bundle.entry(*e, &cache);
                    index_file(&OpenFile {
                        path: &loaded.bundle.entries()[*e].path,
                        file,
                        source: &source,
                        layouts,
                    })
                    .map_err(|err| format!("{}: {err}", loaded.bundle.entries()[*e].path))
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(BundleIndex {
                name: names[i].clone(),
                files,
            })
        },
    );
    let mut failed = Vec::new();
    let mut bundles = Vec::new();
    for (i, r) in indexed.into_iter().enumerate() {
        match r {
            Ok(b) => bundles.push(b),
            Err(e) => failed.push((names[i].clone(), e)),
        }
    }
    let version = VersionIndex::new(bundles);

    // Pass 2: write the document of every indexed bundle.
    let by_name: BTreeMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let payload_root = options.output.join("_payloads");
    let written = parallel(
        version.bundles().len(),
        options.threads,
        |b| -> Result<(), String> {
            let name = &version.bundles()[b].name;
            let loaded = load(&paths[by_name[name.as_str()]], &cache)?;
            let entries = loaded
                .bundle
                .entries()
                .iter()
                .enumerate()
                .map(|(i, entry)| {
                    let source = loaded.bundle.entry(i, &cache);
                    let bytes = source
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
                .map(|(e, _, _)| loaded.bundle.entry(*e, &cache))
                .collect();
            let files: Vec<OpenFile<'_>> = loaded
                .files
                .iter()
                .zip(&sources)
                .map(|((e, file, layouts), source)| OpenFile {
                    path: &loaded.bundle.entries()[*e].path,
                    file,
                    source,
                    layouts,
                })
                .collect();
            let mut out = Vec::new();
            write_bundle(
                &mut out,
                &version,
                b,
                &entries,
                &files,
                &mut DiskPayloads {
                    root: &payload_root,
                },
            )
            .map_err(|e| e.to_string())?;
            let dir = options.output.join(name);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("_objects.json"), out).map_err(|e| e.to_string())
        },
    );
    let mut documents = 0;
    for (b, r) in written.into_iter().enumerate() {
        match r {
            Ok(()) => documents += 1,
            Err(e) => failed.push((version.bundles()[b].name.clone(), e)),
        }
    }

    let mut objects = 0;
    let mut errors: BTreeMap<&str, usize> = BTreeMap::new();
    for bundle in version.bundles() {
        for file in &bundle.files {
            objects += file.objects.len();
            for object in &file.objects {
                if let Some(e) = &object.error {
                    *errors.entry(e.code()).or_default() += 1;
                }
            }
        }
    }
    println!(
        "bundles {} documents {documents} failed {} objects {objects}",
        paths.len(),
        failed.len()
    );
    for (code, n) in &errors {
        println!("object error {code}: {n}");
    }
    for (name, e) in &failed {
        eprintln!("failed {name}: {e}");
    }
    Ok(failed.is_empty())
}
