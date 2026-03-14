use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SyncMode {
    None,

    FDataSync,

    #[default]
    FSync,

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum IndexType {
    #[default]
    BTree,
    Hash,
    Art,
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

#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub binary_enabled: bool,

    pub json_enabled: bool,

    pub compression: Compression,

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

#[derive(Debug, Clone)]
pub struct WalConfig {
    pub enabled: bool,

    pub path: Option<PathBuf>,

    pub max_size: u64,

    pub sync_mode: SyncMode,

    pub checkpoint_interval: Duration,

    pub checkpoint_threshold: u64,
}

impl Default for WalConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            path: None,
            max_size: 64 * 1024 * 1024,
            sync_mode: SyncMode::FSync,
            checkpoint_interval: Duration::from_secs(300),
            checkpoint_threshold: 1000,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CacheConfig {
    pub size: u64,

    pub eviction_policy: EvictionPolicy,

    pub preload: bool,

    pub pin_hot_pages: bool,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            size: 256 * 1024 * 1024,
            eviction_policy: EvictionPolicy::Lru,
            preload: false,
            pin_hot_pages: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AutoPersistConfig {
    pub enabled: bool,

    pub batch_window: Duration,

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

#[derive(Debug, Clone)]
pub struct IndexConfig {
    pub index_type: IndexType,

    pub page_size: u32,

    pub fill_factor: f32,

    pub bloom_filter: bool,

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

#[derive(Debug, Clone)]
pub struct SoftDeleteConfig {
    pub enabled: bool,

    pub retention_period: Duration,

    pub auto_vacuum: bool,

    pub vacuum_interval: Duration,
}

impl Default for SoftDeleteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_period: Duration::from_secs(30 * 24 * 60 * 60),
            auto_vacuum: true,
            vacuum_interval: Duration::from_secs(24 * 60 * 60),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BackupConfig {
    pub auto_backup: bool,

    pub backup_path: Option<PathBuf>,

    pub backup_interval: Duration,

    pub backup_retention: u32,

    pub incremental: bool,
}

impl Default for BackupConfig {
    fn default() -> Self {
        Self {
            auto_backup: false,
            backup_path: None,
            backup_interval: Duration::from_secs(24 * 60 * 60),
            backup_retention: 7,
            incremental: true,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FieldIndex {
    pub index_type: IndexType,
    pub unique: bool,
}

#[derive(Debug, Clone, Default)]
pub struct GlobalConfig {
    pub data_path: PathBuf,

    pub storage: StorageConfig,

    pub wal: WalConfig,

    pub cache: CacheConfig,

    pub auto_persist: AutoPersistConfig,

    pub index: IndexConfig,

    pub soft_delete: SoftDeleteConfig,

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

#[derive(Debug, Clone, Default)]
pub struct StoreTypeConfig {
    pub data_path: Option<PathBuf>,

    pub storage: Option<StorageConfig>,

    pub wal: Option<WalConfig>,

    pub cache_priority: Option<String>,

    pub auto_persist: Option<AutoPersistConfig>,

    pub soft_delete: Option<SoftDeleteConfig>,

    pub indexes: HashMap<String, FieldIndex>,
}

#[derive(Debug, Clone)]
pub struct StoreConfig {
    pub store_type: String,

    pub data_path: PathBuf,

    pub storage: StorageConfig,

    pub wal: WalConfig,

    pub cache: CacheConfig,

    pub auto_persist: AutoPersistConfig,

    pub index: IndexConfig,

    pub soft_delete: SoftDeleteConfig,

    pub backup: BackupConfig,

    pub field_indexes: HashMap<String, FieldIndex>,
}

impl StoreConfig {
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

    pub fn wal_path(&self) -> PathBuf {
        self.wal
            .path
            .clone()
            .unwrap_or_else(|| self.data_path.join("_wal"))
    }

    pub fn index_path(&self) -> PathBuf {
        self.data_path.join("_index")
    }

    pub fn binary_path(&self) -> PathBuf {
        self.data_path.join("data.bin")
    }

    pub fn jsonl_path(&self) -> PathBuf {
        self.data_path.join("data.jsonl")
    }

    pub fn schema_path(&self) -> PathBuf {
        self.data_path.join("schema.json")
    }

    pub fn primary_index_path(&self) -> PathBuf {
        self.index_path().join("primary.idx")
    }
}

#[derive(Debug, Clone, Default)]
pub struct FullConfig {
    pub global: GlobalConfig,
    pub stores: HashMap<String, StoreTypeConfig>,
}

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num_str, unit) = if s.ends_with("ms") {
        (&s[..s.len() - 2], "ms")
    } else if s.ends_with('s') {
        (&s[..s.len() - 1], "s")
    } else if s.ends_with('m') {
        (&s[..s.len() - 1], "m")
    } else if s.ends_with('h') {
        (&s[..s.len() - 1], "h")
    } else if s.ends_with('d') {
        (&s[..s.len() - 1], "d")
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

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().to_uppercase();
    if s.is_empty() {
        return None;
    }

    let (num_str, multiplier) = if s.ends_with("GB") {
        (&s[..s.len() - 2], 1024 * 1024 * 1024)
    } else if s.ends_with("MB") {
        (&s[..s.len() - 2], 1024 * 1024)
    } else if s.ends_with("KB") {
        (&s[..s.len() - 2], 1024)
    } else if s.ends_with('B') {
        (&s[..s.len() - 1], 1)
    } else {
        (s.as_str(), 1)
    };

    let num: u64 = num_str.trim().parse().ok()?;
    Some(num * multiplier)
}

pub fn load_config(path: impl AsRef<Path>) -> io::Result<FullConfig> {
    let content = fs::read_to_string(path)?;
    parse_config(&content)
}

pub fn parse_config(content: &str) -> io::Result<FullConfig> {
    let mut config = FullConfig::default();
    let mut current_section = String::new();
    let mut current_store: Option<String> = None;

    for line in content.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            current_section = section.to_string();

            if section.starts_with("stores.") {
                let parts: Vec<&str> = section.splitn(3, '.').collect();
                if parts.len() >= 2 {
                    let store_name = parts[1].to_string();
                    if !config.stores.contains_key(&store_name) {
                        config
                            .stores
                            .insert(store_name.clone(), StoreTypeConfig::default());
                    }
                    current_store = Some(store_name);
                }
            } else {
                current_store = None;
            }
            continue;
        }

        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();

            let value = if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                &value[1..value.len() - 1]
            } else {
                value
            };

            apply_setting(
                &mut config,
                &current_section,
                current_store.as_deref(),
                key,
                value,
            );
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
        ("global.storage", None) => match key {
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
        },
        ("global.wal", None) => match key {
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
        },
        ("global.cache", None) => match key {
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
        },
        ("global.auto_persist", None) => match key {
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
        },
        ("global.index", None) => match key {
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
        },
        ("global.soft_delete", None) => match key {
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
        },
        ("global.backup", None) => match key {
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
        },

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
                        let storage = store_config
                            .storage
                            .get_or_insert_with(StorageConfig::default);
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
                        let ap = store_config
                            .auto_persist
                            .get_or_insert_with(AutoPersistConfig::default);
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
                        let sd = store_config
                            .soft_delete
                            .get_or_insert_with(SoftDeleteConfig::default);
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
        assert_eq!(
            parse_duration("24h"),
            Some(Duration::from_secs(24 * 60 * 60))
        );
        assert_eq!(
            parse_duration("30d"),
            Some(Duration::from_secs(30 * 24 * 60 * 60))
        );
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
        assert_eq!(
            user_config.data_path,
            Some(PathBuf::from("/var/coral/data/users"))
        );
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
        assert!(!merged.storage.json_enabled);
        assert!(merged.storage.binary_enabled);
    }
}
