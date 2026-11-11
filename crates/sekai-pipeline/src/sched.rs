//! The scheduler: runs jobs on standard library threads within two resources, CPU slots and a
//! memory budget.
//!
//! A job declares the memory it holds from start to end. Inside, it takes CPU slots for each part
//! of its work with [`Slots::cpu`] or [`Slots::hold`], so a job can convert on one slot and encode
//! on two. Jobs start in the order given while they fit. A job that does not fit waits, and lets at
//! most [`BACKFILL`] jobs that do fit, from the [`BACKFILL`] after it, start ahead of it; one that
//! needs more than the whole budget runs alone. With one CPU slot every job runs on the calling
//! thread, in order, and no thread is created.
//!
//! Several runs may share one scheduler at once, each from its own thread. Each keeps its own
//! order, and all of them draw on the same CPU slots and memory budget.
//!
//! Declared needs are estimates. With [`Limits::resident`], a job also waits while the memory the
//! process holds, the needs of the running jobs and its own need together exceed a limit, so
//! memory that estimates miss, or that the allocator keeps after jobs end, holds back new jobs.

use std::cell::OnceCell;
use std::collections::VecDeque;
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// How far a job that does not fit is passed over: later jobs start ahead of it from this many
/// places behind it, this many times. Then the jobs that run finish and it starts.
pub const BACKFILL: usize = 32;

/// The resources a run may use.
#[derive(Debug, Clone)]
pub struct Limits {
    /// CPU slots: the parts of work that run at once.
    pub cpu: usize,
    /// The memory budget of the needs of running jobs, in bytes.
    pub memory: u64,
    /// A limit on the memory of the whole process, if any.
    pub resident: Option<Resident>,
}

/// A limit on the memory of the whole process.
#[derive(Debug, Clone, Copy)]
pub struct Resident {
    /// The most memory the process may hold, in bytes.
    pub limit: u64,
    /// The memory the process holds now, in bytes.
    pub probe: fn() -> u64,
}

impl Limits {
    /// `cpu` CPU slots, with no memory budget.
    pub fn threads(cpu: usize) -> Self {
        Self {
            cpu,
            memory: u64::MAX,
            resident: None,
        }
    }
}

/// What a job holds from its start to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Need {
    /// The estimated peak memory, in bytes.
    pub memory: u64,
}

#[derive(Debug, Default)]
struct State {
    /// The CPU slots in use.
    cpu: usize,
    /// The memory held by running jobs.
    memory: u64,
}

/// The jobs of one run that have not started.
struct Queue {
    /// The jobs, in order.
    waiting: VecDeque<usize>,
    /// The jobs that started ahead of the first waiting job.
    overtaken: usize,
}

/// Runs jobs within [`Limits`].
#[derive(Debug)]
pub struct Scheduler {
    limits: Limits,
    state: Mutex<State>,
    /// Signalled when a job ends or the first waiting job starts: another job may start.
    changed: Condvar,
    /// Signalled when CPU slots are returned.
    freed: Condvar,
}

/// What a running job takes CPU slots through.
#[derive(Debug)]
pub struct Slots<'s> {
    scheduler: &'s Scheduler,
}

/// CPU slots, returned when dropped.
#[derive(Debug)]
#[must_use = "the slots are returned when this is dropped"]
pub struct Held<'s> {
    scheduler: &'s Scheduler,
    slots: usize,
}

impl Drop for Held<'_> {
    fn drop(&mut self) {
        if self.slots > 0 {
            self.scheduler.lock().cpu -= self.slots;
            self.scheduler.freed.notify_all();
        }
    }
}

impl<'s> Slots<'s> {
    /// Takes `slots` CPU slots, waiting until they are free. A request for more slots than there
    /// are takes all of them. Slots must not be taken again while these are held.
    pub fn hold(&self, slots: usize) -> Held<'s> {
        let scheduler = self.scheduler;
        // With one slot, jobs run one after another and there is nothing to wait for.
        if scheduler.limits.cpu <= 1 {
            return Held {
                scheduler,
                slots: 0,
            };
        }
        let slots = slots.clamp(1, scheduler.limits.cpu);
        let mut state = scheduler.lock();
        while state.cpu + slots > scheduler.limits.cpu {
            state = wait(&scheduler.freed, state);
        }
        state.cpu += slots;
        Held { scheduler, slots }
    }

    /// Runs `work` on `slots` CPU slots, taken as [`Slots::hold`] takes them.
    pub fn cpu<R>(&self, slots: usize, work: impl FnOnce() -> R) -> R {
        let _held = self.hold(slots);
        work()
    }
}

/// Runs a closure when dropped, so resources are returned when a job panics too.
struct Release<F: FnMut()>(F);

impl<F: FnMut()> Drop for Release<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

fn wait<'a>(condition: &Condvar, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
    condition
        .wait(state)
        .unwrap_or_else(PoisonError::into_inner)
}

impl Scheduler {
    /// A scheduler for `limits`.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            state: Mutex::default(),
            changed: Condvar::new(),
            freed: Condvar::new(),
        }
    }

    /// The limits of this scheduler.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The memory job `need` holds: all of the budget when it needs more than the budget.
    fn held(&self, need: &Need) -> u64 {
        need.memory.min(self.limits.memory)
    }

    /// Whether `need` can start now. The memory of the process is read at most once into
    /// `probed`. A job that needs memory starts when nothing else holds memory, whatever the
    /// limits, so every job gets to run.
    fn fits(&self, state: &State, need: &Need, probed: &OnceCell<u64>) -> bool {
        let held = self.held(need);
        let memory = state.memory + held <= self.limits.memory;
        // The running jobs count with their whole needs, whether or not they have allocated yet.
        let resident = || match self.limits.resident {
            Some(resident) if held > 0 && state.memory > 0 => {
                probed
                    .get_or_init(resident.probe)
                    .saturating_add(state.memory)
                    .saturating_add(held)
                    <= resident.limit
            }
            _ => true,
        };
        memory && resident()
    }

    /// Runs `job(i, slots)` for each `i` in `0..needs.len()` and returns the results in that
    /// order. Jobs start in index order, except that jobs start ahead of one that does not fit,
    /// as far as [`BACKFILL`] allows.
    ///
    /// # Panics
    ///
    /// Panics when a job panics.
    pub fn run<T: Send>(
        &self,
        needs: &[Need],
        job: impl Fn(usize, &Slots<'_>) -> T + Sync,
    ) -> Vec<T> {
        let slots = Slots { scheduler: self };
        if self.limits.cpu <= 1 {
            return (0..needs.len()).map(|i| job(i, &slots)).collect();
        }
        let queue = Mutex::new(Queue {
            waiting: (0..needs.len()).collect(),
            overtaken: 0,
        });
        let results: Mutex<Vec<Option<T>>> = Mutex::new((0..needs.len()).map(|_| None).collect());
        std::thread::scope(|s| {
            for _ in 0..self.limits.cpu.min(needs.len()) {
                s.spawn(|| {
                    while let Some(i) = self.start(&queue, needs) {
                        let release = Release(|| self.release(&needs[i]));
                        let result = job(i, &slots);
                        drop(release);
                        results.lock().unwrap_or_else(PoisonError::into_inner)[i] = Some(result);
                    }
                });
            }
        });
        results
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .map(|r| r.expect("every job ran"))
            .collect()
    }

    /// Waits until a job of `queue` can start, takes what it needs and returns its number; `None`
    /// once every job of the queue has started. The queue is locked before the state, and kept
    /// while its run waits, so the workers of one run wait on it one at a time.
    fn start(&self, queue: &Mutex<Queue>, needs: &[Need]) -> Option<usize> {
        let mut queue = queue.lock().unwrap_or_else(PoisonError::into_inner);
        let mut state = self.lock();
        loop {
            queue.waiting.front()?;
            if let Some(position) = self.startable(&state, &queue, needs) {
                let i = queue.waiting.remove(position)?;
                let need = &needs[i];
                queue.overtaken = if position == 0 {
                    0
                } else {
                    queue.overtaken + 1
                };
                state.memory += self.held(need);
                drop(state);
                // The next first job may be passed over anew, and without one the run ends.
                if position == 0 {
                    self.changed.notify_all();
                }
                return Some(i);
            }
            state = wait(&self.changed, state);
        }
    }

    /// The position among the waiting jobs of `queue` of the one to start now: the first that
    /// fits, no further behind the first than it may still be passed over.
    fn startable(&self, state: &State, queue: &Queue, needs: &[Need]) -> Option<usize> {
        let reach = if queue.overtaken < BACKFILL {
            1 + BACKFILL
        } else {
            1
        };
        let probed = OnceCell::new();
        queue
            .waiting
            .iter()
            .take(reach)
            .position(|&i| self.fits(state, &needs[i], &probed))
    }

    fn release(&self, need: &Need) {
        let mut state = self.lock();
        state.memory -= self.held(need);
        drop(state);
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::thread::ThreadId;
    use std::time::Duration;

    use super::*;

    /// The most a quantity reached while jobs ran.
    #[derive(Default)]
    struct Peak {
        now: AtomicU64,
        most: AtomicU64,
    }

    impl Peak {
        fn add(&self, n: u64) {
            let now = self.now.fetch_add(n, Ordering::SeqCst) + n;
            self.most.fetch_max(now, Ordering::SeqCst);
        }

        fn sub(&self, n: u64) {
            self.now.fetch_sub(n, Ordering::SeqCst);
        }

        fn most(&self) -> u64 {
            self.most.load(Ordering::SeqCst)
        }
    }

    fn limits(cpu: usize, memory: u64) -> Limits {
        Limits {
            cpu,
            memory,
            resident: None,
        }
    }

    fn pause() {
        std::thread::sleep(Duration::from_millis(2));
    }

    #[test]
    fn memory_and_cpu_stay_within_the_limits() {
        let scheduler = Scheduler::new(limits(4, 100));
        let needs: Vec<Need> = (0..200)
            .map(|i| Need {
                memory: [10, 30, 60, 5][i % 4],
            })
            .collect();
        let (memory, cpu) = (Peak::default(), Peak::default());
        let done = scheduler.run(&needs, |i, slots| {
            memory.add(needs[i].memory);
            slots.cpu(1, || {
                cpu.add(1);
                pause();
                cpu.sub(1);
            });
            slots.cpu(if i % 3 == 0 { 2 } else { 1 }, || {
                cpu.add(if i % 3 == 0 { 2 } else { 1 });
                pause();
                cpu.sub(if i % 3 == 0 { 2 } else { 1 });
            });
            memory.sub(needs[i].memory);
            i
        });
        assert_eq!(done, (0..200).collect::<Vec<_>>());
        assert!(memory.most() <= 100, "memory {}", memory.most());
        assert!(cpu.most() <= 4, "cpu {}", cpu.most());
        assert!(memory.most() > 60, "jobs ran side by side");
    }

    #[test]
    fn a_job_larger_than_the_budget_runs_alone() {
        let scheduler = Scheduler::new(limits(4, 100));
        let needs: Vec<Need> = (0..40)
            .map(|i| Need {
                memory: if i % 10 == 5 { 500 } else { 20 },
            })
            .collect();
        let running = Peak::default();
        let alone = AtomicUsize::new(0);
        scheduler.run(&needs, |i, slots| {
            running.add(1);
            slots.cpu(1, pause);
            if needs[i].memory > 100 && running.now.load(Ordering::SeqCst) == 1 {
                alone.fetch_add(1, Ordering::SeqCst);
            }
            pause();
            running.sub(1);
        });
        assert_eq!(alone.load(Ordering::SeqCst), 4);
    }

    fn memory_needs(memory: impl IntoIterator<Item = u64>) -> Vec<Need> {
        memory.into_iter().map(|memory| Need { memory }).collect()
    }

    #[test]
    fn a_job_that_fits_starts_ahead_of_one_that_does_not() {
        let scheduler = Scheduler::new(limits(4, 100));
        let needs = memory_needs([80, 80, 10]);
        let started = Mutex::new(Vec::new());
        let small_started = || started.lock().unwrap().contains(&2);
        scheduler.run(&needs, |i, _| {
            started.lock().unwrap().push(i);
            // The first job ends only after the last has started, which the second must not
            // hold back.
            while i == 0 && !small_started() {
                pause();
            }
        });
        assert_eq!(started.into_inner().unwrap(), [0, 2, 1]);
    }

    #[test]
    fn a_job_is_passed_over_a_bounded_number_of_times() {
        let scheduler = Scheduler::new(limits(2, 100));
        // The second job needs the whole budget, which is free only when no small job runs.
        let needs = memory_needs([10, 100].into_iter().chain([10; 400]));
        let started = Mutex::new(Vec::new());
        scheduler.run(&needs, |i, _| {
            started.lock().unwrap().push(i);
            pause();
        });
        let started = started.into_inner().unwrap();
        let before = started.iter().position(|&i| i == 1).unwrap();
        assert!(before <= 1 + BACKFILL, "{before} jobs started first");
    }

    #[test]
    fn one_slot_runs_on_the_calling_thread_in_order() {
        let scheduler = Scheduler::new(limits(1, 10));
        let needs = vec![Need { memory: 50 }; 5];
        let caller = std::thread::current().id();
        let threads: Vec<(usize, ThreadId)> = scheduler.run(&needs, |i, slots| {
            slots.cpu(2, || (i, std::thread::current().id()))
        });
        assert_eq!(threads, (0..5).map(|i| (i, caller)).collect::<Vec<_>>());
    }

    #[test]
    fn many_mixed_jobs_finish() {
        for cpu in [2, 3, 8] {
            let scheduler = Scheduler::new(Limits {
                cpu,
                memory: 64,
                resident: None,
            });
            let needs: Vec<Need> = (0..500u64)
                .map(|i| Need {
                    memory: (i * 37) % 90,
                })
                .collect();
            let sum: u64 = scheduler
                .run(&needs, |i, slots| {
                    slots.cpu(i % 3 + 1, || std::hint::black_box(i as u64))
                })
                .into_iter()
                .sum();
            assert_eq!(sum, 499 * 500 / 2);
        }
    }

    /// The memory of a pretend process: what running jobs allocated and what ended jobs left.
    static ALLOCATED: AtomicU64 = AtomicU64::new(0);
    static LEFT: AtomicU64 = AtomicU64::new(0);

    fn resident() -> u64 {
        ALLOCATED.load(Ordering::SeqCst) + LEFT.load(Ordering::SeqCst)
    }

    #[test]
    fn the_memory_of_the_process_stays_within_its_limit() {
        // The budget of needs alone would let eight jobs run at once, while every ended job
        // leaves memory behind that no need accounts for.
        let scheduler = Scheduler::new(Limits {
            cpu: 8,
            memory: 1000,
            resident: Some(Resident {
                limit: 100,
                probe: resident,
            }),
        });
        let needs = vec![Need { memory: 10 }; 300];
        let (peak, together) = (Peak::default(), Peak::default());
        scheduler.run(&needs, |_, slots| {
            together.add(1);
            ALLOCATED.fetch_add(10, Ordering::SeqCst);
            peak.most.fetch_max(resident(), Ordering::SeqCst);
            slots.cpu(1, pause);
            ALLOCATED.fetch_sub(10, Ordering::SeqCst);
            if LEFT.load(Ordering::SeqCst) < 60 {
                LEFT.fetch_add(1, Ordering::SeqCst);
            }
            together.sub(1);
        });
        assert!(peak.most() <= 100, "resident {}", peak.most());
        assert!(together.most() > 1, "jobs ran side by side");
    }

    #[test]
    fn resources_are_returned_when_a_job_panics() {
        let scheduler = Scheduler::new(limits(2, 100));
        let needs = vec![Need { memory: 100 }; 2];
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scheduler.run(&needs, |i, slots| {
                slots.cpu(1, || assert_ne!(i, 0, "the first job fails"));
            })
        }));
        assert!(outcome.is_err());
        let state = scheduler.lock();
        assert_eq!((state.cpu, state.memory), (0, 0));
    }
}
