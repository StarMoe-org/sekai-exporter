//! The command-line program of the exporter.

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
    use sekai_pipeline::Evaluation;

    use crate::monitor::Monitor;

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
mod update;

use std::process::ExitCode;

const USAGE: &str = "\
usage: sekai-exporter <command> [options]

commands:
  export <input> <output> [--tree <dir>] [--threads <n>]
      Convert every bundle file directly inside <input> to canonical assets in
      <output>/store, and list them in <output>/assets.tsv. CPU and memory use
      and the time of each stage are written to <output>/metrics.jsonl. The
      index is published under <output>/published, and --tree also writes a
      path tree.
  update <region> <output> --config <file> [--secrets <dir>] [--only <prefix>]...
         [--tree <dir>] [--storage <dir>] [--threads <n>] [--force]
      Bring <output> up to the current version of a region of the configuration.
      Bundles go to <output>/inputs, which holds game data and must stay private.
      A version that matches the last complete run is left alone unless --force
      is given. --only selects bundles by name prefix, with their dependencies.
  objects <input> <output> [--threads <n>]
      Write the object document of every bundle file directly inside <input>
      to <output>/<bundle>/_objects.json. Payloads larger than the inline limit
      are written to <output>/_payloads/<aa>/<sha256>.bin.
  stats <output> [--top <n>]
      List the costliest kinds of derivation of the runs in <output> and the
      converters whose memory estimates are furthest from what was measured.
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
