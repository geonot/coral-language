//! Persistent store implementation for Coral
//!
//! This module implements the persistent store system as specified in
//! docs/PERSISTENT_STORE_SPEC.md. Key features:
//!
//! - Hierarchical configuration (global → store-type → instance)
//! - System attributes (_index, _uuid, _created_at, _updated_at, _deleted_at, _version)
//! - Dual storage (binary + JSON Lines)
//! - Primary index (index → uuid → offsets)
//! - Write-Ahead Log (WAL) for durability
//!
//! # Directory Structure
//!
//! ```text
//! {data_path}/
//! ├── coral.stores.toml           # Configuration file
//! ├── _meta/
//! │   └── store_registry.json     # All known store types
//! └── {store_type}/
//!     └── {store_name}/
//!         ├── index/
//!         │   └── primary.idx     # index → (uuid, offsets)
//!         ├── data/
//!         │   ├── data.bin        # Binary storage
//!         │   └── data.jsonl      # JSON Lines storage
//!         └── wal/
//!             └── wal-*.log       # Write-ahead log segments
//! ```

mod config;
mod uuid7;
mod index;
mod binary;
mod jsonl;
mod wal;
mod secondary_index;
mod engine;
pub mod ffi;

pub use config::{StoreConfig, GlobalConfig, StoreTypeConfig, parse_config, load_config};
pub use uuid7::Uuid7;
pub use index::{PrimaryIndex, IndexEntry};
pub use binary::{BinaryWriter, BinaryReader, BinaryRecord, StoredValue};
pub use jsonl::{JsonlWriter, JsonlReader, JsonlRecord};
pub use wal::{WalWriter, WalReader, WalEntry};
pub use secondary_index::{SecondaryIndex, SecondaryIndexKind, SecondaryIndexManager};
pub use engine::{StoreEngine, SharedStoreEngine, CachedObject, StoreStats};
pub use ffi::*;

use std::collections::HashMap;
use std::io;
use std::sync::{Mutex, OnceLock};

/// Magic bytes for various file formats
pub const INDEX_MAGIC: &[u8; 8] = b"CORALIDX";
pub const BINARY_MAGIC: &[u8; 8] = b"CORALBIN";
pub const WAL_MAGIC: &[u8; 8] = b"CORALWAL";

/// Current file format versions
pub const INDEX_VERSION: u32 = 1;
pub const BINARY_VERSION: u32 = 1;
pub const WAL_VERSION: u32 = 1;

/// Value type tags for binary encoding
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    Reference = 0xFF,
}

impl TryFrom<u8> for ValueTag {
    type Error = io::Error;
    
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(ValueTag::Unit),
            0x01 => Ok(ValueTag::Bool),
            0x02 => Ok(ValueTag::Int),
            0x03 => Ok(ValueTag::Float),
            0x04 => Ok(ValueTag::String),
            0x05 => Ok(ValueTag::Bytes),
            0x06 => Ok(ValueTag::List),
            0x07 => Ok(ValueTag::Map),
            0x08 => Ok(ValueTag::None),
            0xFF => Ok(ValueTag::Reference),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown value tag: 0x{:02x}", value),
            )),
        }
    }
}

/// Index entry flags
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexFlags {
    Active = 0x0000,
    Deleted = 0x0001,
    SoftDeleted = 0x0002,
    Compressed = 0x0004,
}

/// WAL entry types
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalOpType {
    Insert = 0x01,
    Update = 0x02,
    Delete = 0x03,
    Commit = 0x10,
    Checkpoint = 0x20,
}

impl TryFrom<u8> for WalOpType {
    type Error = io::Error;
    
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x01 => Ok(WalOpType::Insert),
            0x02 => Ok(WalOpType::Update),
            0x03 => Ok(WalOpType::Delete),
            0x10 => Ok(WalOpType::Commit),
            0x20 => Ok(WalOpType::Checkpoint),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("unknown WAL entry type: 0x{:02x}", value),
            )),
        }
    }
}

/// Global registry of open store engines
static ENGINE_REGISTRY: OnceLock<Mutex<HashMap<String, SharedStoreEngine>>> = OnceLock::new();

fn get_engine_registry() -> &'static Mutex<HashMap<String, SharedStoreEngine>> {
    ENGINE_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Open or create a store engine for a given store type and name
pub fn open_store_engine(
    store_type: &str,
    store_name: &str,
    config: StoreConfig,
) -> io::Result<SharedStoreEngine> {
    let key = format!("{}:{}", store_type, store_name);
    let mut registry = get_engine_registry().lock().unwrap();
    
    if let Some(engine) = registry.get(&key) {
        return Ok(engine.clone());
    }
    
    let engine = StoreEngine::open(store_type, store_name, config)?;
    let shared = SharedStoreEngine::new(engine);
    registry.insert(key, shared.clone());
    Ok(shared)
}

/// Save all open store engines
pub fn save_all_engines() -> io::Result<()> {
    let registry = get_engine_registry().lock().unwrap();
    for engine in registry.values() {
        engine.checkpoint()?;
    }
    Ok(())
}

/// Close a store engine
pub fn close_engine(store_type: &str, store_name: &str) -> bool {
    let key = format!("{}:{}", store_type, store_name);
    let mut registry = get_engine_registry().lock().unwrap();
    if let Some(engine) = registry.remove(&key) {
        // Save before closing
        let _ = engine.save();
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_value_tag_roundtrip() {
        for tag in [
            ValueTag::Unit,
            ValueTag::Bool,
            ValueTag::Int,
            ValueTag::Float,
            ValueTag::String,
            ValueTag::Bytes,
            ValueTag::List,
            ValueTag::Map,
            ValueTag::None,
            ValueTag::Reference,
        ] {
            let byte = tag as u8;
            let restored = ValueTag::try_from(byte).unwrap();
            assert_eq!(tag, restored);
        }
    }
    
    #[test]
    fn test_wal_op_type_roundtrip() {
        for op in [
            WalOpType::Insert,
            WalOpType::Update,
            WalOpType::Delete,
            WalOpType::Commit,
            WalOpType::Checkpoint,
        ] {
            let byte = op as u8;
            let restored = WalOpType::try_from(byte).unwrap();
            assert_eq!(op, restored);
        }
    }
}
