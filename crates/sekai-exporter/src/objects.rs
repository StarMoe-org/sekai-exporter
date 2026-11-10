//! `sekai-exporter objects`: object documents for a directory of bundles or a client package.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use sekai_pipeline::{objects, parallel};
use sekai_unity::BoundedBlockCache;
use sekai_unity::document::Payloads;

use crate::export::{self, LayoutTables, Source};

const BLOCK_CACHE_BYTES: usize = 256 << 20;

struct Options {
    input: PathBuf,
    output: PathBuf,
    tables: LayoutTables,
    threads: usize,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Options, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut positional = Vec::new();
    let mut threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let mut tables = LayoutTables::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("threads") => threads = parser.value()?.parse()?,
            Long("class-table") => tables.classes = Some(PathBuf::from(parser.value()?)),
            Long("script-table") => tables.scripts = Some(PathBuf::from(parser.value()?)),
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
        tables,
        threads: threads.max(1),
    })
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
    let Source {
        manifest,
        inputs,
        classes,
    } = export::source(&options.input, &options.tables)?;
    let names: Vec<String> = manifest.into_iter().map(|entry| entry.name).collect();
    let cache = BoundedBlockCache::new(BLOCK_CACHE_BYTES);
    let source = objects::Source {
        inputs: inputs.as_ref(),
        classes: classes.as_ref(),
        cache: &cache,
    };

    // Pass 1: index every bundle.
    let indexed = objects::index(&source, &names, options.threads);
    let mut failed = indexed.failed.clone();

    // Pass 2: write the document of every indexed bundle.
    let payload_root = options.output.join("_payloads");
    let version = &indexed.version;
    let written = parallel(
        version.bundles().len(),
        options.threads,
        |b| -> Result<(), String> {
            let mut payloads = DiskPayloads {
                root: &payload_root,
            };
            let document = objects::document(&source, version, b, &mut payloads)?;
            let dir = options.output.join(&version.bundles()[b].name);
            std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
            std::fs::write(dir.join("_objects.json"), document).map_err(|e| e.to_string())
        },
    );
    let mut documents = 0;
    for (b, r) in written.into_iter().enumerate() {
        match r {
            Ok(()) => documents += 1,
            Err(e) => failed.push((version.bundles()[b].name.clone(), e)),
        }
    }

    let (objects, errors) = indexed.objects();
    println!(
        "bundles {} documents {documents} failed {} objects {objects}",
        names.len(),
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
