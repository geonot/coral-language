mod map_hash;
mod rc_deferred;
mod module_registry;
mod actor;
mod memory_ops;
mod store;
mod weak_ref;
mod cycle_detector;
mod symbol;

// NaN-boxing value representation (M1)
pub mod nanbox;
pub mod nanbox_ffi;

// Split modules (IQ-3)
pub mod actor_ops;
pub mod arithmetic;
pub mod bytes_ops;
pub mod closure_ops;
pub mod encoding_ops;
pub mod error_ffi;
pub mod io_ops;
pub mod json_ops;
pub mod list_ops;
pub mod map_ops;
pub mod math_ops;
pub mod metrics;
pub mod random_ops;
pub mod rc_ops;
pub mod string_ops;
pub mod tagged_ops;
pub mod time_ops;

// Re-export split modules so cross-module calls work
pub use actor_ops::*;
pub use arithmetic::*;
pub use bytes_ops::*;
pub use closure_ops::*;
pub use encoding_ops::*;
pub use error_ffi::*;
pub use io_ops::*;
pub use json_ops::*;
pub use list_ops::*;
pub use map_ops::*;
pub use math_ops::*;
pub use metrics::*;
pub use random_ops::*;
pub use rc_ops::*;
pub use string_ops::*;
pub use tagged_ops::*;
pub use time_ops::*;


// Re-export memory operations for FFI
pub use memory_ops::*;
pub use store::{
    StoreEngine, SharedStoreEngine, StoredValue, StoreConfig,
    open_store_engine, save_all_engines, close_engine,
    // FFI functions
    coral_store_open, coral_store_close, coral_store_save_all,
    coral_store_create, coral_store_get_by_index, coral_store_get_by_uuid,
    coral_store_update, coral_store_soft_delete, coral_store_stats,
    coral_store_count, coral_store_persist, coral_store_checkpoint,
    coral_store_all_indices,
};
pub use weak_ref::{
    WeakRef, notify_value_deallocated, weak_ref_count,
    coral_make_weak_ref, coral_weak_ref_upgrade, coral_weak_ref_is_alive,
    coral_weak_ref_release, coral_weak_ref_clone,
};
pub use cycle_detector::{
    collect_cycles, possible_root, cycle_stats, reset_cycle_detector,
    auto_cycle_collection_enabled,
    coral_collect_cycles, coral_cycles_detected, coral_cycle_values_collected,
    coral_cycle_roots_count, coral_force_cycle_collection, 
    coral_set_auto_cycle_collection, coral_get_auto_cycle_collection,
};
pub use symbol::{
    SymbolId, SymbolTable, global_symbols, intern, resolve,
    coral_symbol_intern, coral_symbol_lookup, coral_symbol_resolve,
    coral_symbol_equals, coral_symbol_count,
};

use libc::{free, malloc};
use std::cell::RefCell;
use std::env;
use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::ptr;
use std::slice;
use std::hash::{Hash, Hasher};
use std::collections::{hash_map::DefaultHasher, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, Once, OnceLock};
use std::thread_local;
use std::time::{SystemTime, UNIX_EPOCH};

pub use module_registry::{Capability, RuntimeModule, RuntimeModuleRegistry, registry as runtime_module_registry};
pub use actor::{
    ActorId, ActorHandle, ActorSystem, ActorConfig, ActorContext,
    Message as ActorMessage, SendResult, MailboxStats,
    current_actor, global_system, get_mailbox_stats,
    DEFAULT_MAILBOX_CAPACITY,
};

const FLAG_INLINE_STRING: u8 = 0b0000_0001;
const FLAG_LIST_ITER: u8 = 0b0000_0010;
const FLAG_MAP_ITER: u8 = 0b0000_0100;
const FLAG_FROZEN: u8 = 0b0000_1000;
/// Value represents an error state - payload points to ErrorMetadata
const FLAG_ERR: u8 = 0b0001_0000;
/// Value is logically absent/None/missing
const FLAG_ABSENT: u8 = 0b0010_0000;
const PAGE_SIZE: usize = 4096;
const VALUE_POOL_LIMIT: usize = 8192;
const LOCAL_POOL_LIMIT: usize = 256;

/// Error metadata stored when FLAG_ERR is set on a value.
/// The error value's payload.ptr points to this struct.
#[repr(C)]
pub struct ErrorMetadata {
    /// Numeric error code (user-defined or 0 for anonymous errors)
    pub code: u32,
    /// Reserved for future use (alignment)
    pub _reserved: u32,
    /// Error name as a string value (e.g., "NotFound", "Connection:Timeout")
    pub name: ValueHandle,
    /// Origin span ID for error tracing (0 if unknown)
    pub origin_span: u64,
}

/// Placeholder for an actor-ready header; currently unused but reserved for atomic RC adoption.
#[repr(C)]
pub struct ValueHeader {
    pub refcount: AtomicU64,
    pub flags: u32,
}

static RETAIN_COUNT: AtomicU64 = AtomicU64::new(0);
static RETAIN_SATURATED: AtomicU64 = AtomicU64::new(0);
static RELEASE_COUNT: AtomicU64 = AtomicU64::new(0);
static RELEASE_UNDERFLOW: AtomicU64 = AtomicU64::new(0);
static LIVE_VALUE_COUNT: AtomicU64 = AtomicU64::new(0);
static VALUE_POOL_HITS: AtomicU64 = AtomicU64::new(0);
static VALUE_POOL_MISSES: AtomicU64 = AtomicU64::new(0);
static HEAP_BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static STRING_BYTES_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static LIST_SLOTS_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static MAP_SLOTS_ALLOCATED: AtomicU64 = AtomicU64::new(0);
static STACK_PAGES_COMMITTED: AtomicU64 = AtomicU64::new(0);
static STACK_BYTES_REQUESTED: AtomicU64 = AtomicU64::new(0);
static METRICS_PATH: OnceLock<PathBuf> = OnceLock::new();
static METRICS_ONCE: Once = Once::new();
static METRICS_ENABLED: AtomicBool = AtomicBool::new(false);
static USAGE_METRICS: OnceLock<Mutex<UsageWindow>> = OnceLock::new();
const USAGE_WINDOW_SECS: u64 = 60;

// Cycle collection trigger counters
static CYCLE_COLLECTION_COUNTER: AtomicU64 = AtomicU64::new(0);
const CYCLE_COLLECTION_THRESHOLD: u64 = 1000; // Trigger cycle collection every 1000 releases

// ── Thread-local ownership ID for non-atomic RC fast path (M2.1) ─────────────
// Thread IDs start at 1; 0 is the sentinel for "shared/atomic mode".
// Assigned once per thread via a global counter.
static THREAD_ID_COUNTER: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static LOCAL_THREAD_ID: u32 = THREAD_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
}

/// Get the current thread's unique ownership ID. Cached in thread-local storage.
#[inline]
pub(crate) fn current_thread_id() -> u32 {
    LOCAL_THREAD_ID.with(|&id| id)
}

pub type ValueHandle = *mut Value;

// Wrapper to mark ValueHandle as Send when explicitly intended to cross threads.
#[derive(Clone, Copy)]
struct SendValueHandle(ValueHandle);
// Safety: Values are refcounted; callers must freeze before sharing.
unsafe impl Send for SendValueHandle {}

struct ValuePool(Vec<ValueHandle>);

// Value handles are opaque pointers managed by the runtime; pool storage is confined to the runtime.
static VALUE_POOL: OnceLock<Mutex<ValuePool>> = OnceLock::new();
unsafe impl Send for ValuePool {}
unsafe impl Sync for ValuePool {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UsageKind {
    StackAllocSuccess,
    StackAllocFailure,
    HeapAllocBytes,
    CopyElided,
    CowBreak,
}

#[derive(Default)]
struct UsageWindow {
    started_at: u64,
    counters: HashMap<UsageKind, u64>,
}

impl UsageWindow {
    fn record(&mut self, kind: UsageKind, amount: u64) {
        *self.counters.entry(kind).or_insert(0) += amount;
    }

    fn is_stale(&self, now: u64) -> bool {
        now.saturating_sub(self.started_at) > USAGE_WINDOW_SECS
    }
}

fn usage_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn record_usage(kind: UsageKind, amount: u64) {
    if !METRICS_ENABLED.load(Ordering::SeqCst) {
        return;
    }
    let window = USAGE_METRICS.get_or_init(|| {
        Mutex::new(UsageWindow {
            started_at: usage_now(),
            counters: HashMap::new(),
        })
    });
    let now = usage_now();
    if let Ok(mut guard) = window.lock() {
        if guard.is_stale(now) {
            guard.counters.clear();
            guard.started_at = now;
        }
        guard.record(kind, amount.max(1));
    }
}

pub fn usage_snapshot() -> HashMap<UsageKind, u64> {
    USAGE_METRICS
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.counters.clone()))
        .unwrap_or_default()
}

pub fn write_usage_snapshot_to(path: &Path) {
    if let Some(map) = USAGE_METRICS
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.counters.clone()))
    {
        if let Ok(mut file) = File::create(path) {
            for (k, v) in map {
                let _ = writeln!(file, "{:?},{}", k, v);
            }
        }
    }
}

thread_local! {
    static RELEASE_QUEUE: RefCell<Option<rc_deferred::ReleaseQueue>> = RefCell::new(None);
    static LOCAL_VALUE_POOL: RefCell<Vec<ValueHandle>> = RefCell::new(Vec::with_capacity(LOCAL_POOL_LIMIT));
}

type ClosureInvokeFn = Option<unsafe extern "C" fn(*mut c_void, *const ValueHandle, usize, *mut ValueHandle)>;
type ClosureReleaseFn = Option<unsafe extern "C" fn(*mut c_void)>;

struct ClosureObject {
    invoke: ClosureInvokeFn,
    release: ClosureReleaseFn,
    env: *mut c_void,
    /// M3.4: Number of captured NaN-boxed i64 values in the env struct.
    capture_count: usize,
}

/// Tagged value for ADT (algebraic data type / sum type) variants.
/// Stores a tag (variant identifier) and a list of field values.
#[repr(C)]
pub struct TaggedValue {
    /// The tag/discriminant identifying which variant this is.
    /// This is a string pointer to the variant name (e.g., "Some", "None").
    pub tag_name: *const u8,
    pub tag_name_len: usize,
    /// Number of fields in this variant.
    pub field_count: usize,
    /// Pointer to array of field values (ValueHandle pointers).
    pub fields: *mut ValueHandle,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValueTag {
    Number = 0,
    Bool = 1,
    String = 2,
    List = 3,
    Map = 4,
    Store = 5,
    Actor = 6,
    Unit = 7,
    Closure = 8,
    Bytes = 9,
    /// Tagged value for ADT (sum type) variants
    /// Payload points to TaggedValue struct
    Tagged = 10,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union Payload {
    pub number: f64,
    pub ptr: *mut c_void,
    pub inline: [u8; 16],
}

impl Default for Payload {
    fn default() -> Self {
        Self { inline: [0u8; 16] }
    }
}

#[repr(C)]
pub struct Value {
    pub tag: u8,
    pub flags: u8,
    pub reserved: u16,
    /// Thread that owns this value. Non-zero means thread-local (non-atomic RC).
    /// Zero means shared/atomic mode (promoted at freeze or cross-thread access).
    /// Fills alignment padding before AtomicU64, so adds 0 bytes to struct size.
    pub owner_thread: u32,
    /// Reference count - uses atomic operations only when owner_thread == 0 (shared mode).
    /// When owner_thread matches current thread, plain load/store is used (fast path).
    pub refcount: AtomicU64,
    #[cfg(feature = "metrics")]
    pub retain_events: AtomicU32,
    #[cfg(feature = "metrics")]
    pub release_events: AtomicU32,
    pub payload: Payload,
}

// Safety: Values are managed by atomic refcounting and only shared across threads as frozen handles.
unsafe impl Send for Value {}
unsafe impl Sync for Value {}

impl Clone for Value {
    fn clone(&self) -> Self {
        Self {
            tag: self.tag,
            flags: self.flags,
            reserved: self.reserved,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(self.refcount.load(Ordering::Relaxed)),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(self.retain_events.load(Ordering::Relaxed)),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(self.release_events.load(Ordering::Relaxed)),
            payload: self.payload,
        }
    }
}

impl Value {
    fn unit() -> Self {
        Self {
            tag: ValueTag::Unit as u8,
            flags: 0,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { inline: [0; 16] },
        }
    }

    fn number(value: f64) -> Self {
        Self {
            tag: ValueTag::Number as u8,
            flags: 0,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { number: value },
        }
    }

    fn boolean(value: bool) -> Self {
        let byte = if value { 1u8 } else { 0u8 };
        let mut inline = [0u8; 16];
        inline[0] = byte;
        Self {
            tag: ValueTag::Bool as u8,
            flags: 0,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { inline },
        }
    }

    fn from_heap(tag: ValueTag, ptr: *mut c_void) -> Self {
        Self {
            tag: tag as u8,
            flags: 0,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { ptr },
        }
    }

    fn from_heap_with_flags(tag: ValueTag, flags: u8, ptr: *mut c_void) -> Self {
        Self {
            tag: tag as u8,
            flags,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { ptr },
        }
    }

    fn inline_string(bytes: &[u8]) -> Self {
        debug_assert!(bytes.len() <= 14);
        let mut inline = [0u8; 16];
        inline[..bytes.len()].copy_from_slice(bytes);
        Self {
            tag: ValueTag::String as u8,
            flags: FLAG_INLINE_STRING | ((bytes.len() as u8) << 1),
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { inline },
        }
    }

    fn is_inline_string(&self) -> bool {
        self.tag == ValueTag::String as u8 && (self.flags & FLAG_INLINE_STRING) != 0
    }

    fn heap_ptr(&self) -> *mut c_void {
        unsafe { self.payload.ptr }
    }

    /// Returns true if this value represents an error state.
    /// NOTE: Inline strings encode their length in the flags byte via `(len << 1) | FLAG_INLINE_STRING`.
    /// For strings of length >= 8, bits 4+ overlap with FLAG_ERR / FLAG_ABSENT.
    /// We guard against this by excluding inline strings from the error/absent checks.
    #[inline]
    fn is_err(&self) -> bool {
        (self.flags & FLAG_INLINE_STRING) == 0 && (self.flags & FLAG_ERR) != 0
    }

    /// Returns true if this value is logically absent/None.
    #[inline]
    fn is_absent(&self) -> bool {
        (self.flags & FLAG_INLINE_STRING) == 0 && (self.flags & FLAG_ABSENT) != 0
    }

    /// Returns true if this value is neither an error nor absent.
    #[inline]
    fn is_ok(&self) -> bool {
        (self.flags & FLAG_INLINE_STRING) != 0 || (self.flags & (FLAG_ERR | FLAG_ABSENT)) == 0
    }

    /// Create an error value with the given metadata.
    fn error(metadata: *mut ErrorMetadata) -> Self {
        Self {
            tag: ValueTag::Unit as u8,  // Error values have unit as base type
            flags: FLAG_ERR,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { ptr: metadata as *mut c_void },
        }
    }

    /// Create an absent/None value.
    fn absent() -> Self {
        Self {
            tag: ValueTag::Unit as u8,
            flags: FLAG_ABSENT,
            reserved: 0,
            owner_thread: current_thread_id(),
            refcount: AtomicU64::new(1),
            #[cfg(feature = "metrics")]
            retain_events: AtomicU32::new(0),
            #[cfg(feature = "metrics")]
            release_events: AtomicU32::new(0),
            payload: Payload { inline: [0; 16] },
        }
    }

    /// Get error metadata if this is an error value.
    fn error_metadata(&self) -> Option<&ErrorMetadata> {
        if self.is_err() {
            unsafe { Some(&*(self.payload.ptr as *const ErrorMetadata)) }
        } else {
            None
        }
    }
}

pub(crate) fn alloc_value(value: Value) -> ValueHandle {
    ensure_runtime_initialized();
    LIVE_VALUE_COUNT.fetch_add(1, Ordering::Relaxed);
    // Fast path: thread-local pool (no locking)
    let local = LOCAL_VALUE_POOL.with(|pool| {
        pool.borrow_mut().pop()
    });
    if let Some(handle) = local {
        VALUE_POOL_HITS.fetch_add(1, Ordering::Relaxed);
        unsafe { ptr::write(handle, value); }
        return handle;
    }
    // Slow path: global pool (with mutex)
    let pool = value_pool();
    if let Ok(mut slots) = pool.lock() {
        if let Some(handle) = slots.0.pop() {
            VALUE_POOL_HITS.fetch_add(1, Ordering::Relaxed);
            unsafe { ptr::write(handle, value); }
            return handle;
        }
    }
    VALUE_POOL_MISSES.fetch_add(1, Ordering::Relaxed);
    record_heap_bytes(std::mem::size_of::<Value>());
    Box::into_raw(Box::new(value))
}

fn value_pool() -> &'static Mutex<ValuePool> {
    // Initialize with a pre-allocated pool to reduce heap churn.
    VALUE_POOL.get_or_init(|| Mutex::new(ValuePool(Vec::with_capacity(VALUE_POOL_LIMIT))))
}

fn recycle_value_box(handle: ValueHandle) -> bool {
    // Fast path: thread-local pool (no locking, no duplicate scan needed
    // since refcounting guarantees each handle is freed exactly once)
    let recycled = LOCAL_VALUE_POOL.with(|pool| {
        if let Ok(mut p) = pool.try_borrow_mut() {
            if p.len() < LOCAL_POOL_LIMIT {
                unsafe {
                    (*handle).refcount.store(0, Ordering::Relaxed);
                    (*handle).owner_thread = 0;
                    #[cfg(feature = "metrics")] {
                        (*handle).retain_events.store(0, Ordering::Relaxed);
                        (*handle).release_events.store(0, Ordering::Relaxed);
                    }
                }
                p.push(handle);
                return true;
            }
        }
        false
    });
    if recycled { return true; }
    // Overflow to global pool
    let pool = value_pool();
    if let Ok(mut slots) = pool.lock() {
        if slots.0.len() < VALUE_POOL_LIMIT {
            unsafe {
                (*handle).refcount.store(0, Ordering::Relaxed);
                (*handle).owner_thread = 0;
                #[cfg(feature = "metrics")] {
                    (*handle).retain_events.store(0, Ordering::Relaxed);
                    (*handle).release_events.store(0, Ordering::Relaxed);
                }
            }
            slots.0.push(handle);
            return true;
        }
    }
    false
}

fn record_heap_bytes(bytes: usize) {
    if bytes == 0 {
        return;
    }
    HEAP_BYTES_ALLOCATED.fetch_add(bytes as u64, Ordering::Relaxed);
}

fn ensure_runtime_initialized() {
    METRICS_ONCE.call_once(|| {
        configure_metrics_from_env();
    });
}

fn configure_metrics_from_env() {
    if let Ok(path) = env::var("CORAL_RUNTIME_METRICS") {
        if path.trim().is_empty() {
            return;
        }
        let resolved = PathBuf::from(path);
        if METRICS_PATH.set(resolved).is_ok() {
            METRICS_ENABLED.store(true, Ordering::SeqCst);
            unsafe {
                libc::atexit(dump_metrics_atexit);
            }
        }
    }
}

extern "C" fn dump_metrics_atexit() {
    dump_metrics_to_configured_path();
}

fn dump_metrics_to_configured_path() {
    if !METRICS_ENABLED.load(Ordering::SeqCst) {
        return;
    }
    if let Some(path) = METRICS_PATH.get() {
        dump_metrics_to_path(path.as_path());
    }
}

fn dump_metrics_to_path(path: &Path) {
    if path.as_os_str().is_empty() {
        return;
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            let _ = fs::create_dir_all(parent);
        }
    }
    if let Ok(mut file) = File::create(path) {
        let metrics = snapshot_runtime_metrics();
        let payload = metrics_json(&metrics);
        let _ = file.write_all(payload.as_bytes());
    }
}

struct StringObject {
    data: Vec<u8>,
}

struct BytesObject {
    data: Vec<u8>,
}

// Made pub(crate) for cycle_detector access
pub struct ListObject {
    pub items: Vec<ValueHandle>,
}

struct ListIter {
    items: Vec<ValueHandle>,
    index: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MapEntry {
    pub key: ValueHandle,
    pub value: ValueHandle,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
// Made pub(crate) for cycle_detector access
pub(crate) enum MapBucketState {
    Empty,
    Tombstone,
    Occupied,
}

// Made pub(crate) for cycle_detector access
#[derive(Clone)]
pub(crate) struct MapBucket {
    pub(crate) state: MapBucketState,
    pub(crate) hash: u64,
    pub(crate) key: ValueHandle,
    pub(crate) value: ValueHandle,
}

impl Default for MapBucket {
    fn default() -> Self {
        Self {
            state: MapBucketState::Empty,
            hash: 0,
            key: ptr::null_mut(),
            value: ptr::null_mut(),
        }
    }
}

// Made pub(crate) for cycle_detector access
pub(crate) struct MapObject {
    pub(crate) buckets: Vec<MapBucket>,
    pub(crate) len: usize,
    tombstones: usize,
}

struct MapIter {
    buckets: Vec<MapBucket>,
    index: usize,
}

#[derive(Clone)]
struct ActorObject {
    handle: ActorHandle,
    system: ActorSystem,
}

fn alloc_string(bytes: &[u8]) -> *mut c_void {
    let data = bytes.to_vec();
    STRING_BYTES_ALLOCATED.fetch_add(data.len() as u64, Ordering::Relaxed);
    record_heap_bytes(std::mem::size_of::<StringObject>() + data.capacity());
    let obj = Box::new(StringObject { data });
    Box::into_raw(obj) as *mut c_void
}

fn alloc_bytes_obj(bytes: &[u8]) -> *mut c_void {
    let data = bytes.to_vec();
    STRING_BYTES_ALLOCATED.fetch_add(data.len() as u64, Ordering::Relaxed);
    record_heap_bytes(std::mem::size_of::<BytesObject>() + data.capacity());
    let obj = Box::new(BytesObject { data });
    Box::into_raw(obj) as *mut c_void
}

fn alloc_list(items: &[ValueHandle]) -> *mut c_void {
    let mut retained = Vec::with_capacity(items.len());
    coral_value_retain_many(items.as_ptr(), items.len());
    for &handle in items {
        if !handle.is_null() {
            retained.push(handle);
        }
    }
    LIST_SLOTS_ALLOCATED.fetch_add(retained.capacity() as u64, Ordering::Relaxed);
    record_heap_bytes(
        std::mem::size_of::<ListObject>()
            + retained.capacity() * std::mem::size_of::<ValueHandle>(),
    );
    let obj = Box::new(ListObject { items: retained });
    Box::into_raw(obj) as *mut c_void
}

fn alloc_map(entries: &[MapEntry]) -> *mut c_void {
    let capacity = (entries.len().next_power_of_two()).max(8);
    MAP_SLOTS_ALLOCATED.fetch_add(capacity as u64, Ordering::Relaxed);
    record_heap_bytes(
        std::mem::size_of::<MapObject>()
            + capacity * std::mem::size_of::<MapBucket>(),
    );
    let mut obj = MapObject {
        buckets: std::iter::repeat_with(MapBucket::default).take(capacity).collect(),
        len: 0,
        tombstones: 0,
    };
    for entry in entries {
        if entry.key.is_null() || entry.value.is_null() {
            continue;
        }
        map_insert(&mut obj, entry.key, entry.value);
    }
    Box::into_raw(Box::new(obj)) as *mut c_void
}

/// Collect child handles from a value, free its heap-allocated structure,
/// and reset the value to Unit. Does NOT release children — they are pushed
/// to `children` for the caller to handle iteratively.
unsafe fn drop_heap_collect_children(value: &mut Value, children: &mut Vec<ValueHandle>) {
    // Handle error metadata cleanup
    if value.is_err() {
        let ptr = value.heap_ptr();
        if !ptr.is_null() {
            unsafe {
                let metadata = Box::from_raw(ptr as *mut ErrorMetadata);
                if !metadata.name.is_null() {
                    children.push(metadata.name);
                }
            }
        }
        *value = Value::unit();
        return;
    }
    
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::String) => {
            if !value.is_inline_string() {
                let ptr = value.heap_ptr();
                if !ptr.is_null() {
                    unsafe { drop(Box::from_raw(ptr as *mut StringObject)); }
                }
            }
        }
        Ok(ValueTag::Bytes) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe { drop(Box::from_raw(ptr as *mut BytesObject)); }
            }
        }
        Ok(ValueTag::List) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    if (value.flags & FLAG_LIST_ITER) != 0 {
                        let mut iter = Box::from_raw(ptr as *mut ListIter);
                        children.extend(iter.items.drain(..));
                    } else {
                        let mut boxed = Box::from_raw(ptr as *mut ListObject);
                        children.extend(boxed.items.drain(..));
                    }
                }
            }
        }
        Ok(ValueTag::Map) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    if (value.flags & FLAG_MAP_ITER) != 0 {
                        let iter = Box::from_raw(ptr as *mut MapIter);
                        for bucket in iter.buckets {
                            if bucket.state == MapBucketState::Occupied {
                                children.push(bucket.key);
                                children.push(bucket.value);
                            }
                        }
                    } else {
                        let mut boxed = Box::from_raw(ptr as *mut MapObject);
                        for bucket in boxed.buckets.iter_mut() {
                            if bucket.state == MapBucketState::Occupied {
                                children.push(bucket.key);
                                children.push(bucket.value);
                            }
                        }
                    }
                }
            }
        }
        Ok(ValueTag::Closure) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                *value = Value::unit();
                return;
            }
            unsafe {
                let closure = Box::from_raw(ptr as *mut ClosureObject);
                if let Some(release) = closure.release {
                    release(closure.env);
                } else if !closure.env.is_null() {
                    coral_heap_free(closure.env);
                }
            }
        }
        Ok(ValueTag::Actor) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                *value = Value::unit();
                return;
            }
            unsafe {
                drop(Box::from_raw(ptr as *mut ActorObject));
            }
        }
        Ok(ValueTag::Tagged) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                *value = Value::unit();
                return;
            }
            unsafe {
                let tagged = Box::from_raw(ptr as *mut TaggedValue);
                for i in 0..tagged.field_count {
                    let field = *tagged.fields.add(i);
                    if !field.is_null() {
                        children.push(field);
                    }
                }
                if tagged.field_count > 0 && !tagged.fields.is_null() {
                    drop(Vec::from_raw_parts(
                        tagged.fields,
                        tagged.field_count,
                        tagged.field_count,
                    ));
                }
            }
        }
        _ => {}
    }
    *value = Value::unit();
}

/// Iteratively free a value and all its transitive children.
/// Uses a worklist to avoid unbounded recursion on deeply nested structures.
unsafe fn drop_heap_value(value: &mut Value) {
    let mut worklist: Vec<ValueHandle> = Vec::new();
    drop_heap_collect_children(value, &mut worklist);

    while let Some(child) = worklist.pop() {
        if child.is_null() {
            continue;
        }
        let child_ref = unsafe { &*child };
        let rc = child_ref.refcount.load(Ordering::Relaxed);
        if rc == 0 {
            // Already freed — skip
            RELEASE_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
            continue;
        }
        RELEASE_COUNT.fetch_add(1, Ordering::Relaxed);

        // Non-atomic fast path for thread-local children (M2.2)
        let owner = child_ref.owner_thread;
        let is_local = owner != 0 && owner == current_thread_id();

        let prev = if is_local {
            // Plain store: no other thread can see this value
            child_ref.refcount.store(rc - 1, Ordering::Relaxed);
            rc
        } else {
            child_ref.refcount.fetch_sub(1, Ordering::Release)
        };

        if prev > 1 {
            // Still referenced — mark as possible cycle root
            cycle_detector::possible_root(child);
            continue;
        }
        // prev == 1: this child is being freed
        if !is_local {
            std::sync::atomic::fence(Ordering::Acquire);
        }
        weak_ref::notify_value_deallocated(child);
        cycle_detector::notify_value_freed(child);
        let child_mut = unsafe { &mut *child };
        // Collect grandchildren before freeing (iterative, not recursive)
        drop_heap_collect_children(child_mut, &mut worklist);
        LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
        if !recycle_value_box(child) {
            unsafe { drop(Box::from_raw(child)); }
        }
    }
}

/// Deallocate a value's heap data WITHOUT releasing child handles.
/// Used by the cycle collector to safely free garbage values whose children
/// may already be freed or are also garbage being collected.
/// After this call the Value slot is reset to Unit.
pub(crate) unsafe fn drop_heap_value_for_gc(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    let value = unsafe { &mut *handle };

    // Clear error metadata — release the name handle to prevent leaks.
    // In the GC path, error values inside cyclic containers would otherwise
    // leak their name string since get_children() doesn't traverse error metadata.
    if value.is_err() {
        let ptr = value.heap_ptr();
        if !ptr.is_null() {
            unsafe {
                let metadata = Box::from_raw(ptr as *mut ErrorMetadata);
                if !metadata.name.is_null() {
                    crate::coral_value_release(metadata.name);
                }
            }
        }
        *value = Value::unit();
        return;
    }

    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::String) => {
            if !value.is_inline_string() {
                let ptr = value.heap_ptr();
                if !ptr.is_null() {
                    unsafe { drop(Box::from_raw(ptr as *mut StringObject)); }
                }
            }
        }
        Ok(ValueTag::Bytes) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe { drop(Box::from_raw(ptr as *mut BytesObject)); }
            }
        }
        Ok(ValueTag::List) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    // Drop list container without releasing child handles
                    if (value.flags & FLAG_LIST_ITER) != 0 {
                        drop(Box::from_raw(ptr as *mut ListIter));
                    } else {
                        drop(Box::from_raw(ptr as *mut ListObject));
                    }
                }
            }
        }
        Ok(ValueTag::Map) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    // Drop map container without releasing child handles
                    if (value.flags & FLAG_MAP_ITER) != 0 {
                        drop(Box::from_raw(ptr as *mut MapIter));
                    } else {
                        drop(Box::from_raw(ptr as *mut MapObject));
                    }
                }
            }
        }
        Ok(ValueTag::Closure) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    let closure = Box::from_raw(ptr as *mut ClosureObject);
                    if let Some(release) = closure.release {
                        release(closure.env);
                    } else if !closure.env.is_null() {
                        coral_heap_free(closure.env);
                    }
                }
            }
        }
        Ok(ValueTag::Actor) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe { drop(Box::from_raw(ptr as *mut ActorObject)); }
            }
        }
        Ok(ValueTag::Tagged) => {
            let ptr = value.heap_ptr();
            if !ptr.is_null() {
                unsafe {
                    let tagged = Box::from_raw(ptr as *mut TaggedValue);
                    // Free the fields array without releasing field handles
                    if tagged.field_count > 0 && !tagged.fields.is_null() {
                        drop(Vec::from_raw_parts(
                            tagged.fields,
                            tagged.field_count,
                            tagged.field_count,
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    *value = Value::unit();
}

/// Free a Value box, decrement live count. Used by cycle collector.
pub(crate) fn dealloc_value_box(handle: ValueHandle) {
    LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
    if !recycle_value_box(handle) {
        unsafe { drop(Box::from_raw(handle)); }
    }
}

impl TryFrom<u8> for ValueTag {
    type Error = ();

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(ValueTag::Number),
            1 => Ok(ValueTag::Bool),
            2 => Ok(ValueTag::String),
            3 => Ok(ValueTag::List),
            4 => Ok(ValueTag::Map),
            5 => Ok(ValueTag::Store),
            6 => Ok(ValueTag::Actor),
            7 => Ok(ValueTag::Unit),
            8 => Ok(ValueTag::Closure),
            9 => Ok(ValueTag::Bytes),
            10 => Ok(ValueTag::Tagged),
            _ => Err(()),
        }
    }
}

fn read_bytes(ptr: *const u8, len: usize) -> Vec<u8> {
    if len == 0 {
        return Vec::new();
    }
    assert!(!ptr.is_null(), "source pointer must not be null when len > 0");
    unsafe { slice::from_raw_parts(ptr, len) }.to_vec()
}

fn inline_string_len(flags: u8) -> usize {
    ((flags & !FLAG_INLINE_STRING) >> 1) as usize
}

pub(crate) fn string_to_bytes(value: &Value) -> Vec<u8> {
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::String) => {
            if value.is_inline_string() {
                let len = inline_string_len(value.flags);
                unsafe {
                    let inline = value.payload.inline;
                    inline[..len].to_vec()
                }
            } else {
                let ptr = value.heap_ptr();
                if ptr.is_null() {
                    return Vec::new();
                }
                unsafe { (*((ptr as *const StringObject))).data.clone() }
            }
        }
        Ok(ValueTag::Bytes) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                return Vec::new();
            }
            unsafe { (*((ptr as *const BytesObject))).data.clone() }
        }
        _ => Vec::new(),
    }
}

/// Convert a string Value to a Rust String.
pub(crate) fn value_to_rust_string(value: &Value) -> String {
    let bytes = string_to_bytes(value);
    String::from_utf8_lossy(&bytes).to_string()
}

fn number_to_i64(value: &Value) -> i64 {
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) => unsafe { value.payload.number as i64 },
        Ok(ValueTag::Bool) => (unsafe { value.payload.inline[0] } & 1) as i64,
        _ => 0,
    }
}

fn handle_to_i64(handle: ValueHandle) -> i64 {
    if handle.is_null() {
        return 0;
    }
    let value = unsafe { &*handle };
    number_to_i64(value)
}

fn list_from_value(value: &Value) -> Option<&ListObject> {
    if value.tag != ValueTag::List as u8 {
        return None;
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { &*(ptr as *const ListObject) })
}

fn map_from_value(value: &Value) -> Option<&MapObject> {
    if value.tag != ValueTag::Map as u8 {
        return None;
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return None;
    }
    Some(unsafe { &*(ptr as *const MapObject) })
}

fn value_to_path(value: &Value) -> Option<PathBuf> {
    if value.tag != ValueTag::String as u8 {
        return None;
    }
    let bytes = string_to_bytes(value);
    String::from_utf8(bytes).ok().map(PathBuf::from)
}

fn freeze_value(handle: ValueHandle) {
    if handle.is_null() {
        return;
    }
    let value = unsafe { &mut *handle };
    value.flags |= FLAG_FROZEN;
    // M2.3: Promote to atomic mode — all subsequent retain/release on this
    // value will use atomic operations since owner_thread == 0 means "shared".
    // This is a one-way transition: once frozen, a value never goes back to
    // thread-local mode.
    value.owner_thread = 0;
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::List) => {
            if let Some(list) = list_from_value(value) {
                for &item in &list.items {
                    freeze_value(item);
                }
            }
        }
        Ok(ValueTag::Map) => {
            if let Some(map) = map_from_value(value) {
                for bucket in &map.buckets {
                    if bucket.state == MapBucketState::Occupied {
                        freeze_value(bucket.key);
                        freeze_value(bucket.value);
                    }
                }
            }
        }
        _ => {}
    }
}

fn is_frozen(handle: ValueHandle) -> bool {
    if handle.is_null() {
        return false;
    }
    let value = unsafe { &*handle };
    (value.flags & FLAG_FROZEN) != 0
}

fn value_deep_clone(handle: ValueHandle) -> ValueHandle {
    if handle.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &*handle };
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) => coral_make_number(unsafe { value.payload.number }),
        Ok(ValueTag::Bool) => coral_make_bool(unsafe { value.payload.inline[0] & 1 }),
        Ok(ValueTag::Unit) => coral_make_unit(),
        Ok(ValueTag::String) => {
            let bytes = string_to_bytes(value);
            coral_make_string(bytes.as_ptr(), bytes.len())
        }
        Ok(ValueTag::Bytes) => {
            let bytes = string_to_bytes(value);
            coral_make_bytes(bytes.as_ptr(), bytes.len())
        }
        Ok(ValueTag::List) => {
            let Some(list) = list_from_value(value) else { return coral_make_unit(); };
            let mut cloned: Vec<ValueHandle> = Vec::with_capacity(list.items.len());
            for &item in &list.items {
                cloned.push(value_deep_clone(item));
            }
            let out = coral_make_list(cloned.as_ptr(), cloned.len());
            unsafe {
                for h in cloned {
                    coral_value_release(h);
                }
            }
            out
        }
        Ok(ValueTag::Map) => {
            let Some(map) = map_from_value(value) else { return coral_make_unit(); };
            let mut entries: Vec<MapEntry> = Vec::with_capacity(map.len);
            for bucket in &map.buckets {
                if bucket.state != MapBucketState::Occupied {
                    continue;
                }
                let k = value_deep_clone(bucket.key);
                let v = value_deep_clone(bucket.value);
                entries.push(MapEntry { key: k, value: v });
            }
            let out = coral_make_map(entries.as_ptr(), entries.len());
            unsafe {
                for entry in entries {
                    coral_value_release(entry.key);
                    coral_value_release(entry.value);
                }
            }
            out
        }
        Ok(ValueTag::Actor) => {
            // Actor handles are immutable and shared by retaining the handle value itself.
            unsafe {
                coral_value_retain(handle);
            }
            handle
        }
        Ok(ValueTag::Closure) => {
            // For now closures are not cloned deeply; share by retain.
            unsafe { coral_value_retain(handle) };
            handle
        }
        Ok(ValueTag::Store) => {
            // Stores are not implemented; share by retain to avoid lossy copies.
            unsafe { coral_value_retain(handle) };
            handle
        }
        Ok(ValueTag::Tagged) => {
            // Deep clone a tagged value by cloning its fields
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                return coral_make_unit();
            }
            let tagged = unsafe { &*(ptr as *const TaggedValue) };
            
            // Clone fields
            let mut cloned_fields: Vec<ValueHandle> = Vec::with_capacity(tagged.field_count);
            for i in 0..tagged.field_count {
                if !tagged.fields.is_null() {
                    let field = unsafe { *tagged.fields.add(i) };
                    cloned_fields.push(value_deep_clone(field));
                }
            }
            
            // Create new tagged value
            let result = coral_make_tagged(
                tagged.tag_name,
                tagged.tag_name_len,
                if cloned_fields.is_empty() { std::ptr::null() } else { cloned_fields.as_ptr() },
                cloned_fields.len(),
            );
            
            // Release cloned fields (coral_make_tagged retains them)
            unsafe {
                for h in cloned_fields {
                    coral_value_release(h);
                }
            }
            
            result
        }
        Err(_) => coral_make_unit(),
    }
}

fn actor_from_value(value: &Value) -> Option<ActorObject> {
    if value.tag != ValueTag::Actor as u8 {
        return None;
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return None;
    }
    let obj = unsafe { &*(ptr as *const ActorObject) };
    Some(obj.clone())
}

fn actor_to_value(handle: ActorHandle, system: ActorSystem) -> ValueHandle {
    let obj = Box::new(ActorObject { handle, system });
    alloc_value(Value::from_heap(ValueTag::Actor, Box::into_raw(obj) as *mut c_void))
}

fn lists_equal(a: &Value, b: &Value) -> bool {
    let list_a = match list_from_value(a) {
        Some(list) => list,
        None => return false,
    };
    let list_b = match list_from_value(b) {
        Some(list) => list,
        None => return false,
    };
    if list_a.items.len() != list_b.items.len() {
        return false;
    }
    for (left, right) in list_a.items.iter().zip(list_b.items.iter()) {
        if !values_equal_handles(*left, *right) {
            return false;
        }
    }
    true
}

fn maps_equal(a: &Value, b: &Value) -> bool {
    let map_a = match map_from_value(a) {
        Some(map) => map,
        None => return false,
    };
    let map_b = match map_from_value(b) {
        Some(map) => map,
        None => return false,
    };
    if map_a.len != map_b.len {
        return false;
    }
    for bucket in &map_a.buckets {
        if bucket.state != MapBucketState::Occupied {
            continue;
        }
        let Some(other) = map_get_entry(map_b, bucket.key) else {
            return false;
        };
        if !values_equal_handles(bucket.value, other.value) {
            return false;
        }
    }
    true
}

fn map_bucket_index(capacity: usize, hash: u64) -> usize {
    (hash as usize) & (capacity - 1)
}

fn map_should_grow(len: usize, tombstones: usize, capacity: usize) -> bool {
    (len + tombstones) * 10 >= capacity * 7
}

fn map_rehash(map: &mut MapObject) {
    let old_capacity = map.buckets.len();
    let new_capacity = (old_capacity * 2).max(8);
    MAP_SLOTS_ALLOCATED.fetch_add((new_capacity - old_capacity) as u64, Ordering::Relaxed);
    let mut new_buckets: Vec<MapBucket> =
        std::iter::repeat_with(MapBucket::default).take(new_capacity).collect();
    for bucket in map.buckets.iter_mut() {
        if bucket.state != MapBucketState::Occupied {
            continue;
        }
        let mut idx = map_bucket_index(new_capacity, bucket.hash);
        loop {
            let slot = &mut new_buckets[idx];
            if slot.state == MapBucketState::Empty {
                *slot = MapBucket {
                    state: MapBucketState::Occupied,
                    hash: bucket.hash,
                    key: bucket.key,
                    value: bucket.value,
                };
                break;
            }
            idx = (idx + 1) & (new_capacity - 1);
        }
    }
    map.buckets = new_buckets;
    map.tombstones = 0;
}

fn map_iter_snapshot(map: &MapObject) -> MapIter {
    let mut buckets: Vec<MapBucket> = Vec::with_capacity(map.buckets.len());
    for bucket in &map.buckets {
        if bucket.state == MapBucketState::Occupied {
            unsafe {
                coral_value_retain(bucket.key);
                coral_value_retain(bucket.value);
            }
        }
        buckets.push(bucket.clone());
    }
    MapIter { buckets, index: 0 }
}

fn map_insert(map: &mut MapObject, key: ValueHandle, value: ValueHandle) -> Option<ValueHandle> {
    if key.is_null() || value.is_null() {
        return None;
    }
    if map.buckets.is_empty() {
        map.buckets = std::iter::repeat_with(MapBucket::default).take(8).collect();
    }
    if map_should_grow(map.len, map.tombstones, map.buckets.len()) {
        map_rehash(map);
    }
    let hash = hash_value(key);
    let capacity = map.buckets.len();
    let mut idx = map_bucket_index(capacity, hash);
    let mut first_tombstone: Option<usize> = None;
    loop {
        let bucket = &mut map.buckets[idx];
        match bucket.state {
            MapBucketState::Empty => {
                let target = first_tombstone.unwrap_or(idx);
                let bucket = &mut map.buckets[target];
                *bucket = MapBucket {
                    state: MapBucketState::Occupied,
                    hash,
                    key,
                    value,
                };
                unsafe {
                    coral_value_retain(key);
                    coral_value_retain(value);
                }
                map.len += 1;
                if first_tombstone.is_some() {
                    map.tombstones -= 1;
                }
                return None;
            }
            MapBucketState::Tombstone => {
                if first_tombstone.is_none() {
                    first_tombstone = Some(idx);
                }
            }
            MapBucketState::Occupied => {
                if bucket.hash == hash && values_equal_handles(bucket.key, key) {
                    unsafe { coral_value_retain(value); }
                    let old = bucket.value;
                    bucket.value = value;
                    return Some(old);
                }
            }
        }
        idx = (idx + 1) & (capacity - 1);
    }
}

fn map_get_entry<'a>(map: &'a MapObject, key: ValueHandle) -> Option<&'a MapBucket> {
    if key.is_null() || map.buckets.is_empty() {
        return None;
    }
    let hash = hash_value(key);
    let capacity = map.buckets.len();
    let mut idx = map_bucket_index(capacity, hash);
    loop {
        let bucket = &map.buckets[idx];
        match bucket.state {
            MapBucketState::Empty => return None,
            MapBucketState::Occupied => {
                if bucket.hash == hash && values_equal_handles(bucket.key, key) {
                    return Some(bucket);
                }
            }
            MapBucketState::Tombstone => {}
        }
        idx = (idx + 1) & (capacity - 1);
    }
}

fn values_equal_handles(a: ValueHandle, b: ValueHandle) -> bool {
    if a.is_null() || b.is_null() {
        return false;
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag != vb.tag {
        return false;
    }
    match ValueTag::try_from(va.tag) {
        Ok(ValueTag::Number) => unsafe { va.payload.number == vb.payload.number },
        Ok(ValueTag::Bool) => unsafe { va.payload.inline[0] == vb.payload.inline[0] },
        Ok(ValueTag::String) | Ok(ValueTag::Bytes) => string_to_bytes(va) == string_to_bytes(vb),
        Ok(ValueTag::Unit) => true,
        Ok(ValueTag::List) => lists_equal(va, vb),
        Ok(ValueTag::Map) => maps_equal(va, vb),
        Ok(ValueTag::Actor) => {
            let left = actor_from_value(va);
            let right = actor_from_value(vb);
            match (left, right) {
                (Some(l), Some(r)) => l.handle.id == r.handle.id,
                _ => ptr::eq(a, b),
            }
        }
        _ => ptr::eq(a, b),
    }
}

fn hash_value(handle: ValueHandle) -> u64 {
    if handle.is_null() {
        return 0;
    }
    let mut hasher = DefaultHasher::new();
    let value = unsafe { &*handle };
    value.tag.hash(&mut hasher);
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) => unsafe { value.payload.number.to_bits().hash(&mut hasher) },
        Ok(ValueTag::Bool) => unsafe { value.payload.inline[0].hash(&mut hasher) },
        Ok(ValueTag::String) | Ok(ValueTag::Bytes) => {
            let bytes = string_to_bytes(value);
            bytes.hash(&mut hasher);
        }
        Ok(ValueTag::List) => {
            if let Some(list) = list_from_value(value) {
                list.items.len().hash(&mut hasher);
                for item in &list.items {
                    hash_value(*item).hash(&mut hasher);
                }
            } else {
                (value.heap_ptr() as usize).hash(&mut hasher);
            }
        }
        Ok(ValueTag::Map) => {
            if let Some(map) = map_from_value(value) {
                map.len.hash(&mut hasher);
                for bucket in &map.buckets {
                    if bucket.state == MapBucketState::Occupied {
                        hash_value(bucket.key).hash(&mut hasher);
                        hash_value(bucket.value).hash(&mut hasher);
                    }
                }
            } else {
                (value.heap_ptr() as usize).hash(&mut hasher);
            }
        }
        Ok(ValueTag::Actor) => {
            if let Some(actor) = actor_from_value(value) {
                actor.handle.id.0.hash(&mut hasher);
            } else {
                (value.heap_ptr() as usize).hash(&mut hasher);
            }
        }
        _ => {
            (value.heap_ptr() as usize).hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_hash(value: ValueHandle) -> u64 {
    hash_value(value)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_number(value: f64) -> ValueHandle {
    alloc_value(Value::number(value))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_bool(value: u8) -> ValueHandle {
    alloc_value(Value::boolean(value != 0))
}

/// Create a string Value from a Rust &str. Convenience helper for internal use.
pub(crate) fn coral_make_string_from_rust(s: &str) -> ValueHandle {
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_string(ptr: *const u8, len: usize) -> ValueHandle {
    if len <= 14 {
        let bytes = read_bytes(ptr, len);
        return alloc_value(Value::inline_string(&bytes));
    }
    let bytes = read_bytes(ptr, len);
    let handle = alloc_string(&bytes);
    alloc_value(Value::from_heap(ValueTag::String, handle))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_bytes(ptr: *const u8, len: usize) -> ValueHandle {
    let bytes = if len == 0 {
        Vec::new()
    } else {
        read_bytes(ptr, len)
    };
    let handle = alloc_bytes_obj(&bytes);
    alloc_value(Value::from_heap(ValueTag::Bytes, handle))
}

// ============================================================================
// Error Value Functions
// ============================================================================

/// Create a Bytes value from an owned Vec<u8>. Used by encoding_ops.
pub(crate) fn coral_bytes_from_vec(data: Vec<u8>) -> ValueHandle {
    let handle = alloc_bytes_obj(&data);
    alloc_value(Value::from_heap(ValueTag::Bytes, handle))
}

// ============================================================================
// End Error Value Functions
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_length(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_number(0.0);
    }
    let value_ref = unsafe { &*value };
    match ValueTag::try_from(value_ref.tag) {
        Ok(ValueTag::List) => coral_list_length(value),
        Ok(ValueTag::Map) => coral_map_length(value),
        Ok(ValueTag::String) | Ok(ValueTag::Bytes) => {
            coral_make_number(string_to_bytes(value_ref).len() as f64)
        }
        _ => coral_make_number(0.0),
    }
}

/// Generic `.get(key)` dispatcher – routes to list-get or map-get based on tag.
#[unsafe(no_mangle)]
pub extern "C" fn coral_value_get(collection: ValueHandle, key: ValueHandle) -> ValueHandle {
    if collection.is_null() {
        return coral_make_absent();
    }
    let v = unsafe { &*collection };
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::List) => coral_list_get(collection, key),
        Ok(ValueTag::Map) => coral_map_get(collection, key),
        _ => coral_make_absent(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_list(items: *const ValueHandle, len: usize) -> ValueHandle {
    let slice = if len == 0 {
        &[][..]
    } else {
        assert!(!items.is_null(), "items pointer must not be null when len > 0");
        unsafe { slice::from_raw_parts(items, len) }
    };
    let handle = alloc_list(slice);
    alloc_value(Value::from_heap(ValueTag::List, handle))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_list_hinted(
    items: *const ValueHandle,
    len: usize,
    _hint: u8,
) -> ValueHandle {
    // TODO: implement stack/arena/COW strategies. For now, delegate to heap and record hint.
    coral_make_list(items, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_map(entries: *const MapEntry, len: usize) -> ValueHandle {
    let slice = if len == 0 {
        &[][..]
    } else {
        assert!(
            !entries.is_null(),
            "entries pointer must not be null when len > 0"
        );
        unsafe { slice::from_raw_parts(entries, len) }
    };
    let handle = alloc_map(slice);
    alloc_value(Value::from_heap(ValueTag::Map, handle))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_map_hinted(
    entries: *const MapEntry,
    len: usize,
    _hint: u8,
) -> ValueHandle {
    // TODO: implement stack/arena/COW strategies. For now, delegate to heap and record hint.
    coral_make_map(entries, len)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_unit() -> ValueHandle {
    alloc_value(Value::unit())
}

/// Create a tagged value (ADT variant).
/// 
/// # Arguments
/// * `tag_name` - Pointer to the tag name string (e.g., "Some", "None")
/// * `tag_name_len` - Length of the tag name string
/// * `fields` - Pointer to array of field values
/// * `field_count` - Number of fields
/// 
/// # Returns
/// A ValueHandle to the tagged value
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_tagged(
    tag_name: *const u8,
    tag_name_len: usize,
    fields: *const ValueHandle,
    field_count: usize,
) -> ValueHandle {
    // Copy the fields array
    let fields_vec = if field_count > 0 && !fields.is_null() {
        let slice = unsafe { slice::from_raw_parts(fields, field_count) };
        // Retain all field values
        for field in slice {
            if !field.is_null() {
                unsafe { coral_value_retain(*field) };
            }
        }
        slice.to_vec()
    } else {
        Vec::new()
    };
    
    let fields_ptr = if fields_vec.is_empty() {
        ptr::null_mut()
    } else {
        let mut boxed = fields_vec.into_boxed_slice();
        let ptr = boxed.as_mut_ptr();
        std::mem::forget(boxed);
        ptr
    };
    
    let tagged = Box::new(TaggedValue {
        tag_name,
        tag_name_len,
        field_count,
        fields: fields_ptr,
    });
    
    let value = Value {
        tag: ValueTag::Tagged as u8,
        flags: 0,
        reserved: 0,
        owner_thread: current_thread_id(),
        refcount: AtomicU64::new(1),
        #[cfg(feature = "metrics")]
        retain_events: AtomicU32::new(0),
        #[cfg(feature = "metrics")]
        release_events: AtomicU32::new(0),
        payload: Payload { ptr: Box::into_raw(tagged) as *mut c_void },
    };
    
    alloc_value(value)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_log(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        println!("()");
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    match ValueTag::try_from(value_ref.tag) {
        Ok(ValueTag::Number) => {
            let number = unsafe { value_ref.payload.number };
            println!("{number}");
        }
        Ok(ValueTag::Bool) => {
            let byte = unsafe { value_ref.payload.inline[0] } & 1;
            println!("{}", if byte != 0 { "true" } else { "false" });
        }
        Ok(ValueTag::String) => {
            let bytes = string_to_bytes(value_ref);
            let text = String::from_utf8_lossy(&bytes);
            println!("{text}");
        }
        Ok(ValueTag::Bytes) => {
            let bytes = string_to_bytes(value_ref);
            let hex = bytes
                .iter()
                .map(|b| format!("{b:02X}"))
                .collect::<Vec<_>>()
                .join(" ");
            println!("[bytes {hex}]");
        }
        Ok(ValueTag::Unit) => println!("()"),
        Ok(ValueTag::Tagged) => {
            let ptr = value_ref.heap_ptr();
            if !ptr.is_null() {
                let tagged = unsafe { &*(ptr as *const TaggedValue) };
                let tag_name = unsafe {
                    let slice = slice::from_raw_parts(tagged.tag_name, tagged.tag_name_len);
                    String::from_utf8_lossy(slice).to_string()
                };
                if tagged.field_count == 0 {
                    println!("{tag_name}");
                } else {
                    print!("{tag_name}(");
                    for i in 0..tagged.field_count {
                        if i > 0 {
                            print!(", ");
                        }
                        let field = unsafe { *tagged.fields.add(i) };
                        if field.is_null() {
                            print!("()");
                        } else {
                            // Print field value inline (simplified)
                            let field_ref = unsafe { &*field };
                            match ValueTag::try_from(field_ref.tag) {
                                Ok(ValueTag::Number) => {
                                    let n = unsafe { field_ref.payload.number };
                                    print!("{n}");
                                }
                                Ok(ValueTag::Bool) => {
                                    let b = unsafe { field_ref.payload.inline[0] } & 1;
                                    print!("{}", if b != 0 { "true" } else { "false" });
                                }
                                Ok(ValueTag::String) => {
                                    let bytes = string_to_bytes(field_ref);
                                    let text = String::from_utf8_lossy(&bytes);
                                    print!("\"{text}\"");
                                }
                                _ => print!("<value>"),
                            }
                        }
                    }
                    println!(")");
                }
            } else {
                println!("<tagged:null>");
            }
        }
        _ => println!("<value tag {}>", value_ref.tag),
    }
    coral_make_unit()
}

// ========== Named Actor Registry FFI ==========

// ========== Timer FFI Functions ==========

// ==================== Math Functions ====================

// ==================== End Math Functions ====================

// ===== Universal iterator next (dispatches list vs map) =====

// ===== Process / Environment =====

// ===== File I/O extensions =====

// ===== stdin =====

// ===== List extensions =====

// ===== Map extensions =====

// ===== Bytes extensions =====

// ===== String ↔ Number =====

// ===== Type checking =====

#[unsafe(no_mangle)]
pub extern "C" fn coral_type_of(value: ValueHandle) -> ValueHandle {
    if value.is_null() { return coral_make_string("none".as_ptr(), 4); }
    let v = unsafe { &*value };
    let name = match ValueTag::try_from(v.tag) {
        Ok(ValueTag::Number) => "number",
        Ok(ValueTag::Bool) => "bool",
        Ok(ValueTag::String) => "string",
        Ok(ValueTag::Bytes) => "bytes",
        Ok(ValueTag::List) => "list",
        Ok(ValueTag::Map) => "map",
        Ok(ValueTag::Closure) => "function",
        Ok(ValueTag::Unit) => "none",
        Ok(ValueTag::Tagged) => "tagged",
        Ok(ValueTag::Actor) => "actor",
        _ => "unknown",
    };
    coral_make_string(name.as_ptr(), name.len())
}

// ===== Character operations =====

struct StackFrame {
    buffer: Vec<u8>,
    cursor: usize,
}

thread_local! {
    static STACK_FRAMES: RefCell<Vec<StackFrame>> = RefCell::new(Vec::new());
}

#[repr(C)]
pub struct CoralRuntimeStats {
    pub retains: u64,
    pub releases: u64,
    pub live_values: u64,
}

#[repr(C)]
pub struct CoralRuntimeMetrics {
    pub retains: u64,
    pub retain_saturated: u64,
    pub releases: u64,
    pub release_underflow: u64,
    pub live_values: u64,
    pub value_pool_hits: u64,
    pub value_pool_misses: u64,
    pub heap_bytes: u64,
    pub string_bytes: u64,
    pub list_slots: u64,
    pub map_slots: u64,
    pub stack_pages: u64,
    pub stack_bytes: u64,
    pub timestamp_ns: u64,
}

#[repr(C)]
pub struct CoralHandleMetrics {
    pub refcount: u64,
    pub retains: u64,
    pub releases: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ptr;
    use std::slice;

    fn list_len(handle: ValueHandle) -> usize {
        if handle.is_null() {
            return 0;
        }
        let value = unsafe { &*handle };
        if value.tag != ValueTag::List as u8 {
            return 0;
        }
        let ptr = value.heap_ptr();
        if ptr.is_null() {
            return 0;
        }
        unsafe { (*(ptr as *const ListObject)).items.len() }
    }

    fn map_len(handle: ValueHandle) -> usize {
        if handle.is_null() {
            return 0;
        }
        let value = unsafe { &*handle };
        if value.tag != ValueTag::Map as u8 {
            return 0;
        }
        let ptr = value.heap_ptr();
        if ptr.is_null() {
            return 0;
        }
        unsafe { (*(ptr as *const MapObject)).len }
    }

    #[test]
    fn number_round_trip() {
        let value = coral_make_number(42.0);
        assert_eq!(coral_value_tag(value), ValueTag::Number as u8);
        assert_eq!(coral_value_as_number(value), 42.0);
        unsafe { coral_value_release(value) };
    }

    #[test]
    fn string_concat() {
        let hello = coral_make_string("hel".as_ptr(), 3);
        let world = coral_make_string("lo".as_ptr(), 2);
        let combined = coral_string_concat(hello, world);
        assert_eq!(coral_value_tag(combined), ValueTag::String as u8);
        unsafe {
            coral_value_release(hello);
            coral_value_release(world);
            coral_value_release(combined);
        }
    }

    #[test]
    fn bool_round_trip() {
        let truthy = coral_make_bool(1);
        let falsy = coral_make_bool(0);
        assert_eq!(coral_value_as_bool(truthy), 1);
        assert_eq!(coral_value_as_bool(falsy), 0);
        unsafe {
            coral_value_release(truthy);
            coral_value_release(falsy);
        }
    }

    #[test]
    fn metrics_capture_string_bytes() {
        let before = snapshot_runtime_metrics();
        let text = "telemetry driven allocation".repeat(2);
        let bytes = text.as_bytes();
        let handle = coral_make_string(bytes.as_ptr(), bytes.len());
        unsafe { coral_value_release(handle) };
        let after = snapshot_runtime_metrics();
        assert!(
            after.string_bytes >= before.string_bytes + bytes.len() as u64,
            "string_bytes should grow"
        );
    }

    #[test]
    fn metrics_capture_list_slots() {
        let before = snapshot_runtime_metrics();
        let handles = [
            coral_make_number(1.0),
            coral_make_number(2.0),
            coral_make_number(3.0),
        ];
        let list = coral_make_list(handles.as_ptr(), handles.len());
        unsafe {
            for handle in handles {
                coral_value_release(handle);
            }
            coral_value_release(list);
        }
        let after = snapshot_runtime_metrics();
        assert!(
            after.list_slots >= before.list_slots + handles.len() as u64,
            "list_slots should grow"
        );
    }

    #[test]
    fn actor_spawn_and_send_counts_messages() {
        use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
        extern "C" fn invoke(env: *mut c_void, _args: *const ValueHandle, _len: usize, out: *mut ValueHandle) {
            if env.is_null() {
                return;
            }
            let counter = unsafe { &*(env as *const Arc<AtomicUsize>) };
            counter.fetch_add(1, Ordering::SeqCst);
            unsafe { *out = coral_make_unit(); }
        }
        extern "C" fn release(env: *mut c_void) {
            if env.is_null() {
                return;
            }
            unsafe { drop(Box::from_raw(env as *mut Arc<AtomicUsize>)); }
        }

        let counter = Arc::new(AtomicUsize::new(0));
        let env_box = Box::new(counter.clone());
        let closure = coral_make_closure(Some(invoke), Some(release), Box::into_raw(env_box) as *mut c_void, 0);
        let actor = coral_actor_spawn(closure);
        assert_ne!(actor, ptr::null_mut());

        let msg = coral_make_number(1.0);
        let _ = coral_actor_send(actor, msg);
        let _ = coral_actor_send(actor, msg);
        unsafe { coral_value_release(msg); }

        std::thread::sleep(std::time::Duration::from_millis(50));
        let _ = coral_actor_stop(actor);
        unsafe {
            coral_value_release(closure);
            coral_value_release(actor);
        }

        assert!(counter.load(Ordering::SeqCst) >= 2);
    }

    #[repr(C)]
    struct TestEnv {
        value: ValueHandle,
    }

    unsafe extern "C" fn test_invoke(
        env: *mut c_void,
        _args: *const ValueHandle,
        _len: usize,
        out: *mut ValueHandle,
    ) {
        let env = unsafe { &*(env as *mut TestEnv) };
        unsafe {
            coral_value_retain(env.value);
        }
        if !out.is_null() {
            unsafe {
                *out = env.value;
            }
        }
    }

    unsafe extern "C" fn test_release(env: *mut c_void) {
        if env.is_null() {
            return;
        }
        let boxed = unsafe { Box::from_raw(env as *mut TestEnv) };
        unsafe {
            coral_value_release(boxed.value);
        }
    }

    #[test]
    fn closure_round_trip() {
        unsafe {
            let captured = coral_make_number(7.0);
            coral_value_retain(captured);
            let env = Box::into_raw(Box::new(TestEnv { value: captured }));
            let closure = coral_make_closure(
                Some(test_invoke),
                Some(test_release),
                env as *mut c_void,
                0,
            );
            let result = coral_closure_invoke(closure, ptr::null(), 0);
            assert_eq!(coral_value_as_number(result), 7.0);
            coral_value_release(result);
            coral_value_release(closure);
            coral_value_release(captured);
        }
    }

    #[test]
    fn value_add_numbers() {
        let a = coral_make_number(2.5);
        let b = coral_make_number(1.5);
        let sum = coral_value_add(a, b);
        assert_eq!(coral_value_tag(sum), ValueTag::Number as u8);
        assert_eq!(coral_value_as_number(sum), 4.0);
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(sum);
        }
    }

    #[test]
    fn value_add_strings() {
        let a = coral_make_string("foo".as_ptr(), 3);
        let b = coral_make_string("bar".as_ptr(), 3);
        let combined = coral_value_add(a, b);
        assert_eq!(coral_value_tag(combined), ValueTag::String as u8);
        let contents = unsafe { string_to_bytes(&*combined) };
        assert_eq!(contents, b"foobar");
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(combined);
        }
    }

    #[test]
    fn value_equals_numbers() {
        let a = coral_make_number(4.2);
        let b = coral_make_number(4.2);
        let c = coral_make_number(5.0);
        let ab = coral_value_equals(a, b);
        let ac = coral_value_equals(a, c);
        assert_eq!(coral_value_as_bool(ab), 1);
        assert_eq!(coral_value_as_bool(ac), 0);
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(c);
            coral_value_release(ab);
            coral_value_release(ac);
        }
    }

    #[test]
    fn bitwise_helpers() {
        let a = coral_make_number(0b1100 as f64);
        let b = coral_make_number(0b1010 as f64);
        let and = coral_value_bitand(a, b);
        let or = coral_value_bitor(a, b);
        let xor = coral_value_bitxor(a, b);
        let not = coral_value_bitnot(a);
        assert_eq!(coral_value_as_number(and), 0b1000 as f64);
        assert_eq!(coral_value_as_number(or), 0b1110 as f64);
        assert_eq!(coral_value_as_number(xor), 0b0110 as f64);
        assert_eq!(coral_value_as_number(not), (!0b1100i64) as f64);
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(and);
            coral_value_release(or);
            coral_value_release(xor);
            coral_value_release(not);
        }
    }

    #[test]
    fn bytes_length_and_slice() {
        let data = coral_make_bytes("abcdef".as_ptr(), 6);
        let len = coral_bytes_length(data);
        assert_eq!(coral_value_as_number(len), 6.0);
        let slice = coral_bytes_slice(data, 1, 3);
        let slice_len = coral_bytes_length(slice);
        assert_eq!(coral_value_as_number(slice_len), 3.0);
        unsafe {
            coral_value_release(data);
            coral_value_release(len);
            coral_value_release(slice);
            coral_value_release(slice_len);
        }
    }

    #[test]
    fn fs_read_write_round_trip() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let path = std::env::temp_dir().join(format!(
            "coral_runtime_fs_{}_{}.txt",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path_str = path.to_string_lossy();
        let path_value = coral_make_string(path_str.as_bytes().as_ptr(), path_str.len());
        let payload = coral_make_string("payload".as_ptr(), 7);
        let write_ok = coral_fs_write(path_value, payload);
        assert_eq!(coral_value_as_bool(write_ok), 1);
        let exists = coral_fs_exists(path_value);
        assert_eq!(coral_value_as_bool(exists), 1);
        let read = coral_fs_read(path_value);
        assert_eq!(coral_value_tag(read), ValueTag::Bytes as u8);
        let contents = unsafe { string_to_bytes(&*read) };
        assert_eq!(contents, b"payload");
        fs::remove_file(&path).unwrap();
        unsafe {
            coral_value_release(path_value);
            coral_value_release(payload);
            coral_value_release(write_ok);
            coral_value_release(exists);
            coral_value_release(read);
        }
    }

    #[test]
    fn rc_churn_stress_smoke() {
        // Churn through a moderate number of list/map/string allocations to catch obvious RC regressions.
        const OUTER: usize = 200;
        const INNER: usize = 100;
        for i in 0..OUTER {
            let mut handles = Vec::with_capacity(INNER);
            for j in 0..INNER {
                let num = coral_make_number((i * j) as f64);
                handles.push(num);
            }
            let list = coral_make_list(handles.as_ptr(), handles.len());
            let map_entries: Vec<MapEntry> = handles
                .iter()
                .enumerate()
                .map(|(idx, &h)| MapEntry {
                    key: coral_make_number(idx as f64),
                    value: h,
                })
                .collect();
            let map = coral_make_map(map_entries.as_ptr(), map_entries.len());

            unsafe {
                // Pop a few values to exercise release paths.
                for _ in 0..3 {
                    let popped = coral_list_pop(list);
                    coral_value_release(popped);
                }
                coral_value_release(map);
                coral_value_release(list);
                for handle in handles {
                    coral_value_release(handle);
                }
            }
        }
    }

    #[test]
    fn value_equals_strings() {
        let a = coral_make_string("foo".as_ptr(), 3);
        let b = coral_make_string("foo".as_ptr(), 3);
        let c = coral_make_string("bar".as_ptr(), 3);
        let ab = coral_value_equals(a, b);
        let ac = coral_value_equals(a, c);
        assert_eq!(coral_value_as_bool(ab), 1);
        assert_eq!(coral_value_as_bool(ac), 0);
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(c);
            coral_value_release(ab);
            coral_value_release(ac);
        }
    }

    #[test]
    fn bitwise_operations() {
        let a = coral_make_number(6.0);
        let b = coral_make_number(3.0);
        let and = coral_value_bitand(a, b);
        let or = coral_value_bitor(a, b);
        let xor = coral_value_bitxor(a, b);
        let shift_two = coral_make_number(2.0);
        let shift_one = coral_make_number(1.0);
        let shl = coral_value_shift_left(b, shift_two);
        let shr = coral_value_shift_right(a, shift_one);
        assert_eq!(coral_value_as_number(and), 2.0);
        assert_eq!(coral_value_as_number(or), 7.0);
        assert_eq!(coral_value_as_number(xor), 5.0);
        assert_eq!(coral_value_as_number(shl), 12.0);
        assert_eq!(coral_value_as_number(shr), 3.0);
        unsafe {
            coral_value_release(a);
            coral_value_release(b);
            coral_value_release(and);
            coral_value_release(or);
            coral_value_release(xor);
            coral_value_release(shl);
            coral_value_release(shr);
            coral_value_release(shift_two);
            coral_value_release(shift_one);
        }
    }

    #[test]
    fn bytes_round_trip() {
        let sample = coral_make_bytes("abc".as_ptr(), 3);
        assert_eq!(coral_value_tag(sample), ValueTag::Bytes as u8);
        let length = coral_bytes_length(sample);
        assert_eq!(coral_value_as_number(length), 3.0);
        unsafe {
            coral_value_release(sample);
            coral_value_release(length);
        }
    }

    #[test]
    fn list_push_appends_value() {
        let empty_handles: [ValueHandle; 0] = [];
        let list = coral_make_list(empty_handles.as_ptr(), empty_handles.len());
        let value = coral_make_number(42.0);
        let list_after = coral_list_push(list, value);
        assert!(!list_after.is_null());
        assert_eq!(list_len(list_after), 1);
        unsafe {
            coral_value_release(value);
            coral_value_release(list);
            coral_value_release(list_after);
        }
    }

    #[test]
    fn value_equals_lists() {
        let a1 = coral_make_number(1.0);
        let a2 = coral_make_number(2.0);
        let arr1 = [a1, a2];
        let list1 = coral_make_list(arr1.as_ptr(), arr1.len());
        unsafe {
            coral_value_release(a1);
            coral_value_release(a2);
        }

        let b1 = coral_make_number(1.0);
        let b2 = coral_make_number(2.0);
        let arr2 = [b1, b2];
        let list2 = coral_make_list(arr2.as_ptr(), arr2.len());
        unsafe {
            coral_value_release(b1);
            coral_value_release(b2);
        }

        let c1 = coral_make_number(2.0);
        let c2 = coral_make_number(3.0);
        let arr3 = [c1, c2];
        let list3 = coral_make_list(arr3.as_ptr(), arr3.len());
        unsafe {
            coral_value_release(c1);
            coral_value_release(c2);
        }

        let eq = coral_value_equals(list1, list2);
        let ne = coral_value_equals(list1, list3);
        assert_eq!(coral_value_as_bool(eq), 1);
        assert_eq!(coral_value_as_bool(ne), 0);
        unsafe {
            coral_value_release(eq);
            coral_value_release(ne);
            coral_value_release(list1);
            coral_value_release(list2);
            coral_value_release(list3);
        }
    }

    #[test]
    fn list_length_reports_count() {
        let v1 = coral_make_number(1.0);
        let v2 = coral_make_number(2.0);
        let items = [v1, v2];
        let list = coral_make_list(items.as_ptr(), items.len());
        unsafe {
            coral_value_release(v1);
            coral_value_release(v2);
        }
        let len = coral_list_length(list);
        assert_eq!(coral_value_as_number(len), 2.0);
        unsafe {
            coral_value_release(len);
            coral_value_release(list);
        }
    }

    #[test]
    fn list_pop_removes_last_element() {
        let v1 = coral_make_number(10.0);
        let v2 = coral_make_number(20.0);
        let items = [v1, v2];
        let list = coral_make_list(items.as_ptr(), items.len());
        unsafe {
            coral_value_release(v1);
            coral_value_release(v2);
        }
        assert_eq!(list_len(list), 2);
        let popped = coral_list_pop(list);
        assert_eq!(list_len(list), 1);
        assert_eq!(coral_value_as_number(popped), 20.0);
        unsafe {
            coral_value_release(popped);
            coral_value_release(list);
        }
    }

    #[test]
    fn map_length_counts_entries() {
        let key = coral_make_string("foo".as_ptr(), 3);
        let value = coral_make_number(1.0);
        let entries = [MapEntry { key, value }];
        let map = coral_make_map(entries.as_ptr(), entries.len());
        unsafe {
            coral_value_release(key);
            coral_value_release(value);
        }
        let len = coral_map_length(map);
        assert_eq!(coral_value_as_number(len), 1.0);
        unsafe {
            coral_value_release(len);
            coral_value_release(map);
        }
    }

    #[test]
    fn list_get_returns_item() {
        let v1 = coral_make_number(10.0);
        let v2 = coral_make_number(20.0);
        let items = [v1, v2];
        let list = coral_make_list(items.as_ptr(), items.len());
        unsafe {
            coral_value_release(v1);
            coral_value_release(v2);
        }
        let index = coral_make_number(1.0);
        let value = coral_list_get(list, index);
        assert_eq!(coral_value_as_number(value), 20.0);
        unsafe {
            coral_value_release(index);
            coral_value_release(value);
            coral_value_release(list);
        }
    }

    #[test]
    fn map_literal_round_trip() {
        let key1 = coral_make_string("foo".as_ptr(), 3);
        let val1 = coral_make_number(10.0);
        let key2 = coral_make_string("bar".as_ptr(), 3);
        let val2 = coral_make_number(20.0);
        let entries = [
            MapEntry { key: key1, value: val1 },
            MapEntry { key: key2, value: val2 },
        ];
        let map = coral_make_map(entries.as_ptr(), entries.len());
        assert_eq!(map_len(map), 2);
        unsafe {
            coral_value_release(key1);
            coral_value_release(val1);
            coral_value_release(key2);
            coral_value_release(val2);
            coral_value_release(map);
        }
    }

    #[test]
    fn map_get_returns_value() {
        let key1 = coral_make_string("foo".as_ptr(), 3);
        let val1 = coral_make_number(10.0);
        let key2 = coral_make_string("bar".as_ptr(), 3);
        let val2 = coral_make_number(20.0);
        let entries = [
            MapEntry { key: key1, value: val1 },
            MapEntry { key: key2, value: val2 },
        ];
        let map = coral_make_map(entries.as_ptr(), entries.len());
        let lookup_key = coral_make_string("bar".as_ptr(), 3);
        let value = coral_map_get(map, lookup_key);
        assert_eq!(coral_value_as_number(value), 20.0);
        unsafe {
            coral_value_release(key1);
            coral_value_release(val1);
            coral_value_release(key2);
            coral_value_release(val2);
            coral_value_release(map);
            coral_value_release(lookup_key);
            coral_value_release(value);
        }
    }

    #[test]
    fn map_set_updates_existing_entry() {
        let key = coral_make_string("foo".as_ptr(), 3);
        let val = coral_make_number(10.0);
        let entries = [MapEntry { key, value: val }];
        let map = coral_make_map(entries.as_ptr(), entries.len());
        let new_value = coral_make_number(42.0);
        let updated = coral_map_set(map, key, new_value);
        let lookup = coral_map_get(updated, key);
        assert_eq!(coral_value_as_number(lookup), 42.0);
        unsafe {
            coral_value_release(key);
            coral_value_release(val);
            coral_value_release(map);
            coral_value_release(new_value);
            coral_value_release(updated);
            coral_value_release(lookup);
        }
    }

    #[test]
    fn map_set_inserts_new_entry() {
        let map = coral_make_map(ptr::null(), 0);
        let key = coral_make_string("foo".as_ptr(), 3);
        let value = coral_make_number(99.0);
        let updated = coral_map_set(map, key, value);
        let lookup = coral_map_get(updated, key);
        assert_eq!(coral_value_as_number(lookup), 99.0);
        unsafe {
            coral_value_release(map);
            coral_value_release(updated);
            coral_value_release(key);
            coral_value_release(value);
            coral_value_release(lookup);
        }
    }

    #[test]
    fn map_equality_ignores_insertion_order() {
        let k1 = coral_make_string("a".as_ptr(), 1);
        let v1 = coral_make_number(1.0);
        let k2 = coral_make_string("b".as_ptr(), 1);
        let v2 = coral_make_number(2.0);

        let entries1 = [MapEntry { key: k1, value: v1 }, MapEntry { key: k2, value: v2 }];
        let map1 = coral_make_map(entries1.as_ptr(), entries1.len());

        let entries2 = [MapEntry { key: k2, value: v2 }, MapEntry { key: k1, value: v1 }];
        let map2 = coral_make_map(entries2.as_ptr(), entries2.len());

        let eq = coral_value_equals(map1, map2);
        assert_eq!(coral_value_as_bool(eq), 1);

        unsafe {
            coral_value_release(k1);
            coral_value_release(v1);
            coral_value_release(k2);
            coral_value_release(v2);
            coral_value_release(map1);
            coral_value_release(map2);
            coral_value_release(eq);
        }
    }

    #[test]
    fn map_hash_aligns_with_structural_equality() {
        let k1 = coral_make_string("x".as_ptr(), 1);
        let v1 = coral_make_number(10.0);
        let k2 = coral_make_string("y".as_ptr(), 1);
        let v2 = coral_make_number(20.0);
        let entries1 = [MapEntry { key: k1, value: v1 }, MapEntry { key: k2, value: v2 }];
        let entries2 = [MapEntry { key: k2, value: v2 }, MapEntry { key: k1, value: v1 }];
        let map1 = coral_make_map(entries1.as_ptr(), entries1.len());
        let map2 = coral_make_map(entries2.as_ptr(), entries2.len());
        let diff_key = coral_make_string("z".as_ptr(), 1);
        let diff_entries = [MapEntry { key: diff_key, value: v1 }];
        let map3 = coral_make_map(diff_entries.as_ptr(), diff_entries.len());

        let h1 = coral_value_hash(map1);
        let h2 = coral_value_hash(map2);
        let h3 = coral_value_hash(map3);
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);

        unsafe {
            coral_value_release(k1);
            coral_value_release(v1);
            coral_value_release(k2);
            coral_value_release(v2);
            coral_value_release(diff_key);
            coral_value_release(map1);
            coral_value_release(map2);
            coral_value_release(map3);
        }
    }

    #[test]
    fn list_iterator_survives_parent_release() {
        let v1 = coral_make_number(11.0);
        let v2 = coral_make_number(22.0);
        let items = [v1, v2];
        let list = coral_make_list(items.as_ptr(), items.len());
        unsafe {
            coral_value_release(v1);
            coral_value_release(v2);
        }
        let iter = coral_list_iter(list);
        unsafe { coral_value_release(list); }

        let first = coral_list_iter_next(iter);
        let second = coral_list_iter_next(iter);
        let done = coral_list_iter_next(iter);
        assert_eq!(coral_value_tag(done), ValueTag::Unit as u8);
        assert_eq!(coral_value_as_number(first), 11.0);
        assert_eq!(coral_value_as_number(second), 22.0);

        unsafe {
            coral_value_release(first);
            coral_value_release(second);
            coral_value_release(done);
            coral_value_release(iter);
        }
    }

    #[test]
    fn map_iterator_is_snapshot_after_mutation() {
        let k1 = coral_make_string("k1".as_ptr(), 2);
        let v1 = coral_make_number(1.0);
        let entries = [MapEntry { key: k1, value: v1 }];
        let map = coral_make_map(entries.as_ptr(), entries.len());
        let iter = coral_map_iter(map);

        let k2 = coral_make_string("k2".as_ptr(), 2);
        let v2 = coral_make_number(2.0);
        let updated = coral_map_set(map, k2, v2);

        let pair = coral_map_iter_next(iter);
        assert_eq!(list_len(pair), 2);
        let idx0 = coral_make_number(0.0);
        let idx1 = coral_make_number(1.0);
        let key = coral_list_get(pair, idx0);
        let val = coral_list_get(pair, idx1);
        let done = coral_map_iter_next(iter);
        assert_eq!(coral_value_tag(done), ValueTag::Unit as u8);
        let key_bytes = unsafe { string_to_bytes(&*key) };
        assert_eq!(key_bytes, b"k1");
        assert_eq!(coral_value_as_number(val), 1.0);

        unsafe {
            coral_value_release(k1);
            coral_value_release(v1);
            coral_value_release(k2);
            coral_value_release(v2);
            coral_value_release(updated);
            coral_value_release(map);
            coral_value_release(iter);
            coral_value_release(pair);
            coral_value_release(key);
            coral_value_release(val);
            coral_value_release(done);
            coral_value_release(idx0);
            coral_value_release(idx1);
        }
    }

    unsafe extern "C" fn double_invoke(
        _env: *mut c_void,
        args: *const ValueHandle,
        len: usize,
        out: *mut ValueHandle,
    ) {
        let args = slice::from_raw_parts(args, len);
        let input = coral_value_as_number(args[0]);
        if !out.is_null() {
            *out = coral_make_number(input * 2.0);
        }
    }

    unsafe extern "C" fn even_predicate(
        _env: *mut c_void,
        args: *const ValueHandle,
        len: usize,
        out: *mut ValueHandle,
    ) {
        let args = slice::from_raw_parts(args, len);
        let input = coral_value_as_number(args[0]) as i64;
        let is_even = if input % 2 == 0 { 1 } else { 0 };
        if !out.is_null() {
            *out = coral_make_bool(is_even);
        }
    }

    unsafe extern "C" fn sum_invoke(
        _env: *mut c_void,
        args: *const ValueHandle,
        len: usize,
        out: *mut ValueHandle,
    ) {
        let args = slice::from_raw_parts(args, len);
        let a = coral_value_as_number(args[0]);
        let b = coral_value_as_number(args[1]);
        if !out.is_null() {
            *out = coral_make_number(a + b);
        }
    }

    #[test]
    fn list_map_filter_reduce_round_trip() {
        let values = [
            coral_make_number(1.0),
            coral_make_number(2.0),
            coral_make_number(3.0),
            coral_make_number(4.0),
        ];
        let list = coral_make_list(values.as_ptr(), values.len());
        unsafe {
            for v in values {
                coral_value_release(v);
            }
        }

        let double = coral_make_closure(Some(double_invoke), None, ptr::null_mut(), 0);
        let even = coral_make_closure(Some(even_predicate), None, ptr::null_mut(), 0);
        let sum = coral_make_closure(Some(sum_invoke), None, ptr::null_mut(), 0);

        let mapped = coral_list_map(list, double);
        let filtered = coral_list_filter(mapped, even);
        let seed = coral_make_number(0.0);
        let reduced = coral_list_reduce(filtered, seed, sum);

        assert_eq!(list_len(mapped), 4);
        assert_eq!(list_len(filtered), 4);
        assert_eq!(coral_value_as_number(reduced), 20.0);

        unsafe {
            coral_value_release(list);
            coral_value_release(mapped);
            coral_value_release(filtered);
            coral_value_release(seed);
            coral_value_release(reduced);
            coral_value_release(double);
            coral_value_release(even);
            coral_value_release(sum);
        }
    }

    /// Snapshot live value count before an operation, run it, then assert
    /// no net leaks (live count returns to baseline).
    fn assert_no_leak<F: FnOnce()>(f: F) {
        let before = LIVE_VALUE_COUNT.load(Ordering::SeqCst);
        f();
        let after = LIVE_VALUE_COUNT.load(Ordering::SeqCst);
        assert_eq!(before, after, "leak detected: {} values before, {} after (delta {})",
            before, after, after as i64 - before as i64);
    }

    #[test]
    fn leak_detect_numbers() {
        assert_no_leak(|| {
            let a = coral_make_number(1.0);
            let b = coral_make_number(2.0);
            unsafe {
                coral_value_release(a);
                coral_value_release(b);
            }
        });
    }

    #[test]
    fn leak_detect_strings() {
        assert_no_leak(|| {
            // Use 20 chars to force heap allocation (>14 = not inline)
            let text = "this is a long text!";
            let s = coral_make_string(text.as_ptr(), text.len());
            assert_eq!(coral_value_tag(s), ValueTag::String as u8);
            unsafe { coral_value_release(s); }
        });
    }

    #[test]
    fn leak_detect_inline_string() {
        assert_no_leak(|| {
            // Inline string (≤14 bytes) — exercises flag collision guard
            // (len=11 sets bit 4 which overlaps FLAG_ERR)
            let text = "hello world";
            let s = coral_make_string(text.as_ptr(), text.len());
            assert_eq!(coral_value_tag(s), ValueTag::String as u8);
            unsafe { coral_value_release(s); }
        });
    }

    #[test]
    fn leak_detect_list_with_children() {
        assert_no_leak(|| {
            let a = coral_make_number(1.0);
            let b = coral_make_number(2.0);
            let items = [a, b];
            let list = coral_make_list(items.as_ptr(), 2);
            // List retains children; releasing list should release children too
            unsafe {
                coral_value_release(a);
                coral_value_release(b);
                coral_value_release(list);
            }
        });
    }

    #[test]
    fn leak_detect_nested_list() {
        assert_no_leak(|| {
            let inner_item = coral_make_number(42.0);
            let inner = coral_make_list(&inner_item as *const _, 1);
            let outer = coral_make_list(&inner as *const _, 1);
            unsafe {
                coral_value_release(inner_item);
                coral_value_release(inner);
                coral_value_release(outer);
            }
        });
    }

    #[test]
    fn leak_detect_map() {
        assert_no_leak(|| {
            let key = coral_make_string("key".as_ptr(), 3);
            let val = coral_make_number(99.0);
            let mut map = coral_make_map(ptr::null(), 0);
            // coral_map_set retains the map (returns a new owning reference).
            // We must release both the original make_map ref and the map_set ref.
            map = coral_map_set(map, key, val);
            unsafe {
                coral_value_release(key);   // drop our creation ref (map still holds one)
                coral_value_release(val);   // drop our creation ref (map still holds one)
                coral_value_release(map);   // drop map_set return ref (rc 2→1)
                coral_value_release(map);   // drop make_map ref (rc 1→0, frees map+children)
            }
        });
    }

    // ── M2: Non-Atomic RC Fast Path Tests ────────────────────────────────

    #[test]
    fn m2_value_has_owner_thread() {
        // M2.1: New values should be stamped with the current thread's ID
        let val = coral_make_number(1.0);
        let value_ref = unsafe { &*val };
        let tid = current_thread_id();
        assert_ne!(tid, 0, "thread ID should never be 0 (reserved for shared)");
        assert_eq!(value_ref.owner_thread, tid);
        unsafe { coral_value_release(val); }
    }

    #[test]
    fn m2_nonatomic_retain_release() {
        // M2.2: Retain and release should work correctly on thread-local values
        let val = coral_make_number(42.0);
        let value_ref = unsafe { &*val };
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        // Retain should increment
        unsafe { coral_value_retain(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 2);

        // Another retain
        unsafe { coral_value_retain(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 3);

        // Release back down
        unsafe { coral_value_release(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 2);

        unsafe { coral_value_release(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        // Final release frees
        unsafe { coral_value_release(val); }
    }

    #[test]
    fn m2_string_nonatomic_rc() {
        // Heap-allocated string should also use non-atomic fast path
        let s = coral_make_string("hello world testing".as_ptr(), 19);
        let value_ref = unsafe { &*s };
        assert_eq!(value_ref.owner_thread, current_thread_id());
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        unsafe { coral_value_retain(s); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 2);

        unsafe { coral_value_release(s); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        unsafe { coral_value_release(s); }
    }

    #[test]
    fn m2_freeze_promotes_to_atomic() {
        // M2.3: freeze_value should set owner_thread to 0 (shared mode)
        let val = coral_make_string("freeze me".as_ptr(), 9);
        let value_ref = unsafe { &*val };
        assert_ne!(value_ref.owner_thread, 0, "before freeze, should be thread-local");

        freeze_value(val);

        let value_ref = unsafe { &*val };
        assert_eq!(value_ref.owner_thread, 0, "after freeze, should be in shared mode");
        assert!(is_frozen(val), "should be flagged as frozen");

        // Retain/release should still work (via atomic path now)
        unsafe { coral_value_retain(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 2);

        unsafe { coral_value_release(val); }
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        unsafe { coral_value_release(val); }
    }

    #[test]
    fn m2_freeze_list_promotes_children() {
        // M2.3: Freezing a list should also promote all children
        let items: Vec<ValueHandle> = vec![
            coral_make_number(1.0),
            coral_make_string("hello".as_ptr(), 5),
        ];
        let list = coral_make_list(items.as_ptr(), items.len());

        freeze_value(list);

        // List itself should be promoted
        let list_ref = unsafe { &*list };
        assert_eq!(list_ref.owner_thread, 0);

        // Children should also be promoted
        if let Some(list_obj) = list_from_value(list_ref) {
            for &item in &list_obj.items {
                if !item.is_null() {
                    let child_ref = unsafe { &*item };
                    assert_eq!(child_ref.owner_thread, 0,
                        "child should be promoted to shared mode after freeze");
                }
            }
        }

        unsafe { coral_value_release(list); }
    }

    #[test]
    fn m2_thread_ids_are_unique() {
        // M2.1: Thread IDs should be unique across threads
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();

        for _ in 0..4 {
            let tx = tx.clone();
            std::thread::spawn(move || {
                tx.send(current_thread_id()).unwrap();
            });
        }
        drop(tx);

        let mut ids: Vec<u32> = rx.iter().collect();
        let main_id = current_thread_id();
        ids.push(main_id);
        ids.sort();
        ids.dedup();
        // All thread IDs should be unique (no duplicates removed)
        assert_eq!(ids.len(), 5, "5 threads should have 5 unique IDs");
        assert!(!ids.contains(&0), "no thread should have ID 0 (reserved for shared)");
    }

    #[test]
    fn m2_cross_thread_retain_release() {
        // Frozen values should be safely retainable/releasable from other threads
        let val = coral_make_string("shared data".as_ptr(), 11);
        freeze_value(val);

        // Retain so the spawned thread can release
        unsafe { coral_value_retain(val); }
        let value_ref = unsafe { &*val };
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 2);

        // Use usize to safely send the raw pointer across threads
        let val_addr = val as usize;
        let handle = std::thread::spawn(move || {
            let ptr = val_addr as ValueHandle;
            // This thread sees owner_thread == 0, uses atomic path
            unsafe { coral_value_release(ptr); }
        });
        handle.join().unwrap();

        // Should be back to rc=1
        let value_ref = unsafe { &*val };
        assert_eq!(value_ref.refcount.load(Ordering::Relaxed), 1);

        unsafe { coral_value_release(val); }
    }
}
