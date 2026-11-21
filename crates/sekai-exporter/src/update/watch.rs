//! Keeping the sites of regions up to date: `update --watch`.

use std::thread;
use std::time::{Duration, Instant};

use sekai_catalog::MemoryHoldings;

use super::journal::journaled;
use super::site::Site;
use super::{Options, Run, configuration, export_version, look, region_of};
use crate::bucket::Bucket;
use crate::monitor::Monitor;

/// The longest a region waits after failures in a row.
const LONGEST_WAIT: Duration = Duration::from_secs(86_400);

/// Looks at each region of `options` every `interval`, and updates the ones whose version
/// changed, one after another. Returns only when the configuration or its bucket cannot be
/// opened; a region that fails is looked at again later.
pub(super) fn watch(options: &Options, interval: Duration) -> Result<bool, lexopt::Error> {
    let config = configuration(options)?;
    let bucket = Bucket::open(&config, options.secrets.as_deref())?;
    let site = bucket
        .as_ref()
        .map_or_else(|| Site::local(&options.output), Site::Bucket);
    let mut regions: Vec<Watched<'_>> = options
        .regions
        .iter()
        .map(|region| Watched::new(region))
        .collect();
    let every = interval.as_secs();
    println!("watching {} every {every} s", options.regions.join(" "));
    loop {
        let round = Instant::now();
        for watched in regions.iter_mut().filter(|watched| watched.due <= round) {
            let kept = keep(options, watched.region, &config, &site);
            watched.after(kept, interval, Instant::now());
        }
        thread::sleep(interval.saturating_sub(round.elapsed()));
    }
}

/// Brings `region` up to its current version at `site` when it changed, and returns whether the
/// region is at its current version.
///
/// With a bucket, the update works in `<output>/<region>`, which is removed after it, so that
/// nothing builds up from one update to the next. The metrics of the last update of the region
/// are kept in `<output>/<region>.metrics.jsonl`.
fn keep(options: &Options, region: &str, config: &str, site: &Site<'_>) -> Result<bool, String> {
    let scratch = matches!(site, Site::Bucket(_));
    let work = if scratch {
        options.output.join(region)
    } else {
        options.output.clone()
    };
    let run = Run {
        options,
        region,
        work: &work,
    };
    let looked = look(&run, config, site)?;
    let Some(change) = looked.change else {
        return Ok(true);
    };
    println!("{} {} {change}", run.state_key(), looked.version);
    std::fs::create_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let metrics = options.output.join(format!("{region}.metrics.jsonl"));
    let updated = site.while_held(&format!("watch {region}"), || {
        Monitor::watch(&metrics, |monitor| {
            let region = region_of(&run, config)?;
            let requests = site.requests(options.threads);
            journaled(site, MemoryHoldings::default(), requests, |journal| {
                export_version(&run, region, site, journal, monitor)
                    .map(|outcome| outcome.succeeded())
            })
        })
    });
    if scratch {
        std::fs::remove_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    let updated = updated?.map_err(|e| format!("{}: {e}", metrics.display()))?;
    updated.map_err(|e| e.to_string())
}

/// A region that `update --watch` keeps up to date, and when it is looked at next.
struct Watched<'o> {
    region: &'o str,
    /// How many times in a row keeping the region up to date failed.
    failures: u32,
    due: Instant,
}

impl<'o> Watched<'o> {
    fn new(region: &'o str) -> Self {
        Self {
            region,
            failures: 0,
            due: Instant::now(),
        }
    }

    /// Takes what keeping the region up to date gave at `now`. A region at its current version is
    /// looked at again in the next round; after failures in a row it waits the interval doubled
    /// once for each of them, up to a day.
    fn after(&mut self, kept: Result<bool, String>, interval: Duration, now: Instant) {
        let failure = match kept {
            Ok(true) => {
                self.failures = 0;
                self.due = now;
                return;
            }
            Ok(false) => "the update did not complete".to_owned(),
            Err(e) => e,
        };
        self.failures += 1;
        let doubled = interval.saturating_mul(1 << self.failures.min(16));
        let wait = doubled.min(LONGEST_WAIT);
        self.due = now + wait;
        eprintln!(
            "{}: {failure}; looking again in {} s",
            self.region,
            wait.as_secs()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_region_that_fails_waits_longer_each_time_up_to_a_day() {
        let mut watched = Watched::new("jp");
        let now = Instant::now();
        let minute = Duration::from_secs(60);
        let waits: Vec<Duration> = (0..12)
            .map(|_| {
                watched.after(Err("unreachable".to_owned()), minute, now);
                watched.due - now
            })
            .collect();
        assert_eq!(waits[..3], [minute * 2, minute * 4, minute * 8]);
        assert_eq!(waits[11], LONGEST_WAIT);

        watched.after(Ok(false), minute, now);
        assert_eq!(watched.due - now, LONGEST_WAIT);
        watched.after(Ok(true), minute, now);
        assert_eq!((watched.failures, watched.due), (0, now));
    }
}
