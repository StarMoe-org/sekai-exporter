//! The `dir` source: bundles that are already in a local directory.

use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use sekai_format::identity::{AssetIdentity, ClientIdentity, VersionIdentity};
use sha2::{Digest, Sha256};

use super::{BundleEntry, Source, SourceError};
use crate::sink::Sink;

pub(super) struct Dir {
    root: PathBuf,
}

fn read_error(path: &Path, e: io::Error) -> SourceError {
    SourceError::new("read", format!("{}: {e}", path.display()))
}

/// Every file under `directory`, as (name with `/` separators, path), appended to `out`.
fn walk(
    directory: &Path,
    prefix: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), SourceError> {
    for item in fs::read_dir(directory).map_err(|e| read_error(directory, e))? {
        let item = item.map_err(|e| read_error(directory, e))?;
        let path = item.path();
        let name = format!("{prefix}{}", item.file_name().to_string_lossy());
        let kind = item.file_type().map_err(|e| read_error(&path, e))?;
        if kind.is_dir() {
            walk(&path, &format!("{name}/"), out)?;
        } else if kind.is_file() {
            out.push((name, path));
        }
    }
    Ok(())
}

impl Dir {
    pub(super) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Every file of the directory, in name order.
    fn files(&self) -> Result<Vec<(String, PathBuf)>, SourceError> {
        let mut files = Vec::new();
        walk(&self.root, "", &mut files)?;
        files.sort();
        Ok(files)
    }
}

impl Source for Dir {
    fn identity(
        &self,
        _previous: Option<&VersionIdentity>,
    ) -> Result<VersionIdentity, SourceError> {
        let files = self.files()?;
        if files.is_empty() {
            return Err(SourceError::new(
                "no_version",
                format!("{} holds no files", self.root.display()),
            ));
        }
        let mut digest = Sha256::new();
        let mut buffer = vec![0; 1 << 16];
        for (name, path) in &files {
            digest.update(name.as_bytes());
            digest.update([0]);
            let mut file = File::open(path).map_err(|e| read_error(path, e))?;
            loop {
                let read = file.read(&mut buffer).map_err(|e| read_error(path, e))?;
                if read == 0 {
                    break;
                }
                digest.update(&buffer[..read]);
            }
            digest.update([0]);
        }
        let value: String = digest
            .finalize()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        Ok(VersionIdentity {
            asset: AssetIdentity::Dir { value },
            client: ClientIdentity {
                application_version: String::new(),
                application_hash: None,
                multiplayer_version: None,
            },
        })
    }

    fn manifest(&self, _identity: &VersionIdentity) -> Result<Vec<BundleEntry>, SourceError> {
        self.files()?
            .into_iter()
            .map(|(name, path)| {
                let size = fs::metadata(&path).map_err(|e| read_error(&path, e))?.len();
                Ok(BundleEntry {
                    name,
                    declared: String::new(),
                    download_path: None,
                    size,
                    dependencies: Vec::new(),
                })
            })
            .collect()
    }

    fn fetch(
        &self,
        _identity: &VersionIdentity,
        entry: &BundleEntry,
        sink: &mut dyn Sink,
    ) -> Result<u64, SourceError> {
        let path = self.root.join(&entry.name);
        let mut file = File::open(&path).map_err(|e| read_error(&path, e))?;
        io::copy(&mut file, sink).map_err(|e| read_error(&path, e))
    }
}
