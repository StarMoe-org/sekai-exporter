//! The bucket that the `publish.s3` section of a configuration names: where `update` publishes the
//! views of a region, what the catalog records as held there, and where the catalog itself is kept
//! between runs.

use std::collections::BTreeMap;
use std::path::Path;

use sekai_catalog::{Key, Objects, Remote};
#[cfg(feature = "s3")]
use sekai_pipeline::metrics::Recorder;
use sekai_publish::Plan;

use crate::monitor::Monitor;

/// The state file of the bucket that holds the catalog.
#[cfg(feature = "s3")]
const CATALOG: &str = "catalog.redb";

/// The `publish.s3` section of the configuration, if there is one.
fn section(config: &str) -> Result<Option<serde_json::Value>, lexopt::Error> {
    let config: serde_json::Value = serde_json::from_str(config).map_err(|e| e.to_string())?;
    Ok(config.get("publish").and_then(|p| p.get("s3")).cloned())
}

/// A bucket to publish to, with the layout of the canonical view there.
#[cfg(feature = "s3")]
pub(crate) struct Bucket {
    s3: sekai_publish::s3::S3,
    layout: sekai_publish::Layout,
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
        use sekai_publish::Layout;
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
            None | Some("sqlite") => Layout::Sqlite,
            Some("index") => Layout::Index,
            Some(other) => {
                return Err(
                    format!("publish.s3.layout {other:?} must be \"sqlite\" or \"index\"").into(),
                );
            }
        };
        let s3 = S3::new(bucket, credentials).map_err(|e| e.to_string())?;
        Ok(Some(Self { s3, layout }))
    }

    /// The bucket as the remote that objects are read back from.
    pub(crate) fn remote(&self) -> &dyn Remote {
        &self.s3
    }

    /// Every object of the store in the bucket with its size, from a listing of the whole store.
    pub(crate) fn holdings(&self, monitor: &Monitor) -> Result<BTreeMap<Key, u64>, lexopt::Error> {
        let _span = monitor.span("publish", "listing", "");
        Ok(self.s3.holdings().map_err(|e| e.to_string())?)
    }

    /// Replaces the catalog at `path` with the one the bucket keeps. A bucket that keeps none yet
    /// leaves the catalog at `path`, if there is one, to start from.
    pub(crate) fn pull_catalog(&self, path: &Path, monitor: &Monitor) -> Result<(), lexopt::Error> {
        let _span = monitor.span("state", "pull", CATALOG);
        let pulled = self
            .s3
            .fetch_state(CATALOG, path)
            .map_err(|e| e.to_string())?;
        if !pulled {
            println!("bucket keeps no catalog yet");
        }
        Ok(())
    }

    /// Writes the catalog at `path` to the bucket, for the next run to pull.
    pub(crate) fn push_catalog(&self, path: &Path, monitor: &Monitor) -> Result<(), lexopt::Error> {
        let _span = monitor.span("state", "push", CATALOG);
        Ok(self
            .s3
            .store_state(CATALOG, path)
            .map_err(|e| e.to_string())?)
    }

    /// Publishes the canonical view, `(target, plan)`, and the storage view, when there is one,
    /// to the bucket, whose holdings are those of `objects`. The canonical view is published in
    /// the layout of the configuration, and the storage view with its path index alone.
    pub(crate) fn publish(
        &self,
        objects: &Objects<'_>,
        canonical: (&str, &Plan),
        storage: Option<(&str, &Plan)>,
        threads: usize,
        monitor: &Monitor,
    ) -> Result<(), lexopt::Error> {
        use sekai_publish::Layout;

        let views = std::iter::once((canonical, self.layout))
            .chain(storage.map(|view| (view, Layout::Paths)));
        for ((target, plan), layout) in views {
            let _span = monitor.span("publish", "s3", target);
            let (_, report) =
                sekai_publish::s3::publish_with(objects, &self.s3, target, plan, layout, threads)
                    .map_err(|e| e.to_string())?;
            println!(
                "bucket {target} uploaded {} present {} pointer {}",
                report.uploaded,
                report.present,
                if report.pointer_written {
                    "written"
                } else {
                    "unchanged"
                }
            );
        }
        Ok(())
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

    pub(crate) fn remote(&self) -> &dyn Remote {
        match *self {}
    }

    pub(crate) fn holdings(&self, _monitor: &Monitor) -> Result<BTreeMap<Key, u64>, lexopt::Error> {
        match *self {}
    }

    pub(crate) fn pull_catalog(
        &self,
        _path: &Path,
        _monitor: &Monitor,
    ) -> Result<(), lexopt::Error> {
        match *self {}
    }

    pub(crate) fn push_catalog(
        &self,
        _path: &Path,
        _monitor: &Monitor,
    ) -> Result<(), lexopt::Error> {
        match *self {}
    }

    pub(crate) fn publish(
        &self,
        _objects: &Objects<'_>,
        _canonical: (&str, &Plan),
        _storage: Option<(&str, &Plan)>,
        _threads: usize,
        _monitor: &Monitor,
    ) -> Result<(), lexopt::Error> {
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
