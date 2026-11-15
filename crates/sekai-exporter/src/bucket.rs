//! The bucket that the `publish.s3` section of a configuration names: where `update` publishes the
//! views of a region and keeps its state between runs.

use std::collections::BTreeMap;
use std::io;
use std::path::Path;

use sekai_catalog::{Key, Objects, Remote};
use sekai_format::publication::Pointer;
#[cfg(feature = "s3")]
use sekai_pipeline::metrics::Recorder;
use sekai_pipeline::state::StateFiles;
use sekai_publish::{Layout, Object};

use crate::monitor::Monitor;

/// The `publish.s3` section of the configuration, if there is one.
fn section(config: &str) -> Result<Option<serde_json::Value>, lexopt::Error> {
    let config: serde_json::Value = serde_json::from_str(config).map_err(|e| e.to_string())?;
    Ok(config.get("publish").and_then(|p| p.get("s3")).cloned())
}

/// A bucket to publish to, with the layout of the canonical view there.
#[cfg(feature = "s3")]
pub(crate) struct Bucket {
    s3: sekai_publish::s3::S3,
    layout: Layout,
}

/// No bucket: this build cannot publish to one.
#[cfg(not(feature = "s3"))]
pub(crate) enum Bucket {}

#[cfg(feature = "s3")]
impl Bucket {
    /// The bucket of the `publish.s3` section of `config`, with its access key read from the
    /// secrets named there: environment variables, or else files in `secrets`.
    pub(crate) fn open(
        config: &str,
        secrets: Option<&Path>,
    ) -> Result<Option<Self>, lexopt::Error> {
        use sekai_publish::s3::{self, Credentials, S3};

        let Some(section) = section(config)? else {
            return Ok(None);
        };
        let text = |key: &str| -> Result<Option<String>, lexopt::Error> {
            let value = key
                .split('.')
                .try_fold(&section, |value, part| value.get(part));
            value
                .map(|value| {
                    value.as_str().map(str::to_owned).ok_or_else(|| {
                        lexopt::Error::from(format!("publish.s3.{key} is not a string"))
                    })
                })
                .transpose()
        };
        let required = |key: &str| {
            text(key)?.ok_or_else(|| lexopt::Error::from(format!("publish.s3.{key} is not set")))
        };
        let credentials = Credentials {
            access_key_id: secret(&required("secrets.access_key_id")?, secrets)?,
            secret_access_key: secret(&required("secrets.secret_access_key")?, secrets)?,
        };
        let bucket = s3::Bucket {
            endpoint: required("endpoint")?,
            region: required("region")?,
            prefix: text("prefix")?.unwrap_or_default(),
            connections: match section.get("connections") {
                None => 8,
                Some(value) => value
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or("publish.s3.connections is not a number")?,
            },
        };
        let layout = match text("layout")?.as_deref() {
            None | Some("index") => Layout::Index,
            Some(other) => {
                return Err(format!("publish.s3.layout {other:?} must be \"index\"").into());
            }
        };
        let s3 = S3::new(bucket, credentials).map_err(|e| e.to_string())?;
        Ok(Some(Self { s3, layout }))
    }

    /// The layout of the canonical view in the bucket.
    pub(crate) fn layout(&self) -> Layout {
        self.layout
    }

    /// The bucket as the remote that objects are read back from.
    pub(crate) fn remote(&self) -> &dyn Remote {
        &self.s3
    }

    /// The number of requests to the bucket in flight at once.
    pub(crate) fn connections(&self) -> usize {
        self.s3.connections()
    }

    /// Every object of the store in the bucket with its size, from a listing of the whole store.
    pub(crate) fn holdings(&self, monitor: &Monitor) -> Result<BTreeMap<Key, u64>, lexopt::Error> {
        let _span = monitor.span("publish", "listing", "");
        Ok(self.s3.holdings().map_err(|e| e.to_string())?)
    }

    /// The names of the state files in the state directory `directory`, from a listing of it.
    pub(crate) fn state_names(
        &self,
        directory: &str,
        monitor: &Monitor,
    ) -> Result<Vec<String>, lexopt::Error> {
        let _span = monitor.span("publish", "listing", directory);
        Ok(self.s3.state_names(directory).map_err(|e| e.to_string())?)
    }

    /// The current pointer of `target` in the bucket.
    pub(crate) fn current(&self, target: &str) -> Result<Option<Pointer>, String> {
        self.s3.current(target).map_err(|e| e.to_string())
    }

    /// Uploads the objects of `referred` that the holdings of `objects` do not record.
    pub(crate) fn store(&self, objects: &Objects<'_>, referred: Vec<Object>) -> Result<(), String> {
        let stored = sekai_publish::s3::store_objects(objects, &self.s3, referred);
        let stored = stored.map_err(|e| e.to_string())?;
        println!(
            "bucket uploaded {} present {}",
            stored.uploaded, stored.present
        );
        Ok(())
    }

    /// Makes `pointer` the current publication of `target` in the bucket.
    pub(crate) fn make_current(&self, target: &str, pointer: &Pointer) -> Result<(), String> {
        let written = sekai_publish::s3::make_current(&self.s3, target, pointer);
        let written = written.map_err(|e| e.to_string())?;
        let pointer = if written { "written" } else { "unchanged" };
        println!("bucket {target} pointer {pointer}");
        Ok(())
    }
}

/// The state files of the bucket, under its `state/`.
#[cfg(feature = "s3")]
impl StateFiles for Bucket {
    fn read(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        self.s3.read_state(name).map_err(io::Error::other)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.s3.write_state(name, bytes).map_err(io::Error::other)
    }
}

#[cfg(not(feature = "s3"))]
impl Bucket {
    /// Refuses a configuration that names a bucket, which this build cannot publish to.
    pub(crate) fn open(
        config: &str,
        _secrets: Option<&Path>,
    ) -> Result<Option<Self>, lexopt::Error> {
        match section(config)? {
            Some(_) => {
                Err("the configuration names an S3 bucket, but this build has no s3 feature".into())
            }
            None => Ok(None),
        }
    }

    pub(crate) fn layout(&self) -> Layout {
        match *self {}
    }

    pub(crate) fn remote(&self) -> &dyn Remote {
        match *self {}
    }

    pub(crate) fn connections(&self) -> usize {
        match *self {}
    }

    pub(crate) fn holdings(&self, _monitor: &Monitor) -> Result<BTreeMap<Key, u64>, lexopt::Error> {
        match *self {}
    }

    pub(crate) fn state_names(
        &self,
        _directory: &str,
        _monitor: &Monitor,
    ) -> Result<Vec<String>, lexopt::Error> {
        match *self {}
    }

    pub(crate) fn current(&self, _target: &str) -> Result<Option<Pointer>, String> {
        match *self {}
    }

    pub(crate) fn store(
        &self,
        _objects: &Objects<'_>,
        _referred: Vec<Object>,
    ) -> Result<(), String> {
        match *self {}
    }

    pub(crate) fn make_current(&self, _target: &str, _pointer: &Pointer) -> Result<(), String> {
        match *self {}
    }
}

#[cfg(not(feature = "s3"))]
impl StateFiles for Bucket {
    fn read(&self, _name: &str) -> io::Result<Option<Vec<u8>>> {
        match *self {}
    }

    fn write(&self, _name: &str, _bytes: &[u8]) -> io::Result<()> {
        match *self {}
    }
}

/// The value of the secret `name`: the environment variable of that name, or else the file of that
/// name in `secrets`, without surrounding whitespace.
#[cfg(feature = "s3")]
fn secret(name: &str, secrets: Option<&Path>) -> Result<String, lexopt::Error> {
    if let Ok(value) = std::env::var(name) {
        return Ok(value.trim().to_owned());
    }
    secrets
        .and_then(|directory| std::fs::read_to_string(directory.join(name)).ok())
        .map(|value| value.trim().to_owned())
        .ok_or_else(|| format!("the secret {name} is not set").into())
}
