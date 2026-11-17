//! Record packs: records of units and derivations, as gzipped JSON lines in one state file named
//! by its SHA-256. A pack is never changed once written; the same records always give the same
//! packs.

use std::io::{self, BufRead, BufReader, Write};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sekai_catalog::Key;
use sekai_catalog::store::parse_hex;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;
use sha2::{Digest, Sha256};

use crate::eval::hex;
use crate::reuse::{Record, UnitKey};

/// The directory of the record packs among the state files.
pub const RECORD_PACKS: &str = "records/packs";

/// The bytes of records, as lines before compression, that fill a pack.
pub const PACK_BYTES: usize = 8 << 20;

/// The directory of unit records kept one file each, which `gc` moves into packs.
pub const UNIT_RECORDS: &str = "records/units";

/// The directory of derivation records kept one file each, which `gc` moves into packs.
pub const DERIVATION_RECORDS: &str = "records/derivations";

const FORMAT: &str = "record-pack";
const VERSION: u32 = 1;

/// What a record is the record of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Subject {
    /// The unit with this key.
    Unit(UnitKey),
    /// The derivation with this fingerprint.
    Derivation([u8; 32]),
}

/// A record, as a pack holds it: its subject and its JSON.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Entry {
    /// What it is the record of.
    pub subject: Subject,
    record: String,
}

/// What a derivation record holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DerivationRecord {
    /// The asset document, and its size.
    Document { key: String, size: u64 },
    /// The root holds no asset of its form.
    Nothing,
}

impl DerivationRecord {
    /// The document the record names, with its size.
    pub(super) fn document(&self) -> io::Result<Option<(Key, u64)>> {
        match self {
            Self::Document { key, size } => {
                let key = key.parse().map_err(|e| invalid("derivation record", e))?;
                Ok(Some((key, *size)))
            }
            Self::Nothing => Ok(None),
        }
    }
}

/// The header line of a pack.
#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    version: u32,
}

/// A line of a pack after its header: one record, under the key of its subject.
#[derive(Deserialize)]
struct Line<'a> {
    unit: Option<&'a str>,
    derivation: Option<&'a str>,
    #[serde(borrow)]
    record: &'a RawValue,
}

fn invalid(name: &str, e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("{name}: {e}"))
}

impl Entry {
    /// The record of the unit `key`.
    pub(super) fn unit(key: UnitKey, record: &Record) -> io::Result<Self> {
        let record = String::from_utf8(record.bytes()?).map_err(io::Error::other)?;
        Ok(Self {
            subject: Subject::Unit(key),
            record,
        })
    }

    /// The record of the derivation `fingerprint`.
    pub(super) fn derivation(fingerprint: [u8; 32], record: &DerivationRecord) -> io::Result<Self> {
        Ok(Self {
            subject: Subject::Derivation(fingerprint),
            record: serde_json::to_string(record).map_err(io::Error::other)?,
        })
    }

    /// The JSON of the record.
    pub fn json(&self) -> &str {
        &self.record
    }

    pub(super) fn into_json(self) -> String {
        self.record
    }

    /// The record, read as that of a derivation.
    pub(super) fn derivation_record(&self) -> io::Result<DerivationRecord> {
        serde_json::from_str(&self.record).map_err(|e| invalid("derivation record", e))
    }

    /// The objects the record names: the asset documents of a unit record, or the document of a
    /// derivation record. A unit record that cannot be read names nothing, since a run evaluates
    /// its unit again.
    pub fn objects(&self) -> io::Result<Vec<Key>> {
        match self.subject {
            Subject::Unit(_) => Record::read(self.record.as_bytes())
                .map_or(Ok(Vec::new()), |record| record.documents()),
            Subject::Derivation(_) => {
                let document = self.derivation_record()?.document()?;
                Ok(document.into_iter().map(|(key, _size)| key).collect())
            }
        }
    }

    /// The line of the entry in a pack.
    fn line(&self) -> String {
        let (kind, key) = match &self.subject {
            Subject::Unit(key) => ("unit", key.to_string()),
            Subject::Derivation(fingerprint) => ("derivation", hex(fingerprint)),
        };
        format!("{{\"{kind}\":\"{key}\",\"record\":{}}}\n", self.record)
    }

    /// The bytes of its line in a pack, which [`PACK_BYTES`] counts.
    pub fn size(&self) -> usize {
        self.line().len()
    }

    /// The entry of the line `line` of a pack.
    fn read(line: &str) -> Result<Self, String> {
        let line: Line<'_> = serde_json::from_str(line).map_err(|e| e.to_string())?;
        let key = |hex: &str| parse_hex(hex).ok_or_else(|| format!("{hex} is not a key"));
        let subject = match (line.unit, line.derivation) {
            (Some(unit), None) => Subject::Unit(UnitKey(key(unit)?)),
            (None, Some(derivation)) => Subject::Derivation(key(derivation)?),
            _ => return Err("a line names one unit or one derivation".to_owned()),
        };
        Ok(Self {
            subject,
            record: line.record.get().to_owned(),
        })
    }

    /// The record that a file of [`UNIT_RECORDS`] or [`DERIVATION_RECORDS`] named `file` holds,
    /// or `None` when the file is no record that can be read.
    pub fn of_file(file: &str, bytes: &[u8]) -> Option<Self> {
        let (directory, name) = file.rsplit_once('/')?;
        let key = parse_hex(name.strip_suffix(".json")?)?;
        let record = std::str::from_utf8(bytes).ok()?.to_owned();
        let subject = if directory.starts_with(UNIT_RECORDS) {
            Record::read(bytes).map(|_| Subject::Unit(UnitKey(key)))
        } else if directory.starts_with(DERIVATION_RECORDS) {
            let read = serde_json::from_str::<DerivationRecord>(&record);
            read.ok().map(|_| Subject::Derivation(key))
        } else {
            None
        }?;
        Some(Self { subject, record })
    }
}

/// A pack, as a state file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pack {
    /// The name of its state file, in [`RECORD_PACKS`].
    pub name: String,
    /// The bytes of the file.
    pub bytes: Vec<u8>,
}

impl Pack {
    /// The packs that hold `entries`: in subject order, with one record of each subject, and as
    /// many records in each as fill [`PACK_BYTES`].
    pub fn of(mut entries: Vec<Entry>) -> io::Result<Vec<Self>> {
        entries.sort();
        entries.dedup_by_key(|entry| entry.subject);
        let mut packs = Vec::new();
        let mut lines = String::new();
        for entry in &entries {
            let line = entry.line();
            if !lines.is_empty() && lines.len() + line.len() > PACK_BYTES {
                packs.push(Self::holding(&std::mem::take(&mut lines))?);
            }
            lines.push_str(&line);
        }
        if !lines.is_empty() {
            packs.push(Self::holding(&lines)?);
        }
        Ok(packs)
    }

    /// The pack of the record lines `lines`.
    fn holding(lines: &str) -> io::Result<Self> {
        let header = Header {
            format: FORMAT.to_owned(),
            version: VERSION,
        };
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        serde_json::to_writer(&mut gzip, &header).map_err(io::Error::other)?;
        gzip.write_all(b"\n")?;
        gzip.write_all(lines.as_bytes())?;
        let bytes = gzip.finish()?;
        let sha256: [u8; 32] = Sha256::digest(&bytes).into();
        Ok(Self {
            name: format!("{RECORD_PACKS}/{}.json.gz", hex(&sha256)),
            bytes,
        })
    }

    /// The records of the pack `bytes`, the state file `name`.
    pub fn read(name: &str, bytes: &[u8]) -> io::Result<Vec<Entry>> {
        let mut lines = BufReader::new(GzDecoder::new(bytes)).lines();
        let header = lines.next().transpose()?.unwrap_or_default();
        let header: Header = serde_json::from_str(&header).map_err(|e| invalid(name, e))?;
        if header.format != FORMAT || header.version != VERSION {
            let found = format!("{} {}", header.format, header.version);
            return Err(invalid(name, format!("{found} is not {FORMAT} {VERSION}")));
        }
        lines
            .map(|line| Entry::read(&line?).map_err(|e| invalid(name, e)))
            .collect()
    }
}

/// Whether `name`, a state file in [`RECORD_PACKS`], is a pack.
pub fn is_pack(name: &str) -> bool {
    name.strip_prefix(RECORD_PACKS)
        .and_then(|name| name.strip_prefix('/'))
        .and_then(|name| name.strip_suffix(".json.gz"))
        .is_some_and(|sha256| parse_hex(sha256).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn derivation(byte: u8, size: u64) -> Entry {
        let key = Key::of(&[byte], "webp").to_string();
        Entry::derivation([byte; 32], &DerivationRecord::Document { key, size }).unwrap()
    }

    #[test]
    fn a_pack_gives_back_its_records_in_subject_order_once_each() {
        let unit = Entry::unit(UnitKey([9; 32]), &Record::default()).unwrap();
        let nothing = Entry::derivation([5; 32], &DerivationRecord::Nothing).unwrap();
        let entries = vec![
            derivation(7, 3),
            nothing.clone(),
            unit.clone(),
            derivation(7, 4),
        ];
        let packs = Pack::of(entries).unwrap();
        assert_eq!(packs.len(), 1);
        assert!(is_pack(&packs[0].name), "{}", packs[0].name);
        let read = Pack::read(&packs[0].name, &packs[0].bytes).unwrap();
        assert_eq!(read, [unit, nothing, derivation(7, 3)]);
        let key = Key::of(&[7], "webp");
        assert_eq!(read[2].objects().unwrap(), [key]);
        assert!(read[1].objects().unwrap().is_empty());
    }

    #[test]
    fn the_same_records_give_the_same_packs() {
        let entries: Vec<Entry> = (0..=255).map(|byte| derivation(byte, 1)).collect();
        let mut reversed = entries.clone();
        reversed.reverse();
        assert_eq!(Pack::of(entries).unwrap(), Pack::of(reversed).unwrap());
    }

    #[test]
    fn records_beyond_the_pack_size_go_to_further_packs() {
        let entry = derivation(1, 1);
        let count = PACK_BYTES / entry.size() + 10;
        let entries = (0..count).map(|i| {
            let mut fingerprint = [0; 32];
            fingerprint[..8].copy_from_slice(&(i as u64).to_be_bytes());
            Entry {
                subject: Subject::Derivation(fingerprint),
                ..entry.clone()
            }
        });
        let packs = Pack::of(entries.collect()).unwrap();
        assert_eq!(packs.len(), 2);
        let read: usize = packs
            .iter()
            .map(|p| Pack::read(&p.name, &p.bytes).unwrap().len())
            .sum();
        assert_eq!(read, count);
    }

    #[test]
    fn a_pack_of_another_format_is_refused() {
        let mut gzip = GzEncoder::new(Vec::new(), Compression::default());
        gzip.write_all(b"{\"format\":\"record-pack\",\"version\":2}\n")
            .unwrap();
        let bytes = gzip.finish().unwrap();
        assert!(Pack::read("records/packs/x.json.gz", &bytes).is_err());
    }

    #[test]
    fn single_record_files_are_read_as_entries() {
        let record = serde_json::to_vec(&DerivationRecord::Nothing).unwrap();
        let name = format!("{DERIVATION_RECORDS}/05/{}.json", hex(&[5; 32]));
        let entry = Entry::of_file(&name, &record).unwrap();
        assert_eq!(
            entry,
            Entry::derivation([5; 32], &DerivationRecord::Nothing).unwrap()
        );
        assert_eq!(Entry::of_file(&name, b"not json"), None);
        let unit = Record::default().bytes().unwrap();
        let name = format!("{UNIT_RECORDS}/09/{}.json", hex(&[9; 32]));
        let entry = Entry::of_file(&name, &unit).unwrap();
        assert_eq!(entry.subject, Subject::Unit(UnitKey([9; 32])));
    }
}
