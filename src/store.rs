use crossbeam::utils::CachePadded;
use std::collections::HashMap;

// Typical per-core cache sizes. Pick one when constructing a Store.
pub const L1_BYTES: usize = 32 * 1024;
pub const L2_BYTES: usize = 256 * 1024;
pub const L3_BYTES: usize = 8 * 1024 * 1024;

pub enum CacheLevel {
    L1,
    L2,
    L3,
}

impl CacheLevel {
    pub fn bytes(&self) -> usize {
        match self {
            CacheLevel::L1 => L1_BYTES,
            CacheLevel::L2 => L2_BYTES,
            CacheLevel::L3 => L3_BYTES,
        }
    }
}

// Hot accounting fields are cache-line padded so they don't share a line with
// adjacent workers' stores if multiple Store instances are laid out in an array.
struct Inner {
    map: HashMap<Vec<u8>, Vec<u8>>,
    // Tracks raw data bytes only (key.len() + value.len()).
    // Actual allocator usage is higher: ~50 bytes of HashMap entry overhead
    // plus 24-byte Vec headers per key and value are not counted here.
    bytes_used: usize,
    capacity: usize,
}

pub struct Store(CachePadded<Inner>);

impl Store {
    pub fn new(capacity: usize) -> Self {
        Store(CachePadded::new(Inner {
            map: HashMap::new(),
            bytes_used: 0,
            capacity,
        }))
    }

    pub fn for_level(level: CacheLevel) -> Self {
        Self::new(level.bytes())
    }

    pub fn get(&self, key: &[u8]) -> Option<&[u8]> {
        self.0.map.get(key).map(Vec::as_slice)
    }

    /// Returns `Err` if inserting would exceed the capacity budget.
    pub fn set(&mut self, key: Vec<u8>, value: Vec<u8>) -> Result<(), &'static str> {
        let new_entry = key.len() + value.len();
        let evicted = self.0.map.get(&key).map_or(0, |v| key.len() + v.len());
        let projected = self.0.bytes_used - evicted + new_entry;

        if projected > self.0.capacity {
            return Err("store capacity exceeded");
        }

        self.0.bytes_used = projected;
        self.0.map.insert(key, value);
        Ok(())
    }

    /// Returns `true` if the key existed and was removed.
    pub fn del(&mut self, key: &[u8]) -> bool {
        match self.0.map.remove(key) {
            Some(v) => {
                self.0.bytes_used -= key.len() + v.len();
                true
            }
            None => false,
        }
    }

    pub fn bytes_used(&self) -> usize {
        self.0.bytes_used
    }

    pub fn capacity(&self) -> usize {
        self.0.capacity
    }

    pub fn len(&self) -> usize {
        self.0.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.map.is_empty()
    }

    pub fn flush(&mut self) {
        self.0.map.clear();
        self.0.bytes_used = 0;
    }

    pub fn keys(&self) -> impl Iterator<Item = &[u8]> {
        self.0.map.keys().map(Vec::as_slice)
    }
}
