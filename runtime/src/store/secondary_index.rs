//! Secondary Index — Hash and Ordered (BTree) indexes on arbitrary fields.
//!
//! # Index Types
//! - **Hash**: O(1) equality lookups on a single field (HashMap<StoredValue, Vec<u64>>).
//! - **Ordered**: O(log n) range queries on an ordered field (BTreeMap<StoredValue, Vec<u64>>).
//!
//! Indexes are maintained in-memory and rebuilt on store open.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

use super::binary::StoredValue;

/// The kind of secondary index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryIndexKind {
    Hash,
    Ordered,
}

/// A secondary index on a named field.
#[derive(Debug)]
pub struct SecondaryIndex {
    /// Name of the field being indexed.
    pub field_name: String,
    /// Kind of index.
    pub kind: SecondaryIndexKind,
    /// Hash index (only populated when kind == Hash).
    hash: HashMap<StoredValue, Vec<u64>>,
    /// Ordered index (only populated when kind == Ordered).
    ordered: BTreeMap<StoredValue, Vec<u64>>,
    /// Set of all indexed object ids (for efficient remove).
    entries: HashSet<u64>,
}

impl SecondaryIndex {
    /// Create a new empty secondary index on `field_name`.
    pub fn new(field_name: &str, kind: SecondaryIndexKind) -> Self {
        Self {
            field_name: field_name.to_string(),
            kind,
            hash: HashMap::new(),
            ordered: BTreeMap::new(),
            entries: HashSet::new(),
        }
    }

    /// Insert an object into this index.
    pub fn insert(&mut self, index: u64, value: &StoredValue) {
        self.entries.insert(index);
        match self.kind {
            SecondaryIndexKind::Hash => {
                self.hash.entry(value.clone()).or_default().push(index);
            }
            SecondaryIndexKind::Ordered => {
                self.ordered.entry(value.clone()).or_default().push(index);
            }
        }
    }

    /// Remove an object from this index (scans all entries for the object id).
    pub fn remove(&mut self, index: u64, value: &StoredValue) {
        if !self.entries.remove(&index) {
            return;
        }
        match self.kind {
            SecondaryIndexKind::Hash => {
                if let Some(ids) = self.hash.get_mut(value) {
                    ids.retain(|&id| id != index);
                    if ids.is_empty() {
                        self.hash.remove(value);
                    }
                }
            }
            SecondaryIndexKind::Ordered => {
                if let Some(ids) = self.ordered.get_mut(value) {
                    ids.retain(|&id| id != index);
                    if ids.is_empty() {
                        self.ordered.remove(value);
                    }
                }
            }
        }
    }

    /// Update: remove old value, insert new value.
    pub fn update(&mut self, index: u64, old_value: &StoredValue, new_value: &StoredValue) {
        self.remove(index, old_value);
        self.insert(index, new_value);
    }

    /// Equality lookup — works for both Hash and Ordered indexes.
    pub fn find_eq(&self, value: &StoredValue) -> Vec<u64> {
        match self.kind {
            SecondaryIndexKind::Hash => {
                self.hash.get(value).cloned().unwrap_or_default()
            }
            SecondaryIndexKind::Ordered => {
                self.ordered.get(value).cloned().unwrap_or_default()
            }
        }
    }

    /// Range query (inclusive bounds) — only meaningful for Ordered indexes.
    /// Returns all object ids whose indexed value is in [min, max].
    pub fn find_range(&self, min: &StoredValue, max: &StoredValue) -> Vec<u64> {
        if self.kind != SecondaryIndexKind::Ordered {
            return vec![];
        }
        let mut result = Vec::new();
        for (_key, ids) in self.ordered.range(min.clone()..=max.clone()) {
            result.extend(ids);
        }
        result
    }

    /// Get all object ids in this index (for Ordered, returns in sorted order).
    pub fn all_ids(&self) -> Vec<u64> {
        match self.kind {
            SecondaryIndexKind::Hash => {
                let mut ids: Vec<u64> = self.hash.values().flatten().copied().collect();
                ids.sort();
                ids
            }
            SecondaryIndexKind::Ordered => {
                self.ordered.values().flatten().copied().collect()
            }
        }
    }

    /// Number of distinct values indexed.
    pub fn cardinality(&self) -> usize {
        match self.kind {
            SecondaryIndexKind::Hash => self.hash.len(),
            SecondaryIndexKind::Ordered => self.ordered.len(),
        }
    }

    /// Total number of indexed entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the index is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Clear this index.
    pub fn clear(&mut self) {
        self.hash.clear();
        self.ordered.clear();
        self.entries.clear();
    }
}

/// Manager for all secondary indexes on a store.
#[derive(Debug)]
pub struct SecondaryIndexManager {
    /// field_name → SecondaryIndex
    indexes: HashMap<String, SecondaryIndex>,
}

impl SecondaryIndexManager {
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

    /// Create a new secondary index. Returns error if one already exists for this field.
    pub fn create_index(&mut self, field_name: &str, kind: SecondaryIndexKind) -> io::Result<()> {
        if self.indexes.contains_key(field_name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("Index already exists on field '{}'", field_name),
            ));
        }
        self.indexes.insert(
            field_name.to_string(),
            SecondaryIndex::new(field_name, kind),
        );
        Ok(())
    }

    /// Drop a secondary index.
    pub fn drop_index(&mut self, field_name: &str) -> bool {
        self.indexes.remove(field_name).is_some()
    }

    /// Get a reference to an index.
    pub fn get_index(&self, field_name: &str) -> Option<&SecondaryIndex> {
        self.indexes.get(field_name)
    }

    /// List all indexed field names.
    pub fn indexed_fields(&self) -> Vec<String> {
        self.indexes.keys().cloned().collect()
    }

    /// Notify indexes of a new object being inserted.
    pub fn on_insert(&mut self, index: u64, fields: &[(String, StoredValue)]) {
        for (field_name, value) in fields {
            if let Some(sec_idx) = self.indexes.get_mut(field_name) {
                sec_idx.insert(index, value);
            }
        }
    }

    /// Notify indexes of a field update.
    pub fn on_update(
        &mut self,
        index: u64,
        old_fields: &[(String, StoredValue)],
        new_fields: &[(String, StoredValue)],
    ) {
        for sec_idx in self.indexes.values_mut() {
            let old_val = old_fields.iter().find(|(n, _)| n == &sec_idx.field_name);
            let new_val = new_fields.iter().find(|(n, _)| n == &sec_idx.field_name);
            match (old_val, new_val) {
                (Some((_, ov)), Some((_, nv))) => sec_idx.update(index, ov, nv),
                (Some((_, ov)), None) => sec_idx.remove(index, ov),
                (None, Some((_, nv))) => sec_idx.insert(index, nv),
                (None, None) => {}
            }
        }
    }

    /// Notify indexes of a delete.
    pub fn on_delete(&mut self, index: u64, fields: &[(String, StoredValue)]) {
        for (field_name, value) in fields {
            if let Some(sec_idx) = self.indexes.get_mut(field_name) {
                sec_idx.remove(index, value);
            }
        }
    }

    /// Rebuild all indexes from a set of cached objects.
    pub fn rebuild(&mut self, objects: &[(u64, Vec<(String, StoredValue)>, bool)]) {
        // Clear all indexes first
        for idx in self.indexes.values_mut() {
            idx.clear();
        }
        // Re-insert all non-deleted objects
        for (id, fields, is_deleted) in objects {
            if !is_deleted {
                self.on_insert(*id, fields);
            }
        }
    }

    /// Find objects by equality on an indexed field.
    pub fn find_eq(&self, field_name: &str, value: &StoredValue) -> Option<Vec<u64>> {
        self.indexes.get(field_name).map(|idx| idx.find_eq(value))
    }

    /// Find objects by range on an ordered indexed field.
    pub fn find_range(
        &self,
        field_name: &str,
        min: &StoredValue,
        max: &StoredValue,
    ) -> Option<Vec<u64>> {
        self.indexes.get(field_name).map(|idx| idx.find_range(min, max))
    }

    /// Check if a field has an index.
    pub fn has_index(&self, field_name: &str) -> bool {
        self.indexes.contains_key(field_name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_index_insert_find() {
        let mut idx = SecondaryIndex::new("name", SecondaryIndexKind::Hash);
        idx.insert(1, &StoredValue::String("Alice".to_string()));
        idx.insert(2, &StoredValue::String("Bob".to_string()));
        idx.insert(3, &StoredValue::String("Alice".to_string()));

        let results = idx.find_eq(&StoredValue::String("Alice".to_string()));
        assert_eq!(results.len(), 2);
        assert!(results.contains(&1));
        assert!(results.contains(&3));

        let results = idx.find_eq(&StoredValue::String("Bob".to_string()));
        assert_eq!(results, vec![2]);

        let results = idx.find_eq(&StoredValue::String("Charlie".to_string()));
        assert!(results.is_empty());
    }

    #[test]
    fn test_hash_index_remove() {
        let mut idx = SecondaryIndex::new("name", SecondaryIndexKind::Hash);
        idx.insert(1, &StoredValue::String("Alice".to_string()));
        idx.insert(2, &StoredValue::String("Alice".to_string()));
        idx.remove(1, &StoredValue::String("Alice".to_string()));

        let results = idx.find_eq(&StoredValue::String("Alice".to_string()));
        assert_eq!(results, vec![2]);
    }

    #[test]
    fn test_hash_index_update() {
        let mut idx = SecondaryIndex::new("name", SecondaryIndexKind::Hash);
        idx.insert(1, &StoredValue::String("Alice".to_string()));

        idx.update(
            1,
            &StoredValue::String("Alice".to_string()),
            &StoredValue::String("Alicia".to_string()),
        );

        assert!(idx.find_eq(&StoredValue::String("Alice".to_string())).is_empty());
        assert_eq!(idx.find_eq(&StoredValue::String("Alicia".to_string())), vec![1]);
    }

    #[test]
    fn test_ordered_index_range_query() {
        let mut idx = SecondaryIndex::new("age", SecondaryIndexKind::Ordered);
        idx.insert(1, &StoredValue::Int(25));
        idx.insert(2, &StoredValue::Int(30));
        idx.insert(3, &StoredValue::Int(35));
        idx.insert(4, &StoredValue::Int(40));
        idx.insert(5, &StoredValue::Int(20));

        // Range [25, 35] should return ids 1, 2, 3
        let results = idx.find_range(&StoredValue::Int(25), &StoredValue::Int(35));
        assert_eq!(results.len(), 3);
        assert!(results.contains(&1));
        assert!(results.contains(&2));
        assert!(results.contains(&3));
    }

    #[test]
    fn test_ordered_index_equality() {
        let mut idx = SecondaryIndex::new("score", SecondaryIndexKind::Ordered);
        idx.insert(1, &StoredValue::Int(100));
        idx.insert(2, &StoredValue::Int(200));
        idx.insert(3, &StoredValue::Int(100));

        let results = idx.find_eq(&StoredValue::Int(100));
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_manager_lifecycle() {
        let mut mgr = SecondaryIndexManager::new();

        // Create indexes
        mgr.create_index("name", SecondaryIndexKind::Hash).unwrap();
        mgr.create_index("age", SecondaryIndexKind::Ordered).unwrap();

        // Duplicate should fail
        assert!(mgr.create_index("name", SecondaryIndexKind::Hash).is_err());

        // Insert an object
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        mgr.on_insert(1, &fields);

        // Find by name
        let ids = mgr.find_eq("name", &StoredValue::String("Alice".to_string()));
        assert_eq!(ids, Some(vec![1]));

        // Find by age range
        let ids = mgr.find_range("age", &StoredValue::Int(25), &StoredValue::Int(35));
        assert_eq!(ids, Some(vec![1]));

        // Update
        let new_fields = vec![
            ("name".to_string(), StoredValue::String("Alicia".to_string())),
            ("age".to_string(), StoredValue::Int(31)),
        ];
        mgr.on_update(1, &fields, &new_fields);

        assert_eq!(
            mgr.find_eq("name", &StoredValue::String("Alice".to_string())),
            Some(vec![])
        );
        assert_eq!(
            mgr.find_eq("name", &StoredValue::String("Alicia".to_string())),
            Some(vec![1])
        );

        // Delete
        mgr.on_delete(1, &new_fields);
        assert_eq!(
            mgr.find_eq("name", &StoredValue::String("Alicia".to_string())),
            Some(vec![])
        );

        // Drop index
        assert!(mgr.drop_index("name"));
        assert!(mgr.find_eq("name", &StoredValue::String("x".to_string())).is_none());
    }

    #[test]
    fn test_manager_rebuild() {
        let mut mgr = SecondaryIndexManager::new();
        mgr.create_index("status", SecondaryIndexKind::Hash).unwrap();

        let objects = vec![
            (1, vec![("status".to_string(), StoredValue::String("active".to_string()))], false),
            (2, vec![("status".to_string(), StoredValue::String("inactive".to_string()))], false),
            (3, vec![("status".to_string(), StoredValue::String("active".to_string()))], true), // deleted
        ];

        mgr.rebuild(&objects);

        let active = mgr.find_eq("status", &StoredValue::String("active".to_string())).unwrap();
        assert_eq!(active, vec![1]); // id 3 is deleted, not included
    }

    #[test]
    fn test_cardinality_and_len() {
        let mut idx = SecondaryIndex::new("tag", SecondaryIndexKind::Hash);
        idx.insert(1, &StoredValue::String("a".to_string()));
        idx.insert(2, &StoredValue::String("b".to_string()));
        idx.insert(3, &StoredValue::String("a".to_string()));

        assert_eq!(idx.cardinality(), 2); // "a" and "b"
        assert_eq!(idx.len(), 3);         // 3 entries total
    }
}
