//! Work shared by the callers that ask for it at the same time.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// Work in progress, by key. Callers that ask for the same key while it is made wait for the one
/// that makes it and share its result; a caller that comes later makes it anew.
#[derive(Debug)]
pub struct Flights<K, V> {
    running: Mutex<HashMap<K, Arc<OnceLock<V>>>>,
}

impl<K, V> Default for Flights<K, V> {
    fn default() -> Self {
        Self {
            running: Mutex::default(),
        }
    }
}

impl<K: Hash + Eq + Clone, V: Clone> Flights<K, V> {
    /// The result of `make` for `key`, from this call or from one that runs at the same time.
    /// When the caller that makes it panics, another makes it.
    pub fn run(&self, key: &K, make: impl FnOnce() -> V) -> V {
        let flight = self.running().entry(key.clone()).or_default().clone();
        let value = flight.get_or_init(make).clone();
        // The first caller to return ends the flight.
        let mut running = self.running();
        if running
            .get(key)
            .is_some_and(|current| Arc::ptr_eq(current, &flight))
        {
            running.remove(key);
        }
        value
    }

    fn running(&self) -> std::sync::MutexGuard<'_, HashMap<K, Arc<OnceLock<V>>>> {
        // The map holds no partial state when a caller panics.
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Barrier;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn callers_at_the_same_time_share_one_result() {
        let flights = Flights::default();
        let made = AtomicUsize::new(0);
        let together = Barrier::new(8);
        let values: Vec<usize> = std::thread::scope(|s| {
            let callers: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        together.wait();
                        flights.run(&"key", || {
                            // Every caller has asked before the first result is ready.
                            std::thread::sleep(std::time::Duration::from_millis(50));
                            made.fetch_add(1, Ordering::SeqCst) + 1
                        })
                    })
                })
                .collect();
            callers.into_iter().map(|c| c.join().unwrap()).collect()
        });
        assert_eq!(values, [1; 8]);
        assert_eq!(made.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_later_caller_makes_the_result_anew() {
        let flights = Flights::default();
        assert_eq!(flights.run(&1, || "first"), "first");
        assert_eq!(flights.run(&1, || "second"), "second");
        assert_eq!(flights.run(&2, || "other"), "other");
    }
}
