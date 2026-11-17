//! The derivations of a run, put at the site and recorded while the run goes on.
//!
//! A derivation is recorded as soon as the objects it names are at the site, so the uploads
//! overlap with the conversions that follow, and a run that stops later leaves the derivations in
//! the record packs it wrote for the next run to take.
//!
//! The output of an asset is uploaded: its asset document and the members it names. One uploader
//! takes the outputs made since its last upload together and puts their objects at the site in one
//! store, which keeps as many requests in flight as the site takes at once. A file, such as the
//! encoding of a member, is put at the site by the output or the view that names it, which gives
//! its media type; it is recorded after the first store that leaves it at the site.
//!
//! After the first failure nothing more is uploaded, and the next derivation that is put reports
//! the failure, which stops the evaluation.

use std::io;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::Scope;

use sekai_catalog::{Derived, Key, Objects};
use sekai_pipeline::records::Records;

use super::document_objects;
use super::site::{Site, Stored};

/// The derivations of a run on their way to the site.
pub(super) struct Uploads<'a> {
    site: &'a Site<'a>,
    objects: &'a Objects<'a>,
    records: &'a Records<'a>,
    queue: Mutex<Queue>,
    /// Signalled when a derivation is sent or no more will be.
    sent: Condvar,
    /// Signalled when an upload ends.
    uploaded: Condvar,
}

/// A derivation of the run: its fingerprint and what it gave.
type Made = ([u8; 32], Derived);

/// The derivations waiting to be recorded, and what the uploader did.
#[derive(Default)]
struct Queue {
    /// The outputs of assets waiting for the uploader.
    outputs: Vec<Made>,
    /// The derivations whose objects the site did not hold at the last store.
    unheld: Vec<Made>,
    /// Whether the uploader is putting outputs at the site.
    uploading: bool,
    /// Whether no more outputs will be sent.
    closed: bool,
    stored: Stored,
    /// The first upload that failed.
    failure: Option<String>,
}

/// The uploader of [`Uploads`]. Dropping it lets it upload what was sent and stop.
pub(super) struct Uploader<'u, 'a>(&'u Uploads<'a>);

impl Drop for Uploader<'_, '_> {
    fn drop(&mut self) {
        lock(&self.0.queue).closed = true;
        self.0.sent.notify_all();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'a> Uploads<'a> {
    /// Uploads to `site` the objects that `objects` reads, and records the derivations in
    /// `records`.
    pub(super) fn new(
        site: &'a Site<'a>,
        objects: &'a Objects<'a>,
        records: &'a Records<'a>,
    ) -> Self {
        Self {
            site,
            objects,
            records,
            queue: Mutex::default(),
            sent: Condvar::new(),
            uploaded: Condvar::new(),
        }
    }

    /// Starts the uploader in `scope`. It runs until the returned guard is dropped and every
    /// output sent before then is uploaded.
    pub(super) fn run<'u, 'scope>(&'u self, scope: &'scope Scope<'scope, '_>) -> Uploader<'u, 'a>
    where
        'u: 'scope,
    {
        scope.spawn(|| self.work());
        Uploader(self)
    }

    /// Hands the output `derived` of the asset derivation `fingerprint` to the uploader. Fails
    /// when an earlier upload failed.
    pub(super) fn send_output(&self, fingerprint: [u8; 32], derived: Derived) -> io::Result<()> {
        self.send((fingerprint, derived), |queue| &mut queue.outputs)
    }

    /// Keeps `file`, the output of the derivation `fingerprint`, until a store puts it at the
    /// site. Fails when an earlier upload failed.
    pub(super) fn send_file(&self, fingerprint: [u8; 32], file: Key) -> io::Result<()> {
        let derived = Derived::Document(file);
        self.send((fingerprint, derived), |queue| &mut queue.unheld)
    }

    /// Adds `made` to the list of the queue that `list` names, unless this run made it already.
    fn send(&self, made: Made, list: impl FnOnce(&mut Queue) -> &mut Vec<Made>) -> io::Result<()> {
        let (fingerprint, derived) = &made;
        if !self.records.remember(*fingerprint, derived) {
            return Ok(());
        }
        let mut queue = lock(&self.queue);
        if let Some(failure) = &queue.failure {
            return Err(io::Error::other(failure.clone()));
        }
        list(&mut queue).push(made);
        self.sent.notify_one();
        Ok(())
    }

    /// Waits until every output sent is recorded, and returns what the uploads stored, or the
    /// first failure.
    pub(super) fn finish(&self) -> Result<Stored, String> {
        let mut queue = lock(&self.queue);
        while queue.uploading || (!queue.outputs.is_empty() && queue.failure.is_none()) {
            queue = self
                .uploaded
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
        match &queue.failure {
            Some(failure) => Err(failure.clone()),
            None => Ok(queue.stored),
        }
    }

    /// Records the files that a store after [`Uploads::finish`] put at the site.
    pub(super) fn record_stored(&self) -> Result<(), String> {
        let unheld = std::mem::take(&mut lock(&self.queue).unheld);
        self.record_held(unheld).map(drop)
    }

    /// Uploads the outputs sent, a batch at a time, until no more will come or one fails.
    fn work(&self) {
        while let Some((outputs, unheld)) = self.next() {
            let uploaded = self.upload(outputs, unheld);
            let mut queue = lock(&self.queue);
            queue.uploading = false;
            match uploaded {
                Ok((stored, unheld)) => {
                    queue.stored += stored;
                    queue.unheld.extend(unheld);
                }
                Err(failure) => queue.failure = Some(failure),
            }
            self.uploaded.notify_all();
        }
    }

    /// Every output waiting, once there is one, with the derivations unheld so far, or `None`
    /// once no more will come or an upload failed.
    fn next(&self) -> Option<(Vec<Made>, Vec<Made>)> {
        let mut queue = lock(&self.queue);
        loop {
            if queue.failure.is_some() {
                return None;
            }
            if !queue.outputs.is_empty() {
                queue.uploading = true;
                let outputs = std::mem::take(&mut queue.outputs);
                return Some((outputs, std::mem::take(&mut queue.unheld)));
            }
            if queue.closed {
                return None;
            }
            queue = self
                .sent
                .wait(queue)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Puts at the site the objects of `outputs`, then records them and those of `unheld` that
    /// the site now holds. Returns what it stored and the derivations still unheld.
    fn upload(&self, outputs: Vec<Made>, unheld: Vec<Made>) -> Result<(Stored, Vec<Made>), String> {
        let mut named = Vec::new();
        for (_, derived) in &outputs {
            if let Derived::Document(key) = derived {
                let objects = document_objects(self.objects, key);
                named.extend(objects.map_err(|e| format!("{key}: {e}"))?);
            }
        }
        let stored = self.site.store(self.objects, named)?;
        let unheld = self.record_held(outputs.into_iter().chain(unheld).collect())?;
        Ok((stored, unheld))
    }

    /// Records those of `made` whose objects the site holds, and returns the others.
    fn record_held(&self, made: Vec<Made>) -> Result<Vec<Made>, String> {
        let mut unheld = Vec::new();
        for (fingerprint, derived) in made {
            let recorded = self.records.record_derivation(&fingerprint, &derived);
            if !recorded.map_err(|e| format!("records: {e}"))? {
                unheld.push((fingerprint, derived));
            }
        }
        Ok(unheld)
    }
}
