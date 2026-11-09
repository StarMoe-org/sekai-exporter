//! Watching a run: CPU and memory samples, and the spans of the evaluator.
//!
//! A run writes one JSON document per line to `metrics.jsonl` and shows a status line on standard
//! error. Nothing here reaches the store or any published file.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::Path;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use cpu_time::ThreadTime;
use sekai_pipeline::metrics::{Recorder, Span};
use serde_json::json;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
/// How many samples pass between status lines.
const STATUS_EVERY: u64 = 5;
const GIB: f64 = (1u64 << 30) as f64;

/// What the work on one derivation cost, summed over its spans.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Cost {
    pub(crate) kind: String,
    pub(crate) wall_us: u64,
    pub(crate) cpu_us: u64,
    pub(crate) read: u64,
    pub(crate) written: u64,
    pub(crate) estimated_memory: u64,
    pub(crate) actual_memory: u64,
}

/// Whether a stage is work on a derivation, and the kind of the derivation it names.
fn derivation_kind<'a>(stage: &str, label: &'a str) -> Option<Option<&'a str>> {
    match stage {
        "convert" | "encode" | "transform" => Some(Some(label)),
        "store" => Some(None),
        _ => None,
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct Total {
    count: u64,
    wall_us: u64,
    cpu_us: u64,
    read: u64,
    written: u64,
}

/// Records the run to a file. See the module documentation.
pub(crate) struct Monitor {
    start: Instant,
    sink: Mutex<BufWriter<File>>,
    totals: Mutex<BTreeMap<(&'static str, String), Total>>,
    costs: Mutex<BTreeMap<String, Cost>>,
    done: Mutex<bool>,
    wake: Condvar,
}

impl Monitor {
    /// What each derivation of the run cost, by name.
    pub(crate) fn costs(&self) -> BTreeMap<String, Cost> {
        self.costs.lock().unwrap().clone()
    }

    /// Runs `body` while samples are taken, then writes the summary.
    pub(crate) fn watch<T>(path: &Path, body: impl FnOnce(&Monitor) -> T) -> io::Result<T> {
        let monitor = Monitor {
            start: Instant::now(),
            sink: Mutex::new(BufWriter::new(File::create(path)?)),
            totals: Mutex::new(BTreeMap::new()),
            costs: Mutex::new(BTreeMap::new()),
            done: Mutex::new(false),
            wake: Condvar::new(),
        };
        let (result, peak) = std::thread::scope(|s| {
            let sampler = s.spawn(|| monitor.sample());
            let result = body(&monitor);
            *monitor.done.lock().unwrap() = true;
            monitor.wake.notify_all();
            (result, sampler.join().expect("the sampler does not panic"))
        });
        monitor.finish(&peak)?;
        Ok(result)
    }

    fn millis(&self) -> u64 {
        self.start.elapsed().as_millis() as u64
    }

    fn write(&self, value: &serde_json::Value) {
        let mut sink = self.sink.lock().unwrap();
        // A full disk must not stop the run; the store write that follows reports it.
        let _ = writeln!(sink, "{value}");
    }

    fn sample(&self) -> Peak {
        let mut peak = Peak::default();
        let Ok(pid) = sysinfo::get_current_pid() else {
            return peak;
        };
        let Ok(mut system) = System::new() else {
            return peak;
        };
        let kind = ProcessRefreshKind::nothing().with_cpu().with_memory();
        // CPU use is measured between two refreshes.
        system.refresh_cpu_usage();
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, kind);
        let mut samples = 0u64;
        let mut done = self.done.lock().unwrap();
        while !*done {
            done = self.wake.wait_timeout(done, SAMPLE_INTERVAL).unwrap().0;
            if *done {
                break;
            }
            system.refresh_cpu_usage();
            system.refresh_memory();
            system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, kind);
            let Some(process) = system.process(pid) else {
                continue;
            };
            let sample = Sample {
                cores: f64::from(process.cpu_usage()) / 100.0,
                rss: process.memory(),
                system_cpu: f64::from(system.global_cpu_usage()),
                system_used: system.used_memory(),
                system_total: system.total_memory(),
            };
            peak.add(&sample);
            self.write(&json!({
                "kind": "sample",
                "t_ms": self.millis(),
                "process_cores": sample.cores,
                "process_rss": sample.rss,
                "system_cpu_pct": sample.system_cpu,
                "system_mem_used": sample.system_used,
                "system_mem_total": sample.system_total,
            }));
            samples += 1;
            if samples.is_multiple_of(STATUS_EVERY) {
                eprintln!(
                    "[{:>6}s] cpu {:.1} cores (machine {:.0}%)  memory {:.2} GiB (machine {:.1}/{:.1} GiB)",
                    self.start.elapsed().as_secs(),
                    sample.cores,
                    sample.system_cpu,
                    sample.rss as f64 / GIB,
                    sample.system_used as f64 / GIB,
                    sample.system_total as f64 / GIB,
                );
            }
        }
        peak
    }

    fn finish(&self, peak: &Peak) -> io::Result<()> {
        let totals = self.totals.lock().unwrap();
        let mut rows: Vec<_> = totals.iter().collect();
        rows.sort_by_key(|(_, total)| std::cmp::Reverse(total.wall_us));
        let elapsed = self.start.elapsed().as_secs_f64();
        eprintln!(
            "run {elapsed:.1}s  peak memory {:.2} GiB  mean cpu {:.1} cores  peak machine cpu {:.0}%  peak machine memory {:.1} GiB",
            peak.rss as f64 / GIB,
            peak.mean_cores(),
            peak.system_cpu,
            peak.system_used as f64 / GIB,
        );
        for ((stage, label), total) in &rows {
            eprintln!(
                "  {stage:<10} {label:<16} {:>8}  wall {:>9.1}s  cpu {:>9.1}s  read {:>8.1} MiB  written {:>8.1} MiB",
                total.count,
                total.wall_us as f64 / 1e6,
                total.cpu_us as f64 / 1e6,
                total.read as f64 / (1u64 << 20) as f64,
                total.written as f64 / (1u64 << 20) as f64,
            );
            self.write(&json!({
                "kind": "total",
                "stage": stage,
                "label": label,
                "count": total.count,
                "wall_us": total.wall_us,
                "cpu_us": total.cpu_us,
                "read": total.read,
                "written": total.written,
            }));
        }
        self.write(&json!({
            "kind": "summary",
            "elapsed_ms": self.millis(),
            "peak_rss": peak.rss,
            "mean_cores": peak.mean_cores(),
            "peak_system_cpu_pct": peak.system_cpu,
            "peak_system_mem_used": peak.system_used,
        }));
        self.sink.lock().unwrap().flush()
    }
}

struct Sample {
    cores: f64,
    rss: u64,
    system_cpu: f64,
    system_used: u64,
    system_total: u64,
}

#[derive(Debug, Default)]
struct Peak {
    rss: u64,
    cores_sum: f64,
    samples: u64,
    system_cpu: f64,
    system_used: u64,
}

impl Peak {
    fn add(&mut self, sample: &Sample) {
        self.rss = self.rss.max(sample.rss);
        self.cores_sum += sample.cores;
        self.samples += 1;
        self.system_cpu = self.system_cpu.max(sample.system_cpu);
        self.system_used = self.system_used.max(sample.system_used);
    }

    fn mean_cores(&self) -> f64 {
        self.cores_sum / self.samples.max(1) as f64
    }
}

struct MonitorSpan<'a> {
    monitor: &'a Monitor,
    stage: &'static str,
    label: String,
    subject: String,
    started_ms: u64,
    wall: Instant,
    cpu: ThreadTime,
    read: u64,
    written: u64,
    memory: (u64, u64),
}

impl Span for MonitorSpan<'_> {
    fn bytes(&mut self, read: u64, written: u64) {
        self.read += read;
        self.written += written;
    }

    fn memory(&mut self, estimated: u64, actual: u64) {
        self.memory = (estimated, actual);
    }
}

impl Drop for MonitorSpan<'_> {
    fn drop(&mut self) {
        let wall_us = self.wall.elapsed().as_micros() as u64;
        let cpu_us = self.cpu.elapsed().as_micros() as u64;
        self.monitor.write(&json!({
            "kind": "span",
            "t_ms": self.started_ms,
            "stage": self.stage,
            "label": self.label,
            "subject": self.subject,
            "wall_us": wall_us,
            "cpu_us": cpu_us,
            "read": self.read,
            "written": self.written,
        }));
        if let Some(kind) = derivation_kind(self.stage, &self.label) {
            let mut costs = self.monitor.costs.lock().unwrap();
            let cost = costs.entry(self.subject.clone()).or_default();
            if let Some(kind) = kind {
                kind.clone_into(&mut cost.kind);
            }
            cost.wall_us += wall_us;
            cost.cpu_us += cpu_us;
            cost.read += self.read;
            cost.written += self.written;
            cost.estimated_memory = cost.estimated_memory.max(self.memory.0);
            cost.actual_memory = cost.actual_memory.max(self.memory.1);
        }
        let mut totals = self.monitor.totals.lock().unwrap();
        let total = totals
            .entry((self.stage, std::mem::take(&mut self.label)))
            .or_default();
        total.count += 1;
        total.wall_us += wall_us;
        total.cpu_us += cpu_us;
        total.read += self.read;
        total.written += self.written;
    }
}

impl Recorder for Monitor {
    fn span<'a>(&'a self, stage: &'static str, label: &str, subject: &str) -> Box<dyn Span + 'a> {
        Box::new(MonitorSpan {
            monitor: self,
            stage,
            label: label.to_owned(),
            subject: subject.to_owned(),
            started_ms: self.millis(),
            wall: Instant::now(),
            cpu: ThreadTime::now(),
            read: 0,
            written: 0,
            memory: (0, 0),
        })
    }
}
