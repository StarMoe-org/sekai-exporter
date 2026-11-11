//! `sekai-exporter serve`: the HTTP server of `sekai-serve` over an output directory, as
//! described in `docs/serving.md`.

use std::ffi::OsString;
use std::path::PathBuf;

use sekai_serve::Routes;
use sekai_serve::server::{Options, Server};

use crate::budget;

struct Arguments {
    output: PathBuf,
    routes: PathBuf,
    options: Options,
}

fn parse(args: impl Iterator<Item = OsString>) -> Result<Arguments, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let mut output = None;
    let mut routes = None;
    let mut options = Options::default();
    while let Some(arg) = parser.next()? {
        match arg {
            Long("routes") => routes = Some(PathBuf::from(parser.value()?)),
            Long("listen") => options.listen = parser.value()?.parse()?,
            Long("threads") => options.threads = parser.value()?.parse()?,
            Long("connections") => options.connections = parser.value()?.parse()?,
            Long("cache") => {
                let bytes = budget::parse_bytes(&parser.value()?.string()?)?;
                options.limits.index_bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
            }
            Value(v) if output.is_none() => output = Some(PathBuf::from(v)),
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(Arguments {
        output: output.ok_or("expected <output>")?,
        routes: routes.ok_or("expected --routes <file>")?,
        options,
    })
}

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    let Arguments {
        output,
        routes,
        options,
    } = parse(args)?;
    let read = std::fs::read(&routes)
        .map_err(|e| e.to_string())
        .and_then(|bytes| Routes::from_json(&bytes).map_err(|e| e.to_string()));
    let routes = read.map_err(|e| format!("{}: {e}", routes.display()))?;
    let server = Server::bind(&output, routes, options).map_err(|e| e.to_string())?;
    let address = server.local_addr().map_err(|e| e.to_string())?;
    eprintln!("serving on http://{address}");
    match server.run().map_err(|e| e.to_string())? {}
}
