//! The files of records at a site: the record packs, and the records kept one file each.

use std::io;

use sekai_catalog::Key;
use sekai_pipeline::parallel;
use sekai_pipeline::records::{DERIVATION_RECORDS, Entry, Pack, RECORD_PACKS, UNIT_RECORDS};
use sekai_publish::Listed;

use crate::update::site::Site;

/// A file of records: a pack, or a record kept in a file of its own.
pub(crate) struct RecordFile {
    pub(crate) file: Listed,
    /// Its records, each with the objects it names.
    pub(crate) records: Vec<(Entry, Vec<Key>)>,
}

/// The files of records at a site.
pub(crate) struct RecordFiles<'a> {
    pub(crate) site: &'a Site<'a>,
    /// How many files are read or written at once.
    pub(crate) requests: usize,
}

/// The name of the state file `file` among the state files.
pub(crate) fn state_name(file: &Listed) -> &str {
    file.name.strip_prefix("state/").unwrap_or(&file.name)
}

impl RecordFiles<'_> {
    /// The record packs of the site, with their records.
    pub(crate) fn packs(&self) -> Result<Vec<RecordFile>, String> {
        let files = self.site.list(&format!("state/{RECORD_PACKS}"))?;
        self.read(files, Pack::read)
    }

    /// The files of the records of units kept one file each, with their records.
    pub(crate) fn units(&self) -> Result<Vec<RecordFile>, String> {
        self.singles(UNIT_RECORDS)
    }

    /// The files of the records of derivations kept one file each, with their records.
    pub(crate) fn derivations(&self) -> Result<Vec<RecordFile>, String> {
        self.singles(DERIVATION_RECORDS)
    }

    /// The files of single records in the state directory `directory`, with their records; a
    /// file that holds no record that can be read holds none.
    fn singles(&self, directory: &str) -> Result<Vec<RecordFile>, String> {
        let files = self.site.list_sharded(&format!("state/{directory}"))?;
        self.read(files, |name, bytes| {
            Ok(Entry::of_file(name, bytes).into_iter().collect())
        })
    }

    /// The record files `files`, with the records that `read` finds in each and the objects each
    /// names. A file gone since the listing holds none.
    fn read(
        &self,
        files: Vec<Listed>,
        read: impl Fn(&str, &[u8]) -> io::Result<Vec<Entry>> + Sync,
    ) -> Result<Vec<RecordFile>, String> {
        let records = parallel(files.len(), self.requests, |i| {
            let name = state_name(&files[i]);
            let named = || -> io::Result<Vec<(Entry, Vec<Key>)>> {
                let Some(bytes) = self.site.files().read(name)? else {
                    return Ok(Vec::new());
                };
                let entries = read(name, &bytes)?;
                let objects = entries.iter().map(Entry::objects);
                let objects = objects.collect::<io::Result<Vec<_>>>()?;
                Ok(entries.into_iter().zip(objects).collect())
            };
            named().map_err(|e| format!("{name}: {e}"))
        });
        let files = files.into_iter().zip(records);
        let files = files.map(|(file, records)| {
            Ok(RecordFile {
                file,
                records: records?,
            })
        });
        files.collect()
    }

    /// Writes `packs` among the state files of the site.
    pub(crate) fn write(&self, packs: &[Pack]) -> Result<(), String> {
        let written = parallel(packs.len(), self.requests, |i| {
            let pack = &packs[i];
            let written = self.site.files().write(&pack.name, &pack.bytes);
            written.map_err(|e| format!("{}: {e}", pack.name))
        });
        written.into_iter().collect()
    }
}
