//! The records of the state of a region, backed by the catalog of the output directory.
//!
//! An output directory that earlier versions of `update` wrote keeps the records of units and
//! derivations in its catalog. A run finds there what the records lack, and the units it takes
//! from there are recorded with the run, so that later runs on any machine find them.

use std::io;

use sekai_catalog::db::CatalogDerivations;
use sekai_catalog::{Derivations, Derived};
use sekai_pipeline::records::Records;
use sekai_pipeline::reuse::{Found, Recorded, UnitKey};

/// The records, and the catalog behind them when the output directory has one.
pub(super) struct Backed<'a> {
    pub(super) records: &'a Records<'a>,
    pub(super) catalog: Option<&'a CatalogDerivations>,
}

impl Derivations for Backed<'_> {
    fn get(&self, fingerprint: &[u8; 32]) -> io::Result<Option<Derived>> {
        if let Some(derived) = self.records.get(fingerprint)? {
            return Ok(Some(derived));
        }
        match self.catalog {
            Some(catalog) => catalog.get(fingerprint),
            None => Ok(None),
        }
    }

    fn put(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        self.records.put(fingerprint, derived)
    }

    fn flush(&self) -> io::Result<()> {
        self.records.flush()
    }
}

impl Recorded for Backed<'_> {
    fn record(&self, unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        if let Some(found) = self.records.record(unit, key)? {
            return Ok(Some(found));
        }
        match self.catalog {
            Some(catalog) => catalog.catalog().record(unit, key),
            None => Ok(None),
        }
    }
}
