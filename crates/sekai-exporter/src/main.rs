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

    /// Refuses the client package of a region, which this build cannot read.
    #[cfg(feature = "cdn")]
    pub(crate) fn update_source(
        _package: &sekai_game::cdn::Package,
    ) -> Result<crate::update::Supply, String> {
        Err("this build has no client source: enable the client feature".to_owned())
    }
}
mod export;
#[cfg(feature = "cdn")]
mod gc;
#[cfg(feature = "cdn")]
mod migrate;
mod monitor;
mod objects;
#[cfg(feature = "cdn")]
mod record_files;
#[cfg(feature = "client")]
mod script_table;
#[cfg(feature = "serve")]
mod serve;
#[cfg(feature = "catalog-db")]
mod stats;
#[cfg(all(feature = "cdn", feature = "storage-view"))]
mod storage;
#[cfg(all(feature = "cdn", not(feature = "storage-view")))]
mod storage {
    use std::collections::BTreeMap;

    use sekai_catalog::{Derivations, LocalStore, Objects};
    use sekai_convert::Asset;
    use sekai_format::AssetDocument;
    use sekai_format::version_history::Cause;
    use sekai_publish::Binding;
    use sekai_publish::patch::{Change, Lookup};

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

    /// The directories of the storage view that a change makes again, by unit, with what the
    /// change of each follows from.
    pub(crate) type Directories<'a> = BTreeMap<&'a str, Option<Cause>>;

    /// The empty change: a build without the storage view leaves it as it was.
    pub(crate) fn change(
        _units: &Directories<'_>,
        _assets: &[Binding],
        _previous: &Lookup<'_>,
        _objects: &Objects<'_>,
        _derivations: &dyn Derivations,
        _threads: usize,
        _monitor: &Monitor,
    ) -> Result<Change, String> {
        Ok(Change::default())
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
      data is one bundle named player, the cue sheets of its streaming assets
      a second named streamingassets, and its assets are named by load path.
      --script-table is a gzipped JSON lines file of script layouts, so that
      MonoBehaviour objects can be read too.
      Each bundle is evaluated with the bundles that hold
      the external files it names. What became of every object of each bundle
      is in <output>/ledger.tsv. CPU and memory use and the time of each stage
      are written to <output>/metrics.jsonl. The index is published under
      <output>/published, and --tree also writes a path tree.
  update <region>... <output> --config <file> [--secrets <dir>]
         [--only <prefix>]... [--threads <n>] [--memory <bytes>] [--plain]
         [--force] [--verify] [--rebuild-tables] [--republish] [--check]
         [--client] [--watch <interval>]
      Bring the site of each region of the configuration up to its current
      version, one region after another. The site is the bucket of the
      configuration, or else <output>. The regions share the store of the site,
      so --verify lists it once for all of them.
      Bundles go to <output>/inputs, which holds game data and must stay
      private, and new objects go to <output>/store. The canonical and the
      storage view are published at the site with their path indexes: in the
      bucket, or under <output>/published. A version that matches the last
      complete run, evaluated with the same converters, is left alone unless
      --force or --verify is given.
      The site keeps the state of each region under state/: the units, names,
      bundles and publications of its last version, and the records of units
      and derivations that later runs take, so <output> needs nothing from
      earlier runs. Each bundle is evaluated with the bundles it depends on:
      those the manifest lists and those that hold the external files it names.
      When all of them hold the same content as in an earlier run, the result
      that run recorded is taken instead, so only the bundles of changed units
      are fetched, and the publications are patched where those units changed
      them. Content is the hash the manifest declares, or else the bytes, which
      are fetched to compare; --force evaluates every bundle again and records
      the results anew. One run per site goes at a time: a run with a bucket
      holds the lease of the site, state/lease.json, while it runs, and a run
      that finds it held waits and says which run holds it; a lease that its
      run stopped keeping for ten minutes is taken over. A run puts every new
      object at the site before the records and the state that name it, and
      those before the pointers of the views, so an interrupted run leaves the
      last version in place. An object missing from <output>/store is read back
      from the bucket; --verify first lists the bucket's store, uploads every
      object the version needs that it lacks, and stops when one is in neither
      the bucket nor <output>/store. An asset built from tables is kept when
      what it read gives what it gave in the last run; --rebuild-tables builds
      every one again. --republish publishes the views anew even when the
      version is unchanged: units whose bundles hold the same content are taken
      from their records, the assets built from tables are built again, and
      the views are published whole. --only selects bundles of one region by name prefix,
      with their dependencies. A selection is neither recorded as a version
      nor sent to a bucket; it publishes under <output>/published and needs an
      <output> without a complete run. --check only looks up the current
      version and prints whether it or the evaluation differs from the last
      one the site recorded. It exits with 0 when every version is unchanged
      and 1 when one changed, downloads and writes nothing, and cannot be
      combined with --force, --verify, --rebuild-tables, --republish or --only.
      --client
      publishes the client package that the region's package entry names
      instead of its assets: its player data and the cue sheets of its
      streaming assets are read as bundles, with the class and script tables
      of the entry, and its assets are published in the canonical layout at
      <region>/client, under the version <application version>-<content hash>.
      Its state is kept apart from the region's, under <region>-client.
      --watch keeps the regions up to date and does not return: every
      <interval>, such as 60s or 5m, it looks up the current version of each
      region as --check does, and updates the ones that changed, one after
      another. With a bucket, each update works in <output>/<region>, which is
      removed after it, and its metrics go to <output>/<region>.metrics.jsonl.
      A region whose update fails is looked at again after twice the interval,
      twice that after another failure, and so on up to a day. --watch cannot
      be combined with --check, --force, --verify, --rebuild-tables,
      --republish or --only.
  gc <output> [--config <file>] [--secrets <dir>] [--grace <duration>]
     [--threads <n>] [--delete]
      Find the files of a site that nothing it keeps refers to, and with
      --delete delete them. The site is the bucket of the configuration, or
      else <output>. Kept are every pointer under published/, the objects every
      pointer refers to, the state of each region with the documents of the
      assets it built from tables, and the unit and derivation records whose
      objects are all kept. Files written within the grace period, 1d by
      default (such as 30m, 12h or 7d), are kept too, with the objects that the
      records among them name. Every other object of the store, state chunk
      and record is listed with its size, and without --delete nothing is
      deleted. Records go first, then state chunks and objects, so no record
      names a deleted object. With a bucket, gc holds the lease of the site as
      update does, so no update runs while it deletes.
  migrate joint-stereo <output> [--config <file>] [--secrets <dir>]
          [--threads <n>]
      Rewrite the stereo MP3 files of a site so that every frame is in joint
      stereo mode, as the encoder writes a stereo sound; they decode to the
      same samples. The site is the bucket of the configuration, or else
      <output>. Every asset document that a record names is read, a document
      with a rewritten MP3 member is written again with it, and the records
      and the catalog of <output> are rewritten to name the new documents. The
      new files are at the site before any record names them. Afterwards,
      update --republish publishes the views with the new documents, and gc
      finds the files that nothing names any more. With a bucket, migrate
      holds the lease of the site as update does.
  account <region> --config <file> --secrets <dir>
      Create the guest account of a region and store its credential in <dir>
      under the name of its secret. Run this once; later runs reuse the secret.
  objects <input> <output> [--class-table <file>] [--script-table <file>]
          [--threads <n>]
      Write the object document of every bundle file under <input> to
      <output>/<bundle>/_objects.json. Payloads larger than the inline limit are
      written to <output>/_payloads/<aa>/<sha256>.bin. The tables are the same
      as for export.
  script-table <package> <output> --class-table <file>
               [--script-table <file>]... [--bundles <dir>]... [--threads <n>]
      Write a script table for the client package <package> to <output>, in
      the form that --script-table takes. The layouts a script of the player
      data may have are the rows of each --script-table and the type trees
      that the bundle files under each --bundles directory store for its
      MonoBehaviour objects. A script that the player data stores without a
      type tree is kept with the one layout that reads every object of it to
      its last byte. Each script is printed with what became of it: kept, with
      where its layout was found, or left out because no layout was found,
      none reads every object, with the first error of each, or several do.
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

#[cfg(feature = "client")]
fn script_table(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    script_table::run(args)
}

#[cfg(not(feature = "client"))]
fn script_table(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no client source: enable the client feature".into())
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

#[cfg(feature = "cdn")]
fn gc(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    gc::run(args)
}

#[cfg(not(feature = "cdn"))]
fn gc(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no sites to collect: enable the cdn feature".into())
}

#[cfg(feature = "cdn")]
fn migrate(args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    migrate::run(args)
}

#[cfg(not(feature = "cdn"))]
fn migrate(_args: impl Iterator<Item = std::ffi::OsString>) -> Result<bool, lexopt::Error> {
    Err("this build has no sites to migrate: enable the cdn feature".into())
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
        Some("script-table") => script_table(args),
        Some("update") => update(args),
        Some("gc") => gc(args),
        Some("migrate") => migrate(args),
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
