//! Skeleton for hash-backed map storage with open addressing and tombstones.
//! Not yet wired into Value maps; serves as a scaffold for future integration.
#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused_imports)]

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum BucketState {
    Empty,
    Tombstone,
    Occupied,
}

#[derive(Debug)]
pub struct Bucket<K, V> {
    pub hash: u64,
    pub key: Option<K>,
    pub value: Option<V>,
    pub state: BucketState,
}

impl<K, V> Default for Bucket<K, V> {
    fn default() -> Self {
        Self { hash: 0, key: None, value: None, state: BucketState::Empty }
    }
}

pub struct HashTable<K, V> {
    pub buckets: Vec<Bucket<K, V>>,
    pub len: usize,
    pub tombstones: usize,
}

impl<K: Eq, V> HashTable<K, V> {
    pub fn with_capacity(cap: usize) -> Self {
        let size = cap.next_power_of_two().max(8);
        Self {
            buckets: std::iter::repeat_with(Bucket::default).take(size).collect(),
            len: 0,
            tombstones: 0,
        }
    }

    /// Placeholder insert; real version will hash keys and perform robin-hood probing.
    pub fn insert_placeholder(&mut self, hash: u64, key: K, value: V) {
        let idx = (hash as usize) & (self.buckets.len() - 1);
        let bucket = &mut self.buckets[idx];
        bucket.hash = hash;
        bucket.key = Some(key);
        bucket.value = Some(value);
        bucket.state = BucketState::Occupied;
        self.len += 1;
    }
}
