//! Store Engine - Main coordinator for Coral persistent stores
//!
//! The StoreEngine orchestrates:
//! - WAL for durability
//! - Binary storage for efficient reads
//! - JSONL storage for queries
//! - Primary index for lookups
//!
//! Operations flow: WAL → Index → Binary + JSONL

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use super::binary::{BinaryReader, BinaryRecord, BinaryWriter, StoredValue};
use super::config::{StoreConfig, SyncMode};
use super::index::PrimaryIndex;
use super::jsonl::JsonlWriter;
use super::uuid7::Uuid7;
use super::wal::{WalEntry, WalReader, WalWriter};

/// Store engine that coordinates all storage components
pub struct StoreEngine {
    /// Store type name
    store_type: String,
    /// Store name/instance
    store_name: String,
    /// Base directory
    base_dir: PathBuf,
    /// Configuration
    config: StoreConfig,
    /// Primary index
    index: PrimaryIndex,
    /// Binary writer
    binary_writer: BinaryWriter,
    /// JSONL writer
    jsonl_writer: JsonlWriter,
    /// WAL writer (optional based on config)
    wal_writer: Option<WalWriter>,
    /// In-memory cache of objects (by index)
    cache: HashMap<u64, CachedObject>,
    /// Next sequential index
    next_index: u64,
    /// Dirty flag (uncommitted changes)
    dirty: bool,
    /// Last checkpoint LSN
    last_checkpoint_lsn: u64,
}

/// Cached object in memory
#[derive(Debug, Clone)]
pub struct CachedObject {
    pub index: u64,
    pub version: u32,
    pub uuid: Uuid7,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: i64,
    pub fields: Vec<(String, StoredValue)>,
    /// Whether this object has unsaved changes
    pub dirty: bool,
}

impl CachedObject {
    fn new(uuid: Uuid7, fields: Vec<(String, StoredValue)>) -> Self {
        let now = current_timestamp_ms();
        Self {
            index: 0,
            version: 1,
            uuid,
            created_at: now,
            updated_at: now,
            deleted_at: -1,
            fields,
            dirty: true,
        }
    }
    
    fn from_binary_record(record: BinaryRecord) -> Self {
        Self {
            index: record.index,
            version: record.version,
            uuid: record.uuid,
            created_at: record.created_at,
            updated_at: record.updated_at,
            deleted_at: record.deleted_at,
            fields: record.fields,
            dirty: false,
        }
    }
    
    /// Check if this object is deleted
    pub fn is_deleted(&self) -> bool {
        self.deleted_at >= 0
    }
}

fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl StoreEngine {
    /// Open or create a store
    pub fn open(
        store_type: &str,
        store_name: &str,
        config: StoreConfig,
    ) -> io::Result<Self> {
        // Build the base directory from config
        let base_dir = config.data_path.join(store_name);
        
        fs::create_dir_all(&base_dir)?;
        
        // Create subdirectories
        let data_dir = base_dir.join("data");
        let index_dir = base_dir.join("index");
        
        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&index_dir)?;
        
        // Open components
        let index = PrimaryIndex::open(index_dir.join("primary.idx"))?;
        let binary_writer = BinaryWriter::open(data_dir.join("data.bin"), store_type)?;
        let jsonl_writer = JsonlWriter::open(data_dir.join("data.jsonl"))?;
        
        // WAL is optional based on config
        let wal_writer = if config.wal.enabled {
            let wal_dir = config.wal_path();
            fs::create_dir_all(&wal_dir)?;
            Some(WalWriter::open(&wal_dir, store_type)?)
        } else {
            None
        };
        
        // Determine next index from existing data
        let next_index = index.len() + 1;
        
        let mut engine = Self {
            store_type: store_type.to_string(),
            store_name: store_name.to_string(),
            base_dir,
            config,
            index,
            binary_writer,
            jsonl_writer,
            wal_writer,
            cache: HashMap::new(),
            next_index,
            dirty: false,
            last_checkpoint_lsn: 0,
        };
        
        // Recover from WAL if needed
        engine.recover()?;
        
        Ok(engine)
    }
    
    /// Recover from WAL after crash
    fn recover(&mut self) -> io::Result<()> {
        let wal_dir = self.config.wal_path();
        
        if self.wal_writer.is_some() && wal_dir.exists() {
            if let Ok(reader) = WalReader::open(&wal_dir) {
                if let Ok(entries) = reader.entries_from(self.last_checkpoint_lsn) {
                    for result in entries {
                        if let Ok(entry) = result {
                            self.replay_wal_entry(&entry)?;
                        }
                    }
                }
            }
        }
        Ok(())
    }
    
    fn replay_wal_entry(&mut self, entry: &WalEntry) -> io::Result<()> {
        use super::WalOpType;
        
        match entry.op_type {
            WalOpType::Insert => {
                let fields = entry.parse_payload()?;
                let obj = CachedObject {
                    index: entry.index,
                    version: 1,
                    uuid: entry.uuid.clone(),
                    created_at: entry.timestamp_ms,
                    updated_at: entry.timestamp_ms,
                    deleted_at: -1,
                    fields,
                    dirty: false,
                };
                self.cache.insert(entry.index, obj);
                if entry.index >= self.next_index {
                    self.next_index = entry.index + 1;
                }
            }
            WalOpType::Update => {
                let fields = entry.parse_payload()?;
                if let Some(obj) = self.cache.get_mut(&entry.index) {
                    obj.fields = fields;
                    obj.updated_at = entry.timestamp_ms;
                    obj.version += 1;
                }
            }
            WalOpType::Delete => {
                if let Some(obj) = self.cache.get_mut(&entry.index) {
                    obj.deleted_at = entry.timestamp_ms;
                }
            }
            WalOpType::Commit => {
                // Changes are already applied
            }
            WalOpType::Checkpoint => {
                self.last_checkpoint_lsn = entry.lsn;
            }
        }
        Ok(())
    }
    
    /// Create a new object
    pub fn create(&mut self, fields: Vec<(String, StoredValue)>) -> io::Result<u64> {
        let uuid = Uuid7::new();
        let index = self.next_index;
        self.next_index += 1;
        
        let mut obj = CachedObject::new(uuid.clone(), fields.clone());
        obj.index = index;
        
        // Write to WAL first (if enabled)
        if let Some(ref mut wal) = self.wal_writer {
            let lsn = wal.next_lsn();
            let entry = WalEntry::insert(lsn, index, uuid.clone(), &fields);
            wal.write_entry(&entry)?;
            
            if self.config.wal.sync_mode == SyncMode::FSync {
                wal.sync()?;
            }
        }
        
        // Write to storage immediately if auto_persist is enabled
        if self.config.auto_persist.enabled {
            self.persist_object(&obj)?;
            obj.dirty = false;
        }
        
        self.cache.insert(index, obj);
        self.dirty = true;
        
        Ok(index)
    }
    
    /// Get an object by index
    pub fn get(&mut self, index: u64) -> io::Result<Option<&CachedObject>> {
        // Check cache first
        if self.cache.contains_key(&index) {
            return Ok(self.cache.get(&index));
        }
        
        // Load from storage
        if let Some(entry) = self.index.get_by_index(index) {
            let reader = BinaryReader::open(
                self.base_dir.join("data").join("data.bin")
            )?;
            let record = reader.read_record(entry.binary_offset)?;
            let obj = CachedObject::from_binary_record(record);
            self.cache.insert(index, obj);
            return Ok(self.cache.get(&index));
        }
        
        Ok(None)
    }
    
    /// Get an object by UUID
    pub fn get_by_uuid(&mut self, uuid: &Uuid7) -> io::Result<Option<&CachedObject>> {
        // Check cache first
        for (_, obj) in &self.cache {
            if &obj.uuid == uuid {
                let idx = obj.index;
                return Ok(self.cache.get(&idx));
            }
        }
        
        // Check index
        if let Some(entry) = self.index.get_by_uuid(uuid) {
            return self.get(entry.index);
        }
        
        Ok(None)
    }
    
    /// Update an object's fields
    pub fn update(
        &mut self,
        index: u64,
        fields: Vec<(String, StoredValue)>,
    ) -> io::Result<()> {
        // Ensure object is loaded
        self.get(index)?;
        
        let now = current_timestamp_ms();
        
        if let Some(obj) = self.cache.get_mut(&index) {
            // Write to WAL first
            if let Some(ref mut wal) = self.wal_writer {
                let lsn = wal.next_lsn();
                let entry = WalEntry::update(lsn, index, obj.uuid.clone(), &fields);
                wal.write_entry(&entry)?;
                
                if self.config.wal.sync_mode == SyncMode::FSync {
                    wal.sync()?;
                }
            }
            
            obj.fields = fields;
            obj.updated_at = now;
            obj.version += 1;
            obj.dirty = true;
            
            // Auto-persist if enabled
            if self.config.auto_persist.enabled {
                let obj_clone = obj.clone();
                self.persist_object(&obj_clone)?;
                if let Some(obj) = self.cache.get_mut(&index) {
                    obj.dirty = false;
                }
            }
            
            self.dirty = true;
        }
        
        Ok(())
    }
    
    /// Update a single field
    pub fn update_field(
        &mut self,
        index: u64,
        field_name: &str,
        value: StoredValue,
    ) -> io::Result<()> {
        // Ensure object is loaded
        self.get(index)?;
        
        let now = current_timestamp_ms();
        
        if let Some(obj) = self.cache.get_mut(&index) {
            // Update or add the field
            let mut found = false;
            for (name, val) in &mut obj.fields {
                if name == field_name {
                    *val = value.clone();
                    found = true;
                    break;
                }
            }
            if !found {
                obj.fields.push((field_name.to_string(), value.clone()));
            }
            
            // Write to WAL
            if let Some(ref mut wal) = self.wal_writer {
                let lsn = wal.next_lsn();
                let entry = WalEntry::update(lsn, index, obj.uuid.clone(), &obj.fields);
                wal.write_entry(&entry)?;
                
                if self.config.wal.sync_mode == SyncMode::FSync {
                    wal.sync()?;
                }
            }
            
            obj.updated_at = now;
            obj.version += 1;
            obj.dirty = true;
            
            // Auto-persist if enabled
            if self.config.auto_persist.enabled {
                let obj_clone = obj.clone();
                self.persist_object(&obj_clone)?;
                if let Some(obj) = self.cache.get_mut(&index) {
                    obj.dirty = false;
                }
            }
            
            self.dirty = true;
        }
        
        Ok(())
    }
    
    /// Soft delete an object
    pub fn delete(&mut self, index: u64) -> io::Result<()> {
        // Ensure object is loaded
        self.get(index)?;
        
        let now = current_timestamp_ms();
        
        if let Some(obj) = self.cache.get_mut(&index) {
            if obj.is_deleted() {
                return Ok(()); // Already deleted
            }
            
            // Write to WAL
            if let Some(ref mut wal) = self.wal_writer {
                let lsn = wal.next_lsn();
                let entry = WalEntry::delete(lsn, index, obj.uuid.clone());
                wal.write_entry(&entry)?;
                
                if self.config.wal.sync_mode == SyncMode::FSync {
                    wal.sync()?;
                }
            }
            
            obj.deleted_at = now;
            obj.updated_at = now;
            obj.version += 1;
            obj.dirty = true;
            
            // Update index flags
            self.index.mark_deleted(index);
            
            self.dirty = true;
        }
        
        Ok(())
    }
    
    /// Persist an object to binary and JSONL storage
    fn persist_object(&mut self, obj: &CachedObject) -> io::Result<()> {
        // Write to binary
        let (binary_offset, binary_len) = self.binary_writer.write_record(
            obj.index,
            obj.version,
            &obj.uuid,
            obj.created_at,
            obj.updated_at,
            obj.deleted_at,
            &obj.fields,
        )?;
        
        // Write to JSONL
        let (jsonl_offset, jsonl_len) = self.jsonl_writer.write_record(
            obj.index,
            obj.version,
            &obj.uuid,
            obj.created_at,
            obj.updated_at,
            obj.deleted_at,
            &obj.fields,
        )?;
        
        // Update or insert into index
        if self.index.get_by_index(obj.index).is_some() {
            self.index.update_offsets(obj.index, binary_offset, binary_len, jsonl_offset, jsonl_len);
        } else {
            // Create new index entry
            let mut entry = super::index::IndexEntry::new(obj.index, obj.uuid.clone());
            entry.binary_offset = binary_offset;
            entry.binary_length = binary_len;
            entry.json_offset = jsonl_offset;
            entry.json_length = jsonl_len;
            self.index.insert(entry);
        }
        
        Ok(())
    }
    
    /// Save all dirty objects to storage
    pub fn save(&mut self) -> io::Result<()> {
        let dirty_indices: Vec<u64> = self.cache
            .iter()
            .filter(|(_, obj)| obj.dirty)
            .map(|(idx, _)| *idx)
            .collect();
        
        for index in dirty_indices {
            if let Some(obj) = self.cache.get(&index) {
                let obj_clone = obj.clone();
                self.persist_object(&obj_clone)?;
            }
            if let Some(obj) = self.cache.get_mut(&index) {
                obj.dirty = false;
            }
        }
        
        // Save index
        self.index.save()?;
        
        // Commit WAL
        if let Some(ref mut wal) = self.wal_writer {
            wal.commit()?;
        }
        
        // Flush JSONL
        self.jsonl_writer.flush()?;
        
        self.dirty = false;
        
        Ok(())
    }
    
    /// Create a checkpoint
    pub fn checkpoint(&mut self) -> io::Result<()> {
        // Save all pending changes
        self.save()?;
        
        // Write checkpoint to WAL
        if let Some(ref mut wal) = self.wal_writer {
            let lsn = wal.checkpoint()?;
            self.last_checkpoint_lsn = lsn;
            
            // Truncate old WAL segments
            wal.truncate_before(lsn)?;
        }
        
        Ok(())
    }
    
    /// Get all non-deleted object indexes
    pub fn all(&self) -> Vec<u64> {
        let mut indices: Vec<u64> = self.cache
            .iter()
            .filter(|(_, obj)| !obj.is_deleted())
            .map(|(idx, _)| *idx)
            .collect();
        
        // Also include indexes not in cache (from persistent storage)
        for i in 1..self.next_index {
            if !self.cache.contains_key(&i) {
                // Check if it exists in the index (but not deleted)
                if let Some(entry) = self.index.get_by_index(i) {
                    if !entry.is_deleted() {
                        indices.push(i);
                    }
                }
            }
        }
        
        indices.sort();
        indices.dedup();
        indices
    }
    
    /// Count non-deleted objects
    pub fn count(&self) -> u64 {
        self.all().len() as u64
    }
    
    /// Get store statistics
    pub fn stats(&self) -> StoreStats {
        StoreStats {
            store_type: self.store_type.clone(),
            store_name: self.store_name.clone(),
            total_objects: self.next_index - 1,
            cached_objects: self.cache.len() as u64,
            dirty_objects: self.cache.values().filter(|o| o.dirty).count() as u64,
            binary_size: self.binary_writer.write_offset(),
            jsonl_size: self.jsonl_writer.write_offset(),
        }
    }
    
    /// Close the store, saving any pending changes
    pub fn close(mut self) -> io::Result<()> {
        self.save()
    }
}

/// Store statistics
#[derive(Debug, Clone)]
pub struct StoreStats {
    pub store_type: String,
    pub store_name: String,
    pub total_objects: u64,
    pub cached_objects: u64,
    pub dirty_objects: u64,
    pub binary_size: u64,
    pub jsonl_size: u64,
}

/// Thread-safe wrapper for StoreEngine
pub struct SharedStoreEngine {
    inner: Arc<RwLock<StoreEngine>>,
}

impl SharedStoreEngine {
    pub fn new(engine: StoreEngine) -> Self {
        Self {
            inner: Arc::new(RwLock::new(engine)),
        }
    }
    
    pub fn create(&self, fields: Vec<(String, StoredValue)>) -> io::Result<u64> {
        self.inner.write().unwrap().create(fields)
    }
    
    pub fn get(&self, index: u64) -> io::Result<Option<CachedObject>> {
        let mut engine = self.inner.write().unwrap();
        Ok(engine.get(index)?.cloned())
    }
    
    pub fn update(
        &self,
        index: u64,
        fields: Vec<(String, StoredValue)>,
    ) -> io::Result<()> {
        self.inner.write().unwrap().update(index, fields)
    }
    
    pub fn update_field(
        &self,
        index: u64,
        field_name: &str,
        value: StoredValue,
    ) -> io::Result<()> {
        self.inner.write().unwrap().update_field(index, field_name, value)
    }
    
    pub fn delete(&self, index: u64) -> io::Result<()> {
        self.inner.write().unwrap().delete(index)
    }
    
    pub fn save(&self) -> io::Result<()> {
        self.inner.write().unwrap().save()
    }
    
    pub fn checkpoint(&self) -> io::Result<()> {
        self.inner.write().unwrap().checkpoint()
    }
    
    pub fn all(&self) -> Vec<u64> {
        self.inner.read().unwrap().all()
    }
    
    pub fn count(&self) -> u64 {
        self.inner.read().unwrap().count()
    }
    
    pub fn stats(&self) -> StoreStats {
        self.inner.read().unwrap().stats()
    }
}

impl Clone for SharedStoreEngine {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::config::StoreConfig;
    
    fn unique_test_path(name: &str) -> String {
        format!("/tmp/coral_test_{}_{}", name, std::process::id())
    }
    
    #[test]
    fn test_engine_create_get() {
        let path = unique_test_path("engine_create");
        let _ = fs::remove_dir_all(&path);
        
        let config = StoreConfig::minimal("TestStore", &path);
        let mut engine = StoreEngine::open("TestStore", "default", config).unwrap();
        
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        
        let idx = engine.create(fields).unwrap();
        assert_eq!(idx, 1);
        
        let obj = engine.get(idx).unwrap().unwrap();
        assert_eq!(obj.index, 1);
        assert_eq!(obj.fields.len(), 2);
        
        let _ = fs::remove_dir_all(&path);
    }
    
    #[test]
    fn test_engine_update() {
        let path = unique_test_path("engine_update");
        let _ = fs::remove_dir_all(&path);
        
        let config = StoreConfig::minimal("TestStore2", &path);
        let mut engine = StoreEngine::open("TestStore2", "default", config).unwrap();
        
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        
        let idx = engine.create(fields).unwrap();
        
        engine.update_field(idx, "age", StoredValue::Int(31)).unwrap();
        
        let obj = engine.get(idx).unwrap().unwrap();
        let age = obj.fields.iter().find(|(n, _)| n == "age").unwrap();
        assert_eq!(age.1, StoredValue::Int(31));
        assert_eq!(obj.version, 2);
        
        let _ = fs::remove_dir_all(&path);
    }
    
    #[test]
    fn test_engine_delete() {
        let path = unique_test_path("engine_delete");
        let _ = fs::remove_dir_all(&path);
        
        let config = StoreConfig::minimal("TestStore3", &path);
        let mut engine = StoreEngine::open("TestStore3", "default", config).unwrap();
        
        let idx = engine.create(vec![
            ("name".to_string(), StoredValue::String("Bob".to_string())),
        ]).unwrap();
        
        engine.delete(idx).unwrap();
        
        let obj = engine.get(idx).unwrap().unwrap();
        assert!(obj.is_deleted());
        assert!(obj.deleted_at >= 0);
        
        let _ = fs::remove_dir_all(&path);
    }
    
    #[test]
    fn test_engine_persistence() {
        let path = unique_test_path("engine_persist");
        let _ = fs::remove_dir_all(&path);
        
        let config = StoreConfig::minimal("TestStore4", &path);
        let uuid: Uuid7;
        
        // Create and save
        {
            let mut engine = StoreEngine::open("TestStore4", "default", config.clone()).unwrap();
            let idx = engine.create(vec![
                ("name".to_string(), StoredValue::String("Charlie".to_string())),
            ]).unwrap();
            
            uuid = engine.get(idx).unwrap().unwrap().uuid.clone();
            engine.save().unwrap();
        }
        
        // Reopen and verify
        {
            let mut engine = StoreEngine::open("TestStore4", "default", config).unwrap();
            let obj = engine.get(1).unwrap().unwrap();
            assert_eq!(obj.uuid, uuid);
            
            let name = obj.fields.iter().find(|(n, _)| n == "name").unwrap();
            assert_eq!(name.1, StoredValue::String("Charlie".to_string()));
        }
        
        let _ = fs::remove_dir_all(&path);
    }
}
