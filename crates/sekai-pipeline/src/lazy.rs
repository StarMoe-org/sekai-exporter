//! The lazy evaluator: computes a derivation when it is requested.
//!
//! The assets of a bundle are named and fingerprinted from the unit of that bundle, exactly as the
//! streaming evaluator computes them: the bundle with every bundle it depends on. A request opens
//! only that window and converts and encodes one asset. It uses no threads of its own.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

use sekai_catalog::Derived;
use sekai_format::SourceObject;

use crate::canonical::Files;
use crate::eval::{self, Output, RootKey, Subject, source_objects};
use crate::stream::{Entry, Inputs, PARSED_BYTES, Root, Stream, Units, plan_unit};
use crate::version::{BundleError, Parsed, Version};

/// Maximum number of unit windows kept open in the LRU cache.
const MAX_OPEN_WINDOWS: usize = 16;

#[derive(Default)]
struct State {
    /// The fingerprints being computed.
    running: BTreeSet<[u8; 32]>,
    /// The assets that failed: they are not recorded, so they are remembered here.
    failed: BTreeMap<[u8; 32], (Derived, String)>,
}

#[derive(Default)]
struct Windows<'a> {
    /// Most-recently-used windows, newest at the back.
    recent: VecDeque<Arc<Opened<'a>>>,
    /// Units currently being opened by a thread.
    opening: BTreeSet<usize>,
}

/// The window of a unit, opened, with the assets of its bundle.
struct Opened<'a> {
    unit: usize,
    version: Version<'a>,
    /// The assets of the bundle, in name order.
    roots: Vec<Root<'a>>,
    unnamed: Vec<(String, RootKey)>,
    errors: Vec<BundleError>,
}

/// An asset of a bundle, as planned before it is converted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedAsset {
    /// The name of the asset.
    pub name: String,
    /// The asset form.
    pub form: &'static str,
    /// Where the asset starts in its bundle.
    pub root: RootKey,
}

/// Evaluates the assets of the bundles of a manifest one request at a time.
pub struct Lazy<'a> {
    units: Units<'a>,
    inputs: &'a dyn Inputs,
    stream: Stream<'a>,
    parsed: Arc<Parsed>,
    windows: Mutex<Windows<'a>>,
    window_ready: Condvar,
    state: Mutex<State>,
    finished: Condvar,
}

impl Drop for Lazy<'_> {
    fn drop(&mut self) {
        let _ = self.stream.derivations.flush();
    }
}

/// A unit window being opened; released when dropped.
struct Opening<'l, 'a> {
    unit: usize,
    windows: &'l Mutex<Windows<'a>>,
    ready: &'l Condvar,
}

impl Drop for Opening<'_, '_> {
    fn drop(&mut self) {
        lock(self.windows).opening.remove(&self.unit);
        self.ready.notify_all();
    }
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl<'a> Lazy<'a> {
    /// Prepares the bundles of `manifest`, whose bytes come from `inputs`, for requests. Nothing
    /// is read before the first request. Of `stream`, the converters, naming, store, records,
    /// indexes, block cache, classes, fusion, recorder and memory limit are used.
    pub fn new(manifest: &'a [Entry], inputs: &'a dyn Inputs, stream: Stream<'a>) -> Self {
        Self {
            units: Units::new(manifest),
            inputs,
            stream,
            parsed: Arc::new(Parsed::new(PARSED_BYTES)),
            windows: Mutex::default(),
            window_ready: Condvar::new(),
            state: Mutex::default(),
            finished: Condvar::new(),
        }
    }

    /// Flushes any buffered derivation records to persistent storage.
    pub fn flush(&self) -> io::Result<()> {
        self.stream.derivations.flush()
    }

    /// Manifest entries that cannot be evaluated, in order of name.
    pub fn errors(&self) -> &[BundleError] {
        &self.units.errors
    }

    /// The bundles whose assets can be requested, in name order.
    pub fn bundles(&self) -> impl Iterator<Item = &str> {
        self.units.entries.iter().map(|entry| entry.name.as_str())
    }

    /// The assets of `bundle`, in name order. Empty for a bundle the manifest does not list.
    pub fn assets(&self, bundle: &str) -> io::Result<Vec<PlannedAsset>> {
        let Some(opened) = self.bundle(bundle)? else {
            return Ok(Vec::new());
        };
        let assets = opened.roots.iter().map(|root| PlannedAsset {
            name: root.planned.name.clone(),
            form: root.planned.converter.name(),
            root: root.key.clone(),
        });
        Ok(assets.collect())
    }

    /// Roots of `bundle` that cannot be told apart from another root by name, as
    /// their logical paths and keys.
    pub fn unnamed(&self, bundle: &str) -> io::Result<Vec<(String, RootKey)>> {
        Ok(self
            .bundle(bundle)?
            .map(|opened| opened.unnamed.clone())
            .unwrap_or_default())
    }

    /// The bundles of the window of `bundle` that could not be read, opened or indexed.
    pub fn bundle_errors(&self, bundle: &str) -> io::Result<Vec<BundleError>> {
        Ok(self
            .bundle(bundle)?
            .map(|opened| opened.errors.clone())
            .unwrap_or_default())
    }

    /// The asset `name`, computed if it has not been. `None` if no bundle has such an asset.
    pub fn asset(&self, name: &str) -> io::Result<Option<Output>> {
        match self.find(name)? {
            Some((opened, position)) => self.output(&opened, position),
            None => Ok(None),
        }
    }

    /// The objects the conversion of the asset `name` may read: its root first, then every object
    /// the root reaches. Empty for an asset whose root is a file of a bundle; `None` if no bundle
    /// has such an asset. Nothing is converted.
    pub fn closure(&self, name: &str) -> io::Result<Option<Vec<SourceObject>>> {
        let Some((opened, position)) = self.find(name)? else {
            return Ok(None);
        };
        Ok(Some(match &opened.roots[position].planned.subject {
            Subject::Object(objects) => source_objects(opened.version.index(), objects),
            Subject::File(_) => Vec::new(),
        }))
    }

    /// The window that plans the asset `name`, and the position of the asset among its roots.
    fn find(&self, name: &str) -> io::Result<Option<(Arc<Opened<'a>>, usize)>> {
        // An asset is named under the directory of its bundle, so its name starts with the
        // bundle name; the longest bundle name comes first.
        let ends = name
            .char_indices()
            .filter(|&(_, c)| c == '/' || c == '@')
            .map(|(i, _)| i)
            .chain([name.len()]);
        let ends: BTreeSet<usize> = ends.collect();
        for end in ends.into_iter().rev() {
            let Some(&unit) = self.units.positions.get(&name[..end]) else {
                continue;
            };
            let opened = self.open(unit)?;
            let found = opened
                .roots
                .binary_search_by(|root| root.planned.name.as_str().cmp(name));
            if let Ok(position) = found {
                return Ok(Some((opened, position)));
            }
        }
        Ok(None)
    }

    /// The assets that start at `root` of the bundle `bundle`, computed if they have not been.
    pub fn rooted_at(&self, bundle: &str, root: &RootKey) -> io::Result<Vec<Output>> {
        let Some(opened) = self.bundle(bundle)? else {
            return Ok(Vec::new());
        };
        let positions = (0..opened.roots.len()).filter(|&i| opened.roots[i].key == *root);
        positions
            .filter_map(|position| self.output(&opened, position).transpose())
            .collect()
    }

    fn bundle(&self, bundle: &str) -> io::Result<Option<Arc<Opened<'a>>>> {
        match self.units.positions.get(bundle) {
            Some(&unit) => self.open(unit).map(Some),
            None => Ok(None),
        }
    }

    /// Opens the window of `unit` and plans the assets of its bundle.
    fn open(&self, unit: usize) -> io::Result<Arc<Opened<'a>>> {
        let mut windows = lock(&self.windows);
        loop {
            if let Some(idx) = windows.recent.iter().position(|o| o.unit == unit) {
                let opened = windows.recent.remove(idx).expect("index came from recent");
                windows.recent.push_back(opened.clone());
                return Ok(opened);
            }
            if windows.opening.insert(unit) {
                break;
            }
            windows = self
                .window_ready
                .wait(windows)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(windows);
        let _opening = Opening {
            unit,
            windows: &self.windows,
            ready: &self.window_ready,
        };

        let mut errors = Vec::new();
        let mut inputs = Vec::new();
        for &entry in &self.units.windows[unit] {
            let name = &self.units.entries[entry].name;
            match self.inputs.open(name) {
                Ok(input) => inputs.push(input),
                Err(e) => errors.push(BundleError {
                    name: name.clone(),
                    code: "unreadable",
                    message: e.to_string(),
                }),
            }
        }
        let (version, more) = Version::open_indexed(
            inputs,
            self.stream.cache,
            self.stream.classes,
            self.stream.indexes,
            1,
        )?;
        errors.extend(more);
        errors.sort_by(|a, b| a.name.cmp(&b.name));
        let version = version.sharing(self.parsed.clone());
        let mut roots = Vec::new();
        let mut unnamed = Vec::new();
        {
            let index = version.index();
            let position = |entry: usize| {
                let name = self.units.entries[entry].name.as_str();
                index
                    .bundles()
                    .binary_search_by(|bundle| bundle.name.as_str().cmp(name))
                    .ok()
            };
            if let Some(bundle) = position(unit) {
                let window: BTreeSet<usize> = self.units.windows[unit]
                    .iter()
                    .filter_map(|&entry| position(entry))
                    .collect();
                plan_unit(
                    &version,
                    bundle,
                    &window,
                    &self.stream,
                    None,
                    &mut roots,
                    &mut BTreeSet::new(),
                    &mut unnamed,
                );
            }
        }
        roots.sort_by(|a, b| a.planned.name.cmp(&b.planned.name));
        unnamed.sort();
        let opened = Arc::new(Opened {
            unit,
            version,
            roots,
            unnamed,
            errors,
        });
        let mut windows = lock(&self.windows);
        if windows.recent.len() >= MAX_OPEN_WINDOWS {
            windows.recent.pop_front();
        }
        windows.recent.push_back(opened.clone());
        Ok(opened)
    }

    fn output(&self, opened: &Opened<'_>, position: usize) -> io::Result<Option<Output>> {
        let root = &opened.roots[position];
        let (derived, error) = self.derived(opened, root)?;
        Ok(match derived {
            Derived::Document(document) => Some(Output {
                name: root.planned.name.clone(),
                bundle: opened.version.index().bundles()[root.planned.subject.bundle()]
                    .name
                    .clone(),
                form: root.planned.converter.name(),
                document,
                error,
            }),
            Derived::Nothing => None,
        })
    }

    /// The output of `root`. Concurrent requests for one fingerprint compute it once.
    fn derived(
        &self,
        opened: &Opened<'_>,
        root: &Root<'_>,
    ) -> io::Result<(Derived, Option<String>)> {
        let fingerprint = root.planned.fingerprint;
        let mut state = lock(&self.state);
        loop {
            if let Some((derived, code)) = state.failed.get(&fingerprint) {
                return Ok((derived.clone(), Some(code.clone())));
            }
            if let Some(derived) = self.stream.derivations.get(&fingerprint)? {
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
        let stored = match root.refusal(&opened.version, self.stream.limit) {
            Some(error) => Some(eval::document(
                &root.planned,
                &opened.version,
                self.stream.store,
                Vec::new(),
                error,
            )?),
            None => eval::derive(
                &root.planned,
                &opened.version,
                Files {
                    store: self.stream.store,
                    records: Some(self.stream.derivations),
                    recorder: self.stream.recorder,
                },
                self.stream.fusion,
            )?,
        };
        let (derived, error) = eval::outcome(stored.as_ref());
        match &error {
            Some(code) => {
                lock(&self.state)
                    .failed
                    .insert(fingerprint, (derived.clone(), code.clone()));
            }
            None => {
                self.stream.derivations.put(fingerprint, derived.clone())?;
            }
        }
        Ok((derived, error))
    }
}
