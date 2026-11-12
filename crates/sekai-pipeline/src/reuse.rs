//! Units whose results earlier runs recorded.
//!
//! The result of a unit follows from the bytes of the bundles of its window and from what the
//! evaluation runs with: the converters, the class layouts, the size limit of assets, the naming
//! of roots and the evaluator itself. When the manifest declares a hash for every bundle of a
//! window, the unit is keyed by those hashes and by the rest of these, and the result a run gives
//! the unit is recorded under that key. A later run takes the recorded result instead, and fetches
//! and evaluates only the windows of the units it has none for. A unit is recorded only when no
//! bundle of its window had an error.
//!
//! A record holds the assets of its unit before the assets of different bundles that are given the
//! same name are told apart, since that depends on the other units; it happens once all of them are
//! gathered. Records are shared only by runs that name roots in the same way.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::io;

use sekai_catalog::UnitRecords;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::eval::{self, Output};
use crate::stream::{self, Entry, Gathered, Inputs, Outside, Stream, Streamed, Units};

/// The revision of how units are evaluated and recorded, which is part of every unit key.
const REVISION: &str = "unit 1";

/// A manifest split into the units that take a recorded result and the entries the others need.
pub struct Reuse<'m> {
    /// The recorded result of each unit that takes one, by unit name.
    recorded: Vec<(&'m str, Record)>,
    /// The units to evaluate whose results can be recorded.
    keyed: Vec<Keyed<'m>>,
    /// The entries to evaluate: the windows of the units without a recorded result, and the
    /// entries that are no unit, such as those that share a name, so their errors are reported.
    rest: Vec<Entry>,
    records: &'m dyn UnitRecords,
}

/// A unit to evaluate, with its key and the names of the bundles of its window.
struct Keyed<'m> {
    unit: &'m str,
    key: [u8; 32],
    window: Vec<&'m str>,
}

impl<'m> Reuse<'m> {
    /// Splits `manifest` into the units that `records` hold a result for, as an evaluation with
    /// `stream` would give it, and the entries the other units need. A unit that is in the window
    /// of another unit to evaluate is evaluated too.
    pub fn new(
        manifest: &'m [Entry],
        stream: &Stream<'_>,
        records: &'m dyn UnitRecords,
    ) -> io::Result<Self> {
        Self::split(manifest, stream, records, |key| {
            Ok(records.get(key)?.and_then(|bytes| Record::read(&bytes)))
        })
    }

    /// Evaluates every unit of `manifest` and records the results in `records` again, replacing
    /// what they held.
    pub fn refresh(
        manifest: &'m [Entry],
        stream: &Stream<'_>,
        records: &'m dyn UnitRecords,
    ) -> io::Result<Self> {
        Self::split(manifest, stream, records, |_| Ok(None))
    }

    /// Splits `manifest` with the recorded result that `lookup` finds for each unit key.
    fn split(
        manifest: &'m [Entry],
        stream: &Stream<'_>,
        records: &'m dyn UnitRecords,
        lookup: impl Fn(&[u8; 32]) -> io::Result<Option<Record>>,
    ) -> io::Result<Self> {
        let units = Units::new(manifest);
        let context = context(stream);
        let found = (0..units.entries.len())
            .map(|unit| {
                let key = key(&context, &units, unit);
                let record = key.as_ref().map(&lookup).transpose()?.flatten();
                Ok((key, record))
            })
            .collect::<io::Result<Vec<_>>>()?;
        let needed: BTreeSet<usize> = found
            .iter()
            .enumerate()
            .filter(|(_, (_, record))| record.is_none())
            .flat_map(|(unit, _)| units.windows[unit].iter().copied())
            .collect();

        let name = |entry: usize| units.entries[entry].name.as_str();
        let mut recorded = Vec::new();
        let mut keyed = Vec::new();
        for (unit, (key, record)) in found.into_iter().enumerate() {
            match record {
                Some(record) if !needed.contains(&unit) => recorded.push((name(unit), record)),
                _ => keyed.extend(key.map(|key| {
                    Keyed {
                        unit: name(unit),
                        key,
                        window: units.windows[unit]
                            .iter()
                            .map(|&entry| name(entry))
                            .collect(),
                    }
                })),
            }
        }
        Ok(Self {
            recorded,
            keyed,
            rest: entries_to_evaluate(manifest, &units, &needed),
            records,
        })
    }

    /// The entries to fetch and evaluate.
    pub fn rest(&self) -> &[Entry] {
        &self.rest
    }

    /// How many units take a recorded result.
    pub fn reused(&self) -> usize {
        self.recorded.len()
    }

    /// Evaluates the entries of [`Reuse::rest`], whose bytes come from `inputs`, as
    /// [`stream::evaluate`] does, records the result of each unit whose window had no error, and
    /// adds the recorded results of the other units.
    pub fn evaluate(self, inputs: &dyn Inputs, stream: &Stream<'_>) -> io::Result<Streamed> {
        let mut gathered = stream::gather(&self.rest, inputs, stream, &mut || Ok(()))?;
        self.record(&gathered)?;
        for (unit, record) in self.recorded {
            record.add_to(unit, stream, &mut gathered)?;
        }
        Ok(stream::finish(gathered))
    }

    /// Records the result of each keyed unit in `gathered` whose window had no error.
    fn record(&self, gathered: &Gathered) -> io::Result<()> {
        let failed: HashSet<&str> = gathered
            .errors
            .keys()
            .map(|(name, _)| name.as_str())
            .collect();
        let mut records = Record::of_each(gathered);
        let mut results = Vec::new();
        for keyed in &self.keyed {
            if keyed.window.iter().any(|bundle| failed.contains(bundle)) {
                continue;
            }
            let record = records.remove(keyed.unit).unwrap_or_default();
            results.push((keyed.key, record.bytes()?));
        }
        self.records.put(&results)
    }
}

/// The entries of `manifest` to evaluate: those of the units in `needed`, and those that are no
/// unit, such as entries that share a name, so that their errors are reported.
fn entries_to_evaluate(
    manifest: &[Entry],
    units: &Units<'_>,
    needed: &BTreeSet<usize>,
) -> Vec<Entry> {
    let name = |entry: usize| units.entries[entry].name.as_str();
    let unit_names: HashSet<&str> = (0..units.entries.len()).map(name).collect();
    let needed_names: HashSet<&str> = needed.iter().map(|&entry| name(entry)).collect();
    manifest
        .iter()
        .filter(|entry| {
            needed_names.contains(entry.name.as_str()) || !unit_names.contains(entry.name.as_str())
        })
        .cloned()
        .collect()
}

/// What every unit key of an evaluation with `stream` starts with: the revision, the forms of the
/// converters in their order, the class layouts and the size limit of assets.
fn context(stream: &Stream<'_>) -> String {
    let mut text = format!("{REVISION}\n");
    for converter in stream.converters {
        text.push_str(&format!(
            "form {} {}\n",
            converter.name(),
            converter.version()
        ));
        text.push_str(&eval::lossy_encoder(converter.as_ref()));
    }
    if let Some(identity) = stream.classes.identity() {
        text.push_str(&format!("classes {}\n", eval::hex(&identity)));
    }
    if let Some(limit) = stream.limit {
        text.push_str(&format!("limit {limit}\n"));
    }
    text
}

/// The key of `unit`, when the manifest declares a hash for every bundle of its window.
fn key(context: &str, units: &Units<'_>, unit: usize) -> Option<[u8; 32]> {
    let mut text = format!("{context}unit {}\n", units.entries[unit].name);
    for &entry in &units.windows[unit] {
        let entry = units.entries[entry];
        if entry.declared.is_empty() {
            return None;
        }
        text.push_str(&format!("bundle {} {}\n", entry.name, entry.declared));
    }
    Some(Sha256::digest(text.as_bytes()).into())
}

/// What a unit gave: its assets, the roots it left unnamed and the references that leave its
/// window.
#[derive(Debug, Default, Serialize, Deserialize)]
struct Record {
    assets: Vec<RecordedAsset>,
    /// The logical path and path ID of each root left unnamed.
    unnamed: Vec<(String, i64)>,
    outside: Vec<Outside>,
}

/// An asset of a unit, with the logical path and path ID of its root.
#[derive(Debug, Serialize, Deserialize)]
struct RecordedAsset {
    name: String,
    form: String,
    document: String,
    error: Option<String>,
    logical: String,
    path_id: i64,
}

impl Record {
    /// The record in `bytes`, or `None` when they hold none, so that the unit is evaluated again.
    fn read(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    fn bytes(&self) -> io::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(io::Error::other)
    }

    /// The record of each unit that gave something in `gathered`, by unit name.
    fn of_each(gathered: &Gathered) -> BTreeMap<&str, Record> {
        let mut records: BTreeMap<&str, Record> = BTreeMap::new();
        for (output, logical, path_id) in &gathered.outputs {
            records
                .entry(&output.bundle)
                .or_default()
                .assets
                .push(RecordedAsset {
                    name: output.name.clone(),
                    form: output.form.to_owned(),
                    document: output.document.to_string(),
                    error: output.error.clone(),
                    logical: logical.clone(),
                    path_id: *path_id,
                });
        }
        for (unit, logical, path_id) in &gathered.unnamed {
            let unnamed = (logical.clone(), *path_id);
            records.entry(unit).or_default().unnamed.push(unnamed);
        }
        for outside in &gathered.outside {
            let record = records.entry(&outside.unit).or_default();
            record.outside.push(outside.clone());
        }
        records
    }

    /// Adds the record to `gathered` as what `unit` gave in an evaluation with `stream`.
    fn add_to(self, unit: &str, stream: &Stream<'_>, gathered: &mut Gathered) -> io::Result<()> {
        let invalid = |what: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the record of unit {unit} {what}"),
            )
        };
        for asset in self.assets {
            let form = stream
                .converters
                .iter()
                .map(|converter| converter.name())
                .find(|&form| form == asset.form)
                .ok_or_else(|| invalid(format!("names the unknown form {}", asset.form)))?;
            let document = asset
                .document
                .parse()
                .map_err(|_| invalid(format!("names the document {}", asset.document)))?;
            let output = Output {
                name: asset.name,
                bundle: unit.to_owned(),
                form,
                document,
                error: asset.error,
            };
            gathered
                .outputs
                .push((output, asset.logical, asset.path_id));
        }
        let unnamed = self.unnamed.into_iter();
        let unnamed = unnamed.map(|(logical, path_id)| (unit.to_owned(), logical, path_id));
        gathered.unnamed.extend(unnamed);
        gathered.outside.extend(self.outside);
        Ok(())
    }
}
