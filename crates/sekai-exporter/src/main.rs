//! The command-line program of the exporter.

mod export;
mod monitor;
mod objects;

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
  objects <input> <output> [--threads <n>]
      Write the object document of every bundle file directly inside <input>
      to <output>/<bundle>/_objects.json. Payloads larger than the inline limit
      are written to <output>/_payloads/<aa>/<sha256>.bin.
";

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let result = match args.next().as_deref().and_then(|c| c.to_str()) {
        Some("export") => export::run(args),
        Some("objects") => objects::run(args),
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
