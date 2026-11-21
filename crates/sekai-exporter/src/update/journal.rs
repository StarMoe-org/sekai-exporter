//! The upload journal: the objects that runs put at a bucket, kept among its state files so that
//! the run after an interrupted one does not put them there again.
//!
//! A run writes the uploads that the bucket confirmed to a journal file once the objects of a
//! version are all there, a while after its last journal file, and when it stops before it is
//! done. A journal file lists one object and its size per line, gzipped, and is named by its
//! SHA-256 under [`UPLOADS`]. The first region of the next run that has work reads every journal
//! file and takes the objects they list as held. Once a run has brought every region it updates
//! to its version, the publications and records it made name what later runs build on, so it
//! deletes the journal files it read and wrote. `gc`, which may delete the objects they list,
//! deletes them first.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Write};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use sekai_catalog::{Holdings, Key, MemoryHoldings};
use sekai_pipeline::parallel;

use super::site::Site;

/// The directory of the journal files among the state files.
pub(crate) const UPLOADS: &str = "uploads";

/// How long after its last journal file a run writes the uploads confirmed since.
const WRITE_INTERVAL: Duration = Duration::from_secs(300);

/// The first line of a journal file.
const HEADER: &str = "upload-journal 1";

/// What a site holds, kept in memory, with the uploads to a bucket written to its journal. The
/// output directory serves its local store, so nothing is uploaded to it and it keeps no journal.
pub(crate) struct Journal<'a> {
    held: MemoryHoldings,
    site: &'a Site<'a>,
    /// How many journal files are read at once.
    requests: usize,
    written: Mutex<Written>,
}

/// The journal files of a run, and the uploads in none of them yet.
struct Written {
    /// Whether the journal files of earlier runs were read.
    read: bool,
    /// The journal files the run read or wrote.
    files: Vec<String>,
    /// The uploads confirmed since the last journal file.
    waiting: Vec<(Key, u64)>,
    /// When the run started or last wrote a journal file.
    since: Instant,
}

/// Runs `work` with the journal of `site`, which holds what `held` records, reading up to
/// `requests` journal files at once. Then deletes the journal files once `work` says it brought
/// every region to its version, and else writes the uploads it confirmed since the last journal
/// file.
pub(crate) fn journaled<E: From<String>>(
    site: &Site<'_>,
    held: MemoryHoldings,
    requests: usize,
    work: impl FnOnce(&Journal<'_>) -> Result<bool, E>,
) -> Result<bool, E> {
    let journal = Journal::new(site, held, requests);
    let done = work(&journal);
    if matches!(done, Ok(true)) {
        journal.close()?;
    } else if let Err(e) = journal.keep() {
        // The run failed or stopped already; what it could not keep it uploads again next time.
        eprintln!("journal: {e}");
    }
    done
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'a> Journal<'a> {
    /// The journal of `site`, holding what `held` records, reading up to `requests` journal files
    /// at once.
    fn new(site: &'a Site<'a>, held: MemoryHoldings, requests: usize) -> Self {
        Self {
            held,
            site,
            requests,
            written: Mutex::new(Written {
                read: false,
                files: Vec::new(),
                waiting: Vec::new(),
                since: Instant::now(),
            }),
        }
    }

    /// Takes as held what the journal files of earlier runs list, the first time a region has
    /// work, unless the run holds a listing of the whole bucket, which knows better.
    pub(crate) fn read_earlier(&self) -> io::Result<()> {
        let mut written = lock(&self.written);
        if written.read || matches!(self.site, Site::Local { .. }) {
            return Ok(());
        }
        written.read = true;
        let files = self.site.files().list(UPLOADS)?;
        if !files.is_empty() && !self.held.complete() {
            let state = self.site.files();
            let read = parallel(files.len(), self.requests, |i| state.read(&files[i]));
            let mut listed = 0;
            for bytes in read {
                // A journal file gone since the listing lists nothing the run can take.
                let uploads = read_journal(&bytes?.unwrap_or_default())?;
                listed += uploads.len();
                self.held.hold(&uploads)?;
            }
            println!("journal lists {listed} objects that earlier runs uploaded");
        }
        written.files.extend(files);
        Ok(())
    }

    /// What the site holds.
    pub(crate) fn held(&self) -> &MemoryHoldings {
        &self.held
    }

    /// Writes the uploads in no journal file yet: once the objects of a version are all at the
    /// site, and for a run that stops before it is done.
    pub(crate) fn keep(&self) -> io::Result<()> {
        let waiting = std::mem::take(&mut lock(&self.written).waiting);
        self.write(&waiting)
    }

    /// Deletes the journal files the run read and wrote, once it has brought every region it
    /// updates to its version.
    fn close(&self) -> Result<(), String> {
        let files = std::mem::take(&mut lock(&self.written).files);
        files
            .iter()
            .try_for_each(|name| self.site.delete_state(name))
    }

    /// Writes `uploads` to a new journal file.
    fn write(&self, uploads: &[(Key, u64)]) -> io::Result<()> {
        if uploads.is_empty() {
            return Ok(());
        }
        let bytes = journal_bytes(uploads)?;
        let name = format!("{UPLOADS}/{}", Key::of(&bytes, "txt.gz"));
        self.site.files().write(&name, &bytes)?;
        lock(&self.written).files.push(name);
        Ok(())
    }
}

impl Holdings for Journal<'_> {
    fn size(&self, key: &Key) -> io::Result<Option<u64>> {
        self.held.size(key)
    }

    fn sizes(&self, prefix: u8) -> io::Result<HashMap<Key, u64>> {
        self.held.sizes(prefix)
    }

    /// Records that the site holds `objects`, and writes the uploads waiting to a journal file
    /// once the last one is old enough.
    fn hold(&self, objects: &[(Key, u64)]) -> io::Result<()> {
        self.held.hold(objects)?;
        if matches!(self.site, Site::Local { .. }) {
            return Ok(());
        }
        let due = {
            let mut written = lock(&self.written);
            written.waiting.extend_from_slice(objects);
            let due = written.since.elapsed() >= WRITE_INTERVAL;
            if due {
                written.since = Instant::now();
            }
            due.then(|| std::mem::take(&mut written.waiting))
        };
        due.map_or(Ok(()), |uploads| self.write(&uploads))
    }

    fn release(&self, key: &Key) -> io::Result<()> {
        self.held.release(key)
    }

    fn complete(&self) -> bool {
        self.held.complete()
    }
}

/// The bytes of a journal file that lists `uploads`.
fn journal_bytes(uploads: &[(Key, u64)]) -> io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    writeln!(encoder, "{HEADER}")?;
    for (key, size) in uploads {
        writeln!(encoder, "{key} {size}")?;
    }
    encoder.finish()
}

/// The uploads that the journal file `bytes` lists; none for an empty file.
fn read_journal(bytes: &[u8]) -> io::Result<Vec<(Key, u64)>> {
    if bytes.is_empty() {
        return Ok(Vec::new());
    }
    let invalid =
        |line: &str| io::Error::new(io::ErrorKind::InvalidData, format!("journal: {line}"));
    let mut lines = BufReader::new(GzDecoder::new(bytes)).lines();
    let header = lines.next().transpose()?.unwrap_or_default();
    if header != HEADER {
        return Err(invalid(&header));
    }
    lines
        .map(|line| {
            let line = line?;
            let (key, size) = line.split_once(' ').ok_or_else(|| invalid(&line))?;
            let key: Key = key.parse().map_err(|_| invalid(&line))?;
            let size = size.parse().map_err(|_| invalid(&line))?;
            Ok((key, size))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_journal_file_lists_what_was_written_to_it() {
        let uploads = vec![(Key::of(b"a", "png"), 1), (Key::of(b"bc", "json"), 2)];
        let bytes = journal_bytes(&uploads).unwrap();
        assert_eq!(read_journal(&bytes).unwrap(), uploads);
        assert!(read_journal(&[]).unwrap().is_empty());
        let other = journal_bytes(&[]).unwrap();
        assert!(read_journal(&other).unwrap().is_empty());
        assert!(read_journal(b"not gzip").is_err());
    }
}
