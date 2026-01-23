//! Binary storage format for Coral persistent stores
//!
//! File format (data.bin):
//! - File header: 64 bytes
//! - Object records: variable length, 8-byte aligned
//! - Free list: at end of file
//!
//! Each record contains:
//! - Record header (24 bytes)
//! - System fields (fixed layout)
//! - User fields (schema-driven)

use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use super::uuid7::Uuid7;
use super::{BINARY_MAGIC, BINARY_VERSION, ValueTag};

/// File header size
const FILE_HEADER_SIZE: usize = 64;

/// Record header size
const RECORD_HEADER_SIZE: usize = 24;

/// System fields size (uuid + timestamps)
const SYSTEM_FIELDS_SIZE: usize = 16 + 8 + 8 + 8; // uuid + created_at + updated_at + deleted_at

/// Binary file header
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct BinaryFileHeader {
    /// Magic bytes: "CORALBIN"
    pub magic: [u8; 8],
    /// File format version
    pub version: u32,
    /// Store type hash
    pub store_type_hash: u64,
    /// Schema version
    pub schema_version: u32,
    /// Object count
    pub object_count: u64,
    /// File size (for validation)
    pub file_size: u64,
    /// Free space offset (start of free list)
    pub free_space_offset: u64,
    /// Checksum
    pub checksum: u64,
}

impl BinaryFileHeader {
    pub fn new(store_type: &str) -> Self {
        use std::hash::{Hash, Hasher};
        use std::collections::hash_map::DefaultHasher;
        
        let mut hasher = DefaultHasher::new();
        store_type.hash(&mut hasher);
        let type_hash = hasher.finish();
        
        Self {
            magic: *BINARY_MAGIC,
            version: BINARY_VERSION,
            store_type_hash: type_hash,
            schema_version: 1,
            object_count: 0,
            file_size: FILE_HEADER_SIZE as u64,
            free_space_offset: 0,
            checksum: 0,
        }
    }
    
    pub fn serialize(&self) -> [u8; FILE_HEADER_SIZE] {
        let mut buf = [0u8; FILE_HEADER_SIZE];
        buf[0..8].copy_from_slice(&self.magic);
        buf[8..12].copy_from_slice(&self.version.to_le_bytes());
        buf[12..20].copy_from_slice(&self.store_type_hash.to_le_bytes());
        buf[20..24].copy_from_slice(&self.schema_version.to_le_bytes());
        buf[24..32].copy_from_slice(&self.object_count.to_le_bytes());
        buf[32..40].copy_from_slice(&self.file_size.to_le_bytes());
        buf[40..48].copy_from_slice(&self.free_space_offset.to_le_bytes());
        buf[48..56].copy_from_slice(&self.checksum.to_le_bytes());
        // 8 bytes padding
        buf
    }
    
    pub fn deserialize(data: &[u8; FILE_HEADER_SIZE]) -> io::Result<Self> {
        let mut magic = [0u8; 8];
        magic.copy_from_slice(&data[0..8]);
        
        if &magic != BINARY_MAGIC {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid binary file magic",
            ));
        }
        
        let version = u32::from_le_bytes(data[8..12].try_into().unwrap());
        if version > BINARY_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unsupported binary version: {}", version),
            ));
        }
        
        Ok(Self {
            magic,
            version,
            store_type_hash: u64::from_le_bytes(data[12..20].try_into().unwrap()),
            schema_version: u32::from_le_bytes(data[20..24].try_into().unwrap()),
            object_count: u64::from_le_bytes(data[24..32].try_into().unwrap()),
            file_size: u64::from_le_bytes(data[32..40].try_into().unwrap()),
            free_space_offset: u64::from_le_bytes(data[40..48].try_into().unwrap()),
            checksum: u64::from_le_bytes(data[48..56].try_into().unwrap()),
        })
    }
}

/// Record header (24 bytes)
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct RecordHeader {
    /// Total record length (including header)
    pub record_length: u32,
    /// Flags (active, deleted, compressed)
    pub flags: u16,
    /// Field count (user fields only)
    pub field_count: u16,
    /// Sequential index
    pub index: u64,
    /// Version for optimistic concurrency
    pub version: u32,
    /// CRC32 checksum of payload
    pub checksum: u32,
}

impl RecordHeader {
    pub fn serialize(&self) -> [u8; RECORD_HEADER_SIZE] {
        let mut buf = [0u8; RECORD_HEADER_SIZE];
        buf[0..4].copy_from_slice(&self.record_length.to_le_bytes());
        buf[4..6].copy_from_slice(&self.flags.to_le_bytes());
        buf[6..8].copy_from_slice(&self.field_count.to_le_bytes());
        buf[8..16].copy_from_slice(&self.index.to_le_bytes());
        buf[16..20].copy_from_slice(&self.version.to_le_bytes());
        buf[20..24].copy_from_slice(&self.checksum.to_le_bytes());
        buf
    }
    
    pub fn deserialize(data: &[u8; RECORD_HEADER_SIZE]) -> Self {
        Self {
            record_length: u32::from_le_bytes(data[0..4].try_into().unwrap()),
            flags: u16::from_le_bytes(data[4..6].try_into().unwrap()),
            field_count: u16::from_le_bytes(data[6..8].try_into().unwrap()),
            index: u64::from_le_bytes(data[8..16].try_into().unwrap()),
            version: u32::from_le_bytes(data[16..20].try_into().unwrap()),
            checksum: u32::from_le_bytes(data[20..24].try_into().unwrap()),
        }
    }
}

/// Record flags
pub const FLAG_ACTIVE: u16 = 0x0000;
pub const FLAG_DELETED: u16 = 0x0001;
pub const FLAG_SOFT_DELETED: u16 = 0x0002;
pub const FLAG_COMPRESSED: u16 = 0x0004;

/// A stored value that can be serialized to binary format
#[derive(Debug, Clone, PartialEq)]
pub enum StoredValue {
    Unit,
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    List(Vec<StoredValue>),
    Map(Vec<(String, StoredValue)>),
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
                write_varint(buf, bytes.len() as u64);
                buf.extend_from_slice(bytes);
            }
            StoredValue::Bytes(b) => {
                buf.push(ValueTag::Bytes as u8);
                write_varint(buf, b.len() as u64);
                buf.extend_from_slice(b);
            }
            StoredValue::List(items) => {
                buf.push(ValueTag::List as u8);
                write_varint(buf, items.len() as u64);
                for item in items {
                    item.serialize(buf);
                }
            }
            StoredValue::Map(pairs) => {
                buf.push(ValueTag::Map as u8);
                write_varint(buf, pairs.len() as u64);
                for (k, v) in pairs {
                    let key_bytes = k.as_bytes();
                    write_varint(buf, key_bytes.len() as u64);
                    buf.extend_from_slice(key_bytes);
                    v.serialize(buf);
                }
            }
        }
    }
    
    /// Deserialize from bytes
    pub fn deserialize(data: &[u8], pos: &mut usize) -> io::Result<Self> {
        if *pos >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected end of data",
            ));
        }
        
        let tag = ValueTag::try_from(data[*pos])?;
        *pos += 1;
        
        match tag {
            ValueTag::Unit => Ok(StoredValue::Unit),
            ValueTag::None => Ok(StoredValue::None),
            ValueTag::Bool => {
                if *pos >= data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing bool value",
                    ));
                }
                let v = data[*pos] != 0;
                *pos += 1;
                Ok(StoredValue::Bool(v))
            }
            ValueTag::Int => {
                if *pos + 8 > data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing int value",
                    ));
                }
                let v = i64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Ok(StoredValue::Int(v))
            }
            ValueTag::Float => {
                if *pos + 8 > data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing float value",
                    ));
                }
                let v = f64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Ok(StoredValue::Float(v))
            }
            ValueTag::String => {
                let len = read_varint(data, pos)? as usize;
                if *pos + len > data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing string data",
                    ));
                }
                let s = String::from_utf8_lossy(&data[*pos..*pos + len]).to_string();
                *pos += len;
                Ok(StoredValue::String(s))
            }
            ValueTag::Bytes => {
                let len = read_varint(data, pos)? as usize;
                if *pos + len > data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing bytes data",
                    ));
                }
                let b = data[*pos..*pos + len].to_vec();
                *pos += len;
                Ok(StoredValue::Bytes(b))
            }
            ValueTag::List => {
                let len = read_varint(data, pos)? as usize;
                let mut items = Vec::with_capacity(len);
                for _ in 0..len {
                    items.push(Self::deserialize(data, pos)?);
                }
                Ok(StoredValue::List(items))
            }
            ValueTag::Map => {
                let len = read_varint(data, pos)? as usize;
                let mut pairs = Vec::with_capacity(len);
                for _ in 0..len {
                    let key_len = read_varint(data, pos)? as usize;
                    if *pos + key_len > data.len() {
                        return Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            "missing map key",
                        ));
                    }
                    let key = String::from_utf8_lossy(&data[*pos..*pos + key_len]).to_string();
                    *pos += key_len;
                    let value = Self::deserialize(data, pos)?;
                    pairs.push((key, value));
                }
                Ok(StoredValue::Map(pairs))
            }
            ValueTag::Reference => {
                // References are stored as indexes
                if *pos + 8 > data.len() {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "missing reference value",
                    ));
                }
                let idx = i64::from_le_bytes(data[*pos..*pos + 8].try_into().unwrap());
                *pos += 8;
                Ok(StoredValue::Int(idx)) // For now, store as int
            }
        }
    }
}

/// Write a varint (variable-length integer)
fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Read a varint
fn read_varint(data: &[u8], pos: &mut usize) -> io::Result<u64> {
    let mut value: u64 = 0;
    let mut shift = 0;
    
    loop {
        if *pos >= data.len() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "truncated varint",
            ));
        }
        
        let byte = data[*pos];
        *pos += 1;
        
        value |= ((byte & 0x7F) as u64) << shift;
        
        if byte & 0x80 == 0 {
            break;
        }
        
        shift += 7;
        if shift >= 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "varint too long",
            ));
        }
    }
    
    Ok(value)
}

/// Calculate CRC32 checksum
fn crc32(data: &[u8]) -> u32 {
    // Simple CRC32 implementation
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

/// Binary data file writer
pub struct BinaryWriter {
    path: PathBuf,
    header: BinaryFileHeader,
    write_offset: u64,
}

impl BinaryWriter {
    /// Create or open a binary data file
    pub fn open(path: impl AsRef<Path>, store_type: &str) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        
        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        
        if path.exists() {
            Self::load(&path, store_type)
        } else {
            let header = BinaryFileHeader::new(store_type);
            let mut writer = Self {
                path,
                header,
                write_offset: FILE_HEADER_SIZE as u64,
            };
            writer.write_header()?;
            Ok(writer)
        }
    }
    
    fn load(path: &Path, store_type: &str) -> io::Result<Self> {
        let mut file = File::open(path)?;
        let mut header_buf = [0u8; FILE_HEADER_SIZE];
        file.read_exact(&mut header_buf)?;
        let header = BinaryFileHeader::deserialize(&header_buf)?;
        
        // Verify store type matches
        let expected_hash = {
            use std::hash::{Hash, Hasher};
            use std::collections::hash_map::DefaultHasher;
            let mut hasher = DefaultHasher::new();
            store_type.hash(&mut hasher);
            hasher.finish()
        };
        
        if header.store_type_hash != expected_hash {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "store type mismatch",
            ));
        }
        
        Ok(Self {
            path: path.to_path_buf(),
            header,
            write_offset: header.file_size,
        })
    }
    
    fn write_header(&mut self) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.path)?;
        file.write_all(&self.header.serialize())?;
        file.flush()?;
        Ok(())
    }
    
    /// Write a record to the binary file
    /// Returns (offset, length)
    pub fn write_record(
        &mut self,
        index: u64,
        version: u32,
        uuid: &Uuid7,
        created_at: i64,
        updated_at: i64,
        deleted_at: i64,
        fields: &[(String, StoredValue)],
    ) -> io::Result<(u64, u32)> {
        // Serialize the payload
        let mut payload = Vec::new();
        
        // System fields
        payload.extend_from_slice(uuid.as_bytes());
        payload.extend_from_slice(&created_at.to_le_bytes());
        payload.extend_from_slice(&updated_at.to_le_bytes());
        payload.extend_from_slice(&deleted_at.to_le_bytes());
        
        // User fields
        for (name, value) in fields {
            let name_bytes = name.as_bytes();
            write_varint(&mut payload, name_bytes.len() as u64);
            payload.extend_from_slice(name_bytes);
            value.serialize(&mut payload);
        }
        
        // Calculate checksum
        let checksum = crc32(&payload);
        
        // Build record header
        let record_length = (RECORD_HEADER_SIZE + payload.len()) as u32;
        let header = RecordHeader {
            record_length,
            flags: FLAG_ACTIVE,
            field_count: fields.len() as u16,
            index,
            version,
            checksum,
        };
        
        // Pad to 8-byte alignment
        let padding = (8 - (record_length as usize % 8)) % 8;
        let total_length = record_length as usize + padding;
        
        // Write to file
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .open(&self.path)?;
        file.seek(SeekFrom::Start(self.write_offset))?;
        file.write_all(&header.serialize())?;
        file.write_all(&payload)?;
        if padding > 0 {
            file.write_all(&vec![0u8; padding])?;
        }
        
        let offset = self.write_offset;
        self.write_offset += total_length as u64;
        
        // Update header
        self.header.object_count += 1;
        self.header.file_size = self.write_offset;
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&self.header.serialize())?;
        file.flush()?;
        
        Ok((offset, total_length as u32))
    }
    
    /// Get current write offset
    pub fn write_offset(&self) -> u64 {
        self.write_offset
    }
    
    /// Get object count
    pub fn object_count(&self) -> u64 {
        self.header.object_count
    }
}

/// Binary data file reader
pub struct BinaryReader {
    path: PathBuf,
    header: BinaryFileHeader,
}

impl BinaryReader {
    /// Open a binary data file for reading
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        
        let mut header_buf = [0u8; FILE_HEADER_SIZE];
        file.read_exact(&mut header_buf)?;
        let header = BinaryFileHeader::deserialize(&header_buf)?;
        
        Ok(Self { path, header })
    }
    
    /// Read a record at a given offset
    pub fn read_record(&self, offset: u64) -> io::Result<BinaryRecord> {
        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(offset))?;
        
        // Read header
        let mut header_buf = [0u8; RECORD_HEADER_SIZE];
        file.read_exact(&mut header_buf)?;
        let header = RecordHeader::deserialize(&header_buf);
        
        // Read payload
        let payload_len = header.record_length as usize - RECORD_HEADER_SIZE;
        let mut payload = vec![0u8; payload_len];
        file.read_exact(&mut payload)?;
        
        // Verify checksum
        let actual_checksum = crc32(&payload);
        if actual_checksum != header.checksum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "record checksum mismatch",
            ));
        }
        
        // Parse payload
        let mut pos = 0;
        
        // System fields
        let mut uuid_bytes = [0u8; 16];
        uuid_bytes.copy_from_slice(&payload[pos..pos + 16]);
        pos += 16;
        
        let created_at = i64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        let updated_at = i64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        let deleted_at = i64::from_le_bytes(payload[pos..pos + 8].try_into().unwrap());
        pos += 8;
        
        // User fields
        let mut fields = Vec::with_capacity(header.field_count as usize);
        for _ in 0..header.field_count {
            let name_len = read_varint(&payload, &mut pos)? as usize;
            let name = String::from_utf8_lossy(&payload[pos..pos + name_len]).to_string();
            pos += name_len;
            let value = StoredValue::deserialize(&payload, &mut pos)?;
            fields.push((name, value));
        }
        
        Ok(BinaryRecord {
            index: header.index,
            version: header.version,
            flags: header.flags,
            uuid: Uuid7::from_bytes(uuid_bytes),
            created_at,
            updated_at,
            deleted_at,
            fields,
        })
    }
    
    /// Get object count
    pub fn object_count(&self) -> u64 {
        self.header.object_count
    }
}

/// A record read from binary storage
#[derive(Debug, Clone)]
pub struct BinaryRecord {
    pub index: u64,
    pub version: u32,
    pub flags: u16,
    pub uuid: Uuid7,
    pub created_at: i64,
    pub updated_at: i64,
    pub deleted_at: i64,
    pub fields: Vec<(String, StoredValue)>,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_varint_roundtrip() {
        let values = [0, 1, 127, 128, 255, 256, 16383, 16384, u64::MAX];
        
        for &value in &values {
            let mut buf = Vec::new();
            write_varint(&mut buf, value);
            let mut pos = 0;
            let restored = read_varint(&buf, &mut pos).unwrap();
            assert_eq!(value, restored, "varint roundtrip failed for {}", value);
        }
    }
    
    #[test]
    fn test_stored_value_roundtrip() {
        let values = vec![
            StoredValue::Unit,
            StoredValue::None,
            StoredValue::Bool(true),
            StoredValue::Bool(false),
            StoredValue::Int(-42),
            StoredValue::Int(0),
            StoredValue::Int(i64::MAX),
            StoredValue::Float(3.14159),
            StoredValue::String("hello world".to_string()),
            StoredValue::String("".to_string()),
            StoredValue::Bytes(vec![1, 2, 3, 4]),
            StoredValue::List(vec![
                StoredValue::Int(1),
                StoredValue::String("two".to_string()),
            ]),
            StoredValue::Map(vec![
                ("key1".to_string(), StoredValue::Int(42)),
                ("key2".to_string(), StoredValue::String("value".to_string())),
            ]),
        ];
        
        for value in values {
            let mut buf = Vec::new();
            value.serialize(&mut buf);
            let mut pos = 0;
            let restored = StoredValue::deserialize(&buf, &mut pos).unwrap();
            assert_eq!(value, restored);
        }
    }
    
    #[test]
    fn test_binary_file_roundtrip() {
        let path = "/tmp/coral_test_binary.bin";
        let _ = fs::remove_file(path);
        
        let uuid = Uuid7::new();
        let fields = vec![
            ("name".to_string(), StoredValue::String("Alice".to_string())),
            ("age".to_string(), StoredValue::Int(30)),
        ];
        
        // Write
        let (offset, length) = {
            let mut writer = BinaryWriter::open(path, "TestStore").unwrap();
            writer.write_record(
                1,    // index
                1,    // version
                &uuid,
                1000, // created_at
                1000, // updated_at
                -1,   // deleted_at (not deleted)
                &fields,
            ).unwrap()
        };
        
        // Read back
        {
            let reader = BinaryReader::open(path).unwrap();
            let record = reader.read_record(offset).unwrap();
            
            assert_eq!(record.index, 1);
            assert_eq!(record.version, 1);
            assert_eq!(record.uuid, uuid);
            assert_eq!(record.created_at, 1000);
            assert_eq!(record.fields.len(), 2);
            assert_eq!(record.fields[0].0, "name");
            assert_eq!(record.fields[0].1, StoredValue::String("Alice".to_string()));
        }
        
        let _ = fs::remove_file(path);
    }
}
