# Coral Persistent Object Storage (Stores) - Technical Specification

_Created: 2026-01-06_
_Updated: 2026-01-08_

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [Core Concepts](#2-core-concepts) - Lifecycle, Default Attributes, Persistence Modes
3. [Syntax Design](#3-syntax-design) - Store Definition, Creation, Queries, Transactions
4. [Storage Architecture (Overview)](#4-storage-architecture-overview) - Dual Storage Model
5. [Memory-Mapped Access](#5-memory-mapped-access)
6. [Indexing](#6-indexing)
7. [Transactions & Consistency](#7-transactions--consistency) - ACID, Isolation Levels
8. [Journal/Event Sourcing](#8-journalevent-sourcing)
9. [Query Engine](#9-query-engine)
10. [Concurrency](#10-concurrency) - Actor-Safe Access, Locking
11. [Backup & Recovery](#11-backup--recovery)
12. [Implementation Roadmap](#12-implementation-roadmap)
13. [API Reference](#13-api-reference)
14. [Configuration System](#14-configuration-system) - **NEW: Hierarchical Config**
15. [Storage Architecture (Detailed)](#15-storage-architecture-detailed) - **NEW: File Formats, WAL**
16. [Success Criteria](#16-success-criteria)

---

## 1. Executive Summary

This document specifies a cutting-edge persistent object storage system for Coral. The system combines ideas from modern persistent memory research, append-only databases, and object-capability security to create a uniquely Coral-like persistence model.

### 1.1 Design Principles

1. **Coral-Native**: Persistence syntax feels natural to Coral, not bolted on
2. **Safe by Default**: No data loss without explicit action
3. **Simple Mental Model**: Developers reason about objects, not storage
4. **Zero-Copy When Possible**: Memory-mapped, direct access
5. **Concurrent-Safe**: Multiple actors can access stores safely
6. **Incremental**: Persistence is opt-in per store
7. **Configurable**: Hierarchical configuration at global, store-type, and instance levels
8. **Queryable**: Dual storage (binary + JSON) enables both performance and flexibility

---

## 2. Core Concepts


### 2.2 Default Attributes

All persistent store instances automatically receive these system-managed attributes:

| Attribute | Type | Description |
|-----------|------|-------------|
| `_index` | `Int` | Sequential ID (auto-increment per store type) |
| `_uuid` | `String` | Unique identifier (UUIDv7 for time-ordering) |
| `_created_at` | `Int` | Unix timestamp (milliseconds) when created |
| `_updated_at` | `Int` | Unix timestamp (milliseconds) of last modification |
| `_deleted_at` | `Int?` | Unix timestamp of soft delete, or `None` |
| `_version` | `Int` | Optimistic concurrency version number |

```coral
// These are automatically available on any store
store User
    name is ""
    email is ""

user is create User("data/users")
    name is "Alice"
    email is "alice@example.com"

// System attributes are accessible:
log(user._uuid)        // "01941c3e-1234-7abc-8def-0123456789ab"
log(user._index)       // 1
log(user._created_at)  // 1736323200000
log(user._version)     // 1

// Soft delete (sets _deleted_at, doesn't remove data)
soft_delete user

// Hard delete (removes from storage)
delete user
```

### 2.3 Persistence Modes

| Mode | Description | Use Case | WAL | Binary | JSON |
|------|-------------|----------|-----|--------|------|
| **Snapshot** | Periodic full snapshots | Config, settings | Optional | Yes | Optional |
| **Journal** | Append-only log of changes | Event sourcing | Yes | Yes | Optional |
| **Transactional** | ACID guarantees | Financial data | Yes | Yes | Yes |

---

## 3. Syntax Design

### 3.1 Store Definition

```coral
// Ephemeral store (default, current behavior)
store Person
    name is "Unknown"
    age is 0

// Persistent store with explicit annotation
persist store Account
    id is 0
    balance is 0.0
    &transactions    // Reference field for related data

// Persistent store with mode specification
persist(mode: journal) store EventLog
    events is []
```


### 3.3 Field Access and Mutation

```coral
// Read (same as ephemeral)
balance is account.balance

// Mutation triggers persistence
account.balance is account.balance + 50.0

// Batch mutations (single transaction)
with account
    balance is balance + 100.0
    transactions.push(make_Transaction("deposit", 100.0))
```

### 3.4 Queries

```coral
// Query syntax for finding stores
accounts is query Account
    where balance > 1000.0
    order_by balance desc
    limit 10

// Query with projection
names is query Account
    select name
    where active

// Query across relationships
rich_with_transactions is query Account
    where balance > 10000.0
    include transactions
```

### 3.5 Transactions

```coral
// Explicit transaction block
transaction
    from_account.balance is from_account.balance - amount
    to_account.balance is to_account.balance + amount
    
    // If any statement fails, all changes roll back
    if from_account.balance < 0
        throw !!Error:InsufficientFunds

// Transaction with isolation level
transaction(isolation: serializable)
    // Critical section
    ...
```

---

## 4. Storage Architecture (Overview)

> **Note**: For detailed file formats, see Section 15 (Storage Architecture - Detailed).

### 4.1 Dual Storage Model

Coral persistent stores use a **dual storage architecture**:

```
┌──────────────────────────────────────────────────────────────────┐
│                     Dual Storage Architecture                    │
├──────────────────────────────────────────────────────────────────┤
│                                                                  │
│  ┌─────────────┐      ┌─────────────┐      ┌─────────────────┐  │
│  │   Client    │ ──▶  │     WAL     │ ──▶  │  Data Files     │  │
│  │   Write     │      │  (Durability)│      │  (Persistence)  │  │
│  └─────────────┘      └─────────────┘      └─────────────────┘  │
│                                                   │              │
│                              ┌────────────────────┼──────────┐  │
│                              ▼                    ▼          │  │
│                        ┌──────────┐        ┌──────────┐      │  │
│                        │ data.bin │        │data.jsonl│      │  │
│                        │ (Binary) │        │  (JSON)  │      │  │
│                        └──────────┘        └──────────┘      │  │
│                              │                    │          │  │
│                              └────────┬───────────┘          │  │
│                                       ▼                      │  │
│                              ┌─────────────────┐             │  │
│                              │  Primary Index  │             │  │
│                              │  (index→uuid→   │             │  │
│                              │   offsets)      │             │  │
│                              └─────────────────┘             │  │
│                                       │                      │  │
│                              ┌────────┴────────┐             │  │
│                              ▼                 ▼             │  │
│                        ┌──────────┐      ┌──────────┐        │  │
│                        │Secondary │      │Secondary │        │  │
│                        │Index 1   │      │Index N   │        │  │
│                        └──────────┘      └──────────┘        │  │
│                                                              │  │
└──────────────────────────────────────────────────────────────────┘
```

**Why dual storage?**

| Storage | Purpose | Access Pattern |
|---------|---------|----------------|
| **Binary (data.bin)** | Fast reads, memory-mapping, compact | Point lookups, scans |
| **JSON (data.jsonl)** | Human-readable, external tools, debugging | Ad-hoc queries, export |
| **Index (primary.idx)** | Map _index → (uuid, binary_offset, json_offset) | All lookups |

### 4.2 System Attributes

Every persisted object automatically includes these attributes:

```coral
// These fields are managed by the runtime, not user code
store AnyType
    // System-managed (read-only to user code):
    // _index: Int        - Sequential ID, auto-increment
    // _uuid: String      - UUIDv7 (time-sortable)
    // _created_at: Int   - Creation timestamp (ms)
    // _updated_at: Int   - Last modification timestamp (ms)
    // _deleted_at: Int?  - Soft delete timestamp, or None
    // _version: Int      - Optimistic locking version
    
    // User-defined fields:
    name is ""
    value is 0
```

### 4.4 Value Encoding

| Type | Tag | Encoding |
|------|-----|----------|
| Unit | 0x00 | (no data) |
| Bool | 0x01 | 1 byte (0 or 1) |
| Int | 0x02 | varint |
| Float | 0x03 | 8 bytes (IEEE 754) |
| String | 0x04 | varint length + UTF-8 bytes |
| Bytes | 0x05 | varint length + raw bytes |
| List | 0x06 | varint length + elements |
| Map | 0x07 | varint length + key-value pairs |
| Reference | 0xFF | 8 bytes object ID |

---

## 5. Memory-Mapped Access

### 5.1 Zero-Copy Read Path

```
Application         Runtime              OS/Filesystem
     │                 │                      │
     │  account.balance                       │
     │ ──────────────▶ │                      │
     │                 │  Check memory map    │
     │                 │ ─────────────────▶   │
     │                 │  (page already mapped)
     │                 │ ◀─────────────────   │
     │                 │  Return pointer      │
     │  *value         │                      │
     │ ◀────────────── │                      │
```

### 5.2 Write Path with Copy-on-Write

```
Application         Runtime              OS/Filesystem
     │                 │                      │
     │  account.balance is 100.0              │
     │ ──────────────▶ │                      │
     │                 │  Copy page to buffer │
     │                 │  Modify in buffer    │
     │                 │  Append to journal   │
     │                 │ ──────────────────▶  │
     │                 │                      │ fsync
     │                 │  Update memory map   │
     │                 │ ◀──────────────────  │
     │  success        │                      │
     │ ◀────────────── │                      │
```

---

## 6. Indexing

### 6.1 Index Declaration

```coral
persit store User
    @index(unique)      // Primary key index
    id is 0
    
    @index              // Secondary index
    email is ""
    
    @index(composite: ["last_name", "first_name"])
    first_name is ""
    last_name is ""
    
    age is 0            // Not indexed
```

### 6.2 Index Implementation

- **B+ Tree**: Default for most indexes
- **Hash Index**: For equality-only lookups (declared with `@index(hash)`)
- **Bloom Filter**: Optional probabilistic filter for negative lookups

---

## 7. Transactions & Consistency

### 7.1 ACID Guarantees

| Property | Mechanism |
|----------|-----------|
| **Atomicity** | Write-ahead journal; rollback on failure |
| **Consistency** | Schema validation; constraint checks |
| **Isolation** | MVCC (Multi-Version Concurrency Control) |
| **Durability** | fsync after journal write; periodic checkpoint |

### 7.2 Isolation Levels

```coral
// Read committed (default)
transaction
    x is account.balance
    // Other transactions may modify account between reads

// Repeatable read
transaction(isolation: repeatable_read)
    x is account.balance
    y is account.balance
    assert_eq(x, y)  // Guaranteed equal

// Serializable
transaction(isolation: serializable)
    // Full isolation, may fail with conflict
```

### 7.3 Conflict Resolution

```coral
transaction
    account is open Account(id)
    if account.version != expected_version
        throw !!Error:ConcurrentModification
    account.balance is new_balance
    account.version is account.version + 1
```

---

## 8. Journal/Event Sourcing

### 8.1 Journal Store Definition

```coral
persist(mode: journal) store Order
    id is 0
    status is "pending"
    items is []
    total is 0.0

// Every mutation is recorded as an event
order is open Order(123)
order.status is "shipped"  // Recorded as SetField event
```


---

## 9. Query Engine

### 9.1 Query Optimization

```
Query: query Account where balance > 1000 and active

Optimizer steps:
1. Check available indexes
2. Estimate selectivity
3. Choose scan strategy:
   - If "balance" indexed: Index scan + filter
   - If "active" indexed: Index scan + filter
   - Otherwise: Full scan with filter
4. Generate query plan
```


### 9.3 Streaming Results

```coral
// For large result sets, use streaming
stream query Account where balance > 0
    | each(account -> process(account))
    | take(1000)
```

---

## 10. Concurrency

### 10.1 Actor-Safe Access

```coral
// Multiple actors can access same store
actor AccountManager
    @open(msg)
        account is open Account(msg.id)
        send(msg.reply_to, account.balance)

    @deposit(msg)
        account is open Account(msg.id)
        transaction
            account.balance is account.balance + msg.amount

// Transactions prevent race conditions
```

### 10.2 Locking Strategy

| Operation | Lock Type |
|-----------|-----------|
| Read field | Shared (read) lock |
| Write field | Exclusive (write) lock |
| Transaction | Exclusive lock on modified pages |
| Query | Snapshot isolation (no locks) |

### 10.3 Deadlock Prevention

```coral
// Runtime detects deadlock and aborts one transaction
transaction
    a is open Account(1)
    b is open Account(2)
    // Acquire locks in consistent order (by ID)
    // Runtime ensures: lock(1) before lock(2)
```

---


---

## 12. Implementation Roadmap

### Phase 1: Foundation (Weeks 1-4)

#### Tasks
- [ ] Configuration system (`coral.stores.toml` parser)
- [ ] System attributes (_index, _uuid, _created_at, _updated_at, _deleted_at, _version)
- [ ] UUIDv7 generation for time-sortable IDs
- [ ] Directory structure creation and management
- [ ] Schema file generation (`schema.json`)
- [ ] Basic `create` and `open` operations
- [ ] Memory-mapped file abstraction

### Phase 2: Storage Engine (Weeks 5-8)

#### Tasks
- [ ] Binary format writer (`data.bin`)
- [ ] JSON Lines writer (`data.jsonl`)
- [ ] Primary index implementation (`primary.idx`)
- [ ] Index-to-UUID-to-offset mapping
- [ ] Bloom filter for negative lookups
- [ ] Free list management for space reclamation

### Phase 3: WAL & Durability (Weeks 9-12)

#### Tasks
- [ ] WAL segment file format
- [ ] WAL append with configurable sync modes
- [ ] Checkpoint mechanism
- [ ] Recovery on startup (WAL replay)
- [ ] Basic transactions (single store)

### Phase 4: Query System (Weeks 13-16)

#### Tasks
- [ ] Query parser
- [ ] Query optimizer
- [ ] B+ tree secondary index implementation
- [ ] Full scan + filter (JSON-based for complex queries)
- [ ] Index-accelerated lookups

### Phase 5: Concurrency (Weeks 17-20)

#### Tasks
- [ ] MVCC implementation
- [ ] Isolation levels
- [ ] Deadlock detection
- [ ] Actor-safe access

### Phase 6: Advanced Features (Weeks 21-24)

#### Tasks
- [ ] Journal mode / event sourcing
- [ ] Backup/restore (full and incremental)
- [ ] Compaction and vacuum
- [ ] Integrity checking and repair
- [ ] Soft delete with configurable retention
- [ ] Auto-vacuum of expired soft-deleted records

---

## 13. API Reference

### 13.2 Transaction Operations

```coral
transaction                    // Begin transaction (default isolation)
transaction(isolation: level)  // With specific isolation
commit                         // Explicit commit (auto at end of block)
rollback                       // Explicit rollback
```

### 13.3 Query Operations

```coral
query Store                    // Begin query builder
    where condition            // Filter (can use system attrs like _created_at)
    where not _deleted_at      // Exclude soft-deleted (default behavior)
    where _deleted_at          // Include only soft-deleted
    select fields              // Projection
    order_by field [asc|desc]  // Sorting (e.g., order_by _created_at desc)
    limit n                    // Limit results
    offset n                   // Skip results
    include relation           // Eager load relations
    include_deleted            // Include soft-deleted records
```

### 13.4 Index Operations

```coral
@index                         // Declare index on field
@index(unique)                 // Unique index
@index(hash)                   // Hash index
@index(composite: [fields])    // Composite index

```

### 13.6 Lookup by System Attributes

```coral
// Open by sequential index (fast, uses primary index)
user is open User by_index 42

// Open by UUID (fast, uses primary index)
user is open User by_uuid "01941c3e-1234-7abc-8def-0123456789ab"

// Find by time range (uses _created_at index if configured)
recent_users is query User
    where _created_at > (now() - 24.hours)
    order_by _created_at desc

// Find recently modified
modified_today is query User
    where _updated_at > start_of_day()

// Find soft-deleted records pending vacuum
pending_delete is query User
    where _deleted_at
    where _deleted_at < (now() - 30.days)
```


---

## 14. Configuration System

The persistent store system uses a hierarchical configuration model where settings cascade from global defaults to store-type overrides to instance-specific options.

### 14.1 Configuration File (`coral.stores.toml`)

```toml
# coral.stores.toml - Global and store-type configuration

# ═══════════════════════════════════════════════════════════════════
# GLOBAL DEFAULTS
# Applied to all persistent stores unless overridden
# ═══════════════════════════════════════════════════════════════════

[global]
# Base directory for all store data
data_path = "/var/coral/data"

# Storage format settings
[global.storage]
binary_enabled = true           # Store binary representation for performance
json_enabled = true             # Store JSON representation for queryability
compression = "lz4"             # "none", "lz4", "zstd", "snappy"
compression_level = 3           # 1-9 (higher = smaller, slower)

# Write-Ahead Log (WAL) settings
[global.wal]
enabled = true                  # Enable WAL for crash recovery
path = "{data_path}/wal"        # WAL directory (supports {data_path} template)
max_size = "64MB"               # Max WAL size before forced checkpoint
sync_mode = "fsync"             # "none", "fdatasync", "fsync", "full"
checkpoint_interval = "5m"      # Time-based checkpoint trigger
checkpoint_threshold = 1000     # Operation count before checkpoint

# Memory and caching
[global.cache]
size = "256MB"                  # Total cache size for memory-mapped pages
eviction_policy = "lru"         # "lru", "lfu", "arc"
preload = false                 # Preload indexes on startup
pin_hot_pages = true            # Keep frequently accessed pages in memory

# Auto-persistence behavior
[global.auto_persist]
enabled = true                  # Automatically persist on field mutation
batch_window = "10ms"           # Batch writes within this window
batch_max_size = 100            # Max operations in a batch

# Index settings
[global.index]
type = "btree"                  # Default index type: "btree", "hash", "art"
page_size = 4096                # Index page size in bytes
fill_factor = 0.8               # Page fill factor for inserts
bloom_filter = true             # Enable bloom filters for negative lookups
bloom_fpr = 0.01                # Bloom filter false positive rate

# Soft delete behavior
[global.soft_delete]
enabled = true                  # Use soft delete by default
retention_period = "30d"        # How long to keep soft-deleted records
auto_vacuum = true              # Automatically vacuum expired records
vacuum_interval = "24h"         # How often to run vacuum

# Backup settings
[global.backup]
auto_backup = false             # Enable automatic backups
backup_path = "{data_path}/backups"
backup_interval = "24h"         # Time between backups
backup_retention = 7            # Number of backups to keep
incremental = true              # Use incremental backups

# ═══════════════════════════════════════════════════════════════════
# STORE-TYPE OVERRIDES
# Override global settings for specific store types
# Section name must match the store type name
# ═══════════════════════════════════════════════════════════════════

[stores.User]
# Override data path for User stores
data_path = "{global.data_path}/users"

# Users need both binary and JSON for different access patterns
[stores.User.storage]
binary_enabled = true
json_enabled = true

# User data is critical - use strongest durability
[stores.User.wal]
sync_mode = "fsync"

# Index email for fast lookups
[stores.User.indexes]
email = { type = "btree", unique = true }
username = { type = "hash", unique = true }
created_at = { type = "btree" }

# ───────────────────────────────────────────────────────────────────

[stores.EventLog]
# Event logs are append-only, different performance profile
[stores.EventLog.storage]
binary_enabled = true
json_enabled = false            # No JSON needed for event logs
compression = "zstd"
compression_level = 6           # Higher compression for logs

# Optimize WAL for append-heavy workload
[stores.EventLog.wal]
sync_mode = "fdatasync"         # Slightly relaxed durability OK
checkpoint_interval = "15m"     # Less frequent checkpoints

# Events are never truly deleted
[stores.EventLog.soft_delete]
enabled = false

# ───────────────────────────────────────────────────────────────────

[stores.Session]
# Sessions are ephemeral - minimal persistence
[stores.Session.storage]
binary_enabled = true
json_enabled = false

[stores.Session.wal]
enabled = false                 # Sessions can be lost

[stores.Session.auto_persist]
enabled = true
batch_window = "100ms"          # Larger batch window OK

# Sessions expire and get cleaned up
[stores.Session.soft_delete]
retention_period = "7d"
```

### 14.2 Store-Level Configuration in Code

```coral
// Override configuration at store definition time
store Account
    @config(
        auto_persist: true,
        sync_mode: 'fsync',
        json_enabled: true,
        indexes: [
            ('account_number', unique: true, type: 'hash'),
            ('balance', type: 'btree'),
            ('customer_id', type: 'btree')
        ]
    )
    
    account_number is ""
    balance is 0.0
    customer_id is ""

```


## 15. Storage Architecture (Detailed)

### 15.1 Directory Structure

```
{data_path}/
├── coral.stores.toml           # Configuration file
├── _meta/
│   ├── store_registry.json     # All known store types
│   └── schema_versions.json    # Schema migration history
├── wal/
│   ├── wal-00001.log          # Write-ahead log segments
│   ├── wal-00002.log
│   └── checkpoint.meta        # Last checkpoint info
├── users/                      # Store type: User
│   ├── _index/
│   │   ├── primary.idx        # index -> (uuid, binary_offset, json_offset)
│   │   ├── email.idx          # email -> index (secondary)
│   │   └── username.idx       # username -> index (secondary)
│   ├── data.bin               # Binary storage (compact, fast)
│   ├── data.jsonl             # JSON Lines storage (queryable)
│   └── schema.json            # Field definitions and types
├── events/                     # Store type: EventLog
│   ├── _index/
│   │   └── primary.idx
│   ├── data.bin
│   └── schema.json
└── backups/
    ├── 2026-01-08T00:00:00Z/
    └── 2026-01-07T00:00:00Z/
```

### 15.2 Primary Index Structure

The primary index maps sequential IDs to UUIDs and storage offsets:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Primary Index File                          │
├─────────────────────────────────────────────────────────────────┤
│  Header (64 bytes)                                              │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Magic: "CORALIDX" (8 bytes)                                 ││
│  │ Version: u32                                                ││
│  │ Entry Count: u64                                            ││
│  │ Next Index: u64 (next auto-increment value)                 ││
│  │ Bloom Filter Offset: u64                                    ││
│  │ Checksum: u64                                               ││
│  │ Reserved: 16 bytes                                          ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│  Index Entries (sorted by _index for binary search)            │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ For each entry (64 bytes fixed):                            ││
│  │   _index: u64          (sequential ID)                      ││
│  │   _uuid: [u8; 16]      (UUIDv7 bytes)                       ││
│  │   binary_offset: u64   (offset in data.bin, 0 = deleted)    ││
│  │   binary_length: u32   (length in data.bin)                 ││
│  │   json_offset: u64     (offset in data.jsonl)               ││
│  │   json_length: u32     (length in data.jsonl)               ││
│  │   flags: u16           (deleted, compressed, etc.)          ││
│  │   _version: u16        (optimistic concurrency)             ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│  Bloom Filter (for fast negative lookups)                      │
└─────────────────────────────────────────────────────────────────┘
```

### 15.3 Binary Storage Format (`data.bin`)

Optimized for fast reads and memory-mapping:

```
┌─────────────────────────────────────────────────────────────────┐
│                     Binary Data File                            │
├─────────────────────────────────────────────────────────────────┤
│  File Header (64 bytes)                                         │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Magic: "CORALBIN" (8 bytes)                                 ││
│  │ Version: u32                                                ││
│  │ Store Type Hash: u64                                        ││
│  │ Schema Version: u32                                         ││
│  │ Object Count: u64                                           ││
│  │ File Size: u64                                              ││
│  │ Free Space Offset: u64 (start of free list)                 ││
│  │ Checksum: u64                                               ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│  Object Records (variable length, 8-byte aligned)              │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Record Header (24 bytes):                                   ││
│  │   record_length: u32   (including header)                   ││
│  │   flags: u16           (active, deleted, compressed)        ││
│  │   field_count: u16                                          ││
│  │   _index: u64                                               ││
│  │   _version: u32                                             ││
│  │   checksum: u32        (CRC32 of payload)                   ││
│  │                                                             ││
│  │ System Fields (fixed layout):                               ││
│  │   _uuid: [u8; 16]                                           ││
│  │   _created_at: i64                                          ││
│  │   _updated_at: i64                                          ││
│  │   _deleted_at: i64     (-1 if not deleted)                  ││
│  │                                                             ││
│  │ User Fields (schema-driven):                                ││
│  │   For each field:                                           ││
│  │     tag: u8            (type tag)                           ││
│  │     value: [u8; N]     (type-dependent encoding)            ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│  Free List (for space reclamation)                             │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Free block headers link together                            ││
│  │   next_free: u64                                            ││
│  │   block_size: u32                                           ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

### 15.4 JSON Storage Format (`data.jsonl`)

JSON Lines format for easy querying and debugging:

```jsonl
{"_index":1,"_uuid":"01941c3e-1234-7abc-8def-0123456789ab","_created_at":1736323200000,"_updated_at":1736323200000,"_deleted_at":null,"_version":1,"name":"Alice","email":"alice@example.com"}
{"_index":2,"_uuid":"01941c3e-5678-7abc-8def-0123456789cd","_created_at":1736323201000,"_updated_at":1736323215000,"_deleted_at":null,"_version":3,"name":"Bob","email":"bob@example.com"}
{"_index":3,"_uuid":"01941c3e-9abc-7abc-8def-0123456789ef","_created_at":1736323202000,"_updated_at":1736323202000,"_deleted_at":1736409602000,"_version":1,"name":"Charlie","email":"charlie@example.com"}
```

Benefits:
- Human-readable for debugging
- Line-oriented for streaming and external tools (grep, jq, etc.)
- Easy to import into other databases
- Supports external full-text search integration

### 15.5 Write-Ahead Log (WAL)

The WAL ensures durability and crash recovery:

```
┌─────────────────────────────────────────────────────────────────┐
│                     WAL Segment File                            │
├─────────────────────────────────────────────────────────────────┤
│  Segment Header (32 bytes)                                      │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Magic: "CORALWAL" (8 bytes)                                 ││
│  │ Segment ID: u64                                             ││
│  │ Previous Segment: u64 (0 if first)                          ││
│  │ Created At: u64                                             ││
│  └─────────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────────┤
│  WAL Entries (append-only)                                     │
│  ┌─────────────────────────────────────────────────────────────┐│
│  │ Entry Header (16 bytes):                                    ││
│  │   lsn: u64             (Log Sequence Number)                ││
│  │   entry_type: u8       (see below)                          ││
│  │   flags: u8                                                 ││
│  │   payload_length: u16                                       ││
│  │   checksum: u32                                             ││
│  │                                                             ││
│  │ Entry Types:                                                ││
│  │   0x01 = CREATE        (store_type, _index, full_data)      ││
│  │   0x02 = UPDATE        (store_type, _index, field_changes)  ││
│  │   0x03 = DELETE        (store_type, _index)                 ││
│  │   0x04 = SOFT_DELETE   (store_type, _index, timestamp)      ││
│  │   0x10 = TXN_BEGIN     (transaction_id)                     ││
│  │   0x11 = TXN_COMMIT    (transaction_id)                     ││
│  │   0x12 = TXN_ROLLBACK  (transaction_id)                     ││
│  │   0x20 = CHECKPOINT    (checkpoint metadata)                ││
│  │   0x21 = SCHEMA_CHANGE (store_type, schema_delta)           ││
│  └─────────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────────┘
```

#### WAL Workflow

```
Write Path:
1. Write operation requested
2. Validate and serialize operation
3. Append to WAL with LSN
4. fsync (based on sync_mode config)
5. Apply to in-memory state
6. Return success to caller
7. (Async) Apply to binary/JSON files

Recovery Path:
1. On startup, read checkpoint.meta
2. Find last valid checkpoint LSN
3. Replay WAL entries from checkpoint LSN
4. Rebuild in-memory indexes
5. Resume normal operation

Checkpoint Process:
1. Pause new writes momentarily
2. Flush all pending changes to data files
3. Sync all data files
4. Write checkpoint entry to WAL
5. Update checkpoint.meta
6. (Optionally) Archive old WAL segments
7. Resume writes
```

---

## 16. Success Criteria

1. **Correctness**: No data loss under normal operation
2. **Performance**: Read throughput > 100k ops/sec
3. **Durability**: Survive process crash, OS crash, power loss
4. **Simplicity**: Developers can use without deep storage knowledge
5. **Coral-Native**: Syntax feels natural, not foreign
6. **Configurable**: Easy tuning without code changes
7. **Observable**: Clear visibility into storage behavior

---

## Appendix A: Quick Reference

### System Attributes (Automatic)

| Attribute | Type | Description |
|-----------|------|-------------|
| `_index` | `Int` | Sequential ID (auto-increment) |
| `_uuid` | `String` | UUIDv7 (time-sortable, globally unique) |
| `_created_at` | `Int` | Creation timestamp (milliseconds) |
| `_updated_at` | `Int` | Last modification timestamp |
| `_deleted_at` | `Int?` | Soft delete timestamp or `None` |
| `_version` | `Int` | Optimistic concurrency version |

### Configuration Hierarchy

```
Built-in Defaults
    ↓ (overridden by)
Global Config ([global] in coral.stores.toml)
    ↓ (overridden by)
Store-Type Config ([stores.TypeName])
    ↓ (overridden by)
Definition-Time Config (@config(...) on store)
    ↓ (overridden by)
Instance-Time Config (@config(...) at create)
    ↓ (overridden by)
Environment Variables (CORAL_STORES_*)
```

### Storage Files Per Store Type

```
{store_type}/
├── _index/
│   ├── primary.idx      # index → (uuid, offsets)
│   └── {field}.idx      # secondary indexes
├── data.bin             # Binary storage (fast)
├── data.jsonl           # JSON Lines (queryable)
└── schema.json          # Field definitions
```

### WAL Entry Types

| Type | Code | Purpose |
|------|------|---------|
| CREATE | 0x01 | New object created |
| UPDATE | 0x02 | Field(s) modified |
| DELETE | 0x03 | Hard delete |
| SOFT_DELETE | 0x04 | Set _deleted_at |
| TXN_BEGIN | 0x10 | Transaction start |
| TXN_COMMIT | 0x11 | Transaction commit |
| TXN_ROLLBACK | 0x12 | Transaction abort |
| CHECKPOINT | 0x20 | Checkpoint marker |
| SCHEMA_CHANGE | 0x21 | Schema migration |
