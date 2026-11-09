//! The lazy evaluator: computes a derivation when it is requested.
//!
//! Names and fingerprints are computed in advance, from the index of the version, exactly as the
//! eager evaluator computes them. A request converts and encodes one asset and reads only the
//! bundles that asset needs. It uses no threads of its own.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use sekai_catalog::{Derivations, Derived, Store};
use sekai_convert::Converter;
use sekai_format::AssetError;

use crate::eval::{self, Output, Plan, Planned};
use crate::metrics::Recorder;
use crate::version::Version;

#[derive(Default)]
struct State {
    /// The fingerprints being computed.
    running: BTreeSet<[u8; 32]>,
    /// The assets that failed: they are not recorded, so they are remembered here.
    failed: BTreeMap<[u8; 32], (Derived, String)>,
}

/// Evaluates the assets of a version one request at a time.
pub struct Lazy<'a> {
    version: &'a Version<'a>,
    plan: Plan<'a>,
    /// Each root as (bundle name, path ID), with the positions of its derivations in the plan.
    roots: BTreeMap<(&'a str, i64), Vec<usize>>,
    store: &'a dyn Store,
    derivations: &'a dyn Derivations,
    recorder: &'a dyn Recorder,
    memory_limit: Option<u64>,
    state: Mutex<State>,
    finished: Condvar,
}

/// A fingerprint being computed; it is released when this is dropped.
struct Running<'l> {
    fingerprint: [u8; 32],
    state: &'l Mutex<State>,
    finished: &'l Condvar,
}

impl Drop for Running<'_> {
    fn drop(&mut self) {
        lock(self.state).running.remove(&self.fingerprint);
        self.finished.notify_all();
    }
}

fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'a> Lazy<'a> {
    /// Prepares `version` for requests. Only the index of the version is read.
    pub fn new(
        version: &'a Version<'a>,
        converters: &'a [Box<dyn Converter>],
        store: &'a dyn Store,
        derivations: &'a dyn Derivations,
        recorder: &'a dyn Recorder,
        threads: usize,
    ) -> Self {
        let plan = eval::plan(version, converters, recorder, threads);
        let bundles = version.index().bundles();
        let mut roots: BTreeMap<_, Vec<usize>> = BTreeMap::new();
        for (position, planned) in plan.derivations.iter().enumerate() {
            let root = planned.closure[0];
            roots
                .entry((bundles[root.bundle].name.as_str(), root.path_id))
                .or_default()
                .push(position);
        }
        Self {
            version,
            plan,
            roots,
            store,
            derivations,
            recorder,
            memory_limit: None,
            state: Mutex::default(),
            finished: Condvar::new(),
        }
    }

    /// Refuses, with the error `too_large`, assets estimated to need more than `bytes` of memory.
    #[must_use]
    pub fn with_memory_limit(mut self, bytes: u64) -> Self {
        self.memory_limit = Some(bytes);
        self
    }

    /// The names of the assets, in order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.plan.derivations.iter().map(|plan| plan.name.as_str())
    }

    /// The asset `name`, computed if it has not been. `None` if the version has no such asset.
    pub fn asset(&self, name: &str) -> io::Result<Option<Output>> {
        let derivations = &self.plan.derivations;
        match derivations.binary_search_by(|plan| plan.name.as_str().cmp(name)) {
            Ok(position) => self.output(&derivations[position]),
            Err(_) => Ok(None),
        }
    }

    /// The assets rooted at the object `path_id` of the bundle `bundle`, computed if they have
    /// not been.
    pub fn object(&self, bundle: &str, path_id: i64) -> io::Result<Vec<Output>> {
        let positions = self.roots.get(&(bundle, path_id));
        positions
            .into_iter()
            .flatten()
            .filter_map(|position| self.output(&self.plan.derivations[*position]).transpose())
            .collect()
    }

    fn output(&self, plan: &Planned<'_>) -> io::Result<Option<Output>> {
        let (derived, error) = self.derived(plan)?;
        Ok(match derived {
            Derived::Document(document) => Some(Output {
                name: plan.name.clone(),
                form: plan.converter.name(),
                document,
                error,
            }),
            Derived::Nothing => None,
        })
    }

    /// The output of `plan`. Concurrent requests for one fingerprint compute it once.
    fn derived(&self, plan: &Planned<'_>) -> io::Result<(Derived, Option<String>)> {
        let fingerprint = plan.fingerprint;
        let mut state = lock(&self.state);
        loop {
            if let Some((derived, code)) = state.failed.get(&fingerprint) {
                return Ok((derived.clone(), Some(code.clone())));
            }
            if let Some(derived) = self.derivations.get(&fingerprint)? {
                return Ok((derived, None));
            }
            if state.running.insert(fingerprint) {
                break;
            }
            state = self
                .finished
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(state);
        let _running = Running {
            fingerprint,
            state: &self.state,
            finished: &self.finished,
        };
        let (derived, error) = self.compute(plan)?;
        match &error {
            Some(code) => {
                lock(&self.state)
                    .failed
                    .insert(fingerprint, (derived.clone(), code.clone()));
            }
            None => {
                self.derivations.put(fingerprint, derived.clone())?;
                self.derivations.flush()?;
            }
        }
        Ok((derived, error))
    }

    fn compute(&self, plan: &Planned<'_>) -> io::Result<(Derived, Option<String>)> {
        if let Some(limit) = self.memory_limit {
            let estimated = eval::estimated_memory(plan, self.version);
            if estimated > limit {
                let error = AssetError {
                    code: "too_large".to_owned(),
                    message: format!("needs about {estimated} bytes, the limit is {limit}"),
                };
                let document = eval::document(plan, self.version, self.store, Vec::new(), error)?;
                return Ok((Derived::Document(document), Some("too_large".to_owned())));
            }
        }
        eval::derive(plan, self.version, self.store, self.recorder)
    }
}
