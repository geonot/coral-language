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
use super::secondary_index::{SecondaryIndexKind, SecondaryIndexManager};
use super::uuid7::Uuid7;
use super::wal::{WalEntry, WalReader, WalWriter};

pub struct StoreEngine {
    store_type: String,

    store_name: String,

    base_dir: PathBuf,

    config: StoreConfig,

    index: PrimaryIndex,

    binary_writer: BinaryWriter,

    jsonl_writer: JsonlWriter,

    wal_writer: Option<WalWriter>,

    cache: HashMap<u64, CachedObject>,

    next_index: u64,

    dirty: bool,

    last_checkpoint_lsn: u64,

    secondary_indexes: SecondaryIndexManager,
}

#[derive(Debug, Clone)]
pub struct CachedObject {
    pub index: u64,
    pub version: u32,
    pub uuid: Uuid7,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: i64,
    pub fields: Vec<(String, StoredValue)>,

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
    pub fn open(store_type: &str, store_name: &str, config: StoreConfig) -> io::Result<Self> {
        let base_dir = config.data_path.join(store_name);

        fs::create_dir_all(&base_dir)?;

        let data_dir = base_dir.join("data");
        let index_dir = base_dir.join("index");

        fs::create_dir_all(&data_dir)?;
        fs::create_dir_all(&index_dir)?;

        let index = PrimaryIndex::open(index_dir.join("primary.idx"))?;
        let binary_writer = BinaryWriter::open(data_dir.join("data.bin"), store_type)?;
        let jsonl_writer = JsonlWriter::open(data_dir.join("data.jsonl"))?;

        let wal_writer = if config.wal.enabled {
            let wal_dir = config.wal_path();
            fs::create_dir_all(&wal_dir)?;
            Some(WalWriter::open(&wal_dir, store_type)?)
        } else {
            None
        };

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
            secondary_indexes: SecondaryIndexManager::new(),
        };

        engine.recover()?;

        Ok(engine)
    }

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
                    fields: fields.clone(),
                    dirty: false,
                };
                self.cache.insert(entry.index, obj);
                self.secondary_indexes.on_insert(entry.index, &fields);
                if entry.index >= self.next_index {
                    self.next_index = entry.index + 1;
                }
            }
            WalOpType::Update => {
                let fields = entry.parse_payload()?;
                if let Some(obj) = self.cache.get_mut(&entry.index) {
                    let old_fields = obj.fields.clone();
                    obj.fields = fields.clone();
                    obj.updated_at = entry.timestamp_ms;
                    obj.version += 1;
                    self.secondary_indexes
                        .on_update(entry.index, &old_fields, &fields);
                }
            }
            WalOpType::Delete => {
                if let Some(obj) = self.cache.get_mut(&entry.index) {
                    let deleted_fields = obj.fields.clone();
                    obj.deleted_at = entry.timestamp_ms;
                    self.secondary_indexes
                        .on_delete(entry.index, &deleted_fields);
                }
            }
            WalOpType::Commit => {}
            WalOpType::Checkpoint => {
                self.last_checkpoint_lsn = entry.lsn;
            }
        }
        Ok(())
    }

    pub fn create(&mut self, fields: Vec<(String, StoredValue)>) -> io::Result<u64> {
        let uuid = Uuid7::new();
        let index = self.next_index;
        self.next_index += 1;

        let mut obj = CachedObject::new(uuid.clone(), fields.clone());
        obj.index = index;

        if let Some(ref mut wal) = self.wal_writer {
            let lsn = wal.next_lsn();
            let entry = WalEntry::insert(lsn, index, uuid.clone(), &fields);
            wal.write_entry(&entry)?;

            if self.config.wal.sync_mode == SyncMode::FSync {
                wal.sync()?;
            }
        }

        if self.config.auto_persist.enabled {
            self.persist_object(&obj)?;
            obj.dirty = false;
        }

        self.cache.insert(index, obj);
        self.secondary_indexes.on_insert(index, &fields);
        self.dirty = true;

        Ok(index)
    }

    pub fn get(&mut self, index: u64) -> io::Result<Option<&CachedObject>> {
        if self.cache.contains_key(&index) {
            return Ok(self.cache.get(&index));
        }

        if let Some(entry) = self.index.get_by_index(index) {
            let reader = BinaryReader::open(self.base_dir.join("data").join("data.bin"))?;
            let record = reader.read_record(entry.binary_offset)?;
            let obj = CachedObject::from_binary_record(record);
            self.cache.insert(index, obj);
            return Ok(self.cache.get(&index));
        }

        Ok(None)
    }

    pub fn get_by_uuid(&mut self, uuid: &Uuid7) -> io::Result<Option<&CachedObject>> {
        for (_, obj) in &self.cache {
            if &obj.uuid == uuid {
                let idx = obj.index;
                return Ok(self.cache.get(&idx));
            }
        }

        if let Some(entry) = self.index.get_by_uuid(uuid) {
            return self.get(entry.index);
        }

        Ok(None)
    }

    pub fn update(&mut self, index: u64, fields: Vec<(String, StoredValue)>) -> io::Result<()> {
        self.get(index)?;

        let now = current_timestamp_ms();

        if let Some(obj) = self.cache.get_mut(&index) {
            if let Some(ref mut wal) = self.wal_writer {
                let lsn = wal.next_lsn();
                let entry = WalEntry::update(lsn, index, obj.uuid.clone(), &fields);
                wal.write_entry(&entry)?;

                if self.config.wal.sync_mode == SyncMode::FSync {
                    wal.sync()?;
                }
            }

            let old_fields = obj.fields.clone();
            obj.fields = fields.clone();
            obj.updated_at = now;
            obj.version += 1;
            obj.dirty = true;

            if self.config.auto_persist.enabled {
                let obj_clone = obj.clone();
                self.persist_object(&obj_clone)?;
                if let Some(obj) = self.cache.get_mut(&index) {
                    obj.dirty = false;
                }
            }

            self.secondary_indexes
                .on_update(index, &old_fields, &fields);
            self.dirty = true;
        }

        Ok(())
    }

    pub fn update_field(
        &mut self,
        index: u64,
        field_name: &str,
        value: StoredValue,
    ) -> io::Result<()> {
        self.get(index)?;

        let now = current_timestamp_ms();

        if let Some(obj) = self.cache.get_mut(&index) {
            let old_fields = obj.fields.clone();

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

            if let Some(ref mut wal) = self.wal_writer {
                let lsn = wal.next_lsn();
                let entry = WalEntry::update(lsn, index, obj.uuid.clone(), &obj.fields);
                wal.write_entry(&entry)?;

                if self.config.wal.sync_mode == SyncMode::FSync {
                    wal.sync()?;
                }
            }

            let new_fields = obj.fields.clone();
            obj.updated_at = now;
            obj.version += 1;
            obj.dirty = true;

            if self.config.auto_persist.enabled {
                let obj_clone = obj.clone();
                self.persist_object(&obj_clone)?;
                if let Some(obj) = self.cache.get_mut(&index) {
                    obj.dirty = false;
                }
            }

            self.secondary_indexes
                .on_update(index, &old_fields, &new_fields);
            self.dirty = true;
        }

        Ok(())
    }

    pub fn delete(&mut self, index: u64) -> io::Result<()> {
        self.get(index)?;

        let now = current_timestamp_ms();

        if let Some(obj) = self.cache.get_mut(&index) {
            if obj.is_deleted() {
                return Ok(());
            }

            let deleted_fields = obj.fields.clone();

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

            self.index.mark_deleted(index);

            self.secondary_indexes.on_delete(index, &deleted_fields);
            self.dirty = true;
        }

        Ok(())
    }

    fn persist_object(&mut self, obj: &CachedObject) -> io::Result<()> {
        let (binary_offset, binary_len) = self.binary_writer.write_record(
            obj.index,
            obj.version,
            &obj.uuid,
            obj.created_at,
            obj.updated_at,
            obj.deleted_at,
            &obj.fields,
        )?;

        let (jsonl_offset, jsonl_len) = self.jsonl_writer.write_record(
            obj.index,
            obj.version,
            &obj.uuid,
            obj.created_at,
            obj.updated_at,
            obj.deleted_at,
            &obj.fields,
        )?;

        if self.index.get_by_index(obj.index).is_some() {
            self.index.update_offsets(
                obj.index,
                binary_offset,
                binary_len,
                jsonl_offset,
                jsonl_len,
            );
        } else {
            let mut entry = super::index::IndexEntry::new(obj.index, obj.uuid.clone());
            entry.binary_offset = binary_offset;
            entry.binary_length = binary_len;
            entry.json_offset = jsonl_offset;
            entry.json_length = jsonl_len;
            self.index.insert(entry);
        }

        Ok(())
    }

    pub fn save(&mut self) -> io::Result<()> {
        let dirty_indices: Vec<u64> = self
            .cache
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

        self.index.save()?;

        if let Some(ref mut wal) = self.wal_writer {
            wal.commit()?;
        }

        self.jsonl_writer.flush()?;

        self.dirty = false;

        Ok(())
    }

    pub fn checkpoint(&mut self) -> io::Result<()> {
        self.save()?;

        if let Some(ref mut wal) = self.wal_writer {
            let lsn = wal.checkpoint()?;
            self.last_checkpoint_lsn = lsn;

            wal.truncate_before(lsn)?;
        }

        Ok(())
    }

    pub fn all(&self) -> Vec<u64> {
        let mut indices: Vec<u64> = self
            .cache
            .iter()
            .filter(|(_, obj)| !obj.is_deleted())
            .map(|(idx, _)| *idx)
            .collect();

        for i in 1..self.next_index {
            if !self.cache.contains_key(&i) {
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

    pub fn count(&self) -> u64 {
        self.all().len() as u64
    }

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

    pub fn close(mut self) -> io::Result<()> {
        self.save()
    }

    pub fn compact_wal(&mut self) -> io::Result<(u64, u64)> {
        self.save()?;

        let wal_dir = self.config.wal_path();
        if !wal_dir.exists() {
            return Ok((0, 0));
        }
        let wal_writer = match self.wal_writer {
            Some(ref w) => w,
            None => return Ok((0, 0)),
        };

        let old_size = Self::total_wal_size(&wal_dir)?;
        if old_size == 0 {
            return Ok((0, 0));
        }

        let reader = match WalReader::open(&wal_dir) {
            Ok(r) => r,
            Err(_) => return Ok((old_size, old_size)),
        };
        let entries_iter = reader.entries_from(0)?;

        use std::collections::BTreeMap;
        let mut latest: BTreeMap<u64, WalEntry> = BTreeMap::new();
        for result in entries_iter {
            let entry = match result {
                Ok(e) => e,
                Err(_) => continue,
            };
            match entry.op_type {
                super::WalOpType::Commit | super::WalOpType::Checkpoint => continue,
                super::WalOpType::Delete => {
                    latest.insert(entry.index, entry);
                }
                _ => {
                    let replace = match latest.get(&entry.index) {
                        Some(existing) => entry.lsn > existing.lsn,
                        None => true,
                    };
                    if replace {
                        latest.insert(entry.index, entry);
                    }
                }
            }
        }

        latest.retain(|_idx, entry| entry.op_type != super::WalOpType::Delete);

        drop(self.wal_writer.take());

        for dir_entry in fs::read_dir(&wal_dir)? {
            let dir_entry = dir_entry?;
            let name = dir_entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("wal-") && name.ends_with(".log") {
                fs::remove_file(dir_entry.path())?;
            }
        }

        let mut new_wal = WalWriter::open(&wal_dir, &self.store_type)?;

        for (_idx, entry) in &latest {
            let new_lsn = new_wal.next_lsn();
            let new_entry = WalEntry {
                lsn: new_lsn,
                op_type: entry.op_type,
                timestamp_ms: entry.timestamp_ms,
                index: entry.index,
                uuid: entry.uuid.clone(),
                payload: entry.payload.clone(),
            };
            new_wal.write_entry(&new_entry)?;
        }

        new_wal.commit()?;

        self.wal_writer = Some(new_wal);

        let new_size = Self::total_wal_size(&wal_dir)?;

        Ok((old_size, new_size))
    }

    fn total_wal_size(wal_dir: &std::path::Path) -> io::Result<u64> {
        let mut total = 0u64;
        if !wal_dir.exists() {
            return Ok(0);
        }
        for entry in fs::read_dir(wal_dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with("wal-") && name.ends_with(".log") {
                total += entry.metadata()?.len();
            }
        }
        Ok(total)
    }

    pub fn create_secondary_index(
        &mut self,
        field_name: &str,
        kind: SecondaryIndexKind,
    ) -> io::Result<()> {
        self.secondary_indexes.create_index(field_name, kind)?;

        let entries: Vec<(u64, Vec<(String, StoredValue)>, bool)> = self
            .cache
            .iter()
            .map(|(id, obj)| (*id, obj.fields.clone(), obj.is_deleted()))
            .collect();
        self.secondary_indexes.rebuild(&entries);
        Ok(())
    }

    pub fn drop_secondary_index(&mut self, field_name: &str) -> bool {
        self.secondary_indexes.drop_index(field_name)
    }

    pub fn find_by_field(&mut self, field_name: &str, value: &StoredValue) -> io::Result<Vec<u64>> {
        Ok(self
            .secondary_indexes
            .find_eq(field_name, value)
            .unwrap_or_default())
    }

    pub fn find_by_field_range(
        &mut self,
        field_name: &str,
        min: &StoredValue,
        max: &StoredValue,
    ) -> io::Result<Vec<u64>> {
        Ok(self
            .secondary_indexes
            .find_range(field_name, min, max)
            .unwrap_or_default())
    }

    pub fn indexed_fields(&self) -> Vec<String> {
        self.secondary_indexes.indexed_fields()
    }
}

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

    pub fn update(&self, index: u64, fields: Vec<(String, StoredValue)>) -> io::Result<()> {
        self.inner.write().unwrap().update(index, fields)
    }

    pub fn update_field(&self, index: u64, field_name: &str, value: StoredValue) -> io::Result<()> {
        self.inner
            .write()
            .unwrap()
            .update_field(index, field_name, value)
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

    pub fn get_by_uuid(&self, uuid: &Uuid7) -> io::Result<Option<CachedObject>> {
        let mut engine = self.inner.write().unwrap();
        Ok(engine.get_by_uuid(uuid)?.cloned())
    }

    pub fn stats(&self) -> StoreStats {
        self.inner.read().unwrap().stats()
    }

    pub fn compact_wal(&self) -> io::Result<(u64, u64)> {
        self.inner.write().unwrap().compact_wal()
    }

    pub fn create_secondary_index(
        &self,
        field_name: &str,
        kind: SecondaryIndexKind,
    ) -> io::Result<()> {
        self.inner
            .write()
            .unwrap()
            .create_secondary_index(field_name, kind)
    }

    pub fn drop_secondary_index(&self, field_name: &str) -> bool {
        self.inner.write().unwrap().drop_secondary_index(field_name)
    }

    pub fn find_by_field(&self, field_name: &str, value: &StoredValue) -> io::Result<Vec<u64>> {
        self.inner.write().unwrap().find_by_field(field_name, value)
    }

    pub fn find_by_field_range(
        &self,
        field_name: &str,
        min: &StoredValue,
        max: &StoredValue,
    ) -> io::Result<Vec<u64>> {
        self.inner
            .write()
            .unwrap()
            .find_by_field_range(field_name, min, max)
    }

    pub fn indexed_fields(&self) -> Vec<String> {
        self.inner.read().unwrap().indexed_fields()
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

        engine
            .update_field(idx, "age", StoredValue::Int(31))
            .unwrap();

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

        let idx = engine
            .create(vec![(
                "name".to_string(),
                StoredValue::String("Bob".to_string()),
            )])
            .unwrap();

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

        {
            let mut engine = StoreEngine::open("TestStore4", "default", config.clone()).unwrap();
            let idx = engine
                .create(vec![(
                    "name".to_string(),
                    StoredValue::String("Charlie".to_string()),
                )])
                .unwrap();

            uuid = engine.get(idx).unwrap().unwrap().uuid.clone();
            engine.save().unwrap();
        }

        {
            let mut engine = StoreEngine::open("TestStore4", "default", config).unwrap();
            let obj = engine.get(1).unwrap().unwrap();
            assert_eq!(obj.uuid, uuid);

            let name = obj.fields.iter().find(|(n, _)| n == "name").unwrap();
            assert_eq!(name.1, StoredValue::String("Charlie".to_string()));
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_shared_engine_get_by_uuid() {
        let path = unique_test_path("shared_uuid");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("TestUuid", &path);
        let engine = StoreEngine::open("TestUuid", "default", config).unwrap();
        let shared = SharedStoreEngine::new(engine);

        let fields = vec![
            ("name".to_string(), StoredValue::String("Diana".to_string())),
            ("score".to_string(), StoredValue::Int(100)),
        ];
        let idx = shared.create(fields).unwrap();

        let obj = shared.get(idx).unwrap().unwrap();
        let uuid = obj.uuid.clone();

        let found = shared.get_by_uuid(&uuid).unwrap().unwrap();
        assert_eq!(found.index, idx);
        assert_eq!(found.fields.len(), 2);

        let name = found.fields.iter().find(|(n, _)| n == "name").unwrap();
        assert_eq!(name.1, StoredValue::String("Diana".to_string()));

        let fake = Uuid7::new();
        assert!(shared.get_by_uuid(&fake).unwrap().is_none());

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_engine_full_persistence_cycle() {
        let path = unique_test_path("full_persist");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("FullPersist", &path);
        let uuid1: Uuid7;
        let uuid2: Uuid7;

        {
            let mut engine = StoreEngine::open("FullPersist", "default", config.clone()).unwrap();

            let idx1 = engine
                .create(vec![
                    ("name".to_string(), StoredValue::String("Alice".to_string())),
                    ("age".to_string(), StoredValue::Int(30)),
                ])
                .unwrap();

            let idx2 = engine
                .create(vec![
                    ("name".to_string(), StoredValue::String("Bob".to_string())),
                    ("active".to_string(), StoredValue::Bool(true)),
                ])
                .unwrap();

            uuid1 = engine.get(idx1).unwrap().unwrap().uuid.clone();
            uuid2 = engine.get(idx2).unwrap().unwrap().uuid.clone();

            engine
                .update_field(idx1, "age", StoredValue::Int(31))
                .unwrap();

            engine.delete(idx2).unwrap();

            engine.save().unwrap();
        }

        {
            let mut engine = StoreEngine::open("FullPersist", "default", config.clone()).unwrap();

            let alice = engine.get(1).unwrap().unwrap();
            assert_eq!(alice.uuid, uuid1);

            let age = alice.fields.iter().find(|(n, _)| n == "age").unwrap();
            assert_eq!(age.1, StoredValue::Int(31));

            let bob = engine.get(2).unwrap().unwrap();
            assert_eq!(bob.uuid, uuid2);
            assert!(bob.is_deleted());

            let found = engine.get_by_uuid(&uuid1).unwrap().unwrap();
            assert_eq!(found.index, 1);

            assert!(engine.count() >= 1);
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_compact_wal_reduces_size() {
        let path = unique_test_path("compact_size");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("CompactSize", &path);
        let mut engine = StoreEngine::open("CompactSize", "default", config).unwrap();

        let idx = engine
            .create(vec![("counter".to_string(), StoredValue::Int(0))])
            .unwrap();

        for i in 1..=50 {
            engine
                .update_field(idx, "counter", StoredValue::Int(i))
                .unwrap();
        }

        let (old_size, new_size) = engine.compact_wal().unwrap();
        assert!(old_size > 0, "WAL should have had data before compaction");
        assert!(
            new_size < old_size,
            "Compacted WAL ({new_size}) should be smaller than original ({old_size})"
        );

        let obj = engine.get(idx).unwrap().unwrap();
        let counter = obj.fields.iter().find(|(n, _)| n == "counter").unwrap();
        assert_eq!(counter.1, StoredValue::Int(50));

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_compact_wal_preserves_data_integrity() {
        let path = unique_test_path("compact_integrity");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("CompactInteg", &path);
        let mut engine = StoreEngine::open("CompactInteg", "default", config.clone()).unwrap();

        let idx1 = engine
            .create(vec![
                ("name".to_string(), StoredValue::String("Alice".to_string())),
                ("score".to_string(), StoredValue::Int(100)),
            ])
            .unwrap();

        let idx2 = engine
            .create(vec![
                ("name".to_string(), StoredValue::String("Bob".to_string())),
                ("score".to_string(), StoredValue::Int(200)),
            ])
            .unwrap();

        let idx3 = engine
            .create(vec![
                (
                    "name".to_string(),
                    StoredValue::String("Charlie".to_string()),
                ),
                ("score".to_string(), StoredValue::Int(300)),
            ])
            .unwrap();

        engine
            .update_field(idx1, "score", StoredValue::Int(150))
            .unwrap();
        engine
            .update_field(idx2, "score", StoredValue::Int(250))
            .unwrap();

        engine.delete(idx3).unwrap();

        engine.compact_wal().unwrap();

        let a = engine.get(idx1).unwrap().unwrap();
        let a_score = a.fields.iter().find(|(n, _)| n == "score").unwrap();
        assert_eq!(a_score.1, StoredValue::Int(150));

        let b = engine.get(idx2).unwrap().unwrap();
        let b_score = b.fields.iter().find(|(n, _)| n == "score").unwrap();
        assert_eq!(b_score.1, StoredValue::Int(250));

        let c = engine.get(idx3).unwrap().unwrap();
        assert!(c.is_deleted());

        let idx4 = engine
            .create(vec![(
                "name".to_string(),
                StoredValue::String("Diana".to_string()),
            )])
            .unwrap();
        assert!(idx4 > idx3);
        let d = engine.get(idx4).unwrap().unwrap();
        let d_name = d.fields.iter().find(|(n, _)| n == "name").unwrap();
        assert_eq!(d_name.1, StoredValue::String("Diana".to_string()));

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_compact_wal_concurrent_read() {
        let path = unique_test_path("compact_concurrent");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("CompactConc", &path);
        let engine = StoreEngine::open("CompactConc", "default", config).unwrap();
        let shared = SharedStoreEngine::new(engine);

        for i in 0..10 {
            let idx = shared
                .create(vec![("val".to_string(), StoredValue::Int(i))])
                .unwrap();

            for j in 1..=5 {
                shared
                    .update_field(idx, "val", StoredValue::Int(i * 100 + j))
                    .unwrap();
            }
        }

        let reader = shared.clone();
        let read_handle = std::thread::spawn(move || {
            let mut reads = 0u32;
            for _ in 0..100 {
                for idx in 1..=10 {
                    if let Ok(Some(obj)) = reader.get(idx) {
                        assert_eq!(obj.index, idx);
                        reads += 1;
                    }
                }
            }
            reads
        });

        let result = shared.compact_wal();
        assert!(
            result.is_ok(),
            "Compaction should succeed: {:?}",
            result.err()
        );

        let reads = read_handle.join().expect("Reader thread should not panic");
        assert!(reads > 0, "Reader should have completed some reads");

        for idx in 1..=10u64 {
            let obj = shared.get(idx).unwrap().unwrap();
            let val = obj.fields.iter().find(|(n, _)| n == "val").unwrap();
            if let StoredValue::Int(v) = &val.1 {
                assert_eq!(
                    *v,
                    (idx as i64 - 1) * 100 + 5,
                    "Object {idx} should have final update value"
                );
            } else {
                panic!("Expected Int value for object {idx}, got {:?}", val.1);
            }
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_wal_recovery_after_crash() {
        let path = unique_test_path("wal_crash_recovery");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("WalCrash", &path);

        {
            let mut engine = StoreEngine::open("WalCrash", "default", config.clone()).unwrap();
            engine
                .create(vec![
                    ("name".to_string(), StoredValue::String("Alice".to_string())),
                    ("score".to_string(), StoredValue::Int(100)),
                ])
                .unwrap();
            engine
                .create(vec![
                    ("name".to_string(), StoredValue::String("Bob".to_string())),
                    ("score".to_string(), StoredValue::Int(200)),
                ])
                .unwrap();

            if let Some(ref mut wal) = engine.wal_writer {
                wal.commit().unwrap();
            }
        }

        {
            let mut engine = StoreEngine::open("WalCrash", "default", config.clone()).unwrap();

            let obj1 = engine.get(1).unwrap();
            assert!(obj1.is_some(), "Object 1 should be recovered from WAL");
            let obj1 = obj1.unwrap();
            let name = obj1.fields.iter().find(|(n, _)| n == "name").unwrap();
            assert_eq!(name.1, StoredValue::String("Alice".to_string()));

            let obj2 = engine.get(2).unwrap();
            assert!(obj2.is_some(), "Object 2 should be recovered from WAL");
            let obj2 = obj2.unwrap();
            let name2 = obj2.fields.iter().find(|(n, _)| n == "name").unwrap();
            assert_eq!(name2.1, StoredValue::String("Bob".to_string()));
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_wal_recovery_with_updates() {
        let path = unique_test_path("wal_update_recovery");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("WalUpd", &path);

        {
            let mut engine = StoreEngine::open("WalUpd", "default", config.clone()).unwrap();
            engine
                .create(vec![
                    ("name".to_string(), StoredValue::String("Alice".to_string())),
                    ("score".to_string(), StoredValue::Int(100)),
                ])
                .unwrap();
            engine
                .update_field(1, "score", StoredValue::Int(999))
                .unwrap();
            if let Some(ref mut wal) = engine.wal_writer {
                wal.commit().unwrap();
            }
        }

        {
            let mut engine = StoreEngine::open("WalUpd", "default", config).unwrap();
            let obj = engine.get(1).unwrap().unwrap();
            let score = obj.fields.iter().find(|(n, _)| n == "score").unwrap();
            assert_eq!(
                score.1,
                StoredValue::Int(999),
                "Updated value should survive crash"
            );
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_wal_recovery_with_deletes() {
        let path = unique_test_path("wal_delete_recovery");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("WalDel", &path);

        {
            let mut engine = StoreEngine::open("WalDel", "default", config.clone()).unwrap();
            engine
                .create(vec![(
                    "name".to_string(),
                    StoredValue::String("ToDelete".to_string()),
                )])
                .unwrap();
            engine
                .create(vec![(
                    "name".to_string(),
                    StoredValue::String("Keep".to_string()),
                )])
                .unwrap();
            engine.delete(1).unwrap();
            if let Some(ref mut wal) = engine.wal_writer {
                wal.commit().unwrap();
            }
        }

        {
            let mut engine = StoreEngine::open("WalDel", "default", config).unwrap();
            let obj1 = engine.get(1).unwrap().unwrap();
            assert!(
                obj1.is_deleted(),
                "Deleted object should remain deleted after recovery"
            );

            let obj2 = engine.get(2).unwrap().unwrap();
            assert!(
                !obj2.is_deleted(),
                "Non-deleted object should survive recovery"
            );
        }

        let _ = fs::remove_dir_all(&path);
    }

    #[test]
    fn test_wal_recovery_checkpoint_then_crash() {
        let path = unique_test_path("wal_checkpoint_recovery");
        let _ = fs::remove_dir_all(&path);

        let config = StoreConfig::minimal("WalCkpt", &path);

        {
            let mut engine = StoreEngine::open("WalCkpt", "default", config.clone()).unwrap();

            engine
                .create(vec![(
                    "phase".to_string(),
                    StoredValue::String("before".to_string()),
                )])
                .unwrap();
            engine.save().unwrap();
            engine.checkpoint().unwrap();

            engine
                .create(vec![(
                    "phase".to_string(),
                    StoredValue::String("after".to_string()),
                )])
                .unwrap();
            if let Some(ref mut wal) = engine.wal_writer {
                wal.commit().unwrap();
            }
        }

        {
            let mut engine = StoreEngine::open("WalCkpt", "default", config).unwrap();
            let obj1 = engine.get(1).unwrap();
            assert!(obj1.is_some(), "Pre-checkpoint object should exist");

            let obj2 = engine.get(2).unwrap();
            assert!(
                obj2.is_some(),
                "Post-checkpoint object should be recovered from WAL"
            );
            let phase = obj2
                .unwrap()
                .fields
                .iter()
                .find(|(n, _)| n == "phase")
                .unwrap();
            assert_eq!(phase.1, StoredValue::String("after".to_string()));
        }

        let _ = fs::remove_dir_all(&path);
    }
}
