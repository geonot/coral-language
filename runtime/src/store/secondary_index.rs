use std::collections::{BTreeMap, HashMap, HashSet};
use std::io;

use super::binary::StoredValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondaryIndexKind {
    Hash,
    Ordered,
}

#[derive(Debug)]
pub struct SecondaryIndex {
    pub field_name: String,

    pub kind: SecondaryIndexKind,

    hash: HashMap<StoredValue, Vec<u64>>,

    ordered: BTreeMap<StoredValue, Vec<u64>>,

    entries: HashSet<u64>,
}

impl SecondaryIndex {
    pub fn new(field_name: &str, kind: SecondaryIndexKind) -> Self {
        Self {
            field_name: field_name.to_string(),
            kind,
            hash: HashMap::new(),
            ordered: BTreeMap::new(),
            entries: HashSet::new(),
        }
    }

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

    pub fn update(&mut self, index: u64, old_value: &StoredValue, new_value: &StoredValue) {
        self.remove(index, old_value);
        self.insert(index, new_value);
    }

    pub fn find_eq(&self, value: &StoredValue) -> Vec<u64> {
        match self.kind {
            SecondaryIndexKind::Hash => self.hash.get(value).cloned().unwrap_or_default(),
            SecondaryIndexKind::Ordered => self.ordered.get(value).cloned().unwrap_or_default(),
        }
    }

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

    pub fn all_ids(&self) -> Vec<u64> {
        match self.kind {
            SecondaryIndexKind::Hash => {
                let mut ids: Vec<u64> = self.hash.values().flatten().copied().collect();
                ids.sort();
                ids
            }
            SecondaryIndexKind::Ordered => self.ordered.values().flatten().copied().collect(),
        }
    }

    pub fn cardinality(&self) -> usize {
        match self.kind {
            SecondaryIndexKind::Hash => self.hash.len(),
            SecondaryIndexKind::Ordered => self.ordered.len(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.hash.clear();
        self.ordered.clear();
        self.entries.clear();
    }
}

#[derive(Debug)]
pub struct SecondaryIndexManager {
    indexes: HashMap<String, SecondaryIndex>,
}

impl SecondaryIndexManager {
    pub fn new() -> Self {
        Self {
            indexes: HashMap::new(),
        }
    }

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

    pub fn drop_index(&mut self, field_name: &str) -> bool {
        self.indexes.remove(field_name).is_some()
    }

    pub fn get_index(&self, field_name: &str) -> Option<&SecondaryIndex> {
        self.indexes.get(field_name)
    }

    pub fn indexed_fields(&self) -> Vec<String> {
        self.indexes.keys().cloned().collect()
    }

    pub fn on_insert(&mut self, index: u64, fields: &[(String, StoredValue)]) {
        for (field_name, value) in fields {
            if let Some(sec_idx) = self.indexes.get_mut(field_name) {
                sec_idx.insert(index, value);
            }
        }
    }

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

    pub fn on_delete(&mut self, index: u64, fields: &[(String, StoredValue)]) {
        for (field_name, value) in fields {
            if let Some(sec_idx) = self.indexes.get_mut(field_name) {
                sec_idx.remove(index, value);
            }
        }
    }

    pub fn rebuild(&mut self, objects: &[(u64, Vec<(String, StoredValue)>, bool)]) {
        for idx in self.indexes.values_mut() {
            idx.clear();
        }

        for (id, fields, is_deleted) in objects {
            if !is_deleted {
                self.on_insert(*id, fields);
            }
        }
    }

    pub fn find_eq(&self, field_name: &str, value: &StoredValue) -> Option<Vec<u64>> {
        self.indexes.get(field_name).map(|idx| idx.find_eq(value))
    }

    pub fn find_range(
        &self,
        field_name: &str,
        min: &StoredValue,
        max: &StoredValue,
    ) -> Option<Vec<u64>> {
        self.indexes
            .get(field_name)
            .map(|idx| idx.find_range(min, max))
    }

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

        assert!(
            idx.find_eq(&StoredValue::String("Alice".to_string()))
                .is_empty()
        );
        assert_eq!(
            idx.find_eq(&StoredValue::String("Alicia".to_string())),
            vec![1]
        );
    }

    #[test]
    fn test_ordered_index_range_query() {
        let mut idx = SecondaryIndex::new("age", SecondaryIndexKind::Ordered);
        idx.insert(1, &StoredValue::Int(25));
        idx.insert(2, &StoredValue::Int(30));
        idx.insert(3, &StoredValue::Int(35));
        idx.insert(4, &StoredValue::Int(40));
        idx.insert(5, &StoredValue::Int(20));

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

        mgr.create_index("name", SecondaryIndexKind::Hash).unwrap();
        mgr.create_index("age", SecondaryIndexKind::Ordered)
            .unwrap();

        assert!(mgr.create_index("name", SecondaryIndexKind::Hash).is_err());

        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        mgr.on_insert(1, &fields);

        let ids = mgr.find_eq("name", &StoredValue::String("Alice".to_string()));
        assert_eq!(ids, Some(vec![1]));

        let ids = mgr.find_range("age", &StoredValue::Int(25), &StoredValue::Int(35));
        assert_eq!(ids, Some(vec![1]));

        let new_fields = vec![
            (
                "name".to_string(),
                StoredValue::String("Alicia".to_string()),
            ),
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

        mgr.on_delete(1, &new_fields);
        assert_eq!(
            mgr.find_eq("name", &StoredValue::String("Alicia".to_string())),
            Some(vec![])
        );

        assert!(mgr.drop_index("name"));
        assert!(
            mgr.find_eq("name", &StoredValue::String("x".to_string()))
                .is_none()
        );
    }

    #[test]
    fn test_manager_rebuild() {
        let mut mgr = SecondaryIndexManager::new();
        mgr.create_index("status", SecondaryIndexKind::Hash)
            .unwrap();

        let objects = vec![
            (
                1,
                vec![(
                    "status".to_string(),
                    StoredValue::String("active".to_string()),
                )],
                false,
            ),
            (
                2,
                vec![(
                    "status".to_string(),
                    StoredValue::String("inactive".to_string()),
                )],
                false,
            ),
            (
                3,
                vec![(
                    "status".to_string(),
                    StoredValue::String("active".to_string()),
                )],
                true,
            ),
        ];

        mgr.rebuild(&objects);

        let active = mgr
            .find_eq("status", &StoredValue::String("active".to_string()))
            .unwrap();
        assert_eq!(active, vec![1]);
    }

    #[test]
    fn test_cardinality_and_len() {
        let mut idx = SecondaryIndex::new("tag", SecondaryIndexKind::Hash);
        idx.insert(1, &StoredValue::String("a".to_string()));
        idx.insert(2, &StoredValue::String("b".to_string()));
        idx.insert(3, &StoredValue::String("a".to_string()));

        assert_eq!(idx.cardinality(), 2);
        assert_eq!(idx.len(), 3);
    }
}
