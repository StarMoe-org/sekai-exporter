//! Units whose results earlier runs recorded.
//!
//! The result of a unit follows from the bytes of the bundles of its window and from what the
//! evaluation runs with: the forms whose roots it looks for and the rules that pick them, the
//! converters of the forms the unit has roots of, the class layouts, the size limit of assets, the
//! naming of roots and the evaluator itself. When the content of every bundle of a window is
//! identified ([`Entry::content`]), the unit is keyed by those contents and by the class layouts,
//! the size limit, the naming and the evaluator, and the result a run gives the unit is recorded
//! under that key with the forms the run looked for and the version of each converter the unit
//! has roots of. A later run takes the recorded result while those versions are the same, and
//! fetches and evaluates only the windows of the other units. A new version of a converter
//! therefore evaluates again only the units that have roots of its form.
//!
//! A form that a result was not looked for in changes it only when the unit has roots of that
//! form. A run checks that from the indexes of the bundles of the window ([`EntryIndexes`]),
//! without their bytes, and takes the result when the unit has none; when an index is not known,
//! the unit is evaluated. A new form therefore evaluates again only the units with its roots.
//!
//! A unit is recorded only when no bundle of its window had an error. When an asset of a form
//! failed to convert, the record also holds the revision of the converter of that form, and a later
//! run takes it only while the revision is the same. A converter that learns to read such assets
//! needs a new revision, not a new version: the units with failed assets of its form are evaluated
//! again, and the assets it already produced stay as they are.
//!
//! A record holds the assets of its unit before the assets of different bundles that are given the
//! same name are told apart, since that depends on the other units; it happens once all of them are
//! gathered. Records are shared only by runs that name roots in the same way.
//!
//! A unit whose key and converters are those of the last run of its region keeps what that run
//! gave: the run neither evaluates it nor reads its record, and the state of the region holds its
//! assets. An evaluation gives the result of every other unit, also of those that cannot be
//! recorded, so that the state of the region can be advanced by them.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::io;

use sekai_catalog::store::parse_hex;
use sekai_catalog::{Key, UnitRecords};
use sekai_convert::Converter;
use sekai_unity::ClassLayouts;
use sekai_unity::index::{BundleIndex, VersionIndex};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

use crate::claims::Claimant;
use crate::eval::{self, Evaluation, Output, RootKey};
use crate::stream::{self, Entry, Gathered, Inputs, Outside, Stream, Streamed, Units};

/// The revision of how units are evaluated and recorded, which is part of every unit key. It also
/// changes when a rule that picks the roots of a form picks other objects, since a version of a
/// converter does not cover its rule.
const REVISION: &str = "unit 3";

/// The key of a unit: the SHA-256 of what its result follows from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitKey(pub [u8; 32]);

impl fmt::Display for UnitKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&eval::hex(&self.0))
    }
}

impl Serialize for UnitKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UnitKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_hex(&text)
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("a unit key is 64 hexadecimal digits"))
    }
}

/// What a run finds for a unit under its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// The last run of the region gave the unit under the same key; the state of the region holds
    /// what it gave.
    Kept(Kept),
    /// A result recorded by any run.
    Recorded(Record),
}

/// What the state of a region holds of a unit whose result a later run may keep.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Kept {
    /// The identity of each converter the unit has roots of.
    pub forms: BTreeSet<String>,
    /// The forms whose roots the run that gave the result looked for.
    pub scanned: BTreeSet<String>,
}

impl Found {
    /// The identity of each converter the unit has roots of.
    fn forms(&self) -> &BTreeSet<String> {
        match self {
            Found::Kept(kept) => &kept.forms,
            Found::Recorded(record) => &record.forms,
        }
    }

    /// The forms whose roots the run that gave the result looked for.
    fn scanned(&self) -> &BTreeSet<String> {
        match self {
            Found::Kept(kept) => &kept.scanned,
            Found::Recorded(record) => &record.scanned,
        }
    }

    /// The result as one that the forms `scanned` were looked for in.
    fn scanned_for(self, scanned: &BTreeSet<String>) -> Self {
        match self {
            Found::Kept(kept) => Found::Kept(Kept {
                scanned: scanned.clone(),
                ..kept
            }),
            Found::Recorded(record) => Found::Recorded(Record {
                scanned: scanned.clone(),
                ..record
            }),
        }
    }
}

/// Where a run finds the index of the bundle of a manifest entry without its bytes.
pub trait EntryIndexes: Sync {
    /// The index of the bundle of `entry`, when it is known.
    fn index(&self, entry: &Entry) -> io::Result<Option<BundleIndex>>;
}

/// No index is known.
pub struct NoEntryIndexes;

impl EntryIndexes for NoEntryIndexes {
    fn index(&self, _entry: &Entry) -> io::Result<Option<BundleIndex>> {
        Ok(None)
    }
}

/// Where a run finds what earlier runs gave.
pub trait Recorded: Sync {
    /// What earlier runs gave `unit` under `key`, if they gave it.
    fn record(&self, unit: &str, key: &UnitKey) -> io::Result<Option<Found>>;
}

impl<T: UnitRecords + ?Sized> Recorded for T {
    fn record(&self, _unit: &str, key: &UnitKey) -> io::Result<Option<Found>> {
        let record = self.get(&key.0)?.and_then(|bytes| Record::read(&bytes));
        Ok(record.map(Found::Recorded))
    }
}

/// What a unit gave in a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnitResult {
    /// The name of the unit.
    pub unit: String,
    /// The key of the unit, when the content of every bundle of its window is identified.
    pub key: Option<UnitKey>,
    /// Whether a later run may take the result: the unit has a key, no bundle of its window had
    /// an error and none of its assets failed to convert.
    pub reusable: bool,
    /// What the unit gave.
    pub record: Record,
}

impl UnitResult {
    /// The key under which a later run may take the result, if it may.
    pub fn reusable_key(&self) -> Option<&UnitKey> {
        self.key.as_ref().filter(|_| self.reusable)
    }

    /// The name that each asset of the unit claims, with the asset as its claimant.
    pub fn claims(&self) -> impl Iterator<Item = (String, Claimant)> + '_ {
        self.record.assets.iter().map(|asset| {
            let claimant = Claimant {
                unit: self.unit.clone(),
                form: asset.form.clone(),
                failed: asset.error.is_some(),
            };
            (asset.name.clone(), claimant)
        })
    }
}

/// A manifest split into the units that keep what the last run gave them, those that take a
/// recorded result, and the entries the others need.
pub struct Reuse<'m> {
    /// The units that keep what the last run of their region gave them.
    kept: Vec<String>,
    /// The units that take a recorded result.
    recorded: Vec<UnitResult>,
    /// The units to evaluate.
    pending: Vec<Pending<'m>>,
    /// The entries to evaluate: the windows of the units without a recorded result, and the
    /// entries that are no unit, such as those that share a name, so their errors are reported.
    rest: Vec<Entry>,
    /// The identities of each converter, by form.
    identities: BTreeMap<&'static str, Identities>,
    /// The forms whose roots the run looks for.
    scanned: BTreeSet<String>,
}

/// A unit to evaluate, with its key and the names of the bundles of its window.
struct Pending<'m> {
    unit: &'m str,
    key: Option<UnitKey>,
    window: Vec<&'m str>,
}

impl<'m> Reuse<'m> {
    /// Splits `manifest` into the units that `recorded` finds a result for, as an evaluation
    /// with `stream` would give it, and the entries the other units need. Whether a unit has roots
    /// of a form its result was not looked for in is read from the bundle indexes of `indexes`. A
    /// unit that is in the window of another unit to evaluate is evaluated too. Up to `lookups`
    /// units are looked up in `recorded` at once.
    pub fn new(
        manifest: &'m [Entry],
        stream: &Stream<'_>,
        recorded: &dyn Recorded,
        lookups: usize,
        indexes: &dyn EntryIndexes,
    ) -> io::Result<Self> {
        Self::split(manifest, stream, indexes, lookups, |unit, key| {
            recorded.record(unit, key)
        })
    }

    /// Evaluates every unit of `manifest`.
    pub fn refresh(manifest: &'m [Entry], stream: &Stream<'_>) -> io::Result<Self> {
        let threads = stream.options.threads;
        Self::split(manifest, stream, &NoEntryIndexes, threads, |_, _| Ok(None))
    }

    /// Splits `manifest` with what `lookup` finds for each unit and key, `lookups` units at once.
    fn split(
        manifest: &'m [Entry],
        stream: &Stream<'_>,
        indexes: &dyn EntryIndexes,
        lookups: usize,
        lookup: impl Fn(&str, &UnitKey) -> io::Result<Option<Found>> + Sync,
    ) -> io::Result<Self> {
        let _span = stream.recorder.span("plan", "reuse", "");
        let units = Units::new(manifest);
        let context = context(stream.classes, stream.limit, stream.loose);
        let identities = identities(stream);
        let current: BTreeSet<&str> = identities.values().flat_map(Identities::both).collect();
        let scanned: BTreeSet<String> = identities.keys().map(|&form| form.to_owned()).collect();
        let name = |entry: usize| units.entries[entry].name.as_str();
        let found_for = |unit: usize| -> io::Result<(Option<UnitKey>, Option<Found>)> {
            let key = key(&context, &units, unit);
            let found = match &key {
                Some(key) => lookup(name(unit), key)?,
                None => None,
            };
            let found = found.filter(|found| holds_with(found.forms(), &current));
            let found = match found {
                Some(found) => holds_for(found, &units, unit, stream, indexes)?,
                None => None,
            };
            Ok((key, found.map(|found| found.scanned_for(&scanned))))
        };
        let found = eval::parallel(units.entries.len(), lookups, found_for);
        let found = found.into_iter().collect::<io::Result<Vec<_>>>()?;
        let needed: BTreeSet<usize> = found
            .iter()
            .enumerate()
            .filter(|(_, (_, record))| record.is_none())
            .flat_map(|(unit, _)| units.windows[unit].iter().copied())
            .collect();

        let mut kept = Vec::new();
        let mut recorded = Vec::new();
        let mut pending = Vec::new();
        for (unit, (key, found)) in found.into_iter().enumerate() {
            match found {
                Some(Found::Kept(_)) if !needed.contains(&unit) => {
                    kept.push(name(unit).to_owned());
                }
                Some(Found::Recorded(record)) if !needed.contains(&unit) => {
                    recorded.push(UnitResult {
                        unit: name(unit).to_owned(),
                        key,
                        reusable: true,
                        record,
                    });
                }
                _ => pending.push(Pending {
                    unit: name(unit),
                    key,
                    window: units.windows[unit]
                        .iter()
                        .map(|&entry| name(entry))
                        .collect(),
                }),
            }
        }
        Ok(Self {
            kept,
            recorded,
            pending,
            rest: entries_to_evaluate(manifest, &units, &needed),
            identities,
            scanned,
        })
    }

    /// The entries to fetch and evaluate.
    pub fn rest(&self) -> &[Entry] {
        &self.rest
    }

    /// How many units keep what the last run gave them or take a recorded result.
    pub fn reused(&self) -> usize {
        self.kept.len() + self.recorded.len()
    }

    /// Evaluates the entries of [`Reuse::rest`], whose bytes come from `inputs`, as
    /// [`stream::evaluate`] does, and adds the recorded results of the units that take one.
    pub fn evaluate(self, inputs: &dyn Inputs, stream: &Stream<'_>) -> io::Result<Outcome> {
        let mut gathered = stream::gather(&self.rest, inputs, stream, &mut || Ok(()))?;
        let mut results = self.results(&gathered);
        for result in self.recorded {
            result.record.add_to(&result.unit, stream, &mut gathered)?;
            results.push(result);
        }
        results.sort_by(|a, b| a.unit.cmp(&b.unit));
        Ok(Outcome {
            streamed: stream::finish(gathered),
            results,
            kept: self.kept,
            scanned: self.scanned,
        })
    }

    /// The result of each evaluated unit in `gathered`. One whose window had an error is not
    /// reusable.
    fn results(&self, gathered: &Gathered) -> Vec<UnitResult> {
        let broken: HashSet<&str> = gathered
            .errors
            .keys()
            .map(|(name, _)| name.as_str())
            .collect();
        let mut records = Record::of_each(gathered, &self.identities, &self.scanned);
        self.pending
            .iter()
            .map(|pending| {
                let broken = pending.window.iter().any(|bundle| broken.contains(bundle));
                let record = records.remove(pending.unit);
                UnitResult {
                    unit: pending.unit.to_owned(),
                    key: pending.key,
                    reusable: pending.key.is_some() && !broken,
                    record: record.unwrap_or_else(|| Record::empty(&self.scanned)),
                }
            })
            .collect()
    }
}

/// What a run of a manifest gave.
#[derive(Debug, Default)]
pub struct Outcome {
    /// The evaluation of the units that do not keep what the last run gave them. Of the names
    /// that several of its assets received, none is given; the names of kept units are not
    /// looked at.
    pub streamed: Streamed,
    /// The result of each of those units, in name order.
    pub results: Vec<UnitResult>,
    /// The units that keep what the last run of their region gave them, in manifest order.
    pub kept: Vec<String>,
    /// The forms whose roots the run looked for, which every result now holds for.
    pub scanned: BTreeSet<String>,
}

/// The evaluation that `results` give together, as [`Reuse::evaluate`] gives it from them: their
/// assets in name order, with the names that more than one asset received left out.
pub fn evaluation(results: &[UnitResult], stream: &Stream<'_>) -> io::Result<Evaluation> {
    let mut gathered = Gathered::default();
    for result in results {
        result.record.add_to(&result.unit, stream, &mut gathered)?;
    }
    Ok(stream::finish(gathered).evaluation)
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

/// What the results of units follow from besides the bytes of their bundles, for an evaluation
/// with `converters`, the class layouts `classes`, the size limit of assets `limit` and the
/// classes `loose` of [`Stream::loose`]: the start of every unit key under `unit`, and the
/// identities of each converter under `form <name>`.
/// Evaluations with the same identity give a unit with the same bundles the same result, so a
/// run whose inputs and evaluation identity are those of the last run has nothing to evaluate.
pub fn evaluation_identity(
    converters: &[Box<dyn Converter>],
    classes: &dyn ClassLayouts,
    limit: Option<u64>,
    loose: &[&str],
) -> BTreeMap<String, String> {
    let forms = converters.iter().map(|converter| {
        let identity = Identities::of(converter.as_ref()).failing;
        (format!("form {}", converter.name()), identity)
    });
    let unit = ("unit".to_owned(), context(classes, limit, loose));
    std::iter::once(unit).chain(forms).collect()
}

/// What every unit key of an evaluation with the class layouts `classes`, the size limit of
/// assets `limit` and the classes `loose` of [`Stream::loose`] starts with: the revision, the
/// class layouts, the limit and those classes.
fn context(classes: &dyn ClassLayouts, limit: Option<u64>, loose: &[&str]) -> String {
    let mut text = format!("{REVISION}\n");
    if let Some(identity) = classes.identity() {
        text.push_str(&format!("classes {}\n", eval::hex(&identity)));
    }
    if let Some(limit) = limit {
        text.push_str(&format!("limit {limit}\n"));
    }
    if !loose.is_empty() {
        text.push_str(&format!("loose {}\n", loose.join(" ")));
    }
    text
}

/// What the results of a unit with roots of a form follow from of its converter.
struct Identities {
    /// When every asset of the form converted: the form, the version and the encoders of its
    /// files that lose information.
    converted: String,
    /// When an asset of the form failed: also the revision, which changes when it may convert.
    failing: String,
}

impl Identities {
    fn of(converter: &dyn Converter) -> Self {
        let converted = format!(
            "form {} {}\n{}",
            converter.name(),
            converter.version(),
            eval::lossy_encoders(converter)
        );
        let failing = format!("{converted}revision {}\n", converter.revision());
        Self { converted, failing }
    }

    fn both(&self) -> [&str; 2] {
        [&self.converted, &self.failing]
    }
}

/// The identities of each converter of `stream`, by form.
fn identities(stream: &Stream<'_>) -> BTreeMap<&'static str, Identities> {
    stream
        .converters
        .iter()
        .map(|converter| (converter.name(), Identities::of(converter.as_ref())))
        .collect()
}

/// Whether a result whose unit has roots of converters with the identities `forms` holds for an
/// evaluation whose converters have the identities `current`.
fn holds_with(forms: &BTreeSet<String>, current: &BTreeSet<&str>) -> bool {
    forms.iter().all(|form| current.contains(form.as_str()))
}

/// `found`, when it holds for an evaluation with `stream` of `unit` of `units`: when the unit has
/// no roots of the forms of `stream` that `found` was not looked for in, as the indexes of its
/// window in `indexes` tell.
fn holds_for(
    found: Found,
    units: &Units<'_>,
    unit: usize,
    stream: &Stream<'_>,
    indexes: &dyn EntryIndexes,
) -> io::Result<Option<Found>> {
    let unscanned: Vec<&dyn Converter> = stream
        .converters
        .iter()
        .map(Box::as_ref)
        .filter(|converter| !found.scanned().contains(converter.name()))
        .collect();
    if unscanned.is_empty() {
        return Ok(Some(found));
    }
    let mut window = Vec::new();
    for &entry in &units.windows[unit] {
        match indexes.index(units.entries[entry])? {
            Some(index) => window.push(index),
            None => return Ok(None),
        }
    }
    let index = VersionIndex::new(window);
    let name = units.entries[unit].name.as_str();
    let Some(bundle) = index
        .bundles()
        .iter()
        .position(|bundle| bundle.name == name)
    else {
        return Ok(None);
    };
    let window: BTreeSet<usize> = (0..index.bundles().len()).collect();
    let rootless = !stream::has_roots(&index, bundle, &window, &unscanned, stream.loose);
    Ok(rootless.then_some(found))
}

/// The key of `unit`, when the content of every bundle of its window is identified.
fn key(context: &str, units: &Units<'_>, unit: usize) -> Option<UnitKey> {
    let mut text = format!("{context}unit {}\n", units.entries[unit].name);
    for &entry in &units.windows[unit] {
        let entry = units.entries[entry];
        let content = entry.content.as_ref()?;
        text.push_str(&format!("bundle {} {content}\n", entry.name));
    }
    Some(UnitKey(Sha256::digest(text.as_bytes()).into()))
}

/// What a unit gave: its assets, the roots it left unnamed and the references that leave its
/// window, with the converters that gave them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The forms whose roots the run that gave the result looked for.
    scanned: BTreeSet<String>,
    /// The identity of each converter the unit has roots of.
    forms: BTreeSet<String>,
    assets: Vec<RecordedAsset>,
    /// The logical path and key of each root left unnamed.
    unnamed: Vec<(String, RootKey)>,
    outside: Vec<Outside>,
}

/// An asset of a unit, with the logical path and key of its root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct RecordedAsset {
    name: String,
    form: String,
    document: String,
    error: Option<String>,
    logical: String,
    #[serde(rename = "path_id")]
    root: RootKey,
}

impl RecordedAsset {
    /// The asset document. Fails when the record names an invalid one.
    fn document(&self) -> io::Result<Key> {
        self.document.parse().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "the record of {} names the document {}",
                    self.name, self.document
                ),
            )
        })
    }
}

impl Record {
    /// The record of a unit that gave nothing in an evaluation that looked for the forms
    /// `scanned`.
    fn empty(scanned: &BTreeSet<String>) -> Self {
        Self {
            scanned: scanned.clone(),
            ..Self::default()
        }
    }

    /// The record in `bytes`, or `None` when they hold none, so that the unit is evaluated again.
    pub fn read(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }

    /// The record as bytes, which [`Record::read`] reads back.
    pub fn bytes(&self) -> io::Result<Vec<u8>> {
        serde_json::to_vec(self).map_err(io::Error::other)
    }

    /// The identity of each converter the unit has roots of.
    pub fn forms(&self) -> &BTreeSet<String> {
        &self.forms
    }

    /// The asset document of each asset of the unit. Fails when the record names an invalid one.
    pub fn documents(&self) -> io::Result<Vec<Key>> {
        let documents = self.assets.iter().map(RecordedAsset::document);
        documents.collect()
    }

    /// The asset document of the asset of the unit that claims `name`, when the unit has one.
    /// Fails when the record names no valid document for it.
    pub fn document(&self, name: &str) -> io::Result<Option<Key>> {
        let asset = self.assets.iter().find(|asset| asset.name == name);
        asset.map(RecordedAsset::document).transpose()
    }

    /// The record of each unit that gave something in `gathered`, an evaluation that looked for
    /// the forms `scanned`, by unit name, with the `identities` of the converters by form.
    fn of_each<'g>(
        gathered: &'g Gathered,
        identities: &BTreeMap<&str, Identities>,
        scanned: &BTreeSet<String>,
    ) -> BTreeMap<&'g str, Record> {
        let empty = || Record::empty(scanned);
        let failing: BTreeSet<(&str, &str)> = gathered
            .outputs
            .iter()
            .filter(|(output, ..)| output.error.is_some())
            .map(|(output, ..)| (output.bundle.as_str(), output.form))
            .collect();
        let mut records: BTreeMap<&str, Record> = BTreeMap::new();
        for (unit, forms) in &gathered.forms {
            let identity = |form: &&str| {
                let identities = &identities[form];
                match failing.contains(&(unit.as_str(), *form)) {
                    true => identities.failing.clone(),
                    false => identities.converted.clone(),
                }
            };
            records.entry(unit).or_insert_with(empty).forms = forms.iter().map(identity).collect();
        }
        for (output, logical, root) in &gathered.outputs {
            records
                .entry(&output.bundle)
                .or_insert_with(empty)
                .assets
                .push(RecordedAsset {
                    name: output.name.clone(),
                    form: output.form.to_owned(),
                    document: output.document.to_string(),
                    error: output.error.clone(),
                    logical: logical.clone(),
                    root: root.clone(),
                });
        }
        for (unit, logical, root) in &gathered.unnamed {
            let unnamed = (logical.clone(), root.clone());
            records
                .entry(unit)
                .or_insert_with(empty)
                .unnamed
                .push(unnamed);
        }
        for outside in &gathered.outside {
            let record = records.entry(&outside.unit).or_insert_with(empty);
            record.outside.push(outside.clone());
        }
        records
    }

    /// Adds the record to `gathered` as what `unit` gave in an evaluation with `stream`.
    fn add_to(&self, unit: &str, stream: &Stream<'_>, gathered: &mut Gathered) -> io::Result<()> {
        let invalid = |what: String| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("the record of unit {unit} {what}"),
            )
        };
        for asset in &self.assets {
            let form = stream
                .converters
                .iter()
                .map(|converter| converter.name())
                .find(|&form| form == asset.form)
                .ok_or_else(|| invalid(format!("names the unknown form {}", asset.form)))?;
            let document = asset.document()?;
            let output = Output {
                name: asset.name.clone(),
                bundle: unit.to_owned(),
                form,
                document,
                error: asset.error.clone(),
            };
            let logical = asset.logical.clone();
            gathered.outputs.push((output, logical, asset.root.clone()));
        }
        let unnamed = self.unnamed.iter();
        let unnamed =
            unnamed.map(|(logical, root)| (unit.to_owned(), logical.clone(), root.clone()));
        gathered.unnamed.extend(unnamed);
        gathered.outside.extend(self.outside.iter().cloned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A record of a unit whose roots are objects, as it is stored.
    const OBJECT_RECORD: &str = concat!(
        r#"{"scanned":["audio"],"forms":["audio 5"],"assets":[{"name":"a/b","form":"audio","#,
        r#""document":"d","error":null,"logical":"a/b","path_id":-7}],"unnamed":[["a/c",9]],"#,
        r#""outside":[]}"#,
    );

    #[test]
    fn a_form_is_identified_with_the_encoders_whose_files_lose_information() {
        use sekai_convert::{Asset, Context, ConvertError, Encoders, ObjectId, ObjectIndex};
        use sekai_encode::{mp3, mp4};

        struct Declaring(Encoders);
        impl Converter for Declaring {
            fn name(&self) -> &'static str {
                "form"
            }
            fn version(&self) -> u32 {
                1
            }
            fn encoders(&self) -> Encoders {
                self.0
            }
            fn is_root(&self, _: &ObjectIndex, _: Option<&str>, _: &[String]) -> bool {
                false
            }
            fn convert(&self, _: ObjectId, _: &dyn Context) -> Result<Option<Asset>, ConvertError> {
                Ok(None)
            }
        }

        let identity = |encoders| Identities::of(&Declaring(encoders)).converted;
        let line = |name, version| format!("encoder {name} {version}\n");
        assert_eq!(identity(Encoders::NONE), identity(Encoders::TEXTURE));
        assert!(identity(Encoders::AUDIO).ends_with(&line(mp3::NAME, mp3::VERSION)));
        assert!(identity(Encoders::VIDEO).ends_with(&line(mp4::NAME, mp4::VERSION)));
    }

    #[test]
    fn a_record_of_object_roots_reads_and_writes_as_it_is_stored() {
        let record = Record::read(OBJECT_RECORD.as_bytes()).unwrap();
        assert_eq!(record.assets[0].root, RootKey::Object(-7));
        assert_eq!(record.unnamed, [("a/c".to_owned(), RootKey::Object(9))]);
        assert_eq!(record.bytes().unwrap(), OBJECT_RECORD.as_bytes());
    }

    #[test]
    fn a_file_root_is_stored_by_its_path() {
        let root = RootKey::File {
            file: "x.acb".to_owned(),
        };
        let record = Record {
            assets: vec![RecordedAsset {
                name: "b/x.acb".to_owned(),
                form: "audio".to_owned(),
                document: "d".to_owned(),
                error: None,
                logical: "b/x.acb".to_owned(),
                root: root.clone(),
            }],
            unnamed: vec![("b/y.acb".to_owned(), root)],
            ..Record::default()
        };
        let bytes = record.bytes().unwrap();
        let text = String::from_utf8(bytes.clone()).unwrap();
        assert!(text.contains(r#""path_id":{"file":"x.acb"}"#), "{text}");
        assert!(text.contains(r#"["b/y.acb",{"file":"x.acb"}]"#), "{text}");
        assert_eq!(Record::read(&bytes), Some(record));
    }
}
