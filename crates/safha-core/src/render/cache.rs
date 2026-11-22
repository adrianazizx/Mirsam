//! Byte-capped least-recently-used cache.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;

/// Something whose memory footprint the cache can account for.
pub trait Weighed {
    fn weight(&self) -> usize;
}

struct Entry<V> {
    value: V,
    weight: usize,
    /// Position in `order`; larger is more recently used.
    tick: u64,
}

/// LRU cache bounded by the summed [`Weighed::weight`] of its values.
///
/// Lookups and insertions are O(log n): recency is a monotonically increasing
/// tick kept in an ordered map, so the least recently used entry is always the
/// first key of `order`.
pub struct LruCache<K, V> {
    capacity: usize,
    used: usize,
    tick: u64,
    entries: HashMap<K, Entry<V>>,
    order: BTreeMap<u64, K>,
    evictions: u64,
}

impl<K: Copy + Eq + Hash, V: Clone + Weighed> LruCache<K, V> {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, used: 0, tick: 0, entries: HashMap::new(), order: BTreeMap::new(), evictions: 0 }
    }

    /// Returns a clone of the value and marks it most recently used.
    pub fn get(&mut self, key: &K) -> Option<V> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.entries.get_mut(key)?;
        self.order.remove(&entry.tick);
        entry.tick = tick;
        self.order.insert(tick, *key);
        Some(entry.value.clone())
    }

    #[cfg(test)]
    pub fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    /// Inserts or replaces a value, then evicts least recently used entries
    /// until the cache fits its capacity. A value larger than the whole
    /// capacity is not stored. Returns the number of entries evicted.
    pub fn insert(&mut self, key: K, value: V) -> usize {
        self.remove(&key);
        let weight = value.weight();
        if weight > self.capacity {
            return 0;
        }
        let mut evicted = 0;
        while self.used + weight > self.capacity {
            let Some((_, oldest)) = self.order.pop_first() else { break };
            if let Some(entry) = self.entries.remove(&oldest) {
                self.used -= entry.weight;
                evicted += 1;
            }
        }
        self.evictions += evicted as u64;
        self.tick += 1;
        self.order.insert(self.tick, key);
        self.entries.insert(key, Entry { value, weight, tick: self.tick });
        self.used += weight;
        evicted
    }

    pub fn remove(&mut self, key: &K) -> Option<V> {
        let entry = self.entries.remove(key)?;
        self.order.remove(&entry.tick);
        self.used -= entry.weight;
        Some(entry.value)
    }

    /// Removes every entry whose key matches `predicate`.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let doomed: Vec<K> = self.entries.keys().filter(|key| !keep(key)).copied().collect();
        for key in doomed {
            self.remove(&key);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Summed weight of all entries.
    pub fn used(&self) -> usize {
        self.used
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Entries evicted to make room since the cache was created.
    pub fn evictions(&self) -> u64 {
        self.evictions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Debug, PartialEq)]
    struct Blob(usize);
    impl Weighed for Blob {
        fn weight(&self) -> usize {
            self.0
        }
    }

    #[test]
    fn evicts_least_recently_used_first() {
        let mut cache = LruCache::new(30);
        cache.insert('a', Blob(10));
        cache.insert('b', Blob(10));
        cache.insert('c', Blob(10));
        assert_eq!(cache.used(), 30);
        // Touching `a` makes `b` the oldest.
        assert_eq!(cache.get(&'a'), Some(Blob(10)));
        assert_eq!(cache.insert('d', Blob(10)), 1);
        assert!(!cache.contains(&'b'));
        assert!(cache.contains(&'a') && cache.contains(&'c') && cache.contains(&'d'));
        assert_eq!(cache.evictions(), 1);
    }

    #[test]
    fn evicts_as_many_entries_as_a_large_value_needs() {
        let mut cache = LruCache::new(30);
        for key in 0..3 {
            cache.insert(key, Blob(10));
        }
        assert_eq!(cache.insert(9, Blob(25)), 3);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.used(), 25);
    }

    #[test]
    fn never_stores_a_value_larger_than_the_capacity() {
        let mut cache = LruCache::new(30);
        cache.insert(1, Blob(10));
        assert_eq!(cache.insert(2, Blob(31)), 0);
        assert!(!cache.contains(&2));
        // The existing entry survives.
        assert!(cache.contains(&1));
    }

    #[test]
    fn replacing_a_key_reaccounts_its_weight() {
        let mut cache = LruCache::new(30);
        cache.insert(1, Blob(10));
        cache.insert(1, Blob(20));
        assert_eq!((cache.len(), cache.used()), (1, 20));
        cache.retain(|key| *key != 1);
        assert_eq!((cache.len(), cache.used()), (0, 0));
    }
}
