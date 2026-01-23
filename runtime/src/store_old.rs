//! Persistent store implementation for Coral
//!
//! Provides disk persistence for stores with automatic default fields:
//! - `_index`: Auto-increment primary key  
//! - `_uuid`: UUID identifier
//! - `_created_at`: Unix timestamp of creation
//! - `_updated_at`: Unix timestamp of last update
//! - `_deleted_at`: Unix timestamp of soft deletion (0 if not deleted)
//! - `_version`: Optimistic concurrency version

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Magic bytes for store files
const STORE_MAGIC: &[u8; 8] = b"CORALST\x01";

/// Current store file version
const STORE_VERSION: u32 = 1;

/// Auto-increment counter for store indices
static NEXT_INDEX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// UUID generation using a simple counter + timestamp + random
fn generate_uuid() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::hash::Hasher;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let random: u64 = std::hash::BuildHasher::build_hasher(&std::collections::hash_map::RandomState::new()).finish();
    
    format!("{:016x}-{:04x}-{:04x}", now, count as u16, random as u16)
}

/// Get current Unix timestamp in seconds
fn current_timestamp() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Metadata for default store fields
#[repr(C)]
#[derive(Debug, Clone)]
pub struct StoreMetadata {
    pub index: u64,
    pub uuid: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: i64,  // 0 means not deleted
    pub version: u64,
}

impl StoreMetadata {
    /// Create new metadata with auto-generated values
    pub fn new() -> Self {
        let now = current_timestamp();
        Self {
            index: NEXT_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            uuid: generate_uuid(),
            created_at: now,
            updated_at: now,
            deleted_at: 0,
            version: 1,
        }
    }

    /// Mark as updated, incrementing version
    pub fn touch(&mut self) {
        self.updated_at = current_timestamp();
        self.version += 1;
    }

    /// Soft delete
    pub fn soft_delete(&mut self) {
        self.deleted_at = current_timestamp();
        self.touch();
    }

    /// Undelete
    pub fn restore(&mut self) {
        self.deleted_at = 0;
        self.touch();
    }

    /// Check if soft deleted
    pub fn is_deleted(&self) -> bool {
        self.deleted_at != 0
    }
}

impl Default for StoreMetadata {
    fn default() -> Self {
        Self::new()
    }
}

/// A stored value that can be serialized to disk
#[derive(Debug, Clone)]
pub enum StoredValue {
    Unit,
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<StoredValue>),
    Map(Vec<(StoredValue, StoredValue)>),
}

/// Type tags for binary serialization
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ValueTag {
    Unit = 0x00,
    Bool = 0x01,
    Int = 0x02,
    Float = 0x03,
    String = 0x04,
    Bytes = 0x05,
    List = 0x06,
    Map = 0x07,
    None = 0x08,
}

impl StoredValue {
    /// Serialize to bytes
    pub fn serialize(&self, buf: &mut Vec<u8>) {
        match self {
            StoredValue::Unit => buf.push(ValueTag::Unit as u8),
            StoredValue::None => buf.push(ValueTag::None as u8),
            StoredValue::Bool(b) => {
                buf.push(ValueTag::Bool as u8);
                buf.push(if *b { 1 } else { 0 });
            }
            StoredValue::Int(i) => {
                buf.push(ValueTag::Int as u8);
                buf.extend_from_slice(&i.to_le_bytes());
            }
            StoredValue::Float(f) => {
                buf.push(ValueTag::Float as u8);
                buf.extend_from_slice(&f.to_le_bytes());
            }
            StoredValue::String(s) => {
                buf.push(ValueTag::String as u8);
                let bytes = s.as_bytes();
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
            StoredValue::Bytes(b) => {
                buf.push(ValueTag::Bytes as u8);
                buf.extend_from_slice(&(b.len() as u32).to_le_bytes());
                buf.extend_from_slice(b);
            }
            StoredValue::List(items) => {
                buf.push(ValueTag::List as u8);
                buf.extend_from_slice(&(items.len() as u32).to_le_bytes());
                for item in items {
                    item.serialize(buf);
                }
            }
            StoredValue::Map(pairs) => {
                buf.push(ValueTag::Map as u8);
                buf.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
                for (k, v) in pairs {
                    k.serialize(buf);
                    v.serialize(buf);
                }
            }
        }
    }

    /// Deserialize from bytes
    pub fn deserialize(data: &[u8], pos: &mut usize) -> io::Result<Self> {
        if *pos >= data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "unexpected end of data"));
        }
        
        let tag = data[*pos];
        *pos += 1;
        
        match tag {
            x if x == ValueTag::Unit as u8 => Ok(StoredValue::Unit),
            x if x == ValueTag::None as u8 => Ok(StoredValue::None),
            x if x == ValueTag::Bool as u8 => {
                if *pos >= data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing bool value"));
                }
                let v = data[*pos] != 0;
                *pos += 1;
                Ok(StoredValue::Bool(v))
            }
            x if x == ValueTag::Int as u8 => {
                if *pos + 8 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing int value"));
                }
                let v = i64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Ok(StoredValue::Int(v))
            }
            x if x == ValueTag::Float as u8 => {
                if *pos + 8 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing float value"));
                }
                let v = f64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Ok(StoredValue::Float(v))
            }
            x if x == ValueTag::String as u8 => {
                if *pos + 4 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing string length"));
                }
                let len = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap()) as usize;
                *pos += 4;
                if *pos + len > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing string data"));
                }
                let s = String::from_utf8_lossy(&data[*pos..*pos + len]).to_string();
                *pos += len;
                Ok(StoredValue::String(s))
            }
            x if x == ValueTag::Bytes as u8 => {
                if *pos + 4 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing bytes length"));
                }
                let len = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap()) as usize;
                *pos += 4;
                if *pos + len > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing bytes data"));
                }
                let b = data[*pos..*pos + len].to_vec();
                *pos += len;
                Ok(StoredValue::Bytes(b))
            }
            x if x == ValueTag::List as u8 => {
                if *pos + 4 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing list length"));
                }
                let len = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap()) as usize;
                *pos += 4;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(Self::deserialize(data, pos)?);
                }
                Ok(StoredValue::List(items))
            }
            x if x == ValueTag::Map as u8 => {
                if *pos + 4 > data.len() {
                    return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing map length"));
                }
                let len = u32::from_le_bytes(data[*pos..*pos + 4].try_into().unwrap()) as usize;
                *pos += 4;
                let mut pairs = Vec::with_capacity(len);
                for _ in 0..len {
                    let k = Self::deserialize(data, pos)?;
                    let v = Self::deserialize(data, pos)?;
                    pairs.push((k, v));
                }
                Ok(StoredValue::Map(pairs))
            }
            _ => Err(io::Error::new(io::ErrorKind::InvalidData, format!("unknown value tag: {}", tag))),
        }
    }
}

/// Represents a stored record (store instance)
#[derive(Debug, Clone)]
pub struct StoreRecord {
    /// System metadata
    pub metadata: StoreMetadata,
    /// User-defined fields: field_name -> value
    pub fields: HashMap<String, StoredValue>,
}

impl StoreRecord {
    pub fn new() -> Self {
        Self {
            metadata: StoreMetadata::new(),
            fields: HashMap::new(),
        }
    }

    /// Serialize the entire record to bytes
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        
        // Metadata
        buf.extend_from_slice(&self.metadata.index.to_le_bytes());
        let uuid_bytes = self.metadata.uuid.as_bytes();
        buf.extend_from_slice(&(uuid_bytes.len() as u32).to_le_bytes());
        buf.extend_from_slice(uuid_bytes);
        buf.extend_from_slice(&self.metadata.created_at.to_le_bytes());
        buf.extend_from_slice(&self.metadata.updated_at.to_le_bytes());
        buf.extend_from_slice(&self.metadata.deleted_at.to_le_bytes());
        buf.extend_from_slice(&self.metadata.version.to_le_bytes());
        
        // Fields
        buf.extend_from_slice(&(self.fields.len() as u32).to_le_bytes());
        for (name, value) in &self.fields {
            let name_bytes = name.as_bytes();
            buf.extend_from_slice(&(name_bytes.len() as u32).to_le_bytes());
            buf.extend_from_slice(name_bytes);
            value.serialize(&mut buf);
        }
        
        buf
    }

    /// Deserialize from bytes
    pub fn deserialize(data: &[u8]) -> io::Result<Self> {
        let mut pos = 0;
        
        // Metadata
        if pos + 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing index"));
        }
        let index = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        if pos + 4 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing uuid length"));
        }
        let uuid_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        
        if pos + uuid_len > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing uuid data"));
        }
        let uuid = String::from_utf8_lossy(&data[pos..pos + uuid_len]).to_string();
        pos += uuid_len;
        
        if pos + 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing created_at"));
        }
        let created_at = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        if pos + 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing updated_at"));
        }
        let updated_at = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        if pos + 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing deleted_at"));
        }
        let deleted_at = i64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        if pos + 8 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing version"));
        }
        let version = u64::from_le_bytes(data[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        // Fields
        if pos + 4 > data.len() {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing field count"));
        }
        let field_count = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        
        let mut fields = HashMap::with_capacity(field_count);
        for _ in 0..field_count {
            if pos + 4 > data.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing field name length"));
            }
            let name_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
            pos += 4;
            
            if pos + name_len > data.len() {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "missing field name"));
            }
            let name = String::from_utf8_lossy(&data[pos..pos + name_len]).to_string();
            pos += name_len;
            
            let value = StoredValue::deserialize(data, &mut pos)?;
            fields.insert(name, value);
        }
        
        Ok(Self {
            metadata: StoreMetadata {
                index,
                uuid,
                created_at,
                updated_at,
                deleted_at,
                version,
            },
            fields,
        })
    }
}

/// File header for store files
#[derive(Debug)]
pub struct StoreFileHeader {
    pub magic: [u8; 8],
    pub version: u32,
    pub store_type_hash: u64,
    pub created: u64,
    pub modified: u64,
    pub record_count: u64,
}

impl StoreFileHeader {
    pub fn new(store_type: &str) -> Self {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        
        let mut hasher = DefaultHasher::new();
        store_type.hash(&mut hasher);
        let type_hash = hasher.finish();
        
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        
        Self {
            magic: *STORE_MAGIC,
            version: STORE_VERSION,
            store_type_hash: type_hash,
            created: now,
            modified: now,
            record_count: 0,
        }
    }

    pub fn serialize(&self) -> [u8; 48] {
        let mut buf = [0u8; 48];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..20].copy_from_slice(&self.store_type_hash.to_le_bytes());
        buf[20..28].copy_from_slice(&self.created.to_le_bytes());
        buf[28..36].copy_from_slice(&self.modified.to_le_bytes());
        buf[36..44].copy_from_slice(&self.record_count.to_le_bytes());
        // 4 bytes padding
        buf
    }

    pub fn deserialize(data: &[u8; 48]) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&data[0..8]);
        
        if &magic != STORE_MAGIC {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid store file magic"));
        }
        
        Ok(Self {
            magic,
            version: u32::from_le_bytes(data[8..12].try_into().unwrap()),
            store_type_hash: u64::from_le_bytes(data[12..20].try_into().unwrap()),
            created: u64::from_le_bytes(data[20..28].try_into().unwrap()),
            modified: u64::from_le_bytes(data[28..36].try_into().unwrap()),
            record_count: u64::from_le_bytes(data[36..44].try_into().unwrap()),
        })
    }
}

/// A persistent store that saves records to disk
pub struct PersistentStore {
    /// Store type name
    type_name: String,
    /// Path to the store file
    path: PathBuf,
    /// In-memory cache of records by index
    records: RwLock<HashMap<u64, StoreRecord>>,
    /// Whether changes need to be flushed
    dirty: std::sync::atomic::AtomicBool,
}

impl PersistentStore {
    /// Create or open a persistent store
    pub fn open(type_name: &str, path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        let mut store = Self {
            type_name: type_name.to_string(),
            path,
            records: RwLock::new(HashMap::new()),
            dirty: std::sync::atomic::AtomicBool::new(false),
        };
        
        // Load existing data if file exists
        if store.path.exists() {
            store.load()?;
        }
        
        Ok(store)
    }

    /// Load records from disk
    fn load(&mut self) -> io::Result<()> {
        let file = File::open(&self.path)?;
        let mut reader = BufReader::new(file);
        
        // Read header
        let mut header_buf = [0u8; 48];
        reader.read_exact(&mut header_buf)?;
        let header = StoreFileHeader::deserialize(&header_buf)?;
        
        // Read records
        let mut records = HashMap::new();
        for _ in 0..header.record_count {
            // Read record length
            let mut len_buf = [0u8; 4];
            if reader.read_exact(&mut len_buf).is_err() {
                break; // End of file
            }
            let len = u32::from_le_bytes(len_buf) as usize;
            
            // Read record data
            let mut record_buf = vec![0u8; len];
            reader.read_exact(&mut record_buf)?;
            
            let record = StoreRecord::deserialize(&record_buf)?;
            records.insert(record.metadata.index, record);
        }
        
        // Update the global index counter to be greater than any existing index
        let max_index = records.keys().copied().max().unwrap_or(0);
        let current = NEXT_INDEX.load(std::sync::atomic::Ordering::Relaxed);
        if max_index >= current {
            NEXT_INDEX.store(max_index + 1, std::sync::atomic::Ordering::Relaxed);
        }
        
        *self.records.write().unwrap() = records;
        Ok(())
    }

    /// Save all records to disk
    pub fn save(&self) -> io::Result<()> {
        let records = self.records.read().unwrap();
        
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;
        let mut writer = BufWriter::new(file);
        
        // Write header
        let mut header = StoreFileHeader::new(&self.type_name);
        header.record_count = records.len() as u64;
        header.modified = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        writer.write_all(&header.serialize())?;
        
        // Write records
        for record in records.values() {
            let data = record.serialize();
            writer.write_all(&(data.len() as u32).to_le_bytes())?;
            writer.write_all(&data)?;
        }
        
        writer.flush()?;
        self.dirty.store(false, std::sync::atomic::Ordering::Relaxed);
        
        Ok(())
    }

    /// Insert a new record
    pub fn insert(&self, record: StoreRecord) -> u64 {
        let index = record.metadata.index;
        self.records.write().unwrap().insert(index, record);
        self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        index
    }

    /// Get a record by index
    pub fn get(&self, index: u64) -> Option<StoreRecord> {
        self.records.read().unwrap().get(&index).cloned()
    }

    /// Update a record
    pub fn update(&self, index: u64, f: impl FnOnce(&mut StoreRecord)) -> bool {
        let mut records = self.records.write().unwrap();
        if let Some(record) = records.get_mut(&index) {
            f(record);
            record.metadata.touch();
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
            true
        } else {
            false
        }
    }

    /// Soft delete a record
    pub fn soft_delete(&self, index: u64) -> bool {
        self.update(index, |r| r.metadata.soft_delete())
    }

    /// Hard delete a record
    pub fn delete(&self, index: u64) -> bool {
        let removed = self.records.write().unwrap().remove(&index).is_some();
        if removed {
            self.dirty.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        removed
    }

    /// Get all records (excluding soft-deleted)
    pub fn all(&self) -> Vec<StoreRecord> {
        self.records.read().unwrap()
            .values()
            .filter(|r| !r.metadata.is_deleted())
            .cloned()
            .collect()
    }

    /// Get all records including soft-deleted
    pub fn all_with_deleted(&self) -> Vec<StoreRecord> {
        self.records.read().unwrap().values().cloned().collect()
    }

    /// Query records by a predicate
    pub fn query(&self, predicate: impl Fn(&StoreRecord) -> bool) -> Vec<StoreRecord> {
        self.records.read().unwrap()
            .values()
            .filter(|r| !r.metadata.is_deleted() && predicate(r))
            .cloned()
            .collect()
    }

    /// Get record count (excluding soft-deleted)
    pub fn count(&self) -> usize {
        self.records.read().unwrap()
            .values()
            .filter(|r| !r.metadata.is_deleted())
            .count()
    }

    /// Check if store has unsaved changes
    pub fn is_dirty(&self) -> bool {
        self.dirty.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl Drop for PersistentStore {
    fn drop(&mut self) {
        if self.is_dirty() {
            // Try to save on drop, but don't panic on failure
            let _ = self.save();
        }
    }
}

/// Global registry of open stores
static STORE_REGISTRY: OnceLock<Mutex<HashMap<String, Arc<PersistentStore>>>> = OnceLock::new();

fn get_registry() -> &'static Mutex<HashMap<String, Arc<PersistentStore>>> {
    STORE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Open or create a persistent store by name
pub fn open_store(type_name: &str, path: &str) -> io::Result<Arc<PersistentStore>> {
    let key = format!("{}:{}", type_name, path);
    let mut registry = get_registry().lock().unwrap();
    
    if let Some(store) = registry.get(&key) {
        return Ok(Arc::clone(store));
    }
    
    let store = Arc::new(PersistentStore::open(type_name, path)?);
    registry.insert(key, Arc::clone(&store));
    Ok(store)
}

/// Save all open stores
pub fn save_all_stores() -> io::Result<()> {
    let registry = get_registry().lock().unwrap();
    for store in registry.values() {
        store.save()?;
    }
    Ok(())
}

/// Close a store (removes from registry)
pub fn close_store(type_name: &str, path: &str) -> bool {
    let key = format!("{}:{}", type_name, path);
    let mut registry = get_registry().lock().unwrap();
    registry.remove(&key).is_some()
}

use std::sync::OnceLock;

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_store_metadata() {
        let meta = StoreMetadata::new();
        assert!(meta.index > 0);
        assert!(!meta.uuid.is_empty());
        assert!(meta.created_at > 0);
        assert_eq!(meta.deleted_at, 0);
        assert_eq!(meta.version, 1);
        assert!(!meta.is_deleted());
    }

    #[test]
    fn test_stored_value_serialization() {
        let values = vec![
            StoredValue::Unit,
            StoredValue::None,
            StoredValue::Bool(true),
            StoredValue::Int(-42),
            StoredValue::Float(3.14159),
            StoredValue::String("hello world".to_string()),
            StoredValue::Bytes(vec![1, 2, 3, 4]),
            StoredValue::List(vec![
                StoredValue::Int(1),
                StoredValue::Int(2),
                StoredValue::String("three".to_string()),
            ]),
            StoredValue::Map(vec![
                (StoredValue::String("key".to_string()), StoredValue::Int(42)),
            ]),
        ];

        for value in values {
            let mut buf = Vec::new();
            value.serialize(&mut buf);
            let mut pos = 0;
            let deserialized = StoredValue::deserialize(&buf, &mut pos).unwrap();
            
            // Re-serialize to compare
            let mut buf2 = Vec::new();
            deserialized.serialize(&mut buf2);
            assert_eq!(buf, buf2);
        }
    }

    #[test]
    fn test_store_record_serialization() {
        let mut record = StoreRecord::new();
        record.fields.insert("name".to_string(), StoredValue::String("Alice".to_string()));
        record.fields.insert("age".to_string(), StoredValue::Int(30));
        
        let data = record.serialize();
        let restored = StoreRecord::deserialize(&data).unwrap();
        
        assert_eq!(record.metadata.index, restored.metadata.index);
        assert_eq!(record.metadata.uuid, restored.metadata.uuid);
        assert_eq!(record.fields.len(), restored.fields.len());
    }

    #[test]
    fn test_persistent_store() {
        let path = "/tmp/coral_test_store.dat";
        
        // Clean up from previous runs
        let _ = fs::remove_file(path);
        
        {
            let store = PersistentStore::open("TestStore", path).unwrap();
            
            let mut record1 = StoreRecord::new();
            record1.fields.insert("value".to_string(), StoredValue::Int(100));
            let idx1 = store.insert(record1);
            
            let mut record2 = StoreRecord::new();
            record2.fields.insert("value".to_string(), StoredValue::Int(200));
            let idx2 = store.insert(record2);
            
            assert_eq!(store.count(), 2);
            
            // Soft delete one
            store.soft_delete(idx1);
            assert_eq!(store.count(), 1);
            
            store.save().unwrap();
        }
        
        // Reopen and verify
        {
            let store = PersistentStore::open("TestStore", path).unwrap();
            assert_eq!(store.count(), 1); // Soft-deleted should still be excluded from count
            assert_eq!(store.all_with_deleted().len(), 2); // But still present
        }
        
        // Clean up
        let _ = fs::remove_file(path);
    }
}
