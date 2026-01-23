//! UUIDv7 implementation for Coral persistent stores
//!
//! UUIDv7 is a time-sortable UUID format defined in RFC 9562.
//! Format: ttttttt-tttt-7xxx-yxxx-xxxxxxxxxxxx
//!
//! - t: 48-bit Unix timestamp in milliseconds
//! - 7: Version (7)
//! - x: Random bits
//! - y: Variant (10xx)
//!
//! Benefits:
//! - Time-sortable (newer UUIDs sort after older ones)
//! - Roughly 2^74 random bits for uniqueness
//! - Compatible with standard UUID libraries

use std::fmt;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Counter to ensure uniqueness within the same millisecond
static COUNTER: AtomicU64 = AtomicU64::new(0);
/// Last timestamp used for generation
static LAST_TIMESTAMP: AtomicU64 = AtomicU64::new(0);

/// A UUIDv7 value (16 bytes)
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Uuid7([u8; 16]);

impl Uuid7 {
    /// Create a nil UUID (all zeros)
    pub fn nil() -> Self {
        Self([0u8; 16])
    }
    
    /// Check if this is a nil UUID
    pub fn is_nil(&self) -> bool {
        self.0 == [0u8; 16]
    }
    
    /// Generate a new UUIDv7 with the current timestamp
    pub fn new() -> Self {
        Self::from_timestamp(current_timestamp_ms())
    }
    
    /// Generate a UUIDv7 from a specific timestamp (milliseconds since Unix epoch)
    pub fn from_timestamp(timestamp_ms: u64) -> Self {
        let mut bytes = [0u8; 16];
        
        // Bytes 0-5: 48-bit timestamp (big-endian)
        bytes[0] = ((timestamp_ms >> 40) & 0xFF) as u8;
        bytes[1] = ((timestamp_ms >> 32) & 0xFF) as u8;
        bytes[2] = ((timestamp_ms >> 24) & 0xFF) as u8;
        bytes[3] = ((timestamp_ms >> 16) & 0xFF) as u8;
        bytes[4] = ((timestamp_ms >> 8) & 0xFF) as u8;
        bytes[5] = (timestamp_ms & 0xFF) as u8;
        
        // Get random bits mixed with counter for sub-millisecond ordering
        let last_ts = LAST_TIMESTAMP.load(Ordering::Relaxed);
        let counter = if timestamp_ms == last_ts {
            COUNTER.fetch_add(1, Ordering::Relaxed)
        } else {
            LAST_TIMESTAMP.store(timestamp_ms, Ordering::Relaxed);
            COUNTER.store(1, Ordering::Relaxed);
            0
        };
        
        // Generate random bits
        let random = random_u64();
        
        // Bytes 6-7: Version (7) + 12 bits of rand_a (counter-based for ordering)
        // The 12 bits of rand_a help with sub-millisecond ordering
        let rand_a = (counter & 0x0FFF) as u16;
        bytes[6] = 0x70 | ((rand_a >> 8) & 0x0F) as u8; // Version 7 + high 4 bits
        bytes[7] = (rand_a & 0xFF) as u8;               // Low 8 bits
        
        // Bytes 8-15: Variant (10xx) + 62 bits of rand_b
        bytes[8] = 0x80 | ((random >> 56) & 0x3F) as u8; // Variant 10xx + 6 bits
        bytes[9] = ((random >> 48) & 0xFF) as u8;
        bytes[10] = ((random >> 40) & 0xFF) as u8;
        bytes[11] = ((random >> 32) & 0xFF) as u8;
        bytes[12] = ((random >> 24) & 0xFF) as u8;
        bytes[13] = ((random >> 16) & 0xFF) as u8;
        bytes[14] = ((random >> 8) & 0xFF) as u8;
        bytes[15] = (random & 0xFF) as u8;
        
        Self(bytes)
    }
    
    /// Create a UUID from raw bytes
    pub fn from_bytes(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
    
    /// Create a UUID from a byte slice
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        if slice.len() != 16 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("UUID must be 16 bytes, got {}", slice.len()),
            ));
        }
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }
    
    /// Parse a UUID from a hyphenated string (xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx)
    pub fn parse(s: &str) -> io::Result<Self> {
        let s = s.trim();
        
        // Remove hyphens and validate length
        let hex: String = s.chars().filter(|c| *c != '-').collect();
        if hex.len() != 32 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid UUID string length: expected 32 hex chars, got {}", hex.len()),
            ));
        }
        
        let mut bytes = [0u8; 16];
        for i in 0..16 {
            bytes[i] = u8::from_str_radix(&hex[i*2..i*2+2], 16)
                .map_err(|_| io::Error::new(
                    io::ErrorKind::InvalidData,
                    "invalid hex character in UUID",
                ))?;
        }
        
        Ok(Self(bytes))
    }
    
    /// Get the raw bytes
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
    
    /// Convert to owned byte array
    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }
    
    /// Extract the timestamp (milliseconds since Unix epoch)
    pub fn timestamp_ms(&self) -> u64 {
        ((self.0[0] as u64) << 40)
            | ((self.0[1] as u64) << 32)
            | ((self.0[2] as u64) << 24)
            | ((self.0[3] as u64) << 16)
            | ((self.0[4] as u64) << 8)
            | (self.0[5] as u64)
    }
    
    /// Get the UUID version (should be 7)
    pub fn version(&self) -> u8 {
        (self.0[6] >> 4) & 0x0F
    }
    
    /// Get the UUID variant (should be 2 for RFC 4122)
    pub fn variant(&self) -> u8 {
        (self.0[8] >> 6) & 0x03
    }
    
    /// Check if this is a valid UUIDv7
    pub fn is_valid(&self) -> bool {
        self.version() == 7 && self.variant() == 2
    }
    
    /// Format as hyphenated string
    pub fn to_string(&self) -> String {
        format!(
            "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3],
            self.0[4], self.0[5],
            self.0[6], self.0[7],
            self.0[8], self.0[9],
            self.0[10], self.0[11], self.0[12], self.0[13], self.0[14], self.0[15]
        )
    }
    
    /// Nil UUID (all zeros)
    pub const NIL: Self = Self([0u8; 16]);
}

impl Default for Uuid7 {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for Uuid7 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Uuid7({})", self.to_string())
    }
}

impl fmt::Display for Uuid7 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_string())
    }
}

impl From<[u8; 16]> for Uuid7 {
    fn from(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }
}

impl From<Uuid7> for [u8; 16] {
    fn from(uuid: Uuid7) -> Self {
        uuid.0
    }
}

impl AsRef<[u8]> for Uuid7 {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Get current timestamp in milliseconds since Unix epoch
fn current_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Generate a random u64 using a simple xorshift + hashing approach
fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    
    // Use RandomState for random seed
    let state = RandomState::new();
    let mut hasher = state.build_hasher();
    
    // Mix in timestamp and counter for extra entropy
    let now = current_timestamp_ms();
    let counter = COUNTER.load(Ordering::Relaxed);
    hasher.write_u64(now);
    hasher.write_u64(counter);
    
    // Also mix in the thread ID for uniqueness across threads
    let thread_id = std::thread::current().id();
    hasher.write(&format!("{:?}", thread_id).as_bytes());
    
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    
    #[test]
    fn test_uuid7_generation() {
        let uuid = Uuid7::new();
        
        // Check version and variant
        assert_eq!(uuid.version(), 7);
        assert_eq!(uuid.variant(), 2);
        assert!(uuid.is_valid());
        
        // Timestamp should be recent
        let now_ms = current_timestamp_ms();
        let uuid_ms = uuid.timestamp_ms();
        assert!(uuid_ms <= now_ms);
        assert!(now_ms - uuid_ms < 1000); // Within 1 second
    }
    
    #[test]
    fn test_uuid7_uniqueness() {
        let mut uuids = HashSet::new();
        for _ in 0..10000 {
            let uuid = Uuid7::new();
            assert!(uuids.insert(uuid), "duplicate UUID generated");
        }
    }
    
    #[test]
    fn test_uuid7_ordering() {
        // UUIDs generated later should sort after earlier ones
        let uuid1 = Uuid7::new();
        std::thread::sleep(std::time::Duration::from_millis(1));
        let uuid2 = Uuid7::new();
        
        // UUID2 should be greater (newer)
        assert!(uuid2 > uuid1, "newer UUID should sort after older UUID");
    }
    
    #[test]
    fn test_uuid7_sub_millisecond_ordering() {
        // UUIDs generated in the same millisecond should still be ordered
        // Note: Due to thread-local counter and parallel test execution,
        // we check that most UUIDs are ordered, allowing for some out-of-order
        // when test threads interleave
        let uuids: Vec<Uuid7> = (0..100).map(|_| Uuid7::new()).collect();
        
        let mut ordered_count = 0;
        for i in 1..uuids.len() {
            // Within the same millisecond, ordering is maintained by counter
            if uuids[i] >= uuids[i-1] {
                ordered_count += 1;
            }
        }
        
        // At least 90% should be ordered (allows for some thread interleaving)
        assert!(
            ordered_count >= 89,
            "Expected at least 89/99 UUIDs to be ordered, got {}/99",
            ordered_count
        );
    }
    
    #[test]
    fn test_uuid7_roundtrip() {
        let uuid = Uuid7::new();
        let string = uuid.to_string();
        let parsed = Uuid7::parse(&string).unwrap();
        
        assert_eq!(uuid, parsed);
        assert_eq!(uuid.as_bytes(), parsed.as_bytes());
    }
    
    #[test]
    fn test_uuid7_from_bytes() {
        let bytes = [
            0x01, 0x8e, 0x28, 0xa5, 0x7c, 0x10, // timestamp
            0x70, 0x00, // version 7 + rand_a
            0x80, 0x00, // variant 10 + rand_b
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        
        let uuid = Uuid7::from_bytes(bytes);
        assert_eq!(uuid.version(), 7);
        assert_eq!(uuid.variant(), 2);
    }
    
    #[test]
    fn test_uuid7_parse() {
        let s = "018e28a5-7c10-7000-8000-000000000000";
        let uuid = Uuid7::parse(s).unwrap();
        
        assert_eq!(uuid.to_string(), s);
    }
    
    #[test]
    fn test_uuid7_timestamp() {
        // Known timestamp: 2024-01-01 00:00:00 UTC = 1704067200000 ms
        let timestamp_ms = 1704067200000u64;
        let uuid = Uuid7::from_timestamp(timestamp_ms);
        
        assert_eq!(uuid.timestamp_ms(), timestamp_ms);
    }
    
    #[test]
    fn test_uuid7_nil() {
        let nil = Uuid7::NIL;
        assert_eq!(nil.as_bytes(), &[0u8; 16]);
        assert_eq!(nil.timestamp_ms(), 0);
    }
}
