//! `sekai-exporter account`: creates the guest account of a region, once.

use std::ffi::OsString;
use std::path::PathBuf;

use sekai_game::cdn::{Region, Secrets, create_account};

pub(crate) fn run(args: impl Iterator<Item = OsString>) -> Result<bool, lexopt::Error> {
    use lexopt::prelude::*;
    let mut parser = lexopt::Parser::from_args(args);
    let (mut region, mut config, mut secrets) = (None, None, None);
    while let Some(arg) = parser.next()? {
        match arg {
            Long("config") => config = Some(PathBuf::from(parser.value()?)),
            Long("secrets") => secrets = Some(PathBuf::from(parser.value()?)),
            Value(v) if region.is_none() => region = Some(v.string()?),
            _ => return Err(arg.unexpected()),
        }
    }
    let region = region.ok_or("expected <region>")?;
    let config = config.ok_or("--config is required")?;
    let text =
        std::fs::read_to_string(&config).map_err(|e| format!("{}: {e}", config.display()))?;
    let region = Region::load(&region, &text).map_err(|e| e.to_string())?;
    let name = create_account(region, &Secrets::new(secrets)).map_err(|e| e.to_string())?;
    println!("the account is stored as the secret {name}");
    Ok(true)
}
