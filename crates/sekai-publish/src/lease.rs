//! The lease of the site of a bucket, so that one run at a time writes its state and pointers.
//!
//! The lease is the state file `lease.json`, which names the run that holds it. A run takes the
//! lease by writing the file only when the bucket has none, keeps it by writing it again only
//! while it still has the entity tag of its last write, and gives it up by deleting it the same
//! way. The bucket decides each of these writes, so two runs never both hold the lease. A lease
//! that its holder did not keep for [`Periods::lease`], by the clock of the bucket, was abandoned,
//! and a run takes it over by writing it only while it is that abandoned file. A run whose lease
//! was taken over writes no more state or pointers, so the run that took it over finds the site
//! as the last complete run left it.

use std::sync::atomic::Ordering;
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clock::Utc;
use crate::plan::{PublishError, hex};
use crate::s3::{Condition, S3};

/// The state file of the lease.
pub const LEASE_FILE: &str = "lease.json";

const FORMAT: &str = "site-lease";
const VERSION: u32 = 1;

/// The periods of a lease.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Periods {
    /// How long a lease that its holder does not keep stays its holder's.
    pub lease: Duration,
    /// How often the holder keeps its lease.
    pub keep: Duration,
    /// How often a run that waits for the lease looks at it again.
    pub wait: Duration,
}

impl Periods {
    /// The periods of the runs of the exporter: a lease of ten minutes, kept every two.
    pub const RUNS: Self = Self {
        lease: Duration::from_secs(600),
        keep: Duration::from_secs(120),
        wait: Duration::from_secs(30),
    };
}

/// Who holds the lease: an identifier of the run, what it does, the host it runs on and when it
/// took the lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Holder {
    /// `"site-lease"`.
    pub format: String,
    /// The version of the lease file, 1.
    pub version: u32,
    /// The identifier of the run.
    pub run: String,
    /// What the run does, such as `update jp cn`.
    pub operation: String,
    /// The host the run runs on.
    pub host: String,
    /// When the run took the lease, in RFC 3339.
    pub since: String,
}

impl Holder {
    /// A new run on this host that does `operation`.
    fn new(operation: &str) -> Self {
        let host = std::env::var("HOSTNAME")
            .or_else(|_| std::env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_owned());
        let now = SystemTime::now();
        let nanos = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
        let seed = format!("{host} {} {nanos}", std::process::id());
        Self {
            format: FORMAT.to_owned(),
            version: VERSION,
            run: hex(&Sha256::digest(seed.as_bytes()))[..16].to_owned(),
            operation: operation.to_owned(),
            host,
            since: Utc::of(now).rfc3339(),
        }
    }
}

/// What trying to take the lease gave.
enum Taken {
    /// The lease is ours, last written with this entity tag.
    Ours(String),
    /// Another run holds it.
    Held(Holder),
}

impl S3 {
    /// Runs `work` while this client holds the lease of the site as a run that does `operation`,
    /// with the periods of [`Periods::RUNS`]. See [`S3::while_held_for`].
    pub fn while_held<T>(
        &self,
        operation: &str,
        waiting: &mut dyn FnMut(&Holder),
        work: impl FnOnce() -> T,
    ) -> Result<T, PublishError> {
        self.while_held_for(&Periods::RUNS, operation, waiting, work)
    }

    /// Runs `work` while this client holds the lease of the site as a run that does `operation`.
    /// While another run holds it, the client tells `waiting` who does and looks again after
    /// `periods.wait`. The lease is kept while `work` runs and given up after it.
    pub fn while_held_for<T>(
        &self,
        periods: &Periods,
        operation: &str,
        waiting: &mut dyn FnMut(&Holder),
        work: impl FnOnce() -> T,
    ) -> Result<T, PublishError> {
        let lease = |e: String| PublishError::new("lease", e);
        let mine = Holder::new(operation);
        let bytes = serde_json::to_vec(&mine).map_err(|e| lease(e.to_string()))?;
        let entity_tag = loop {
            match self.take(periods, &mine, &bytes).map_err(lease)? {
                Taken::Ours(entity_tag) => break entity_tag,
                Taken::Held(holder) => {
                    waiting(&holder);
                    thread::sleep(periods.wait);
                }
            }
        };
        self.lease_lost.store(false, Ordering::SeqCst);
        let done = Stop::default();
        let (result, kept) = thread::scope(|scope| {
            let keeping = scope.spawn(|| self.keep(periods, &mine, &bytes, entity_tag, &done));
            let result = work();
            done.stop();
            (result, keeping.join())
        });
        let kept = kept.unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        if let Some(entity_tag) = kept {
            self.delete_state_if(LEASE_FILE, &entity_tag)
                .map_err(lease)?;
        }
        Ok(result)
    }

    /// Tries to take the lease for `mine`, whose file has `bytes`.
    fn take(&self, periods: &Periods, mine: &Holder, bytes: &[u8]) -> Result<Taken, String> {
        loop {
            if let Some(entity_tag) = self.put_state_if(LEASE_FILE, bytes, &Condition::Absent)? {
                return Ok(Taken::Ours(entity_tag));
            }
            let Some(held) = self.state_with_age(LEASE_FILE)? else {
                continue;
            };
            // A write that the bucket took and whose reply was lost leaves the lease ours.
            if is_mine(&held.bytes, mine) {
                return Ok(Taken::Ours(held.entity_tag));
            }
            if held.age < periods.lease {
                let holder = serde_json::from_slice(&held.bytes).map_err(|e| e.to_string())?;
                return Ok(Taken::Held(holder));
            }
            let abandoned = Condition::Matches(held.entity_tag);
            if let Some(entity_tag) = self.put_state_if(LEASE_FILE, bytes, &abandoned)? {
                return Ok(Taken::Ours(entity_tag));
            }
        }
    }

    /// Keeps the lease of `mine`, last written with `entity_tag`, until `done` stops, and returns
    /// its entity tag then, or `None` when it was lost: another run took it over, or it could not
    /// be kept for longer than the lease less one keeping.
    fn keep(
        &self,
        periods: &Periods,
        mine: &Holder,
        bytes: &[u8],
        mut entity_tag: String,
        done: &Stop,
    ) -> Option<String> {
        let mut kept_at = Instant::now();
        while !done.wait(periods.keep) {
            match self.rewrite(mine, bytes, &entity_tag) {
                Ok(Some(written)) => {
                    entity_tag = written;
                    kept_at = Instant::now();
                }
                Err(_) if kept_at.elapsed() < periods.lease.saturating_sub(periods.keep) => {}
                Ok(None) | Err(_) => {
                    self.lease_lost.store(true, Ordering::SeqCst);
                    return None;
                }
            }
        }
        Some(entity_tag)
    }

    /// Writes the lease of `mine` again while it has `entity_tag`, and returns its new entity tag,
    /// or `None` when another run holds it.
    fn rewrite(
        &self,
        mine: &Holder,
        bytes: &[u8],
        entity_tag: &str,
    ) -> Result<Option<String>, String> {
        let condition = Condition::Matches(entity_tag.to_owned());
        if let Some(written) = self.put_state_if(LEASE_FILE, bytes, &condition)? {
            return Ok(Some(written));
        }
        // A write that the bucket took and whose reply was lost leaves the lease ours.
        let held = self.state_with_age(LEASE_FILE)?;
        Ok(held
            .filter(|held| is_mine(&held.bytes, mine))
            .map(|held| held.entity_tag))
    }
}

/// Whether the lease file `bytes` names the run of `mine`.
fn is_mine(bytes: &[u8], mine: &Holder) -> bool {
    serde_json::from_slice::<Holder>(bytes).is_ok_and(|holder| holder.run == mine.run)
}

/// A signal that the work under a lease is done.
#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    changed: Condvar,
}

impl Stop {
    fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.changed.notify_all();
    }

    /// Waits up to `period`, and returns whether the work is done.
    fn wait(&self, period: Duration) -> bool {
        let stopped = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        let (stopped, _) = self
            .changed
            .wait_timeout_while(stopped, period, |stopped| !*stopped)
            .unwrap_or_else(|e| e.into_inner());
        *stopped
    }
}
