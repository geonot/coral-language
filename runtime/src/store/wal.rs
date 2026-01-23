//! Write-Ahead Log (WAL) for Coral persistent stores
//!
//! WAL provides durability guarantees by logging all changes before
//! they are applied to the main storage files.
//!
//! File format:
//! - WAL header: 64 bytes
//! - Entries: variable length
//!
//! Entry format (per spec §15.5):
//! - [8] LSN (log sequence number)
//! - [1] operation type
//! - [8] timestamp_ms
//! - [8] object _index
//! - [16] object _uuid
//! - [4] payload length
//! - [n] payload (serialized data)
//! - [4] CRC32 of entire entry

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use super::binary::StoredValue;
use super::uuid7::Uuid7;
use super::{WAL_MAGIC, WAL_VERSION, WalOpType};

/// WAL header size
const WAL_HEADER_SIZE: usize = 64;

/// Entry header size (before payload)
const ENTRY_HEADER_SIZE: usize = 45;

/// WAL segment maximum size (default 16 MB)
const DEFAULT_SEGMENT_SIZE: u64 = 16 * 1024 * 1024;

/// WAL file header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct WalHeader {
    /// Magic bytes: "CORALWAL"
    pub magic: [u8; 8],
    /// WAL format version
    pub version: u32,
    /// Segment number
    pub segment_number: u64,
    /// First LSN in this segment
    pub first_lsn: u64,
    /// Last committed LSN
    pub last_committed_lsn: u64,
    /// Store type hash
    pub store_type_hash: u64,
    /// Flags
    pub flags: u32,
    /// Reserved/padding
    pub reserved: [u8; 12],
}

impl WalHeader {
    pub fn new(store_type: &str, segment_number: u64, first_lsn: u64) -> Self {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        
        let mut hasher = DefaultHasher::new();
        store_type.hash(&mut hasher);
        let type_hash = hasher.finish();
        
        Self {
            magic: *WAL_MAGIC,
            version: WAL_VERSION,
            segment_number,
            first_lsn,
            last_committed_lsn: 0,
            store_type_hash: type_hash,
            flags: 0,
            reserved: [0u8; 12],
        }
    }
    
    pub fn serialize(&self) -> [u8; WAL_HEADER_SIZE] {
        let mut buf = [0u8; WAL_HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..20].copy_from_slice(&self.segment_number.to_le_bytes());
        buf[20..28].copy_from_slice(&self.first_lsn.to_le_bytes());
        buf[28..36].copy_from_slice(&self.last_committed_lsn.to_le_bytes());
        buf[36..44].copy_from_slice(&self.store_type_hash.to_le_bytes());
        buf[44..48].copy_from_slice(&self.flags.to_le_bytes());
        buf[48..60].copy_from_slice(&self.reserved);
        // 4 bytes padding
        buf
    }
    
    pub fn deserialize(data: &[u8; WAL_HEADER_SIZE]) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&data[0..8]);
        
        if &magic != WAL_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid WAL file magic",
            ));
        }
        
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version > WAL_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported WAL version: {}", version),
            ));
        }
        
        let mut reserved = [0u8; 12];
        reserved.copy_from_slice(&data[48..60]);
        
        Ok(Self {
            magic,
            version,
            segment_number: u64::from_le_bytes(data[12..20].try_into().unwrap()),
            first_lsn: u64::from_le_bytes(data[20..28].try_into().unwrap()),
            last_committed_lsn: u64::from_le_bytes(data[28..36].try_into().unwrap()),
            store_type_hash: u64::from_le_bytes(data[36..44].try_into().unwrap()),
            flags: u32::from_le_bytes(data[44..48].try_into().unwrap()),
            reserved,
        })
    }
}

/// A WAL entry
#[derive(Debug, Clone)]
pub struct WalEntry {
    /// Log sequence number
    pub lsn: u64,
    /// Operation type
    pub op_type: WalOpType,
    /// Timestamp in milliseconds
    pub timestamp_ms: i64,
    /// Object index
    pub index: u64,
    /// Object UUID
    pub uuid: Uuid7,
    /// Payload (serialized field data)
    pub payload: Vec<u8>,
}

impl WalEntry {
    /// Create a new WAL entry for INSERT
    pub fn insert(
        lsn: u64,
        index: u64,
        uuid: Uuid7,
        fields: &[(String, StoredValue)],
    ) -> Self {
        let mut payload = Vec::new();
        serialize_fields(&mut payload, fields);
        
        Self {
            lsn,
            op_type: WalOpType::Insert,
            timestamp_ms: current_timestamp_ms(),
            index,
            uuid,
            payload,
        }
    }
    
    /// Create a new WAL entry for UPDATE
    pub fn update(
        lsn: u64,
        index: u64,
        uuid: Uuid7,
        fields: &[(String, StoredValue)],
    ) -> Self {
        let mut payload = Vec::new();
        serialize_fields(&mut payload, fields);
        
        Self {
            lsn,
            op_type: WalOpType::Update,
            timestamp_ms: current_timestamp_ms(),
            index,
            uuid,
            payload,
        }
    }
    
    /// Create a new WAL entry for DELETE
    pub fn delete(lsn: u64, index: u64, uuid: Uuid7) -> Self {
        Self {
            lsn,
            op_type: WalOpType::Delete,
            timestamp_ms: current_timestamp_ms(),
            index,
            uuid,
            payload: Vec::new(),
        }
    }
    
    /// Create a COMMIT entry
    pub fn commit(lsn: u64) -> Self {
        Self {
            lsn,
            op_type: WalOpType::Commit,
            timestamp_ms: current_timestamp_ms(),
            index: 0,
            uuid: Uuid7::nil(),
            payload: Vec::new(),
        }
    }
    
    /// Create a CHECKPOINT entry
    pub fn checkpoint(lsn: u64) -> Self {
        Self {
            lsn,
            op_type: WalOpType::Checkpoint,
            timestamp_ms: current_timestamp_ms(),
            index: 0,
            uuid: Uuid7::nil(),
            payload: Vec::new(),
        }
    }
    
    /// Serialize the entry
    pub fn serialize(&self) -> Vec<u8> {
        let total_len = ENTRY_HEADER_SIZE + self.payload.len() + 4; // +4 for CRC
        let mut buf = Vec::with_capacity(total_len);
        
        // Header
        buf.extend_from_slice(&self.lsn.to_le_bytes());
        buf.push(self.op_type as u8);
        buf.extend_from_slice(&self.timestamp_ms.to_le_bytes());
        buf.extend_from_slice(&self.index.to_le_bytes());
        buf.extend_from_slice(self.uuid.as_bytes());
        buf.extend_from_slice(&(self.payload.len() as u32).to_le_bytes());
        
        // Payload
        buf.extend_from_slice(&self.payload);
        
        // CRC32
        let crc = crc32(&buf);
        buf.extend_from_slice(&crc.to_le_bytes());
        
        buf
    }
    
    /// Deserialize an entry from bytes
    pub fn deserialize(data: &[u8]) -> io::Result<(Self, usize)> {
        if data.len() < ENTRY_HEADER_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated WAL entry header",
            ));
        }
        
        let lsn = u64::from_le_bytes(data[0..8].try_into().unwrap());
        let op_type = WalOpType::try_from(data[8])?;
        let timestamp_ms = i64::from_le_bytes(data[9..17].try_into().unwrap());
        let index = u64::from_le_bytes(data[17..25].try_into().unwrap());
        
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&data[25..41]);
        let uuid = Uuid7::from_bytes(uuid_bytes);
        
        let payload_len = u32::from_le_bytes(data[41..45].try_into().unwrap()) as usize;
        
        let total_len = ENTRY_HEADER_SIZE + payload_len + 4;
        if data.len() < total_len {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated WAL entry",
            ));
        }
        
        // Verify CRC
        let expected_crc = crc32(&data[..total_len - 4]);
        let actual_crc = u32::from_le_bytes(
            data[total_len - 4..total_len].try_into().unwrap()
        );
        
        if expected_crc != actual_crc {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "WAL entry CRC mismatch",
            ));
        }
        
        let payload = data[45..45 + payload_len].to_vec();
        
        Ok((Self {
            lsn,
            op_type,
            timestamp_ms,
            index,
            uuid,
            payload,
        }, total_len))
    }
    
    /// Parse the payload into field values
    pub fn parse_payload(&self) -> io::Result<Vec<(String, StoredValue)>> {
        deserialize_fields(&self.payload)
    }
}

/// Serialize field list to bytes
fn serialize_fields(buf: &mut Vec<u8>, fields: &[(String, StoredValue)]) {
    // Field count
    buf.extend_from_slice(&(fields.len() as u32).to_le_bytes());
    
    for (name, value) in fields {
        // Field name (length-prefixed)
        let name_bytes = name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        
        // Field value
        value.serialize(buf);
    }
}

/// Deserialize fields from bytes
fn deserialize_fields(data: &[u8]) -> io::Result<Vec<(String, StoredValue)>> {
    if data.len() < 4 {
        return Ok(Vec::new());
    }
    
    let mut pos = 0;
    let count = u32::from_le_bytes(data[0..4].try_into().unwrap()) as usize;
    pos += 4;
    
    let mut fields = Vec::with_capacity(count);
    
    for _ in 0..count {
        if pos + 2 > data.len() {
            break;
        }
        
        let name_len = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap()) as usize;
        pos += 2;
        
        if pos + name_len > data.len() {
            break;
        }
        
        let name = String::from_utf8_lossy(&data[pos..pos + name_len]).to_string();
        pos += name_len;
        
        let value = StoredValue::deserialize(data, &mut pos)?;
        fields.push((name, value));
    }
    
    Ok(fields)
}

/// Calculate CRC32 checksum
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for byte in data {
        crc ^= *byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Get current timestamp in milliseconds
fn current_timestamp_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// WAL segment writer
pub struct WalWriter {
    dir: PathBuf,
    store_type: String,
    current_segment: u64,
    current_lsn: AtomicU64,
    file: Option<BufWriter<File>>,
    file_size: u64,
    max_segment_size: u64,
    header: WalHeader,
}

impl WalWriter {
    /// Open or create a WAL writer
    pub fn open(dir: impl AsRef<Path>, store_type: &str) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;
        
        // Find the latest segment
        let mut latest_segment = 0u64;
        let mut latest_lsn = 0u64;
        
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            
            if name.starts_with("wal-") && name.ends_with(".log") {
                if let Ok(num) = name[4..name.len()-4].parse::<u64>() {
                    if num >= latest_segment {
                        latest_segment = num;
                        
                        // Read header to get LSN
                        let path = entry.path();
                        if let Ok(header) = Self::read_header(&path) {
                            if header.last_committed_lsn > latest_lsn {
                                latest_lsn = header.last_committed_lsn;
                            }
                        }
                    }
                }
            }
        }
        
        // Start with the next LSN
        let current_lsn = if latest_lsn > 0 { latest_lsn + 1 } else { 1 };
        
        let mut writer = Self {
            dir,
            store_type: store_type.to_string(),
            current_segment: latest_segment,
            current_lsn: AtomicU64::new(current_lsn),
            file: None,
            file_size: 0,
            max_segment_size: DEFAULT_SEGMENT_SIZE,
            header: WalHeader::new(store_type, latest_segment, current_lsn),
        };
        
        // Open or create the current segment
        writer.ensure_segment()?;
        
        Ok(writer)
    }
    
    fn read_header(path: &Path) -> io::Result<WalHeader> {
        let mut file = File::open(path)?;
        let mut buf = [0u8; WAL_HEADER_SIZE];
        file.read_exact(&mut buf)?;
        WalHeader::deserialize(&buf)
    }
    
    fn segment_path(&self, segment: u64) -> PathBuf {
        self.dir.join(format!("wal-{:016x}.log", segment))
    }
    
    fn ensure_segment(&mut self) -> io::Result<()> {
        if self.file.is_some() && self.file_size < self.max_segment_size {
            return Ok(());
        }
        
        // Need a new segment
        if self.file.is_some() {
            self.current_segment += 1;
        }
        
        let path = self.segment_path(self.current_segment);
        let current_lsn = self.current_lsn.load(Ordering::SeqCst);
        
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        
        self.file_size = file.metadata()?.len();
        
        // Write header if new file
        if self.file_size == 0 {
            self.header = WalHeader::new(&self.store_type, self.current_segment, current_lsn);
            let mut writer = BufWriter::new(file);
            writer.write_all(&self.header.serialize())?;
            writer.flush()?;
            self.file_size = WAL_HEADER_SIZE as u64;
            self.file = Some(writer);
        } else {
            self.file = Some(BufWriter::new(file));
        }
        
        Ok(())
    }
    
    /// Write an entry to the WAL
    pub fn write_entry(&mut self, entry: &WalEntry) -> io::Result<u64> {
        self.ensure_segment()?;
        
        let data = entry.serialize();
        
        if let Some(ref mut file) = self.file {
            file.write_all(&data)?;
            self.file_size += data.len() as u64;
        }
        
        Ok(entry.lsn)
    }
    
    /// Allocate the next LSN
    pub fn next_lsn(&self) -> u64 {
        self.current_lsn.fetch_add(1, Ordering::SeqCst)
    }
    
    /// Sync the WAL to disk
    pub fn sync(&mut self) -> io::Result<()> {
        if let Some(ref mut file) = self.file {
            file.flush()?;
            file.get_ref().sync_all()?;
        }
        Ok(())
    }
    
    /// Write a commit entry and sync
    pub fn commit(&mut self) -> io::Result<u64> {
        let lsn = self.next_lsn();
        let entry = WalEntry::commit(lsn);
        self.write_entry(&entry)?;
        self.sync()?;
        
        // Update header with committed LSN
        self.header.last_committed_lsn = lsn;
        self.update_header()?;
        
        Ok(lsn)
    }
    
    fn update_header(&mut self) -> io::Result<()> {
        let path = self.segment_path(self.current_segment);
        let mut file = OpenOptions::new()
            .write(true)
            .open(&path)?;
        file.write_all(&self.header.serialize())?;
        file.sync_all()?;
        Ok(())
    }
    
    /// Write a checkpoint entry
    pub fn checkpoint(&mut self) -> io::Result<u64> {
        let lsn = self.next_lsn();
        let entry = WalEntry::checkpoint(lsn);
        self.write_entry(&entry)?;
        self.sync()?;
        Ok(lsn)
    }
    
    /// Get the last committed LSN
    pub fn last_committed_lsn(&self) -> u64 {
        self.header.last_committed_lsn
    }
    
    /// Remove old WAL segments before a given LSN
    pub fn truncate_before(&mut self, lsn: u64) -> io::Result<()> {
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            
            if name.starts_with("wal-") && name.ends_with(".log") {
                if let Ok(num) = name[4..name.len()-4].parse::<u64>() {
                    if num < self.current_segment {
                        let path = entry.path();
                        if let Ok(header) = Self::read_header(&path) {
                            if header.last_committed_lsn < lsn {
                                fs::remove_file(path)?;
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// WAL segment reader
pub struct WalReader {
    dir: PathBuf,
}

impl WalReader {
    /// Open a WAL directory for reading
    pub fn open(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        if !dir.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "WAL directory not found",
            ));
        }
        Ok(Self { dir })
    }
    
    /// Iterate over all entries starting from a given LSN
    pub fn entries_from(&self, start_lsn: u64) -> io::Result<WalIterator> {
        let mut segments = Vec::new();
        
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            
            if name.starts_with("wal-") && name.ends_with(".log") {
                if let Ok(num) = name[4..name.len()-4].parse::<u64>() {
                    segments.push((num, entry.path()));
                }
            }
        }
        
        // Sort by segment number
        segments.sort_by_key(|(num, _)| *num);
        
        Ok(WalIterator {
            segments,
            current_segment: 0,
            file: None,
            buffer: Vec::new(),
            buffer_pos: 0,
            start_lsn,
        })
    }
}

/// Iterator over WAL entries
pub struct WalIterator {
    segments: Vec<(u64, PathBuf)>,
    current_segment: usize,
    file: Option<BufReader<File>>,
    buffer: Vec<u8>,
    buffer_pos: usize,
    start_lsn: u64,
}

impl Iterator for WalIterator {
    type Item = io::Result<WalEntry>;
    
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // Try to read from current buffer
            if self.buffer_pos < self.buffer.len() {
                match WalEntry::deserialize(&self.buffer[self.buffer_pos..]) {
                    Ok((entry, len)) => {
                        self.buffer_pos += len;
                        if entry.lsn >= self.start_lsn {
                            return Some(Ok(entry));
                        }
                        continue;
                    }
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                        // Need more data or next segment
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
            
            // Open next segment if needed
            if self.file.is_none() {
                if self.current_segment >= self.segments.len() {
                    return None;
                }
                
                let (_, ref path) = self.segments[self.current_segment];
                match File::open(path) {
                    Ok(f) => {
                        let mut reader = BufReader::new(f);
                        // Skip header
                        let mut header = [0u8; WAL_HEADER_SIZE];
                        if reader.read_exact(&mut header).is_err() {
                            self.current_segment += 1;
                            continue;
                        }
                        self.file = Some(reader);
                        self.buffer.clear();
                        self.buffer_pos = 0;
                    }
                    Err(_) => {
                        self.current_segment += 1;
                        continue;
                    }
                }
            }
            
            // Read more data from file
            if let Some(ref mut file) = self.file {
                let mut chunk = vec![0u8; 64 * 1024]; // 64KB chunks
                match file.read(&mut chunk) {
                    Ok(0) => {
                        // End of file, move to next segment
                        self.file = None;
                        self.current_segment += 1;
                        continue;
                    }
                    Ok(n) => {
                        // Keep unprocessed data
                        let remaining = self.buffer[self.buffer_pos..].to_vec();
                        self.buffer = remaining;
                        self.buffer.extend_from_slice(&chunk[..n]);
                        self.buffer_pos = 0;
                    }
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_wal_entry_roundtrip() {
        let uuid = Uuid7::new();
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        
        let entry = WalEntry::insert(1, 42, uuid.clone(), &fields);
        let data = entry.serialize();
        let (restored, len) = WalEntry::deserialize(&data).unwrap();
        
        assert_eq!(len, data.len());
        assert_eq!(restored.lsn, 1);
        assert_eq!(restored.op_type, WalOpType::Insert);
        assert_eq!(restored.index, 42);
        assert_eq!(restored.uuid, uuid);
        
        let parsed_fields = restored.parse_payload().unwrap();
        assert_eq!(parsed_fields.len(), 2);
    }
    
    #[test]
    fn test_wal_writer_reader() {
        let dir = "/tmp/coral_test_wal";
        let _ = fs::remove_dir_all(dir);
        
        let uuid1 = Uuid7::new();
        let uuid2 = Uuid7::new();
        
        // Write entries
        {
            let mut writer = WalWriter::open(dir, "TestStore").unwrap();
            
            let lsn1 = writer.next_lsn();
            let entry1 = WalEntry::insert(
                lsn1,
                1,
                uuid1.clone(),
                &[("name".to_string(), StoredValue::String("Alice".to_string()))],
            );
            writer.write_entry(&entry1).unwrap();
            
            let lsn2 = writer.next_lsn();
            let entry2 = WalEntry::insert(
                lsn2,
                2,
                uuid2.clone(),
                &[("name".to_string(), StoredValue::String("Bob".to_string()))],
            );
            writer.write_entry(&entry2).unwrap();
            
            writer.commit().unwrap();
        }
        
        // Read entries
        {
            let reader = WalReader::open(dir).unwrap();
            let entries: Vec<_> = reader.entries_from(1).unwrap().collect();
            
            assert!(entries.len() >= 2); // At least 2 insert entries
            
            // Find our inserts (skip commit entries)
            let inserts: Vec<_> = entries
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.op_type == WalOpType::Insert)
                .collect();
            
            assert_eq!(inserts.len(), 2);
            assert_eq!(inserts[0].index, 1);
            assert_eq!(inserts[1].index, 2);
        }
        
        let _ = fs::remove_dir_all(dir);
    }
}
