//! The scheduler: runs jobs on standard library threads within three resources, CPU slots, a
//! memory budget and the connections of each remote.
//!
//! A job declares the memory it holds from start to end and, optionally, one connection to a
//! remote. Inside, it takes CPU slots for each part of its work with [`Slots::cpu`], so a job can
//! convert on one slot and encode on two. Jobs receive their memory and connection in the order
//! given; a job that does not fit waits, and one that needs more than the whole budget runs
//! alone. With one CPU slot every job runs on the calling thread, in order, and no thread is
//! created.
//!
//! Declared needs are estimates. With [`Limits::resident`], a job also waits while the memory the
//! process holds, the needs of the running jobs and its own need together exceed a limit, so
//! memory that estimates miss, or that the allocator keeps after jobs end, holds back new jobs.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

/// The resources a run may use.
#[derive(Debug, Clone)]
pub struct Limits {
    /// CPU slots: the parts of work that run at once.
    pub cpu: usize,
    /// The memory budget of the needs of running jobs, in bytes.
    pub memory: u64,
    /// The connections each remote allows at once, by remote number.
    pub connections: Vec<usize>,
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

/// What a job holds from its start to its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Need {
    /// The estimated peak memory, in bytes.
    pub memory: u64,
    /// The remote the job connects to, by number in [`Limits::connections`].
    pub remote: Option<usize>,
}

#[derive(Debug, Default)]
struct State {
    /// The CPU slots in use.
    cpu: usize,
    /// The memory held by running jobs.
    memory: u64,
    /// The connections in use, by remote.
    connections: Vec<usize>,
    /// The number of the next job to receive its memory.
    turn: usize,
}

/// Runs jobs within [`Limits`].
#[derive(Debug)]
pub struct Scheduler {
    limits: Limits,
    state: Mutex<State>,
    changed: Condvar,
}

/// What a running job takes CPU slots through.
#[derive(Debug)]
pub struct Slots<'s> {
    scheduler: &'s Scheduler,
}

impl Slots<'_> {
    /// Runs `work` on `slots` CPU slots, waiting until they are free. A request for more slots
    /// than there are takes all of them. Work must not take slots again from inside `work`.
    pub fn cpu<R>(&self, slots: usize, work: impl FnOnce() -> R) -> R {
        let scheduler = self.scheduler;
        let slots = slots.clamp(1, scheduler.limits.cpu.max(1));
        if scheduler.limits.cpu <= 1 {
            return work();
        }
        {
            let mut state = scheduler.lock();
            while state.cpu + slots > scheduler.limits.cpu {
                state = scheduler.wait(state);
            }
            state.cpu += slots;
        }
        let release = Release(|| {
            scheduler.lock().cpu -= slots;
            scheduler.changed.notify_all();
        });
        let result = work();
        drop(release);
        result
    }
}

/// Runs a closure when dropped, so resources are returned when a job panics too.
struct Release<F: FnMut()>(F);

impl<F: FnMut()> Drop for Release<F> {
    fn drop(&mut self) {
        (self.0)();
    }
}

impl Scheduler {
    /// A scheduler for `limits`.
    pub fn new(limits: Limits) -> Self {
        let state = State {
            connections: vec![0; limits.connections.len()],
            ..State::default()
        };
        Self {
            limits,
            state: Mutex::new(state),
            changed: Condvar::new(),
        }
    }

    /// The limits of this scheduler.
    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The memory job `need` holds: all of the budget when it needs more than the budget.
    fn held(&self, need: &Need) -> u64 {
        need.memory.min(self.limits.memory)
    }

    /// Whether `need` can start now. A job that needs memory starts when nothing else holds
    /// memory, whatever the limits, so every job gets to run.
    fn fits(&self, state: &State, need: &Need) -> bool {
        let held = self.held(need);
        let memory = state.memory + held <= self.limits.memory;
        let connection = need.remote.is_none_or(|remote| {
            state.connections[remote] < self.limits.connections[remote].max(1)
        });
        // The running jobs count with their whole needs, whether or not they have allocated yet.
        let resident = || match self.limits.resident {
            Some(resident) if held > 0 && state.memory > 0 => {
                (resident.probe)()
                    .saturating_add(state.memory)
                    .saturating_add(held)
                    <= resident.limit
            }
            _ => true,
        };
        memory && connection && resident()
    }

    /// Runs `job(i, slots)` for each `i` in `0..needs.len()` and returns the results in that
    /// order. Jobs start in index order, each once its need fits.
    ///
    /// # Panics
    ///
    /// Panics when a need names a remote that [`Limits::connections`] does not have, or when a
    /// job panics.
    pub fn run<T: Send>(
        &self,
        needs: &[Need],
        job: impl Fn(usize, &Slots<'_>) -> T + Sync,
    ) -> Vec<T> {
        for need in needs {
            if let Some(remote) = need.remote {
                assert!(
                    remote < self.limits.connections.len(),
                    "unknown remote {remote}"
                );
            }
        }
        let slots = Slots { scheduler: self };
        if self.limits.cpu <= 1 {
            return (0..needs.len()).map(|i| job(i, &slots)).collect();
        }
        let next = AtomicUsize::new(0);
        let results: Mutex<Vec<Option<T>>> = Mutex::new((0..needs.len()).map(|_| None).collect());
        std::thread::scope(|s| {
            for _ in 0..self.limits.cpu.min(needs.len()) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(need) = needs.get(i) else {
                            break;
                        };
                        self.acquire(i, need);
                        let release = Release(|| self.release(need));
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

    /// Waits for the turn of job `i` and for its need to fit, then takes it.
    fn acquire(&self, i: usize, need: &Need) {
        let mut state = self.lock();
        while state.turn != i || !self.fits(&state, need) {
            state = self.wait(state);
        }
        state.turn += 1;
        state.memory += self.held(need);
        if let Some(remote) = need.remote {
            state.connections[remote] += 1;
        }
        drop(state);
        self.changed.notify_all();
    }

    fn release(&self, need: &Need) {
        let mut state = self.lock();
        state.memory -= self.held(need);
        if let Some(remote) = need.remote {
            state.connections[remote] -= 1;
        }
        drop(state);
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicU64;
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
            connections: vec![2],
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
                remote: None,
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
                remote: None,
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

    #[test]
    fn connections_of_a_remote_are_limited() {
        let scheduler = Scheduler::new(limits(6, u64::MAX));
        let needs: Vec<Need> = (0..60)
            .map(|i| Need {
                memory: 0,
                remote: (i % 2 == 0).then_some(0),
            })
            .collect();
        let open = Peak::default();
        scheduler.run(&needs, |i, _| {
            if needs[i].remote.is_some() {
                open.add(1);
                pause();
                open.sub(1);
            }
        });
        assert_eq!(open.most(), 2);
    }

    #[test]
    fn one_slot_runs_on_the_calling_thread_in_order() {
        let scheduler = Scheduler::new(limits(1, 10));
        let needs = vec![
            Need {
                memory: 50,
                remote: Some(0),
            };
            5
        ];
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
                connections: vec![1, 3],
                resident: None,
            });
            let needs: Vec<Need> = (0..500u64)
                .map(|i| Need {
                    memory: (i * 37) % 90,
                    remote: match i % 5 {
                        0 => Some(0),
                        1 => Some(1),
                        _ => None,
                    },
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
            connections: Vec::new(),
            resident: Some(Resident {
                limit: 100,
                probe: resident,
            }),
        });
        let needs = vec![
            Need {
                memory: 10,
                remote: None,
            };
            300
        ];
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
        let needs = vec![
            Need {
                memory: 100,
                remote: Some(0),
            };
            2
        ];
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scheduler.run(&needs, |i, slots| {
                slots.cpu(1, || assert_ne!(i, 0, "the first job fails"));
            })
        }));
        assert!(outcome.is_err());
        let state = scheduler.lock();
        assert_eq!((state.cpu, state.memory, state.connections[0]), (0, 0, 0));
    }
}
