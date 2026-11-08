//! The cache of decompressed UnityFS blocks.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

/// Identifies one decompressed block: the content hash of its bundle and the block's index.
///
/// Keying by content hash lets two regions that ship the same bundle share decompressed blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockKey {
    /// The SHA-256 of the bundle bytes.
    pub bundle: [u8; 32],
    /// The index of the block in the bundle's block table.
    pub block: u32,
}

/// A store of decompressed blocks shared by readers.
pub trait BlockCache: Send + Sync {
    /// Returns the block if it is cached.
    fn get(&self, key: &BlockKey) -> Option<Arc<[u8]>>;
    /// Offers a block to the cache, which may keep or discard it.
    fn insert(&self, key: BlockKey, block: Arc<[u8]>);
}

/// A cache that keeps nothing: every read decompresses the blocks it touches.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoBlockCache;

impl BlockCache for NoBlockCache {
    fn get(&self, _key: &BlockKey) -> Option<Arc<[u8]>> {
        None
    }

    fn insert(&self, _key: BlockKey, _block: Arc<[u8]>) {}
}

/// A cache bounded by the total size of the blocks it holds, which evicts the least recently used
/// block first.
#[derive(Debug)]
pub struct BoundedBlockCache {
    budget: usize,
    inner: Mutex<Lru>,
}

#[derive(Debug, Default)]
struct Lru {
    used: usize,
    tick: u64,
    blocks: HashMap<BlockKey, (Arc<[u8]>, u64)>,
    order: BTreeMap<u64, BlockKey>,
}

impl BoundedBlockCache {
    /// Creates a cache that holds at most `budget` bytes of blocks.
    pub fn new(budget: usize) -> Self {
        BoundedBlockCache {
            budget,
            inner: Mutex::new(Lru::default()),
        }
    }

    /// The number of bytes currently held.
    pub fn used(&self) -> usize {
        self.lock().used
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Lru> {
        // A panic while the lock is held leaves the maps consistent, so the cache stays usable.
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl BlockCache for BoundedBlockCache {
    fn get(&self, key: &BlockKey) -> Option<Arc<[u8]>> {
        let mut lru = self.lock();
        lru.tick += 1;
        let tick = lru.tick;
        let (block, last) = lru.blocks.get_mut(key)?;
        let block = block.clone();
        let old = std::mem::replace(last, tick);
        lru.order.remove(&old);
        lru.order.insert(tick, *key);
        Some(block)
    }

    fn insert(&self, key: BlockKey, block: Arc<[u8]>) {
        if block.len() > self.budget {
            return;
        }
        let mut lru = self.lock();
        if lru.blocks.contains_key(&key) {
            return;
        }
        while lru.used + block.len() > self.budget {
            let Some((_, oldest)) = lru.order.pop_first() else {
                break;
            };
            if let Some((evicted, _)) = lru.blocks.remove(&oldest) {
                lru.used -= evicted.len();
            }
        }
        lru.tick += 1;
        let tick = lru.tick;
        lru.used += block.len();
        lru.order.insert(tick, key);
        lru.blocks.insert(key, (block, tick));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(block: u32) -> BlockKey {
        BlockKey {
            bundle: [0; 32],
            block,
        }
    }

    #[test]
    fn evicts_least_recently_used_within_budget() {
        let cache = BoundedBlockCache::new(10);
        cache.insert(key(0), Arc::from(vec![0u8; 4]));
        cache.insert(key(1), Arc::from(vec![0u8; 4]));
        assert!(cache.get(&key(0)).is_some());
        cache.insert(key(2), Arc::from(vec![0u8; 4]));
        assert!(cache.get(&key(1)).is_none());
        assert!(cache.get(&key(0)).is_some());
        assert!(cache.get(&key(2)).is_some());
        assert_eq!(cache.used(), 8);
    }

    #[test]
    fn rejects_blocks_larger_than_the_budget() {
        let cache = BoundedBlockCache::new(3);
        cache.insert(key(0), Arc::from(vec![0u8; 4]));
        assert_eq!(cache.used(), 0);
    }
}
