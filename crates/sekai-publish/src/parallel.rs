//! The same work on many items at once.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The results of `work` on each of `items`, in item order, computed on up to `threads` threads.
/// The calling thread is one of them, so one thread starts none.
pub(crate) fn map<T: Sync, R: Send>(
    items: &[T],
    threads: usize,
    work: impl Fn(&T) -> R + Sync,
) -> Vec<R> {
    let helpers = threads.min(items.len()).saturating_sub(1);
    if helpers == 0 {
        return items.iter().map(work).collect();
    }
    let next = AtomicUsize::new(0);
    // Each thread takes the next item, so the results of the threads interleave; the index of
    // each result restores the order.
    let take = || {
        let mut done = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = items.get(index) else {
                return done;
            };
            done.push((index, work(item)));
        }
    };
    let mut results: Vec<(usize, R)> = std::thread::scope(|scope| {
        let started: Vec<_> = (0..helpers).map(|_| scope.spawn(take)).collect();
        let mut results = take();
        for thread in started {
            results.extend(thread.join().expect("publishing work does not panic"));
        }
        results
    });
    results.sort_unstable_by_key(|(index, _)| *index);
    results.into_iter().map(|(_, result)| result).collect()
}

#[cfg(test)]
mod tests {
    use super::map;

    #[test]
    fn results_keep_the_order_of_the_items() {
        let items: Vec<u32> = (0..1000).collect();
        for threads in [1, 3, 16] {
            assert_eq!(
                map(&items, threads, |n| n * 2),
                items.iter().map(|n| n * 2).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn no_items_give_no_results() {
        assert!(map(&[] as &[u8], 4, |&b| b).is_empty());
    }
}
