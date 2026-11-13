//! The command-line program of the exporter.

/// Conversions allocate and free large buffers on every thread. mimalloc keeps freed memory for
/// the next allocation instead of returning it to the system and faulting it in again.
#[global_allocator]
static ALLOCATOR: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[cfg(feature = "cdn")]
mod bucket;
mod budget;
#[cfg(feature = "client")]
mod client;
#[cfg(not(feature = "client"))]
mod client {
    use std::path::Path;

    use crate::export::Source;

    /// Refuses a client package that this build cannot read.
    pub(crate) fn source(
        _package: &Path,
        _classes: &Path,
        _scripts: Option<&Path>,
    ) -> Result<Source<'static>, lexopt::Error> {
        Err("this build has no client source: enable the client feature".into())
    }
}
mod export;
mod monitor;
mod objects;
#[cfg(feature = "serve")]
mod serve;
#[cfg(feature = "catalog-db")]
mod stats;
#[cfg(all(feature = "cdn", feature = "storage-view"))]
mod storage;
#[cfg(all(feature = "cdn", not(feature = "storage-view")))]
mod storage {
    use sekai_catalog::{Derivations, LocalStore, Objects};
    use sekai_convert::Asset;
    use sekai_format::AssetDocument;
    use sekai_publish::Plan;

    use crate::export::Destination;
    use crate::monitor::Monitor;

    /// Makes no further files in a build without the storage view.
    pub(crate) struct Fusion;

    impl Fusion {
        pub(crate) fn new(
            _store: &LocalStore,
            _derivations: &dyn Derivations,
            _monitor: &Monitor,
        ) -> Self {
            Self
        }
    }

    impl sekai_pipeline::Fusion for Fusion {
        fn extra(&self, _asset: &Asset, _document: &AssetDocument) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Publishes no view in a build without the storage view, and refuses to write its tree.
    pub(crate) fn publish(
        _index: &Plan,
        _target: &str,
        destination: &Destination<'_>,
        _objects: &Objects<'_>,
        _derivations: &dyn Derivations,
        _threads: usize,
        _monitor: &Monitor,
    ) -> Result<Option<Plan>, lexopt::Error> {
        match destination.tree {
            Some(_) => {
                Err("this build has no storage view: enable the storage-view feature".into())
            }
            None => Ok(None),
        }
    }
}
#[cfg(feature = "cdn")]
mod account;
#[cfg(feature = "cdn")]
mod update;

use std::process::ExitCode;

const USAGE: &str = "\
usage: sekai-exporter <command> [options]

commands:
  export <input> <output> [--class-table <file>] [--script-table <file>]
         [--tree <dir>] [--threads <n>] [--memory <bytes>] [--plain]
      Convert every bundle file under <input>, named by its path under <input>
      with / separators, to canonical assets in <output>/store, and list them in
      <output>/assets.tsv. With --class-table, a TPK file of engine class
      layouts, <input> is instead a client package (APK or XAPK): its player
      data is one bundle named player, and its assets are named by load path.
      --script-table is a gzipped JSON lines file of script layouts, so that
      MonoBehaviour objects can be read too.
      Each bundle is evaluated with the bundles that hold
      the external files it names. What became of every object of each bundle
      is in <output>/ledger.tsv. CPU and memory use and the time of each stage
      are written to <output>/metrics.jsonl. The index is published under
      <output>/published, and --tree also writes a path tree.
  update <region> <output> --config <file> [--secrets <dir>] [--only <prefix>]...
         [--tree <dir>] [--storage <dir>] [--threads <n>] [--memory <bytes>]
         [--plain] [--force] [--verify] [--check]
      Bring <output> up to the current version of a region of the configuration.
      Bundles go to <output>/inputs, which holds game data and must stay
      private. The canonical and the storage view are published under
      <output>/published with their path indexes; --tree and --storage also
      write them as path trees. A version that matches the last complete run is
      left alone unless --force or --verify is given. Each bundle is evaluated
      with the bundles it depends on, and when all of them hold the same content
      as in an earlier run, the result that run recorded is taken instead, so
      only the bundles of changed units are fetched. Content is the hash the
      manifest declares, or else the bytes, which are fetched to compare;
      --force evaluates every bundle again and records the results anew. With a
      bucket, the run starts from the catalog the bucket keeps under state/ and
      writes it back when it is done, so <output> needs nothing from earlier
      runs; one run per bucket goes at a time. The catalog records what the
      bucket holds as each upload is accepted, and a run uploads only what it
      does not record; --verify first replaces that record with a listing of the
      bucket's store, so the bucket gets back what it lost. The bucket also
      keeps the asset documents of the last publication of each region under
      state/, in one file that a run reads at its start; any other object
      missing from <output>/store is read back from the bucket. --only selects
      bundles by name prefix, with their dependencies. A selection is neither
      recorded as a version nor sent to a bucket; it needs an <output> without a
      complete run and empty --tree and --storage folders. --check only looks up
      the current version and prints whether it differs from the last one
      recorded: in the bucket, which keeps it under state/ beside the catalog,
      or else in <output>. It exits with 0 when the version is unchanged and 1
      when it changed, downloads and writes nothing, and cannot be combined
      with --force, --verify, --only, --tree or --storage.
  account <region> --config <file> --secrets <dir>
      Create the guest account of a region and store its credential in <dir>
      under the name of its secret. Run this once; later runs reuse the secret.
  objects <input> <output> [--class-table <file>] [--script-table <file>]
          [--threads <n>]
      Write the object document of every bundle file under <input> to
      <output>/<bundle>/_objects.json. Payloads larger than the inline limit are
      written to <output>/_payloads/<aa>/<sha256>.bin. The tables are the same
      as for export.
  stats <output> [--top <n>]
      List the costliest kinds of derivation of the runs in <output> and the
      converters whose memory estimates are furthest from their decoded size.
  serve <output> --routes <file> [--listen <address>] [--threads <n>]
        [--connections <n>] [--cache <bytes>]
      Answer HTTP requests for the files of the views published in <output>,
      by the paths that the routes file maps to them. --listen is
      127.0.0.1:8080 by default. A new publication takes effect within the
      max_age of the routes file. Here --threads is the number of threads that
      answer requests. --connections is how many connections are held at once,
      256 by default, and --cache the memory that decoded path indexes may
      take, such as 1G, 512M by default. The server only reads <output>.

  --threads sets how many conversions and encodings run at once; a video
  encoding counts as two. --memory sets the memory the process may use, such
  as 8G, half the physical memory by default; conversions wait until their
  estimated memory fits, and one estimated above the budget runs alone.
  --plain evaluates one asset after another on one thread without reading or
  writing the record of earlier conversions, as a reference for the others.
  None of these change the assets.
";

#[cfg(feature = "cdn")]
fn update(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    update::run(args)
}

#[cfg(feature = "catalog-db")]
fn stats(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    stats::run(args)
}

#[cfg(not(feature = "catalog-db"))]
fn stats(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no catalog: enable the catalog-db feature".into())
}

#[cfg(feature = "cdn")]
fn account(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    account::run(args)
}

#[cfg(not(feature = "cdn"))]
fn account(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no distribution sources: enable the cdn feature".into())
}

#[cfg(feature = "serve")]
fn serve(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    serve::run(args)
}

#[cfg(not(feature = "serve"))]
fn serve(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no server: enable the serve feature".into())
}

#[cfg(not(feature = "cdn"))]
fn update(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no distribution sources: enable the cdn feature".into())
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let result = match args.next().as_deref().and_then(|c| c.to_str()) {
        Some("export") => export::run(args),
        Some("objects") => objects::run(args),
        Some("update") => update(args),
        Some("account") => account(args),
        Some("stats") => stats(args),
        Some("serve") => serve(args),
        Some("--version") => {
            println!("sekai-exporter {}", env!("CARGO_PKG_VERSION"));
            Ok(true)
        }
        _ => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}
