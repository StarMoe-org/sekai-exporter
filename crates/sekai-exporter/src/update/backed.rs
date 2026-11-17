//! The records of the state of a region, with the catalog of the output directory in front.
//!
//! An output directory that earlier versions of `update` wrote keeps the records of units and
//! derivations in its catalog. A record is named by the key or fingerprint of what it holds, so
//! the catalog and the records agree on every name they share. A run therefore looks in the
//! catalog first, which costs no request, and in the records after it. The units it takes from the
//! catalog are recorded with the run, so that later runs on any machine find them. A derivation
//! the run makes goes to its uploads, which record it once its objects are at the site.

use std::io;

use sekai_catalog::db::CatalogDerivations;
use sekai_catalog::{Derivations, Derived, Key};
use sekai_pipeline::records::Records;
use sekai_pipeline::reuse::{Found, Recorded, UnitKey};

use super::uploads::Uploads;

/// The records, behind the catalog of the output directory, with the uploads of the run.
pub(super) struct Backed<'a> {
    pub(super) records: &'a Records<'a>,
    pub(super) catalog: &'a CatalogDerivations,
    pub(super) uploads: &'a Uploads<'a>,
    /// How many records are read at once.
    pub(super) requests: usize,
}

impl Derivations for Backed<'_> {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        match self.catalog.get(fingerprint)? {
            Some(derived) => Ok(Some(derived)),
            None => self.records.get(fingerprint),
        }
    }

    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        self.uploads.send_output(fingerprint, derived)
    }

    fn put_file(&self, fingerprint: [u8; 32], file: Key) -> io::Result<()> {
        self.uploads.send_file(fingerprint, file)
    }

    fn flush(&self) -> io::Result<()> {
        self.records.flush()
    }
}

impl Recorded for Backed<'_> {
    fn record(&self, unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        match self.catalog.catalog().record(unit, key)? {
            Some(found) => Ok(Some(found)),
            None => self.records.record(unit, key),
        }
    }
}
