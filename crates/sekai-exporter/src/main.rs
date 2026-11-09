//! The command-line program of the exporter.

mod budget;
#[cfg(feature = "client")]
mod client;
#[cfg(not(feature = "client"))]
mod client {
    use std::path::Path;

    use crate::export::Source;

    /// Refuses a client package that this build cannot read.
    pub(crate) fn source(_package: &Path, _table: &Path) -> Result<Source<'static>, lexopt::Error> {
        Err("this build has no client source: enable the client feature".into())
    }
}
mod export;
mod monitor;
mod objects;
#[cfg(feature = "catalog-db")]
mod stats;
#[cfg(all(feature = "cdn", feature = "storage-view"))]
mod storage;
#[cfg(all(feature = "cdn", not(feature = "storage-view")))]
mod storage {
    use std::path::Path;

    use sekai_catalog::{Derivations, LocalStore};
    use sekai_convert::Asset;
    use sekai_format::AssetDocument;
    use sekai_pipeline::Evaluation;

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

    /// Refuses to publish a view that this build does not have.
    pub(crate) fn publish(
        _evaluation: &Evaluation,
        _version: &str,
        _root: &Path,
        _store: &LocalStore,
        _derivations: &dyn Derivations,
        _threads: usize,
        _monitor: &Monitor,
    ) -> Result<(), lexopt::Error> {
        Err("this build has no storage view: enable the storage-view feature".into())
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
  export <input> <output> [--class-table <file>] [--tree <dir>] [--threads <n>]
         [--memory <bytes>] [--plain]
      Convert every bundle file under <input>, named by its path under <input>
      with / separators, to canonical assets in <output>/store, and list them in
      <output>/assets.tsv. With --class-table, a TPK file of engine class
      layouts, <input> is instead a client package (APK or XAPK): its player
      data is one bundle named player, and its assets are named by load path.
      Each bundle is evaluated with the bundles that hold
      the external files it names. What became of every object of each bundle
      is in <output>/ledger.tsv. CPU and memory use and the time of each stage
      are written to <output>/metrics.jsonl. The index is published under
      <output>/published, and --tree also writes a path tree.
  update <region> <output> --config <file> [--secrets <dir>] [--only <prefix>]...
         [--tree <dir>] [--storage <dir>] [--threads <n>] [--memory <bytes>]
         [--plain] [--force]
      Bring <output> up to the current version of a region of the configuration.
      Bundles go to <output>/inputs, which holds game data and must stay private.
      A version that matches the last complete run is left alone unless --force
      is given. --only selects bundles by name prefix, with their dependencies.
      A selection is neither recorded as a version nor sent to a bucket; it needs
      an <output> without a complete run and empty --tree and --storage folders.
  account <region> --config <file> --secrets <dir>
      Create the guest account of a region and store its credential in <dir>
      under the name of its secret. Run this once; later runs reuse the secret.
  objects <input> <output> [--threads <n>]
      Write the object document of every bundle file directly inside <input>
      to <output>/<bundle>/_objects.json. Payloads larger than the inline limit
      are written to <output>/_payloads/<aa>/<sha256>.bin.
  stats <output> [--top <n>]
      List the costliest kinds of derivation of the runs in <output> and the
      converters whose memory estimates are furthest from their decoded size.

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
