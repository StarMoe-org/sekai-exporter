//! A cache that keeps what was used most recently, within a budget.

use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Values by key, each with a weight. The weights never add up to more than the budget: a value
/// that does not fit drops the values used longest ago until it does, and a value heavier than
/// the whole budget is not kept.
#[derive(Debug)]
pub(crate) struct Lru<K, V> {
    budget: usize,
    weight: usize,
    tick: u64,
    /// The tick of the last use of each key.
    ticks: HashMap<K, u64>,
    /// The entries by the tick of their last use, so the first is the one used longest ago.
    entries: BTreeMap<u64, Entry<K, V>>,
}

#[derive(Debug)]
struct Entry<K, V> {
    key: K,
    value: V,
    weight: usize,
}

impl<K: Hash + Eq + Clone, V> Lru<K, V> {
    /// An empty cache that holds values of up to `budget` in weight.
    pub(crate) fn new(budget: usize) -> Self {
        Self {
            budget,
            weight: 0,
            tick: 0,
            ticks: HashMap::new(),
            entries: BTreeMap::new(),
        }
    }

    /// The value of `key`, which becomes the most recently used.
    pub(crate) fn get<Q>(&mut self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        let used = self.ticks.get_mut(key)?;
        let entry = self.entries.remove(used)?;
        self.tick += 1;
        *used = self.tick;
        Some(&self.entries.entry(self.tick).or_insert(entry).value)
    }

    /// Keeps `value` under `key` as the most recently used, in place of what the key held.
    pub(crate) fn insert(&mut self, key: K, value: V, weight: usize) {
        self.remove(&key);
        if weight > self.budget {
            return;
        }
        // The kept weight is never above the budget, so the room left is their difference.
        while weight > self.budget - self.weight {
            let Some((_, oldest)) = self.entries.pop_first() else {
                break;
            };
            self.ticks.remove(&oldest.key);
            self.weight -= oldest.weight;
        }
        self.tick += 1;
        self.weight += weight;
        self.ticks.insert(key.clone(), self.tick);
        self.entries.insert(self.tick, Entry { key, value, weight });
    }

    /// Drops the value of `key`.
    pub(crate) fn remove<Q>(&mut self, key: &Q)
    where
        K: Borrow<Q>,
        Q: Hash + Eq + ?Sized,
    {
        if let Some(entry) = self
            .ticks
            .remove(key)
            .and_then(|used| self.entries.remove(&used))
        {
            self.weight -= entry.weight;
        }
    }

    /// Drops every value.
    pub(crate) fn clear(&mut self) {
        self.ticks.clear();
        self.entries.clear();
        self.weight = 0;
    }

    /// How many values are kept.
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    /// The weight of the values that are kept.
    pub(crate) fn weight(&self) -> usize {
        self.weight
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(cache: &Lru<&'static str, u32>) -> Vec<&'static str> {
        cache.entries.values().map(|entry| entry.key).collect()
    }

    #[test]
    fn the_value_used_longest_ago_is_dropped_first() {
        let mut cache = Lru::new(3);
        for (key, value) in [("a", 1), ("b", 2), ("c", 3)] {
            cache.insert(key, value, 1);
        }
        assert_eq!(cache.get("a"), Some(&1));
        cache.insert("d", 4, 1);
        assert_eq!(keys(&cache), ["c", "a", "d"]);
        assert_eq!(cache.get("b"), None);
        assert_eq!((cache.len(), cache.weight()), (3, 3));
    }

    #[test]
    fn the_weights_stay_within_the_budget() {
        let mut cache = Lru::new(10);
        cache.insert("a", 1, 4);
        cache.insert("b", 2, 4);
        cache.insert("c", 3, 6);
        assert_eq!(keys(&cache), ["b", "c"]);
        assert_eq!(cache.weight(), 10);
        // A heavier value makes room for itself, and one above the budget is not kept at all.
        cache.insert("d", 4, 10);
        assert_eq!(keys(&cache), ["d"]);
        cache.insert("e", 5, 11);
        assert_eq!(keys(&cache), ["d"]);
        assert_eq!(cache.weight(), 10);
    }

    #[test]
    fn a_key_holds_one_value() {
        let mut cache = Lru::new(4);
        cache.insert("a", 1, 3);
        cache.insert("a", 2, 1);
        assert_eq!(cache.get("a").copied(), Some(2));
        assert_eq!((cache.len(), cache.weight()), (1, 1));
        // A value that no longer fits takes the place of the old one all the same.
        cache.insert("a", 3, 5);
        assert_eq!(cache.get("a"), None);
        assert_eq!((cache.len(), cache.weight()), (0, 0));
        cache.insert("b", 4, 2);
        cache.remove("b");
        cache.remove("b");
        assert_eq!((cache.len(), cache.weight()), (0, 0));
    }

    #[test]
    fn a_cleared_cache_is_empty_and_keeps_its_budget() {
        let mut cache = Lru::new(2);
        cache.insert("a", 1, 1);
        cache.insert("b", 2, 1);
        cache.clear();
        assert_eq!((cache.len(), cache.weight()), (0, 0));
        assert_eq!(cache.get("a"), None);
        for (key, value) in [("c", 3), ("d", 4), ("e", 5)] {
            cache.insert(key, value, 1);
        }
        assert_eq!(keys(&cache), ["d", "e"]);
    }

    #[test]
    fn the_largest_budget_does_not_overflow() {
        let mut cache = Lru::new(usize::MAX);
        cache.insert("a", 1, usize::MAX - 1);
        cache.insert("b", 2, 1);
        assert_eq!(keys(&cache), ["a", "b"]);
        cache.insert("c", 3, usize::MAX);
        assert_eq!(keys(&cache), ["c"]);
        assert_eq!(cache.weight(), usize::MAX);
    }
}
