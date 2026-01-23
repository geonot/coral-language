mod map_hash;
mod rc_deferred;
mod module_registry;
mod actor;
mod memory_ops;
mod store;
mod weak_ref;
mod cycle_detector;
mod symbol;

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
    coral_collect_cycles, coral_cycles_detected, coral_cycle_values_collected,
    coral_cycle_roots_count,
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

type ValueHandle = *mut Value;

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
}

type ClosureInvokeFn = Option<unsafe extern "C" fn(*mut c_void, *const ValueHandle, usize, *mut ValueHandle)>;
type ClosureReleaseFn = Option<unsafe extern "C" fn(*mut c_void)>;

struct ClosureObject {
    invoke: ClosureInvokeFn,
    release: ClosureReleaseFn,
    env: *mut c_void,
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
    number: f64,
    ptr: *mut c_void,
    inline: [u8; 16],
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
    /// Reference count - uses atomic operations for thread-safe actor sharing.
    pub refcount: AtomicU64,
    pub retain_events: u32,
    pub release_events: u32,
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
            refcount: AtomicU64::new(self.refcount.load(Ordering::Relaxed)),
            retain_events: self.retain_events,
            release_events: self.release_events,
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
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
            payload: Payload { inline: [0; 16] },
        }
    }

    fn number(value: f64) -> Self {
        Self {
            tag: ValueTag::Number as u8,
            flags: 0,
            reserved: 0,
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
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
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
            payload: Payload { inline },
        }
    }

    fn from_heap(tag: ValueTag, ptr: *mut c_void) -> Self {
        Self {
            tag: tag as u8,
            flags: 0,
            reserved: 0,
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
            payload: Payload { ptr },
        }
    }

    fn from_heap_with_flags(tag: ValueTag, flags: u8, ptr: *mut c_void) -> Self {
        Self {
            tag: tag as u8,
            flags,
            reserved: 0,
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
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
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
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
    #[inline]
    fn is_err(&self) -> bool {
        (self.flags & FLAG_ERR) != 0
    }

    /// Returns true if this value is logically absent/None.
    #[inline]
    fn is_absent(&self) -> bool {
        (self.flags & FLAG_ABSENT) != 0
    }

    /// Returns true if this value is neither an error nor absent.
    #[inline]
    fn is_ok(&self) -> bool {
        (self.flags & (FLAG_ERR | FLAG_ABSENT)) == 0
    }

    /// Create an error value with the given metadata.
    fn error(metadata: *mut ErrorMetadata) -> Self {
        Self {
            tag: ValueTag::Unit as u8,  // Error values have unit as base type
            flags: FLAG_ERR,
            reserved: 0,
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
            payload: Payload { ptr: metadata as *mut c_void },
        }
    }

    /// Create an absent/None value.
    fn absent() -> Self {
        Self {
            tag: ValueTag::Unit as u8,
            flags: FLAG_ABSENT,
            reserved: 0,
            refcount: AtomicU64::new(1),
            retain_events: 0,
            release_events: 0,
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

fn alloc_value(value: Value) -> ValueHandle {
    ensure_runtime_initialized();
    LIVE_VALUE_COUNT.fetch_add(1, Ordering::Relaxed);
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
    let pool = value_pool();
    if let Ok(mut slots) = pool.lock() {
        if slots.0.iter().any(|&h| h == handle) {
            return false;
        }
        if slots.0.len() < VALUE_POOL_LIMIT {
            unsafe {
                (*handle).refcount.store(0, Ordering::Relaxed);
                (*handle).retain_events = 0;
                (*handle).release_events = 0;
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
pub(crate) struct ListObject {
    pub(crate) items: Vec<ValueHandle>,
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

unsafe fn drop_heap_value(value: &mut Value) {
    // First, handle error metadata cleanup (regardless of tag)
    if value.is_err() {
        let ptr = value.heap_ptr();
        if !ptr.is_null() {
            unsafe {
                let metadata = Box::from_raw(ptr as *mut ErrorMetadata);
                // Release the error name string
                if !metadata.name.is_null() {
                    coral_value_release(metadata.name);
                }
            }
        }
        // Clear the error flag and reset
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
                        for handle in iter.items.drain(..) {
                            coral_value_release(handle);
                        }
                    } else {
                        let mut boxed = Box::from_raw(ptr as *mut ListObject);
                        for handle in boxed.items.drain(..) {
                            coral_value_release(handle);
                        }
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
                                coral_value_release(bucket.key);
                                coral_value_release(bucket.value);
                            }
                        }
                    } else {
                        let mut boxed = Box::from_raw(ptr as *mut MapObject);
                        for bucket in boxed.buckets.iter_mut() {
                            if bucket.state == MapBucketState::Occupied {
                                coral_value_release(bucket.key);
                                coral_value_release(bucket.value);
                            }
                        }
                    }
                }
            }
        }
        Ok(ValueTag::Closure) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
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
                return;
            }
            unsafe {
                drop(Box::from_raw(ptr as *mut ActorObject));
            }
        }
        Ok(ValueTag::Tagged) => {
            let ptr = value.heap_ptr();
            if ptr.is_null() {
                return;
            }
            unsafe {
                let tagged = Box::from_raw(ptr as *mut TaggedValue);
                // Release all field values
                for i in 0..tagged.field_count {
                    let field = *tagged.fields.add(i);
                    if !field.is_null() {
                        coral_value_release(field);
                    }
                }
                // Free the fields array if not empty
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

fn string_to_bytes(value: &Value) -> Vec<u8> {
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
fn value_to_rust_string(value: &Value) -> String {
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
    let new_capacity = (map.buckets.len() * 2).max(8);
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

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_iter(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    let snapshot = map_iter_snapshot(map_obj);
    let boxed = Box::new(snapshot);
    alloc_value(Value::from_heap_with_flags(
        ValueTag::Map,
        FLAG_MAP_ITER,
        Box::into_raw(boxed) as *mut c_void,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_iter(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let v = unsafe { &*value };
    match ValueTag::try_from(v.tag) {
        Ok(ValueTag::List) => coral_list_iter(value),
        Ok(ValueTag::Map) => coral_map_iter(value),
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_iter_next(iter: ValueHandle) -> ValueHandle {
    if iter.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &mut *iter };
    if value.tag != ValueTag::Map as u8 || (value.flags & FLAG_MAP_ITER) == 0 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let iter_obj = unsafe { &mut *(ptr as *mut MapIter) };
    while iter_obj.index < iter_obj.buckets.len() {
        let idx = iter_obj.index;
        iter_obj.index += 1;
        let bucket = &iter_obj.buckets[idx];
        if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() && !bucket.value.is_null() {
            unsafe {
                coral_value_retain(bucket.key);
                coral_value_retain(bucket.value);
            }
            let pair = [bucket.key, bucket.value];
            return coral_make_list(pair.as_ptr(), 2);
        }
    }
    coral_make_unit()
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_iter(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut items: Vec<ValueHandle> = Vec::with_capacity(list_obj.items.len());
    for &h in &list_obj.items {
        if !h.is_null() {
            unsafe { coral_value_retain(h); }
        }
        items.push(h);
    }
    let iter = Box::new(ListIter { items, index: 0 });
    alloc_value(Value::from_heap_with_flags(
        ValueTag::List,
        FLAG_LIST_ITER,
        Box::into_raw(iter) as *mut c_void,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_iter_next(iter: ValueHandle) -> ValueHandle {
    if iter.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &mut *iter };
    if value.tag != ValueTag::List as u8 || (value.flags & FLAG_LIST_ITER) == 0 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let iter_obj = unsafe { &mut *(ptr as *mut ListIter) };
    if iter_obj.index >= iter_obj.items.len() {
        return coral_make_unit();
    }
    let handle = iter_obj.items[iter_obj.index];
    iter_obj.index += 1;
    if handle.is_null() {
        return coral_make_unit();
    }
    unsafe { coral_value_retain(handle); }
    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_map(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut results: Vec<ValueHandle> = Vec::with_capacity(list_obj.items.len());
    for &item in &list_obj.items {
        let args = [item];
        let mapped = coral_closure_invoke(func, args.as_ptr(), args.len());
        results.push(mapped);
    }
    let out = coral_make_list(results.as_ptr(), results.len());
    unsafe {
        for h in results {
            coral_value_release(h);
        }
    }
    out
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_filter(list: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut kept: Vec<ValueHandle> = Vec::new();
    for &item in &list_obj.items {
        let args = [item];
        let predicate = coral_closure_invoke(func, args.as_ptr(), args.len());
        let truthy = coral_value_as_bool(predicate) != 0;
        unsafe { coral_value_release(predicate); }
        if truthy && !item.is_null() {
            unsafe { coral_value_retain(item); }
            kept.push(item);
        }
    }
    let out = coral_make_list(kept.as_ptr(), kept.len());
    unsafe {
        for h in kept {
            coral_value_release(h);
        }
    }
    out
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_reduce(list: ValueHandle, seed: ValueHandle, func: ValueHandle) -> ValueHandle {
    if list.is_null() || func.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let func_value = unsafe { &*func };
    if func_value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let Some(list_obj) = list_from_value(list_value) else {
        return coral_make_unit();
    };
    let mut iter = list_obj.items.iter();
    let mut acc = if !seed.is_null() {
        unsafe { coral_value_retain(seed); }
        seed
    } else {
        match iter.next() {
            Some(first) if !first.is_null() => {
                unsafe { coral_value_retain(*first); }
                *first
            }
            _ => return coral_make_unit(),
        }
    };
    for &item in iter {
        let args = [acc, item];
        let next = coral_closure_invoke(func, args.as_ptr(), args.len());
        unsafe { coral_value_release(acc); }
        acc = next;
    }
    acc
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

/// Create an error value with the given code and name.
/// 
/// # Arguments
/// * `code` - Numeric error code (0 for anonymous errors)
/// * `name_ptr` - Pointer to UTF-8 error name string (e.g., "NotFound")
/// * `name_len` - Length of the name string in bytes
/// 
/// # Returns
/// A new error value with the ERR flag set
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_error(code: u32, name_ptr: *const u8, name_len: usize) -> ValueHandle {
    // Create the error name as a string value
    let name_handle = coral_make_string(name_ptr, name_len);
    
    // Allocate error metadata
    let metadata = Box::new(ErrorMetadata {
        code,
        _reserved: 0,
        name: name_handle,
        origin_span: 0,
    });
    
    record_heap_bytes(std::mem::size_of::<ErrorMetadata>());
    alloc_value(Value::error(Box::into_raw(metadata)))
}

/// Create an error value with the given code, name, and origin span.
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_error_with_span(
    code: u32,
    name_ptr: *const u8,
    name_len: usize,
    origin_span: u64,
) -> ValueHandle {
    let name_handle = coral_make_string(name_ptr, name_len);
    
    let metadata = Box::new(ErrorMetadata {
        code,
        _reserved: 0,
        name: name_handle,
        origin_span,
    });
    
    record_heap_bytes(std::mem::size_of::<ErrorMetadata>());
    alloc_value(Value::error(Box::into_raw(metadata)))
}

/// Create an absent/None value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_make_absent() -> ValueHandle {
    alloc_value(Value::absent())
}

/// Check if a value is an error.
/// Returns 1 if the value has the ERR flag set, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_err(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_err() { 1 } else { 0 }
}

/// Check if a value is absent/None.
/// Returns 1 if the value has the ABSENT flag set, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_absent(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_absent() { 1 } else { 0 }
}

/// Check if a value is ok (neither error nor absent).
/// Returns 1 if the value is ok, 0 otherwise.
#[unsafe(no_mangle)]
pub extern "C" fn coral_is_ok(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() { 1 } else { 0 }
}

/// Get the error name from an error value.
/// Returns the error name string, or unit if not an error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_error_name(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if let Some(metadata) = value_ref.error_metadata() {
        unsafe { coral_value_retain(metadata.name); }
        metadata.name
    } else {
        coral_make_unit()
    }
}

/// Get the error code from an error value.
/// Returns the error code, or 0 if not an error.
#[unsafe(no_mangle)]
pub extern "C" fn coral_error_code(value: ValueHandle) -> u32 {
    if value.is_null() {
        return 0;
    }
    let value_ref = unsafe { &*value };
    if let Some(metadata) = value_ref.error_metadata() {
        metadata.code
    } else {
        0
    }
}

/// Return the value if ok, or the default if error/absent.
/// This retains the returned value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_value_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    if value.is_null() {
        unsafe { coral_value_retain(default); }
        return default;
    }
    let value_ref = unsafe { &*value };
    if value_ref.is_ok() {
        unsafe { coral_value_retain(value); }
        value
    } else {
        unsafe { coral_value_retain(default); }
        default
    }
}

/// Unwrap the value or return the default.
/// Same as coral_value_or but named for familiarity.
#[unsafe(no_mangle)]
pub extern "C" fn coral_unwrap_or(value: ValueHandle, default: ValueHandle) -> ValueHandle {
    coral_value_or(value, default)
}

// ============================================================================
// End Error Value Functions
// ============================================================================

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_length(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_number(0.0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Bytes as u8 && value_ref.tag != ValueTag::String as u8 {
        return coral_make_number(0.0);
    }
    coral_make_number(string_to_bytes(value_ref).len() as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_slice(value: ValueHandle, start: usize, len: usize) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Bytes as u8 && value_ref.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let data = string_to_bytes(value_ref);
    if start >= data.len() {
        return coral_make_bytes(ptr::null(), 0);
    }
    let end = (start + len).min(data.len());
    let slice = &data[start..end];
    let handle = alloc_bytes_obj(slice);
    alloc_value(Value::from_heap(ValueTag::Bytes, handle))
}

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
        refcount: AtomicU64::new(1),
        retain_events: 0,
        release_events: 0,
        payload: Payload { ptr: Box::into_raw(tagged) as *mut c_void },
    };
    
    alloc_value(value)
}

/// Get the tag name of a tagged value.
/// 
/// # Returns
/// A string ValueHandle containing the tag name, or Unit if not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_get_tag(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_unit();
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    coral_make_string(tagged.tag_name, tagged.tag_name_len)
}

/// Check if a tagged value has a specific tag.
/// 
/// # Returns
/// A bool ValueHandle (true if tag matches, false otherwise).
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_is_tag(
    value: ValueHandle,
    tag_name: *const u8,
    tag_name_len: usize,
) -> ValueHandle {
    if value.is_null() {
        return coral_make_bool(0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_bool(0);
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_bool(0);
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    
    if tagged.tag_name_len != tag_name_len {
        return coral_make_bool(0);
    }
    
    let stored = unsafe { slice::from_raw_parts(tagged.tag_name, tagged.tag_name_len) };
    let check = unsafe { slice::from_raw_parts(tag_name, tag_name_len) };
    
    coral_make_bool(if stored == check { 1 } else { 0 })
}

/// Get a field from a tagged value by index.
/// 
/// # Returns
/// The field ValueHandle, or Unit if out of bounds or not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_get_field(value: ValueHandle, index: usize) -> ValueHandle {
    if value.is_null() {
        return coral_make_unit();
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_unit();
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    
    if index >= tagged.field_count || tagged.fields.is_null() {
        return coral_make_unit();
    }
    
    let field = unsafe { *tagged.fields.add(index) };
    if !field.is_null() {
        unsafe { coral_value_retain(field) };
    }
    field
}

/// Get the number of fields in a tagged value.
/// 
/// # Returns
/// A number ValueHandle with the field count, or 0 if not a tagged value.
#[unsafe(no_mangle)]
pub extern "C" fn coral_tagged_field_count(value: ValueHandle) -> ValueHandle {
    if value.is_null() {
        return coral_make_number(0.0);
    }
    let value_ref = unsafe { &*value };
    if value_ref.tag != ValueTag::Tagged as u8 {
        return coral_make_number(0.0);
    }
    let ptr = value_ref.heap_ptr();
    if ptr.is_null() {
        return coral_make_number(0.0);
    }
    let tagged = unsafe { &*(ptr as *const TaggedValue) };
    coral_make_number(tagged.field_count as f64)
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

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_read(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_unit();
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_unit();
    };
    match fs::read(&pb) {
        Ok(bytes) => coral_make_bytes(bytes.as_ptr(), bytes.len()),
        Err(_) => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_exists(path: ValueHandle) -> ValueHandle {
    if path.is_null() {
        return coral_make_bool(0);
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_bool(0);
    };
    let exists = pb.exists();
    coral_make_bool(if exists { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_fs_write(path: ValueHandle, data: ValueHandle) -> ValueHandle {
    if path.is_null() || data.is_null() {
        return coral_make_bool(0);
    }
    let path_ref = unsafe { &*path };
    let Some(pb) = value_to_path(path_ref) else {
        return coral_make_bool(0);
    };
    let bytes = {
        let data_ref = unsafe { &*data };
        string_to_bytes(data_ref)
    };
    let result = fs::write(&pb, bytes);
    coral_make_bool(if result.is_ok() { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_make_closure(
    invoke: ClosureInvokeFn,
    release: ClosureReleaseFn,
    env: *mut c_void,
) -> ValueHandle {
    if invoke.is_none() {
        return coral_make_unit();
    }
    let object = Box::new(ClosureObject { invoke, release, env });
    alloc_value(Value::from_heap(
        ValueTag::Closure,
        Box::into_raw(object) as *mut c_void,
    ))
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_closure_invoke(
    closure: ValueHandle,
    args: *const ValueHandle,
    len: usize,
) -> ValueHandle {
    if closure.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &*closure };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    let ptr = value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let object = unsafe { &*(ptr as *const ClosureObject) };
    let invoke = match object.invoke {
        Some(func) => func,
        None => return coral_make_unit(),
    };
    let mut out: ValueHandle = ptr::null_mut();
    unsafe {
        invoke(object.env, args, len, &mut out);
    }
    if out.is_null() {
        coral_make_unit()
    } else {
        out
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_spawn(handler: ValueHandle) -> ValueHandle {
    if handler.is_null() {
        return coral_make_unit();
    }
    let value = unsafe { &*handler };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handler);
    }
    
    // Encode handler pointer as usize to satisfy Send bound
    let handler_bits = handler as usize;
    let system = actor::global_system().clone();
    let parent = actor::current_actor();
    let handle = system.spawn(parent, move |ctx| {
        let handler = handler_bits as ValueHandle;
        let self_value = actor_to_value(ctx.handle(), ctx.system());
        loop {
            match ctx.recv() {
                Some(actor::Message::User(msg)) => {
                    let args = [self_value, msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe { coral_value_release(result); }
                    unsafe { coral_value_release(msg); }
                }
                Some(actor::Message::Exit) | None => break,
                Some(actor::Message::Failure(reason)) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle { id: parent, sender: entry.sender.clone() };
                                let _ = ctx.system().send(&parent_handle, actor::Message::Failure(reason));
                            }
                        }
                    }
                    break;
                }
                Some(actor::Message::ChildFailure { child_id, reason }) => {
                    // Supervision: handle child failure - by default, propagate to parent
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle { id: parent, sender: entry.sender.clone() };
                                let _ = ctx.system().send(&parent_handle, actor::Message::ChildFailure { child_id, reason });
                            }
                        }
                    }
                }
            }
        }
        unsafe { coral_value_release(self_value); }
        unsafe { coral_value_release(handler); }
    });
    actor_to_value(handle, system)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_send(actor_value: ValueHandle, message: ValueHandle) -> ValueHandle {
    if actor_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(actor_obj) = actor_from_value(unsafe { &*actor_value }) else {
        return coral_make_bool(0);
    };
    freeze_value(message);
    unsafe { coral_value_retain(message); }
    let ok = actor_obj
        .system
        .send(&actor_obj.handle, actor::Message::User(message))
        .is_ok();
    if !ok {
        unsafe { coral_value_release(message); }
    }
    coral_make_bool(if ok { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_stop(actor_value: ValueHandle) -> ValueHandle {
    if actor_value.is_null() {
        return coral_make_bool(0);
    }
    let Some(actor_obj) = actor_from_value(unsafe { &*actor_value }) else {
        return coral_make_bool(0);
    };
    let ok = actor_obj
        .system
        .send(&actor_obj.handle, actor::Message::Exit)
        .is_ok();
    coral_make_bool(if ok { 1 } else { 0 })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_self() -> ValueHandle {
    let Some(id) = actor::current_actor() else { return coral_make_unit(); };
    let system = actor::global_system();
    let maybe_handle = system
        .registry
        .lock()
        .ok()
        .and_then(|reg| reg.get(&id).map(|entry| ActorHandle { id, sender: entry.sender.clone() }));
    if let Some(handle) = maybe_handle {
        actor_to_value(handle, system.clone())
    } else {
        coral_make_unit()
    }
}

// ========== Named Actor Registry FFI ==========

/// Spawn a named actor. Returns the actor value or unit if name is already taken.
/// name_value: String value containing the actor name
/// handler: Closure to handle messages
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_spawn_named(name_value: ValueHandle, handler: ValueHandle) -> ValueHandle {
    if name_value.is_null() || handler.is_null() {
        return coral_make_unit();
    }
    
    // Extract name string
    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_unit();
        }
        value_to_rust_string(name_val)
    };
    
    let value = unsafe { &*handler };
    if value.tag != ValueTag::Closure as u8 {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handler);
    }
    
    let handler_bits = handler as usize;
    let system = actor::global_system().clone();
    let parent = actor::current_actor();
    
    let maybe_handle = system.spawn_named(&name, parent, move |ctx| {
        let handler = handler_bits as ValueHandle;
        let self_value = actor_to_value(ctx.handle(), ctx.system());
        loop {
            match ctx.recv() {
                Some(actor::Message::User(msg)) => {
                    let args = [self_value, msg];
                    let result = coral_closure_invoke(handler, args.as_ptr(), args.len());
                    unsafe { coral_value_release(result); }
                    unsafe { coral_value_release(msg); }
                }
                Some(actor::Message::Exit) | None => break,
                Some(actor::Message::Failure(reason)) => {
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle { id: parent, sender: entry.sender.clone() };
                                let _ = ctx.system().send(&parent_handle, actor::Message::Failure(reason));
                            }
                        }
                    }
                    break;
                }
                Some(actor::Message::ChildFailure { child_id, reason }) => {
                    // Supervision: handle child failure - by default, propagate to parent
                    if let Some(parent) = ctx.parent() {
                        if let Ok(reg) = ctx.system().registry.lock() {
                            if let Some(entry) = reg.get(&parent) {
                                let parent_handle = ActorHandle { id: parent, sender: entry.sender.clone() };
                                let _ = ctx.system().send(&parent_handle, actor::Message::ChildFailure { child_id, reason });
                            }
                        }
                    }
                }
            }
        }
        unsafe { coral_value_release(self_value); }
        unsafe { coral_value_release(handler); }
    });
    
    match maybe_handle {
        Some(handle) => actor_to_value(handle, system),
        None => {
            // Name was already taken
            unsafe { coral_value_release(handler); }
            coral_make_unit()
        }
    }
}

/// Look up an actor by name. Returns the actor value or unit if not found.
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_lookup(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_unit();
    }
    
    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_unit();
        }
        value_to_rust_string(name_val)
    };
    
    let system = actor::global_system();
    match system.lookup_named(&name) {
        Some(handle) => actor_to_value(handle, system.clone()),
        None => coral_make_unit(),
    }
}

/// Register the current actor with a name. Returns true on success, false if name taken.
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_register(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }
    
    let Some(id) = actor::current_actor() else {
        return coral_make_bool(0);
    };
    
    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };
    
    let system = actor::global_system();
    
    // Get the current actor's handle
    let maybe_handle = system
        .registry
        .lock()
        .ok()
        .and_then(|reg| reg.get(&id).map(|entry| ActorHandle { id, sender: entry.sender.clone() }));
    
    if let Some(handle) = maybe_handle {
        let success = system.register_named(&name, handle);
        coral_make_bool(if success { 1 } else { 0 })
    } else {
        coral_make_bool(0)
    }
}

/// Unregister a named actor. Returns true if the name existed.
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_unregister(name_value: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }
    
    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };
    
    let system = actor::global_system();
    let success = system.unregister_named(&name);
    coral_make_bool(if success { 1 } else { 0 })
}

/// Send a message to a named actor. Returns true on success, false if actor not found.
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_send_named(name_value: ValueHandle, message: ValueHandle) -> ValueHandle {
    if name_value.is_null() {
        return coral_make_bool(0);
    }
    
    let name = {
        let name_val = unsafe { &*name_value };
        if name_val.tag != ValueTag::String as u8 {
            return coral_make_bool(0);
        }
        value_to_rust_string(name_val)
    };
    
    let system = actor::global_system();
    
    if let Some(handle) = system.lookup_named(&name) {
        freeze_value(message);
        unsafe { coral_value_retain(message); }
        let ok = system.send(&handle, actor::Message::User(message)).is_ok();
        if !ok {
            unsafe { coral_value_release(message); }
        }
        coral_make_bool(if ok { 1 } else { 0 })
    } else {
        coral_make_bool(0)
    }
}

/// List all registered named actors. Returns a list of name strings.
#[unsafe(no_mangle)]
pub extern "C" fn coral_actor_list_named() -> ValueHandle {
    let system = actor::global_system();
    let named = system.list_named();
    
    let mut names: Vec<ValueHandle> = Vec::with_capacity(named.len());
    for (name, _) in named {
        names.push(coral_make_string(name.as_ptr(), name.len()));
    }
    
    let handle = coral_make_list(names.as_ptr(), names.len());
    // Release our temporary references
    unsafe {
        for name in names {
            coral_value_release(name);
        }
    }
    handle
}

// ========== Timer FFI Functions ==========

/// Helper to extract a number from a ValueHandle.
fn value_to_f64(value: ValueHandle) -> Option<f64> {
    if value.is_null() {
        return None;
    }
    let v = unsafe { &*value };
    if v.tag == ValueTag::Number as u8 {
        Some(unsafe { v.payload.number })
    } else {
        None
    }
}

/// Send a message to an actor after a delay (in milliseconds).
/// Returns a timer token (integer ID) that can be used to cancel the timer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_send_after(
    delay_ms_value: ValueHandle,
    actor_value: ValueHandle,
    message: ValueHandle,
) -> ValueHandle {
    use std::time::Duration;
    
    let delay_ms = match value_to_f64(delay_ms_value) {
        Some(d) if d >= 0.0 => d as u64,
        _ => return coral_make_number(0.0),
    };
    
    // Extract actor handle from value
    let actor_val = if actor_value.is_null() {
        return coral_make_number(0.0);
    } else {
        unsafe { &*actor_value }
    };
    
    if actor_val.tag != ValueTag::Actor as u8 {
        return coral_make_number(0.0);
    }
    
    let actor_ptr = actor_val.heap_ptr();
    if actor_ptr.is_null() {
        return coral_make_number(0.0);
    }
    
    let handle = unsafe { &*(actor_ptr as *const ActorHandle) };
    
    // Freeze and retain the message for sending later
    freeze_value(message);
    unsafe { coral_value_retain(message); }
    
    let system = actor::global_system();
    let token = system.send_after(
        Duration::from_millis(delay_ms),
        handle,
        message,
    );
    
    coral_make_number(token.id().0 as f64)
}

/// Schedule a message to be sent repeatedly to an actor at the given interval (in milliseconds).
/// Returns a timer token (integer ID) that can be used to cancel the timer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_schedule_repeat(
    interval_ms_value: ValueHandle,
    actor_value: ValueHandle,
    message: ValueHandle,
) -> ValueHandle {
    use std::time::Duration;
    
    let interval_ms = match value_to_f64(interval_ms_value) {
        Some(d) if d > 0.0 => d as u64,
        _ => return coral_make_number(0.0),
    };
    
    // Extract actor handle from value
    let actor_val = if actor_value.is_null() {
        return coral_make_number(0.0);
    } else {
        unsafe { &*actor_value }
    };
    
    if actor_val.tag != ValueTag::Actor as u8 {
        return coral_make_number(0.0);
    }
    
    let actor_ptr = actor_val.heap_ptr();
    if actor_ptr.is_null() {
        return coral_make_number(0.0);
    }
    
    let handle = unsafe { &*(actor_ptr as *const ActorHandle) };
    
    // Freeze and retain the message for sending later
    freeze_value(message);
    unsafe { coral_value_retain(message); }
    
    let system = actor::global_system();
    let token = system.schedule_repeat(
        Duration::from_millis(interval_ms),
        handle,
        message,
    );
    
    coral_make_number(token.id().0 as f64)
}

/// Cancel a timer by its ID. Returns true if the timer was cancelled.
#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_cancel(timer_id_value: ValueHandle) -> ValueHandle {
    let timer_id = match value_to_f64(timer_id_value) {
        Some(id) if id > 0.0 => id as u64,
        _ => return coral_make_bool(0),
    };
    
    let system = actor::global_system();
    let cancelled = system.timer_wheel.cancel(actor::TimerId(timer_id));
    coral_make_bool(if cancelled { 1 } else { 0 })
}

/// Get the number of pending timers.
#[unsafe(no_mangle)]
pub extern "C" fn coral_timer_pending_count() -> ValueHandle {
    let system = actor::global_system();
    let count = system.pending_timers();
    coral_make_number(count as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_get(map: ValueHandle, key: ValueHandle) -> ValueHandle {
    if map.is_null() || key.is_null() {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    if let Some(bucket) = map_get_entry(map_obj, key) {
        if bucket.value.is_null() {
            return coral_make_unit();
        }
        unsafe {
            coral_value_retain(bucket.value);
        }
        bucket.value
    } else {
        coral_make_unit()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_keys(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    let mut keys: Vec<ValueHandle> = Vec::with_capacity(map_obj.len);
    for bucket in &map_obj.buckets {
        if bucket.state == MapBucketState::Occupied && !bucket.key.is_null() {
            unsafe { coral_value_retain(bucket.key); }
            keys.push(bucket.key);
        }
    }
    let handle = coral_make_list(keys.as_ptr(), keys.len());
    // the list constructor retained each element; release our Vec-held references
    unsafe {
        for key in keys {
            coral_value_release(key);
        }
    }
    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_set(
    map: ValueHandle,
    key: ValueHandle,
    value: ValueHandle,
) -> ValueHandle {
    if map.is_null() || key.is_null() || value.is_null() {
        return coral_make_unit();
    }
    if is_frozen(map) {
        return coral_make_unit();
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_unit();
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let map_obj = unsafe { &mut *(ptr as *mut MapObject) };
    let replaced = map_insert(map_obj, key, value);
    if let Some(old) = replaced {
        unsafe {
            coral_value_release(old);
            coral_value_retain(map);
        }
        return map;
    }
    unsafe {
        coral_value_retain(map);
    }
    map
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_string_concat(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag == ValueTag::String as u8 && vb.tag == ValueTag::String as u8 {
        let mut bytes = string_to_bytes(va);
        bytes.extend(string_to_bytes(vb));
        let handle = alloc_string(&bytes);
        alloc_value(Value::from_heap(ValueTag::String, handle))
    } else {
        coral_make_unit()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_bytes_concat(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    if va.tag == ValueTag::Bytes as u8 && vb.tag == ValueTag::Bytes as u8 {
        let mut bytes = string_to_bytes(va);
        bytes.extend(string_to_bytes(vb));
        let handle = alloc_bytes_obj(&bytes);
        return alloc_value(Value::from_heap(ValueTag::Bytes, handle));
    }
    coral_make_unit()
}

/// Get a substring of a string.
/// coral_string_slice(str, start, end) returns str[start..end]
/// start and end are byte indices (0-based). If end is greater than length, uses length.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_slice(s: ValueHandle, start: ValueHandle, end: ValueHandle) -> ValueHandle {
    if s.is_null() || start.is_null() || end.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let start_idx = unsafe { (*start).payload.number } as usize;
    let end_idx = (unsafe { (*end).payload.number } as usize).min(bytes.len());
    if start_idx >= bytes.len() || start_idx >= end_idx {
        return coral_make_string(std::ptr::null(), 0);
    }
    let slice = &bytes[start_idx..end_idx];
    coral_make_string(slice.as_ptr(), slice.len())
}

/// Get the character (byte) at a given index.
/// Returns the byte as a single-character string, or Unit if out of bounds.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_char_at(s: ValueHandle, index: ValueHandle) -> ValueHandle {
    if s.is_null() || index.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let idx = unsafe { (*index).payload.number } as usize;
    if idx >= bytes.len() {
        return coral_make_unit();
    }
    let byte = bytes[idx];
    coral_make_string(&byte as *const u8, 1)
}

/// Find the index of a substring in a string.
/// Returns the 0-based index as a number, or -1 if not found.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_index_of(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_number(-1.0);
    }
    let vh = unsafe { &*haystack };
    let vn = unsafe { &*needle };
    if vh.tag != ValueTag::String as u8 || vn.tag != ValueTag::String as u8 {
        return coral_make_number(-1.0);
    }
    let haystack_bytes = string_to_bytes(vh);
    let needle_bytes = string_to_bytes(vn);
    if needle_bytes.is_empty() {
        return coral_make_number(0.0);
    }
    for i in 0..=haystack_bytes.len().saturating_sub(needle_bytes.len()) {
        if haystack_bytes[i..].starts_with(&needle_bytes) {
            return coral_make_number(i as f64);
        }
    }
    coral_make_number(-1.0)
}

/// Split a string by a delimiter.
/// Returns a list of strings.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_split(s: ValueHandle, delimiter: ValueHandle) -> ValueHandle {
    if s.is_null() || delimiter.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let vs = unsafe { &*s };
    let vd = unsafe { &*delimiter };
    if vs.tag != ValueTag::String as u8 || vd.tag != ValueTag::String as u8 {
        return coral_make_list(std::ptr::null(), 0);
    }
    let s_bytes = string_to_bytes(vs);
    let d_bytes = string_to_bytes(vd);
    
    let mut parts: Vec<ValueHandle> = Vec::new();
    
    if d_bytes.is_empty() {
        // Empty delimiter: split into individual characters
        for byte in &s_bytes {
            let part = coral_make_string(byte as *const u8, 1);
            parts.push(part);
        }
    } else {
        let mut start = 0;
        let s_str = String::from_utf8_lossy(&s_bytes);
        let d_str = String::from_utf8_lossy(&d_bytes);
        
        for (i, _) in s_str.match_indices(&*d_str) {
            if i > start {
                let part_bytes = &s_bytes[start..i];
                let part = coral_make_string(part_bytes.as_ptr(), part_bytes.len());
                parts.push(part);
            } else if i == start {
                // Empty part between delimiters
                let part = coral_make_string(std::ptr::null(), 0);
                parts.push(part);
            }
            start = i + d_bytes.len();
        }
        // Add the remaining part
        if start <= s_bytes.len() {
            let part_bytes = &s_bytes[start..];
            let part = coral_make_string(part_bytes.as_ptr(), part_bytes.len());
            parts.push(part);
        }
    }
    
    coral_make_list(parts.as_ptr(), parts.len())
}

/// Convert a string to a list of single-character strings.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_chars(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_list(std::ptr::null(), 0);
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_list(std::ptr::null(), 0);
    }
    let bytes = string_to_bytes(vs);
    let mut chars: Vec<ValueHandle> = Vec::with_capacity(bytes.len());
    for byte in &bytes {
        let char_str = coral_make_string(byte as *const u8, 1);
        chars.push(char_str);
    }
    coral_make_list(chars.as_ptr(), chars.len())
}

/// Check if a string starts with a given prefix.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_starts_with(s: ValueHandle, prefix: ValueHandle) -> ValueHandle {
    if s.is_null() || prefix.is_null() {
        return coral_make_bool(0);
    }
    let vs = unsafe { &*s };
    let vp = unsafe { &*prefix };
    if vs.tag != ValueTag::String as u8 || vp.tag != ValueTag::String as u8 {
        return coral_make_bool(0);
    }
    let s_bytes = string_to_bytes(vs);
    let p_bytes = string_to_bytes(vp);
    coral_make_bool(if s_bytes.starts_with(&p_bytes) { 1 } else { 0 })
}

/// Check if a string ends with a given suffix.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_ends_with(s: ValueHandle, suffix: ValueHandle) -> ValueHandle {
    if s.is_null() || suffix.is_null() {
        return coral_make_bool(0);
    }
    let vs = unsafe { &*s };
    let vx = unsafe { &*suffix };
    if vs.tag != ValueTag::String as u8 || vx.tag != ValueTag::String as u8 {
        return coral_make_bool(0);
    }
    let s_bytes = string_to_bytes(vs);
    let x_bytes = string_to_bytes(vx);
    coral_make_bool(if s_bytes.ends_with(&x_bytes) { 1 } else { 0 })
}

/// Trim whitespace from both ends of a string.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_trim(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let trimmed = s_str.trim();
    coral_make_string(trimmed.as_ptr(), trimmed.len())
}

/// Convert a string to uppercase.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_upper(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let upper = s_str.to_uppercase();
    coral_make_string(upper.as_ptr(), upper.len())
}

/// Convert a string to lowercase.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_to_lower(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    let lower = s_str.to_lowercase();
    coral_make_string(lower.as_ptr(), lower.len())
}

/// Replace all occurrences of a substring with another string.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_replace(s: ValueHandle, old: ValueHandle, new: ValueHandle) -> ValueHandle {
    if s.is_null() || old.is_null() || new.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    let vo = unsafe { &*old };
    let vn = unsafe { &*new };
    if vs.tag != ValueTag::String as u8 || vo.tag != ValueTag::String as u8 || vn.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let s_bytes = string_to_bytes(vs);
    let o_bytes = string_to_bytes(vo);
    let n_bytes = string_to_bytes(vn);
    
    let s_str = String::from_utf8_lossy(&s_bytes);
    let o_str = String::from_utf8_lossy(&o_bytes);
    let n_str = String::from_utf8_lossy(&n_bytes);
    
    let result = s_str.replace(&*o_str, &*n_str);
    coral_make_string(result.as_ptr(), result.len())
}

/// Check if a string contains a substring.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_contains(haystack: ValueHandle, needle: ValueHandle) -> ValueHandle {
    if haystack.is_null() || needle.is_null() {
        return coral_make_bool(0);
    }
    let vh = unsafe { &*haystack };
    let vn = unsafe { &*needle };
    if vh.tag != ValueTag::String as u8 || vn.tag != ValueTag::String as u8 {
        return coral_make_bool(0);
    }
    let h_bytes = string_to_bytes(vh);
    let n_bytes = string_to_bytes(vn);
    
    if n_bytes.is_empty() {
        return coral_make_bool(1);
    }
    
    let h_str = String::from_utf8_lossy(&h_bytes);
    let n_str = String::from_utf8_lossy(&n_bytes);
    
    coral_make_bool(if h_str.contains(&*n_str) { 1 } else { 0 })
}

/// Parse a string as a number.
/// Returns the parsed number or Unit on failure.
#[unsafe(no_mangle)]
pub extern "C" fn coral_string_parse_number(s: ValueHandle) -> ValueHandle {
    if s.is_null() {
        return coral_make_unit();
    }
    let vs = unsafe { &*s };
    if vs.tag != ValueTag::String as u8 {
        return coral_make_unit();
    }
    let bytes = string_to_bytes(vs);
    let s_str = String::from_utf8_lossy(&bytes);
    match s_str.trim().parse::<f64>() {
        Ok(n) => coral_make_number(n),
        Err(_) => coral_make_unit(),
    }
}

/// Convert a number to a string.
#[unsafe(no_mangle)]
pub extern "C" fn coral_number_to_string(n: ValueHandle) -> ValueHandle {
    if n.is_null() {
        return coral_make_unit();
    }
    let vn = unsafe { &*n };
    if vn.tag != ValueTag::Number as u8 {
        return coral_make_unit();
    }
    let num = unsafe { vn.payload.number };
    let s = num.to_string();
    coral_make_string(s.as_ptr(), s.len())
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_add(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if a.is_null() || b.is_null() {
        return coral_make_unit();
    }
    let va = unsafe { &*a };
    let vb = unsafe { &*b };
    
    // Error propagation: if either operand is an error, return that error
    if va.is_err() {
        unsafe { coral_value_retain(a); }
        return a;
    }
    if vb.is_err() {
        unsafe { coral_value_retain(b); }
        return b;
    }
    // Absent propagation
    if va.is_absent() || vb.is_absent() {
        return coral_make_absent();
    }
    
    if va.tag == ValueTag::Number as u8 && vb.tag == ValueTag::Number as u8 {
        let result = unsafe { va.payload.number } + unsafe { vb.payload.number };
        coral_make_number(result)
    } else if va.tag == ValueTag::String as u8 && vb.tag == ValueTag::String as u8 {
        coral_string_concat(a, b)
    } else if va.tag == ValueTag::Bytes as u8 && vb.tag == ValueTag::Bytes as u8 {
        coral_bytes_concat(a, b)
    } else {
        coral_make_unit()
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_equals(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    // Error propagation for equality checks
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return a;
        }
    }
    if !b.is_null() {
        let vb = unsafe { &*b };
        if vb.is_err() {
            unsafe { coral_value_retain(b); }
            return b;
        }
    }
    
    let result = values_equal_handles(a, b);
    coral_make_bool(if result { 1 } else { 0 })
}

/// Helper to propagate errors in binary operations.
/// Returns Some(error_handle) if either operand is an error/absent, None otherwise.
#[inline]
fn propagate_binary_error(a: ValueHandle, b: ValueHandle) -> Option<ValueHandle> {
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return Some(a);
        }
        if va.is_absent() {
            return Some(coral_make_absent());
        }
    }
    if !b.is_null() {
        let vb = unsafe { &*b };
        if vb.is_err() {
            unsafe { coral_value_retain(b); }
            return Some(b);
        }
        if vb.is_absent() {
            return Some(coral_make_absent());
        }
    }
    None
}

/// Helper to propagate errors in unary operations.
#[inline]
fn propagate_unary_error(a: ValueHandle) -> Option<ValueHandle> {
    if !a.is_null() {
        let va = unsafe { &*a };
        if va.is_err() {
            unsafe { coral_value_retain(a); }
            return Some(a);
        }
        if va.is_absent() {
            return Some(coral_make_absent());
        }
    }
    None
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitand(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) & handle_to_i64(b);
    coral_make_number(result as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitor(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) | handle_to_i64(b);
    coral_make_number(result as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitxor(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let result = handle_to_i64(a) ^ handle_to_i64(b);
    coral_make_number(result as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_bitnot(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    let result = !handle_to_i64(value);
    coral_make_number(result as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_shift_left(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let lhs = handle_to_i64(a);
    let rhs = (handle_to_i64(b) & 63) as u32;
    coral_make_number(lhs.wrapping_shl(rhs) as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_shift_right(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    let lhs = handle_to_i64(a);
    let rhs = (handle_to_i64(b) & 63) as u32;
    coral_make_number((lhs >> rhs) as f64)
}

// ==================== Math Functions ====================

/// Extract f64 from a number Value, or return None if not a number.
#[inline]
fn handle_to_f64(handle: ValueHandle) -> Option<f64> {
    if handle.is_null() {
        return None;
    }
    let value = unsafe { &*handle };
    if value.tag == ValueTag::Number as u8 {
        Some(unsafe { value.payload.number })
    } else {
        None
    }
}

/// Absolute value of a number.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_abs(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.abs()),
        None => coral_make_unit(),
    }
}

/// Square root of a number.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sqrt(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sqrt()),
        None => coral_make_unit(),
    }
}

/// Floor of a number.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_floor(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.floor()),
        None => coral_make_unit(),
    }
}

/// Ceiling of a number.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_ceil(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.ceil()),
        None => coral_make_unit(),
    }
}

/// Round a number to nearest integer.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_round(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.round()),
        None => coral_make_unit(),
    }
}

/// Sine of a number (radians).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sin(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sin()),
        None => coral_make_unit(),
    }
}

/// Cosine of a number (radians).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_cos(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.cos()),
        None => coral_make_unit(),
    }
}

/// Tangent of a number (radians).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_tan(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.tan()),
        None => coral_make_unit(),
    }
}

/// Power: a^b
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_pow(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(base), Some(exp)) => coral_make_number(base.powf(exp)),
        _ => coral_make_unit(),
    }
}

/// Minimum of two numbers.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_min(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(x), Some(y)) => coral_make_number(x.min(y)),
        _ => coral_make_unit(),
    }
}

/// Maximum of two numbers.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_max(a: ValueHandle, b: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(a, b) {
        return err;
    }
    match (handle_to_f64(a), handle_to_f64(b)) {
        (Some(x), Some(y)) => coral_make_number(x.max(y)),
        _ => coral_make_unit(),
    }
}

/// Natural logarithm (ln).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_ln(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.ln()),
        None => coral_make_unit(),
    }
}

/// Base-10 logarithm.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_log10(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.log10()),
        None => coral_make_unit(),
    }
}

/// Exponential (e^x).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_exp(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.exp()),
        None => coral_make_unit(),
    }
}

/// Arc sine (inverse sine).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_asin(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.asin()),
        None => coral_make_unit(),
    }
}

/// Arc cosine (inverse cosine).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_acos(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.acos()),
        None => coral_make_unit(),
    }
}

/// Arc tangent (inverse tangent).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_atan(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.atan()),
        None => coral_make_unit(),
    }
}

/// Two-argument arc tangent (atan2).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_atan2(y: ValueHandle, x: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_binary_error(y, x) {
        return err;
    }
    match (handle_to_f64(y), handle_to_f64(x)) {
        (Some(y_val), Some(x_val)) => coral_make_number(y_val.atan2(x_val)),
        _ => coral_make_unit(),
    }
}

/// Hyperbolic sine.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sinh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.sinh()),
        None => coral_make_unit(),
    }
}

/// Hyperbolic cosine.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_cosh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.cosh()),
        None => coral_make_unit(),
    }
}

/// Hyperbolic tangent.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_tanh(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.tanh()),
        None => coral_make_unit(),
    }
}

/// Truncate to integer (towards zero).
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_trunc(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.trunc()),
        None => coral_make_unit(),
    }
}

/// Sign of a number: -1, 0, or 1.
#[unsafe(no_mangle)]
pub extern "C" fn coral_math_sign(value: ValueHandle) -> ValueHandle {
    if let Some(err) = propagate_unary_error(value) {
        return err;
    }
    match handle_to_f64(value) {
        Some(n) => coral_make_number(n.signum()),
        None => coral_make_unit(),
    }
}

// ==================== End Math Functions ====================

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_push(list: ValueHandle, value: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    if is_frozen(list) {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let ptr = list_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let list_obj = unsafe { &mut *(ptr as *mut ListObject) };
    if !value.is_null() {
        unsafe {
            coral_value_retain(value);
        }
        list_obj.items.push(value);
    }
    unsafe {
        coral_value_retain(list);
    }
    list
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_length(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_number(0.0);
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_number(0.0);
    }
    let len = list_from_value(list_value)
        .map(|obj| obj.items.len())
        .unwrap_or(0);
    coral_make_number(len as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_get(list: ValueHandle, index: ValueHandle) -> ValueHandle {
    if list.is_null() || index.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let index_value = unsafe { &*index };
    if index_value.tag != ValueTag::Number as u8 {
        return coral_make_unit();
    }
    let raw_index = unsafe { index_value.payload.number };
    if !raw_index.is_finite() {
        return coral_make_unit();
    }
    let idx = raw_index as isize;
    if raw_index.fract().abs() > f64::EPSILON || idx < 0 {
        return coral_make_unit();
    }
    let list_obj = match list_from_value(list_value) {
        Some(obj) => obj,
        None => return coral_make_unit(),
    };
    let handle = match list_obj.items.get(idx as usize) {
        Some(handle) => *handle,
        None => return coral_make_unit(),
    };
    if handle.is_null() {
        return coral_make_unit();
    }
    unsafe {
        coral_value_retain(handle);
    }
    handle
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_list_pop(list: ValueHandle) -> ValueHandle {
    if list.is_null() {
        return coral_make_unit();
    }
    let list_value = unsafe { &*list };
    if list_value.tag != ValueTag::List as u8 {
        return coral_make_unit();
    }
    let ptr = list_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_unit();
    }
    let list_obj = unsafe { &mut *(ptr as *mut ListObject) };
    match list_obj.items.pop() {
        Some(handle) if !handle.is_null() => unsafe {
            // Transfer ownership: retain for the caller, release the list's hold.
            coral_value_retain(handle);
            coral_value_release(handle);
            handle
        },
        _ => coral_make_unit(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_map_length(map: ValueHandle) -> ValueHandle {
    if map.is_null() {
        return coral_make_number(0.0);
    }
    let map_value = unsafe { &*map };
    if map_value.tag != ValueTag::Map as u8 {
        return coral_make_number(0.0);
    }
    let ptr = map_value.heap_ptr();
    if ptr.is_null() {
        return coral_make_number(0.0);
    }
    let map_obj = unsafe { &*(ptr as *const MapObject) };
    coral_make_number(map_obj.len as f64)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_retain_many(ptrs: *const ValueHandle, len: usize) {
    if ptrs.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { slice::from_raw_parts(ptrs, len) };
    for &p in slice {
        unsafe { coral_value_retain(p); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_release_many(ptrs: *const ValueHandle, len: usize) {
    if ptrs.is_null() || len == 0 {
        return;
    }
    let slice = unsafe { slice::from_raw_parts(ptrs, len) };
    for &p in slice {
        unsafe { coral_value_release(p); }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_tag(value: ValueHandle) -> u8 {
    if value.is_null() {
        return ValueTag::Unit as u8;
    }
    unsafe { (*value).tag }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_number(value: ValueHandle) -> f64 {
    if value.is_null() {
        return 0.0;
    }
    let value = unsafe { &*value };
    if value.tag == ValueTag::Number as u8 {
        unsafe { value.payload.number }
    } else {
        0.0
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_ptr(value: ValueHandle) -> *mut c_void {
    if value.is_null() {
        return ptr::null_mut();
    }
    let value = unsafe { &*value };
    match ValueTag::try_from(value.tag) {
        Ok(ValueTag::Number) | Ok(ValueTag::Bool) | Ok(ValueTag::Unit) => ptr::null_mut(),
        _ => value.heap_ptr(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_as_bool(value: ValueHandle) -> u8 {
    if value.is_null() {
        return 0;
    }
    let value = unsafe { &*value };
    if value.tag == ValueTag::Bool as u8 {
        unsafe { value.payload.inline[0] & 1 }
    } else if value.tag == ValueTag::Number as u8 {
        let num = unsafe { value.payload.number };
        if num.abs() > f64::EPSILON {
            1
        } else {
            0
        }
    } else {
        0
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_value_retain(value: ValueHandle) {
    if value.is_null() {
        return;
    }
    let value = unsafe { &*value };
    let rc = value.refcount.load(Ordering::Relaxed);
    debug_assert!(rc > 0, "retain on freed value");
    if rc == u64::MAX {
        RETAIN_SATURATED.fetch_add(1, Ordering::Relaxed);
        // retain_events is not atomic, but only updated on same thread for debugging
        // In multi-threaded contexts, this becomes unreliable (acceptable for debug stats)
        return;
    }
    // Use fetch_add for atomic increment - Relaxed is sufficient for retain
    // since we don't need to synchronize with any particular memory operations.
    // The Release in coral_value_release will ensure proper visibility.
    value.refcount.fetch_add(1, Ordering::Relaxed);
    RETAIN_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_value_release(value: ValueHandle) {
    if value.is_null() {
        return;
    }
    let value_ref = unsafe { &*value };
    let rc = value_ref.refcount.load(Ordering::Relaxed);
    debug_assert!(rc > 0, "release on freed value");
    if rc == 0 {
        RELEASE_UNDERFLOW.fetch_add(1, Ordering::Relaxed);
        debug_assert!(false, "release underflow on value tag {}", value_ref.tag);
        return;
    }
    RELEASE_COUNT.fetch_add(1, Ordering::Relaxed);
    // Use fetch_sub for atomic decrement with Release ordering
    // The Release ensures all writes before this are visible to other threads
    let prev = value_ref.refcount.fetch_sub(1, Ordering::Release);
    if prev == 1 {
        // Acquire fence to ensure we see all writes before freeing
        std::sync::atomic::fence(Ordering::Acquire);
        
        // Notify weak reference system before deallocation
        weak_ref::notify_value_deallocated(value);
        
        let value_ref_mut = unsafe { &mut *value };
        RELEASE_QUEUE.with(|queue| {
            // Use try_borrow_mut to avoid panic on reentrant releases
            // (e.g., when drop_heap_value releases contained values)
            if let Ok(mut guard) = queue.try_borrow_mut() {
                if let Some(q) = &mut *guard {
                    if let Some(nn) = ptr::NonNull::new(value as *mut c_void) {
                        q.push(nn);
                        return;
                    }
                }
            }
            // Either no queue or reentrant call - free immediately
            unsafe { drop_heap_value(value_ref_mut); }
            LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
            if !recycle_value_box(value) {
                unsafe { drop(Box::from_raw(value)); }
            }
        });
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_queue_init(limit: usize) {
    RELEASE_QUEUE.with(|queue| {
        *queue.borrow_mut() = Some(rc_deferred::ReleaseQueue::with_limit(limit.max(1024)));
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_queue_flush() {
    RELEASE_QUEUE.with(|queue| {
        if let Some(q) = &mut *queue.borrow_mut() {
            q.drain(|ptr| unsafe {
                let value = ptr.as_ptr() as ValueHandle;
                let value_ref = &mut *value;
                drop_heap_value(value_ref);
                LIVE_VALUE_COUNT.fetch_sub(1, Ordering::Relaxed);
                if !recycle_value_box(value) {
                    drop(Box::from_raw(value));
                }
            });
        }
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_heap_alloc(size: usize) -> *mut c_void {
    unsafe { malloc(size) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn coral_heap_free(ptr: *mut c_void) {
    if !ptr.is_null() {
        unsafe { free(ptr); }
    }
}

struct StackFrame {
    buffer: Vec<u8>,
    cursor: usize,
}

thread_local! {
    static STACK_FRAMES: RefCell<Vec<StackFrame>> = RefCell::new(Vec::new());
}

fn align_up(value: usize, align: usize) -> usize {
    let align = align.max(1);
    (value + align - 1) & !(align - 1)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_frame_enter(pages: usize) {
    ensure_runtime_initialized();
    let page_count = pages.max(1);
    let size = page_count * PAGE_SIZE;
    STACK_PAGES_COMMITTED.fetch_add(page_count as u64, Ordering::Relaxed);
    STACK_BYTES_REQUESTED.fetch_add(size as u64, Ordering::Relaxed);
    record_heap_bytes(std::mem::size_of::<StackFrame>() + size);
    record_usage(UsageKind::StackAllocSuccess, size as u64);
    STACK_FRAMES.with(|frames| {
        frames
            .borrow_mut()
            .push(StackFrame { buffer: vec![0u8; size], cursor: 0 });
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_frame_leave() {
    STACK_FRAMES.with(|frames| {
        frames.borrow_mut().pop();
    });
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_stack_alloc(size: usize, align: usize) -> *mut c_void {
    if size == 0 {
        return ptr::null_mut();
    }
    STACK_FRAMES.with(|frames| {
        let mut frames = frames.borrow_mut();
        if let Some(frame) = frames.last_mut() {
            let cursor = align_up(frame.cursor, align.max(1));
            if cursor + size > frame.buffer.len() {
                record_usage(UsageKind::StackAllocFailure, size as u64);
                return ptr::null_mut();
            }
            let ptr = unsafe { frame.buffer.as_mut_ptr().add(cursor) };
            frame.cursor = cursor + size;
            record_usage(UsageKind::StackAllocSuccess, size as u64);
            ptr as *mut c_void
        } else {
            ptr::null_mut()
        }
    })
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_retain_count() -> u64 {
    RETAIN_COUNT.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_release_count() -> u64 {
    RELEASE_COUNT.load(Ordering::Relaxed)
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_live_values() -> u64 {
    LIVE_VALUE_COUNT.load(Ordering::Relaxed)
}

#[repr(C)]
pub struct CoralRuntimeStats {
    pub retains: u64,
    pub releases: u64,
    pub live_values: u64,
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_stats(out: *mut CoralRuntimeStats) {
    if out.is_null() {
        return;
    }
    let stats = CoralRuntimeStats {
        retains: RETAIN_COUNT.load(Ordering::Relaxed),
        releases: RELEASE_COUNT.load(Ordering::Relaxed),
        live_values: LIVE_VALUE_COUNT.load(Ordering::Relaxed),
    };
    unsafe {
        *out = stats;
    }
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

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_metrics(out: *mut CoralRuntimeMetrics) {
    if out.is_null() {
        return;
    }
    unsafe {
        *out = snapshot_runtime_metrics();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_value_metrics(value: ValueHandle, out: *mut CoralHandleMetrics) {
    if value.is_null() || out.is_null() {
        return;
    }
    let value_ref = unsafe { &*value };
    unsafe {
        *out = CoralHandleMetrics {
            refcount: value_ref.refcount.load(Ordering::Relaxed),
            retains: value_ref.retain_events as u64,
            releases: value_ref.release_events as u64,
        };
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn coral_runtime_metrics_dump(path: *const u8, len: usize) {
    if path.is_null() || len == 0 {
        return;
    }
    let bytes = read_bytes(path, len);
    if bytes.is_empty() {
        return;
    }
    if let Ok(text) = String::from_utf8(bytes) {
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            dump_metrics_to_path(Path::new(trimmed));
        }
    }
}

fn snapshot_runtime_metrics() -> CoralRuntimeMetrics {
    CoralRuntimeMetrics {
        retains: RETAIN_COUNT.load(Ordering::Relaxed),
        retain_saturated: RETAIN_SATURATED.load(Ordering::Relaxed),
        releases: RELEASE_COUNT.load(Ordering::Relaxed),
        release_underflow: RELEASE_UNDERFLOW.load(Ordering::Relaxed),
        live_values: LIVE_VALUE_COUNT.load(Ordering::Relaxed),
        value_pool_hits: VALUE_POOL_HITS.load(Ordering::Relaxed),
        value_pool_misses: VALUE_POOL_MISSES.load(Ordering::Relaxed),
        heap_bytes: HEAP_BYTES_ALLOCATED.load(Ordering::Relaxed),
        string_bytes: STRING_BYTES_ALLOCATED.load(Ordering::Relaxed),
        list_slots: LIST_SLOTS_ALLOCATED.load(Ordering::Relaxed),
        map_slots: MAP_SLOTS_ALLOCATED.load(Ordering::Relaxed),
        stack_pages: STACK_PAGES_COMMITTED.load(Ordering::Relaxed),
        stack_bytes: STACK_BYTES_REQUESTED.load(Ordering::Relaxed),
        timestamp_ns: metrics_timestamp_ns(),
    }
}

fn metrics_timestamp_ns() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn metrics_json(metrics: &CoralRuntimeMetrics) -> String {
    format!(
        "{{\n  \"timestamp_ns\": {},\n  \"retains\": {},\n  \"retain_saturated\": {},\n  \"releases\": {},\n  \"release_underflow\": {},\n  \"live_values\": {},\n  \"value_pool_hits\": {},\n  \"value_pool_misses\": {},\n  \"heap_bytes\": {},\n  \"string_bytes\": {},\n  \"list_slots\": {},\n  \"map_slots\": {},\n  \"stack_pages\": {},\n  \"stack_bytes\": {}\n}}\n",
        metrics.timestamp_ns,
        metrics.retains,
        metrics.retain_saturated,
        metrics.releases,
        metrics.release_underflow,
        metrics.live_values,
        metrics.value_pool_hits,
        metrics.value_pool_misses,
        metrics.heap_bytes,
        metrics.string_bytes,
        metrics.list_slots,
        metrics.map_slots,
        metrics.stack_pages,
        metrics.stack_bytes
    )
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
        let closure = coral_make_closure(Some(invoke), Some(release), Box::into_raw(env_box) as *mut c_void);
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

        let double = coral_make_closure(Some(double_invoke), None, ptr::null_mut());
        let even = coral_make_closure(Some(even_predicate), None, ptr::null_mut());
        let sum = coral_make_closure(Some(sum_invoke), None, ptr::null_mut());

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
}
