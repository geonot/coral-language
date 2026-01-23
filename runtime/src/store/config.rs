//! Configuration system for persistent stores
//!
//! Implements hierarchical configuration:
//! 1. Built-in defaults
//! 2. Global config ([global] in coral.stores.toml)
//! 3. Store-type config ([stores.TypeName])
//! 4. Instance-time overrides

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Sync mode for WAL writes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncMode {
    /// No sync (fastest, least durable)
    None,
    /// fdatasync (sync data, not metadata)
    FDataSync,
    /// fsync (sync data and metadata)
    #[default]
    FSync,
    /// Full sync with barriers
    Full,
}

impl SyncMode {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(SyncMode::None),
            "fdatasync" => Some(SyncMode::FDataSync),
            "fsync" => Some(SyncMode::FSync),
            "full" => Some(SyncMode::Full),
            _ => None,
        }
    }
}

/// Compression algorithm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    #[default]
    None,
    Lz4,
    Zstd,
    Snappy,
}

impl Compression {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" => Some(Compression::None),
            "lz4" => Some(Compression::Lz4),
            "zstd" => Some(Compression::Zstd),
            "snappy" => Some(Compression::Snappy),
            _ => None,
        }
    }
}

/// Index type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexType {
    #[default]
    BTree,
    Hash,
    Art, // Adaptive Radix Tree
}

impl IndexType {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "btree" => Some(IndexType::BTree),
            "hash" => Some(IndexType::Hash),
            "art" => Some(IndexType::Art),
            _ => None,
        }
    }
}

/// Cache eviction policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EvictionPolicy {
    #[default]
    Lru,
    Lfu,
    Arc,
}

impl EvictionPolicy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "lru" => Some(EvictionPolicy::Lru),
            "lfu" => Some(EvictionPolicy::Lfu),
            "arc" => Some(EvictionPolicy::Arc),
            _ => None,
        }
    }
}

/// Storage format configuration
#[derive(Debug, Clone)]
pub struct StorageConfig {
    /// Store binary representation
    pub binary_enabled: bool,
    /// Store JSON representation
    pub json_enabled: bool,
    /// Compression algorithm
    pub compression: Compression,
    /// Compression level (1-9)
    pub compression_level: u8,
}

impl Default for StorageConfig {
    fn default() -> Self {
        Self {
            binary_enabled: true,
            json_enabled: true,
            compression: Compression::None,
            compression_level: 3,
        }
    }
}

/// WAL configuration
#[derive(Debug, Clone)]
pub struct WalConfig {
    /// Enable WAL
    pub enabled: bool,
    /// WAL directory path (relative to data_path or absolute)
    pub path: Option<PathBuf>,
    /// Max WAL size before forced checkpoint
    pub max_size: u64,
    /// Sync mode
    pub sync_mode: SyncMode,
    /// Time-based checkpoint interval
    pub checkpoint_interval: Duration,
    /// Operation count before checkpoint
    pub checkpoint_threshold: u64,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None, // Will use {data_path}/wal
            max_size: 64 * 1024 * 1024, // 64MB
            sync_mode: SyncMode::FSync,
            checkpoint_interval: Duration::from_secs(300), // 5 minutes
            checkpoint_threshold: 1000,
        }
    }
}

/// Cache configuration
#[derive(Debug, Clone)]
pub struct CacheConfig {
    /// Total cache size in bytes
    pub size: u64,
    /// Eviction policy
    pub eviction_policy: EvictionPolicy,
    /// Preload indexes on startup
    pub preload: bool,
    /// Pin hot pages in memory
    pub pin_hot_pages: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            size: 256 * 1024 * 1024, // 256MB
            eviction_policy: EvictionPolicy::Lru,
            preload: false,
            pin_hot_pages: true,
        }
    }
}

/// Auto-persistence configuration
#[derive(Debug, Clone)]
pub struct AutoPersistConfig {
    /// Automatically persist on field mutation
    pub enabled: bool,
    /// Batch writes within this window
    pub batch_window: Duration,
    /// Max operations in a batch
    pub batch_max_size: u64,
}

impl Default for AutoPersistConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            batch_window: Duration::from_millis(10),
            batch_max_size: 100,
        }
    }
}

/// Index configuration
#[derive(Debug, Clone)]
pub struct IndexConfig {
    /// Default index type
    pub index_type: IndexType,
    /// Index page size in bytes
    pub page_size: u32,
    /// Page fill factor for inserts (0.0-1.0)
    pub fill_factor: f32,
    /// Enable bloom filters for negative lookups
    pub bloom_filter: bool,
    /// Bloom filter false positive rate
    pub bloom_fpr: f32,
}

impl Default for IndexConfig {
    fn default() -> Self {
        Self {
            index_type: IndexType::BTree,
            page_size: 4096,
            fill_factor: 0.8,
            bloom_filter: true,
            bloom_fpr: 0.01,
        }
    }
}

/// Soft delete configuration
#[derive(Debug, Clone)]
pub struct SoftDeleteConfig {
    /// Use soft delete by default
    pub enabled: bool,
    /// How long to keep soft-deleted records
    pub retention_period: Duration,
    /// Automatically vacuum expired records
    pub auto_vacuum: bool,
    /// How often to run vacuum
    pub vacuum_interval: Duration,
}

impl Default for SoftDeleteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_period: Duration::from_secs(30 * 24 * 60 * 60), // 30 days
            auto_vacuum: true,
            vacuum_interval: Duration::from_secs(24 * 60 * 60), // 24 hours
        }
    }
}

/// Backup configuration
#[derive(Debug, Clone)]
pub struct BackupConfig {
    /// Enable automatic backups
    pub auto_backup: bool,
    /// Backup directory path
    pub backup_path: Option<PathBuf>,
    /// Time between backups
    pub backup_interval: Duration,
    /// Number of backups to keep
    pub backup_retention: u32,
    /// Use incremental backups
    pub incremental: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_backup: false,
            backup_path: None, // Will use {data_path}/backups
            backup_interval: Duration::from_secs(24 * 60 * 60), // 24 hours
            backup_retention: 7,
            incremental: true,
        }
    }
}

/// Field index definition
#[derive(Debug, Clone)]
pub struct FieldIndex {
    pub index_type: IndexType,
    pub unique: bool,
}

/// Global configuration (applies to all stores)
#[derive(Debug, Clone, Default)]
pub struct GlobalConfig {
    /// Base directory for all store data
    pub data_path: PathBuf,
    /// Storage format settings
    pub storage: StorageConfig,
    /// WAL settings
    pub wal: WalConfig,
    /// Cache settings
    pub cache: CacheConfig,
    /// Auto-persist settings
    pub auto_persist: AutoPersistConfig,
    /// Index settings
    pub index: IndexConfig,
    /// Soft delete settings
    pub soft_delete: SoftDeleteConfig,
    /// Backup settings
    pub backup: BackupConfig,
}

impl GlobalConfig {
    pub fn new(data_path: impl Into<PathBuf>) -> Self {
        Self {
            data_path: data_path.into(),
            ..Default::default()
        }
    }
}

/// Store-type specific configuration (overrides global)
#[derive(Debug, Clone, Default)]
pub struct StoreTypeConfig {
    /// Override data path for this store type
    pub data_path: Option<PathBuf>,
    /// Storage overrides
    pub storage: Option<StorageConfig>,
    /// WAL overrides
    pub wal: Option<WalConfig>,
    /// Cache overrides (subset)
    pub cache_priority: Option<String>,
    /// Auto-persist overrides
    pub auto_persist: Option<AutoPersistConfig>,
    /// Soft delete overrides
    pub soft_delete: Option<SoftDeleteConfig>,
    /// Field indexes
    pub indexes: HashMap<String, FieldIndex>,
}

/// Resolved configuration for a specific store
#[derive(Debug, Clone)]
pub struct StoreConfig {
    /// Store type name
    pub store_type: String,
    /// Data path for this store
    pub data_path: PathBuf,
    /// Storage settings
    pub storage: StorageConfig,
    /// WAL settings
    pub wal: WalConfig,
    /// Cache settings
    pub cache: CacheConfig,
    /// Auto-persist settings
    pub auto_persist: AutoPersistConfig,
    /// Index settings
    pub index: IndexConfig,
    /// Soft delete settings
    pub soft_delete: SoftDeleteConfig,
    /// Backup settings
    pub backup: BackupConfig,
    /// Field indexes
    pub field_indexes: HashMap<String, FieldIndex>,
}

impl StoreConfig {
    /// Create a new config by merging global and store-type configs
    pub fn merge(
        store_type: &str,
        global: &GlobalConfig,
        store_type_config: Option<&StoreTypeConfig>,
    ) -> Self {
        let base_path = store_type_config
            .and_then(|c| c.data_path.clone())
            .unwrap_or_else(|| global.data_path.join(store_type.to_lowercase()));
        
        Self {
            store_type: store_type.to_string(),
            data_path: base_path,
            storage: store_type_config
                .and_then(|c| c.storage.clone())
                .unwrap_or_else(|| global.storage.clone()),
            wal: store_type_config
                .and_then(|c| c.wal.clone())
                .unwrap_or_else(|| global.wal.clone()),
            cache: global.cache.clone(),
            auto_persist: store_type_config
                .and_then(|c| c.auto_persist.clone())
                .unwrap_or_else(|| global.auto_persist.clone()),
            index: global.index.clone(),
            soft_delete: store_type_config
                .and_then(|c| c.soft_delete.clone())
                .unwrap_or_else(|| global.soft_delete.clone()),
            backup: global.backup.clone(),
            field_indexes: store_type_config
                .map(|c| c.indexes.clone())
                .unwrap_or_default(),
        }
    }
    
    /// Create a minimal config for testing
    pub fn minimal(store_type: &str, data_path: impl Into<PathBuf>) -> Self {
        let data_path = data_path.into();
        Self {
            store_type: store_type.to_string(),
            data_path,
            storage: StorageConfig::default(),
            wal: WalConfig::default(),
            cache: CacheConfig::default(),
            auto_persist: AutoPersistConfig::default(),
            index: IndexConfig::default(),
            soft_delete: SoftDeleteConfig::default(),
            backup: BackupConfig::default(),
            field_indexes: HashMap::new(),
        }
    }
    
    /// Get the WAL directory path
    pub fn wal_path(&self) -> PathBuf {
        self.wal.path.clone().unwrap_or_else(|| self.data_path.join("_wal"))
    }
    
    /// Get the index directory path
    pub fn index_path(&self) -> PathBuf {
        self.data_path.join("_index")
    }
    
    /// Get the binary data file path
    pub fn binary_path(&self) -> PathBuf {
        self.data_path.join("data.bin")
    }
    
    /// Get the JSON Lines data file path
    pub fn jsonl_path(&self) -> PathBuf {
        self.data_path.join("data.jsonl")
    }
    
    /// Get the schema file path
    pub fn schema_path(&self) -> PathBuf {
        self.data_path.join("schema.json")
    }
    
    /// Get the primary index file path
    pub fn primary_index_path(&self) -> PathBuf {
        self.index_path().join("primary.idx")
    }
}

/// Complete configuration loaded from file
#[derive(Debug, Clone, Default)]
pub struct FullConfig {
    pub global: GlobalConfig,
    pub stores: HashMap<String, StoreTypeConfig>,
}

/// Parse a duration string like "5m", "24h", "30d"
fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    
    let (num_str, unit) = if s.ends_with("ms") {
        (&s[..s.len()-2], "ms")
    } else if s.ends_with('s') {
        (&s[..s.len()-1], "s")
    } else if s.ends_with('m') {
        (&s[..s.len()-1], "m")
    } else if s.ends_with('h') {
        (&s[..s.len()-1], "h")
    } else if s.ends_with('d') {
        (&s[..s.len()-1], "d")
    } else {
        return None;
    };
    
    let num: u64 = num_str.parse().ok()?;
    
    match unit {
        "ms" => Some(Duration::from_millis(num)),
        "s" => Some(Duration::from_secs(num)),
        "m" => Some(Duration::from_secs(num * 60)),
        "h" => Some(Duration::from_secs(num * 60 * 60)),
        "d" => Some(Duration::from_secs(num * 24 * 60 * 60)),
        _ => None,
    }
}

/// Parse a size string like "64MB", "256KB", "1GB"
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }
    
    let (num_str, multiplier) = if s.ends_with("GB") {
        (&s[..s.len()-2], 1024 * 1024 * 1024)
    } else if s.ends_with("MB") {
        (&s[..s.len()-2], 1024 * 1024)
    } else if s.ends_with("KB") {
        (&s[..s.len()-2], 1024)
    } else if s.ends_with('B') {
        (&s[..s.len()-1], 1)
    } else {
        // Assume bytes
        (s.as_str(), 1)
    };
    
    let num: u64 = num_str.trim().parse().ok()?;
    Some(num * multiplier)
}

/// Load configuration from a TOML file
/// 
/// This is a minimal TOML parser that handles the coral.stores.toml format.
/// For full TOML support, consider using the `toml` crate.
pub fn load_config(path: impl AsRef<Path>) -> io::Result<FullConfig> {
    let content = fs::read_to_string(path)?;
    parse_config(&content)
}

/// Parse configuration from a TOML string
pub fn parse_config(content: &str) -> io::Result<FullConfig> {
    let mut config = FullConfig::default();
    let mut current_section = String::new();
    let mut current_store: Option<String> = None;
    
    for line in content.lines() {
        let line = line.trim();
        
        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        
        // Section header
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len()-1];
            current_section = section.to_string();
            
            // Check if this is a store-specific section
            if section.starts_with("stores.") {
                let parts: Vec<&str> = section.splitn(3, '.').collect();
                if parts.len() >= 2 {
                    let store_name = parts[1].to_string();
                    if !config.stores.contains_key(&store_name) {
                        config.stores.insert(store_name.clone(), StoreTypeConfig::default());
                    }
                    current_store = Some(store_name);
                }
            } else {
                current_store = None;
            }
            continue;
        }
        
        // Key-value pair
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos+1..].trim();
            
            // Remove quotes from string values
            let value = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len()-1]
            } else {
                value
            };
            
            // Apply the setting based on current section
            apply_setting(&mut config, &current_section, current_store.as_deref(), key, value);
        }
    }
    
    Ok(config)
}

fn apply_setting(
    config: &mut FullConfig,
    section: &str,
    store_name: Option<&str>,
    key: &str,
    value: &str,
) {
    match (section, store_name) {
        ("global", None) => {
            if key == "data_path" {
                config.global.data_path = PathBuf::from(value);
            }
        }
        ("global.storage", None) => {
            match key {
                "binary_enabled" => config.global.storage.binary_enabled = value == "true",
                "json_enabled" => config.global.storage.json_enabled = value == "true",
                "compression" => {
                    if let Some(c) = Compression::from_str(value) {
                        config.global.storage.compression = c;
                    }
                }
                "compression_level" => {
                    if let Ok(level) = value.parse::<u8>() {
                        config.global.storage.compression_level = level.clamp(1, 9);
                    }
                }
                _ => {}
            }
        }
        ("global.wal", None) => {
            match key {
                "enabled" => config.global.wal.enabled = value == "true",
                "path" => config.global.wal.path = Some(PathBuf::from(value)),
                "max_size" => {
                    if let Some(size) = parse_size(value) {
                        config.global.wal.max_size = size;
                    }
                }
                "sync_mode" => {
                    if let Some(mode) = SyncMode::from_str(value) {
                        config.global.wal.sync_mode = mode;
                    }
                }
                "checkpoint_interval" => {
                    if let Some(dur) = parse_duration(value) {
                        config.global.wal.checkpoint_interval = dur;
                    }
                }
                "checkpoint_threshold" => {
                    if let Ok(n) = value.parse() {
                        config.global.wal.checkpoint_threshold = n;
                    }
                }
                _ => {}
            }
        }
        ("global.cache", None) => {
            match key {
                "size" => {
                    if let Some(size) = parse_size(value) {
                        config.global.cache.size = size;
                    }
                }
                "eviction_policy" => {
                    if let Some(policy) = EvictionPolicy::from_str(value) {
                        config.global.cache.eviction_policy = policy;
                    }
                }
                "preload" => config.global.cache.preload = value == "true",
                "pin_hot_pages" => config.global.cache.pin_hot_pages = value == "true",
                _ => {}
            }
        }
        ("global.auto_persist", None) => {
            match key {
                "enabled" => config.global.auto_persist.enabled = value == "true",
                "batch_window" => {
                    if let Some(dur) = parse_duration(value) {
                        config.global.auto_persist.batch_window = dur;
                    }
                }
                "batch_max_size" => {
                    if let Ok(n) = value.parse() {
                        config.global.auto_persist.batch_max_size = n;
                    }
                }
                _ => {}
            }
        }
        ("global.index", None) => {
            match key {
                "type" => {
                    if let Some(t) = IndexType::from_str(value) {
                        config.global.index.index_type = t;
                    }
                }
                "page_size" => {
                    if let Ok(n) = value.parse() {
                        config.global.index.page_size = n;
                    }
                }
                "fill_factor" => {
                    if let Ok(f) = value.parse::<f32>() {
                        config.global.index.fill_factor = f.clamp(0.1, 1.0);
                    }
                }
                "bloom_filter" => config.global.index.bloom_filter = value == "true",
                "bloom_fpr" => {
                    if let Ok(f) = value.parse::<f32>() {
                        config.global.index.bloom_fpr = f.clamp(0.001, 0.5);
                    }
                }
                _ => {}
            }
        }
        ("global.soft_delete", None) => {
            match key {
                "enabled" => config.global.soft_delete.enabled = value == "true",
                "retention_period" => {
                    if let Some(dur) = parse_duration(value) {
                        config.global.soft_delete.retention_period = dur;
                    }
                }
                "auto_vacuum" => config.global.soft_delete.auto_vacuum = value == "true",
                "vacuum_interval" => {
                    if let Some(dur) = parse_duration(value) {
                        config.global.soft_delete.vacuum_interval = dur;
                    }
                }
                _ => {}
            }
        }
        ("global.backup", None) => {
            match key {
                "auto_backup" => config.global.backup.auto_backup = value == "true",
                "backup_path" => config.global.backup.backup_path = Some(PathBuf::from(value)),
                "backup_interval" => {
                    if let Some(dur) = parse_duration(value) {
                        config.global.backup.backup_interval = dur;
                    }
                }
                "backup_retention" => {
                    if let Ok(n) = value.parse() {
                        config.global.backup.backup_retention = n;
                    }
                }
                "incremental" => config.global.backup.incremental = value == "true",
                _ => {}
            }
        }
        // Store-specific sections
        (s, Some(store)) if s.starts_with("stores.") => {
            if let Some(store_config) = config.stores.get_mut(store) {
                let subsection = if s.contains('.') {
                    s.splitn(3, '.').nth(2).unwrap_or("")
                } else {
                    ""
                };
                
                match subsection {
                    "" => {
                        if key == "data_path" {
                            store_config.data_path = Some(PathBuf::from(value));
                        }
                    }
                    "storage" => {
                        let storage = store_config.storage.get_or_insert_with(StorageConfig::default);
                        match key {
                            "binary_enabled" => storage.binary_enabled = value == "true",
                            "json_enabled" => storage.json_enabled = value == "true",
                            "compression" => {
                                if let Some(c) = Compression::from_str(value) {
                                    storage.compression = c;
                                }
                            }
                            "compression_level" => {
                                if let Ok(level) = value.parse::<u8>() {
                                    storage.compression_level = level.clamp(1, 9);
                                }
                            }
                            _ => {}
                        }
                    }
                    "wal" => {
                        let wal = store_config.wal.get_or_insert_with(WalConfig::default);
                        match key {
                            "enabled" => wal.enabled = value == "true",
                            "sync_mode" => {
                                if let Some(mode) = SyncMode::from_str(value) {
                                    wal.sync_mode = mode;
                                }
                            }
                            "checkpoint_interval" => {
                                if let Some(dur) = parse_duration(value) {
                                    wal.checkpoint_interval = dur;
                                }
                            }
                            _ => {}
                        }
                    }
                    "auto_persist" => {
                        let ap = store_config.auto_persist.get_or_insert_with(AutoPersistConfig::default);
                        match key {
                            "enabled" => ap.enabled = value == "true",
                            "batch_window" => {
                                if let Some(dur) = parse_duration(value) {
                                    ap.batch_window = dur;
                                }
                            }
                            _ => {}
                        }
                    }
                    "soft_delete" => {
                        let sd = store_config.soft_delete.get_or_insert_with(SoftDeleteConfig::default);
                        match key {
                            "enabled" => sd.enabled = value == "true",
                            "retention_period" => {
                                if let Some(dur) = parse_duration(value) {
                                    sd.retention_period = dur;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("10ms"), Some(Duration::from_millis(10)));
        assert_eq!(parse_duration("5s"), Some(Duration::from_secs(5)));
        assert_eq!(parse_duration("5m"), Some(Duration::from_secs(300)));
        assert_eq!(parse_duration("24h"), Some(Duration::from_secs(24 * 60 * 60)));
        assert_eq!(parse_duration("30d"), Some(Duration::from_secs(30 * 24 * 60 * 60)));
        assert_eq!(parse_duration("invalid"), None);
    }
    
    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("1KB"), Some(1024));
        assert_eq!(parse_size("64MB"), Some(64 * 1024 * 1024));
        assert_eq!(parse_size("1GB"), Some(1024 * 1024 * 1024));
    }
    
    #[test]
    fn test_parse_config() {
        let toml = r#"
[global]
data_path = "/var/coral/data"

[global.storage]
binary_enabled = true
json_enabled = true
compression = "lz4"

[global.wal]
enabled = true
sync_mode = "fsync"
checkpoint_interval = "5m"

[stores.User]
data_path = "/var/coral/data/users"

[stores.User.storage]
json_enabled = true

[stores.User.wal]
sync_mode = "fsync"
"#;
        
        let config = parse_config(toml).unwrap();
        
        assert_eq!(config.global.data_path, PathBuf::from("/var/coral/data"));
        assert!(config.global.storage.binary_enabled);
        assert_eq!(config.global.storage.compression, Compression::Lz4);
        assert!(config.global.wal.enabled);
        assert_eq!(config.global.wal.sync_mode, SyncMode::FSync);
        
        let user_config = config.stores.get("User").unwrap();
        assert_eq!(user_config.data_path, Some(PathBuf::from("/var/coral/data/users")));
    }
    
    #[test]
    fn test_store_config_merge() {
        let global = GlobalConfig::new("/var/coral/data");
        let mut store_type = StoreTypeConfig::default();
        store_type.storage = Some(StorageConfig {
            json_enabled: false,
            ..Default::default()
        });
        
        let merged = StoreConfig::merge("User", &global, Some(&store_type));
        
        assert_eq!(merged.store_type, "User");
        assert!(!merged.storage.json_enabled); // Overridden
        assert!(merged.storage.binary_enabled); // From store-type default
    }
}
