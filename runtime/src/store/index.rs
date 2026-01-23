//! Primary index implementation for Coral persistent stores
//!
//! The primary index maps sequential IDs (_index) to UUIDs and storage offsets.
//! This enables fast lookups by either _index or _uuid.
//!
//! File format (per spec):
//! - Header: 64 bytes
//! - Index entries: 64 bytes each, sorted by _index
//! - Bloom filter: variable size at end of file

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::uuid7::Uuid7;
use super::{INDEX_MAGIC, INDEX_VERSION, IndexFlags};

/// Header size in bytes
const HEADER_SIZE: usize = 64;

/// Entry size in bytes (fixed for binary search)
const ENTRY_SIZE: usize = 64;

/// Index file header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct IndexHeader {
    /// Magic bytes: "CORALIDX"
    pub magic: [u8; 8],
    /// File format version
    pub version: u32,
    /// Number of entries
    pub entry_count: u64,
    /// Next auto-increment value
    pub next_index: u64,
    /// Offset to bloom filter (0 if none)
    pub bloom_filter_offset: u64,
    /// Checksum of header
    pub checksum: u64,
    /// Reserved for future use
    pub _reserved: [u8; 16],
}

impl IndexHeader {
    pub fn new() -> Self {
        Self {
            magic: *INDEX_MAGIC,
            version: INDEX_VERSION,
            entry_count: 0,
            next_index: 1,
            bloom_filter_offset: 0,
            checksum: 0,
            _reserved: [0; 16],
        }
    }
    
    pub fn serialize(&self) -> [u8; HEADER_SIZE] {
        let mut buf = [0u8; HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..20].copy_from_slice(&self.entry_count.to_le_bytes());
        buf[20..28].copy_from_slice(&self.next_index.to_le_bytes());
        buf[28..36].copy_from_slice(&self.bloom_filter_offset.to_le_bytes());
        buf[36..44].copy_from_slice(&self.checksum.to_le_bytes());
        // Reserved bytes are already zero
        buf
    }
    
    pub fn deserialize(data: &[u8; HEADER_SIZE]) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&data[0..8]);
        
        if &magic != INDEX_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid index file magic",
            ));
        }
        
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version > INDEX_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported index version: {}", version),
            ));
        }
        
        Ok(Self {
            magic,
            version,
            entry_count: u64::from_le_bytes(data[12..20].try_into().unwrap()),
            next_index: u64::from_le_bytes(data[20..28].try_into().unwrap()),
            bloom_filter_offset: u64::from_le_bytes(data[28..36].try_into().unwrap()),
            checksum: u64::from_le_bytes(data[36..44].try_into().unwrap()),
            _reserved: [0; 16],
        })
    }
}

impl Default for IndexHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// A single index entry (64 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntry {
    /// Sequential ID
    pub index: u64,
    /// UUIDv7 bytes
    pub uuid: [u8; 16],
    /// Offset in data.bin (0 = deleted)
    pub binary_offset: u64,
    /// Length in data.bin
    pub binary_length: u32,
    /// Offset in data.jsonl
    pub json_offset: u64,
    /// Length in data.jsonl
    pub json_length: u32,
    /// Flags (deleted, compressed, etc.)
    pub flags: u16,
    /// Optimistic concurrency version
    pub version: u16,
}

impl IndexEntry {
    pub fn new(index: u64, uuid: Uuid7) -> Self {
        Self {
            index,
            uuid: uuid.to_bytes(),
            binary_offset: 0,
            binary_length: 0,
            json_offset: 0,
            json_length: 0,
            flags: IndexFlags::Active as u16,
            version: 1,
        }
    }
    
    pub fn serialize(&self) -> [u8; ENTRY_SIZE] {
        let mut buf = [0u8; ENTRY_SIZE];
        buf[0..8].copy_from_slice(&self.index.to_le_bytes());
        buf[8..24].copy_from_slice(&self.uuid);
        buf[24..32].copy_from_slice(&self.binary_offset.to_le_bytes());
        buf[32..36].copy_from_slice(&self.binary_length.to_le_bytes());
        buf[36..44].copy_from_slice(&self.json_offset.to_le_bytes());
        buf[44..48].copy_from_slice(&self.json_length.to_le_bytes());
        buf[48..50].copy_from_slice(&self.flags.to_le_bytes());
        buf[50..52].copy_from_slice(&self.version.to_le_bytes());
        // Remaining 12 bytes are padding/reserved
        buf
    }
    
    pub fn deserialize(data: &[u8; ENTRY_SIZE]) -> Self {
        let mut uuid = [0u8; 16];
        uuid.copy_from_slice(&data[8..24]);
        
        Self {
            index: u64::from_le_bytes(data[0..8].try_into().unwrap()),
            uuid,
            binary_offset: u64::from_le_bytes(data[24..32].try_into().unwrap()),
            binary_length: u32::from_le_bytes(data[32..36].try_into().unwrap()),
            json_offset: u64::from_le_bytes(data[36..44].try_into().unwrap()),
            json_length: u32::from_le_bytes(data[44..48].try_into().unwrap()),
            flags: u16::from_le_bytes(data[48..50].try_into().unwrap()),
            version: u16::from_le_bytes(data[50..52].try_into().unwrap()),
        }
    }
    
    /// Get the UUID as a Uuid7
    pub fn uuid(&self) -> Uuid7 {
        Uuid7::from_bytes(self.uuid)
    }
    
    /// Check if this entry is marked as deleted
    pub fn is_deleted(&self) -> bool {
        self.flags & (IndexFlags::Deleted as u16) != 0
    }
    
    /// Check if this entry is soft-deleted
    pub fn is_soft_deleted(&self) -> bool {
        self.flags & (IndexFlags::SoftDeleted as u16) != 0
    }
    
    /// Mark as deleted
    pub fn mark_deleted(&mut self) {
        self.flags |= IndexFlags::Deleted as u16;
        self.binary_offset = 0;
    }
    
    /// Mark as soft-deleted
    pub fn mark_soft_deleted(&mut self) {
        self.flags |= IndexFlags::SoftDeleted as u16;
    }
}

/// In-memory primary index
pub struct PrimaryIndex {
    /// Path to index file
    path: PathBuf,
    /// Header information
    header: IndexHeader,
    /// Entries by index
    by_index: HashMap<u64, IndexEntry>,
    /// UUID to index mapping
    uuid_to_index: HashMap<[u8; 16], u64>,
    /// Dirty flag
    dirty: bool,
}

impl PrimaryIndex {
    /// Create or open a primary index
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        if path.exists() {
            Self::load(&path)
        } else {
            Ok(Self {
                path,
                header: IndexHeader::new(),
                by_index: HashMap::new(),
                uuid_to_index: HashMap::new(),
                dirty: false,
            })
        }
    }
    
    /// Load index from file
    fn load(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let mut reader = BufReader::new(file);
        
        // Read header
        let mut header_buf = [0u8; HEADER_SIZE];
        reader.read_exact(&mut header_buf)?;
        let header = IndexHeader::deserialize(&header_buf)?;
        
        // Read entries
        let mut by_index = HashMap::with_capacity(header.entry_count as usize);
        let mut uuid_to_index = HashMap::with_capacity(header.entry_count as usize);
        
        let mut entry_buf = [0u8; ENTRY_SIZE];
        for _ in 0..header.entry_count {
            if reader.read_exact(&mut entry_buf).is_err() {
                break;
            }
            let entry = IndexEntry::deserialize(&entry_buf);
            uuid_to_index.insert(entry.uuid, entry.index);
            by_index.insert(entry.index, entry);
        }
        
        Ok(Self {
            path: path.to_path_buf(),
            header,
            by_index,
            uuid_to_index,
            dirty: false,
        })
    }
    
    /// Save index to file
    pub fn save(&mut self) -> io::Result<()> {
        if !self.dirty {
            return Ok(());
        }
        
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&self.path)?;
        let mut writer = BufWriter::new(file);
        
        // Update header
        self.header.entry_count = self.by_index.len() as u64;
        
        // Write header
        writer.write_all(&self.header.serialize())?;
        
        // Write entries (sorted by index for binary search)
        let mut entries: Vec<_> = self.by_index.values().collect();
        entries.sort_by_key(|e| e.index);
        
        for entry in entries {
            writer.write_all(&entry.serialize())?;
        }
        
        writer.flush()?;
        self.dirty = false;
        
        Ok(())
    }
    
    /// Allocate a new index
    pub fn allocate_index(&mut self) -> u64 {
        let index = self.header.next_index;
        self.header.next_index += 1;
        self.dirty = true;
        index
    }
    
    /// Get the next index value (without allocating)
    pub fn next_index(&self) -> u64 {
        self.header.next_index
    }
    
    /// Insert a new entry
    pub fn insert(&mut self, entry: IndexEntry) {
        self.uuid_to_index.insert(entry.uuid, entry.index);
        self.by_index.insert(entry.index, entry);
        self.dirty = true;
    }
    
    /// Get entry by index
    pub fn get_by_index(&self, index: u64) -> Option<&IndexEntry> {
        self.by_index.get(&index)
    }
    
    /// Get entry by UUID
    pub fn get_by_uuid(&self, uuid: &Uuid7) -> Option<&IndexEntry> {
        self.uuid_to_index
            .get(uuid.as_bytes())
            .and_then(|idx| self.by_index.get(idx))
    }
    
    /// Update an entry
    pub fn update(&mut self, entry: IndexEntry) -> bool {
        if self.by_index.contains_key(&entry.index) {
            self.by_index.insert(entry.index, entry);
            self.dirty = true;
            true
        } else {
            false
        }
    }
    
    /// Update entry offsets after write
    pub fn update_offsets(
        &mut self,
        index: u64,
        binary_offset: u64,
        binary_length: u32,
        json_offset: u64,
        json_length: u32,
    ) -> bool {
        if let Some(entry) = self.by_index.get_mut(&index) {
            entry.binary_offset = binary_offset;
            entry.binary_length = binary_length;
            entry.json_offset = json_offset;
            entry.json_length = json_length;
            self.dirty = true;
            true
        } else {
            false
        }
    }
    
    /// Mark entry as deleted
    pub fn mark_deleted(&mut self, index: u64) -> bool {
        if let Some(entry) = self.by_index.get_mut(&index) {
            entry.mark_deleted();
            self.dirty = true;
            true
        } else {
            false
        }
    }
    
    /// Mark entry as soft-deleted
    pub fn mark_soft_deleted(&mut self, index: u64) -> bool {
        if let Some(entry) = self.by_index.get_mut(&index) {
            entry.mark_soft_deleted();
            self.dirty = true;
            true
        } else {
            false
        }
    }
    
    /// Increment version for an entry
    pub fn increment_version(&mut self, index: u64) -> Option<u16> {
        if let Some(entry) = self.by_index.get_mut(&index) {
            entry.version = entry.version.saturating_add(1);
            self.dirty = true;
            Some(entry.version)
        } else {
            None
        }
    }
    
    /// Get all non-deleted entries
    pub fn all_active(&self) -> Vec<&IndexEntry> {
        self.by_index
            .values()
            .filter(|e| !e.is_deleted())
            .collect()
    }
    
    /// Get entry count (excluding deleted)
    pub fn count(&self) -> usize {
        self.by_index.values().filter(|e| !e.is_deleted()).count()
    }
    
    /// Get total entry count (including deleted)
    pub fn total_count(&self) -> usize {
        self.by_index.len()
    }
    
    /// Alias for total_count() - returns number of entries
    pub fn len(&self) -> u64 {
        self.by_index.len() as u64
    }
    
    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.by_index.is_empty()
    }
    
    /// Check if dirty
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
}

impl Drop for PrimaryIndex {
    fn drop(&mut self) {
        if self.dirty {
            let _ = self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    
    #[test]
    fn test_index_header_roundtrip() {
        let header = IndexHeader {
            magic: *INDEX_MAGIC,
            version: INDEX_VERSION,
            entry_count: 42,
            next_index: 100,
            bloom_filter_offset: 1234,
            checksum: 5678,
            _reserved: [0; 16],
        };
        
        let serialized = header.serialize();
        let restored = IndexHeader::deserialize(&serialized).unwrap();
        
        assert_eq!(restored.entry_count, 42);
        assert_eq!(restored.next_index, 100);
        assert_eq!(restored.bloom_filter_offset, 1234);
    }
    
    #[test]
    fn test_index_entry_roundtrip() {
        let uuid = Uuid7::new();
        let mut entry = IndexEntry::new(42, uuid);
        entry.binary_offset = 1000;
        entry.binary_length = 256;
        entry.json_offset = 2000;
        entry.json_length = 128;
        
        let serialized = entry.serialize();
        let restored = IndexEntry::deserialize(&serialized);
        
        assert_eq!(restored, entry);
    }
    
    #[test]
    fn test_primary_index_operations() {
        let path = "/tmp/coral_test_primary_index.idx";
        let _ = fs::remove_file(path);
        
        {
            let mut index = PrimaryIndex::open(path).unwrap();
            
            // Allocate and insert entries
            let idx1 = index.allocate_index();
            let uuid1 = Uuid7::new();
            let entry1 = IndexEntry::new(idx1, uuid1);
            index.insert(entry1);
            
            let idx2 = index.allocate_index();
            let uuid2 = Uuid7::new();
            let entry2 = IndexEntry::new(idx2, uuid2);
            index.insert(entry2);
            
            assert_eq!(index.count(), 2);
            
            // Lookup by index
            assert!(index.get_by_index(idx1).is_some());
            
            // Lookup by UUID
            assert!(index.get_by_uuid(&uuid1).is_some());
            
            // Update offsets
            index.update_offsets(idx1, 100, 50, 200, 75);
            let e1 = index.get_by_index(idx1).unwrap();
            assert_eq!(e1.binary_offset, 100);
            
            // Mark deleted
            index.mark_deleted(idx1);
            assert_eq!(index.count(), 1); // Only idx2 is active
            
            index.save().unwrap();
        }
        
        // Reopen and verify
        {
            let index = PrimaryIndex::open(path).unwrap();
            assert_eq!(index.total_count(), 2);
            assert_eq!(index.count(), 1);
        }
        
        let _ = fs::remove_file(path);
    }
}
