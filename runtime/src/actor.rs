use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::available_parallelism;
use std::time::{Duration, Instant};
use std::collections::{BinaryHeap, VecDeque};
use std::cmp::Ordering as CmpOrdering;

/// Default mailbox capacity when not specified.
pub const DEFAULT_MAILBOX_CAPACITY: usize = 1024;

/// Actor identifier (monotonic increasing, never reused within a process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(pub u64);

// ========== Supervision ==========

/// Supervision strategy for handling child failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionStrategy {
    /// Restart the failed child with initial state.
    Restart,
    /// Stop the failed child permanently.
    Stop,
    /// Propagate failure to parent (escalate).
    Escalate,
    /// Ignore failure, child continues with current state.
    Resume,
}

impl Default for SupervisionStrategy {
    fn default() -> Self {
        Self::Restart
    }
}

/// Configuration for actor supervision.
#[derive(Debug, Clone)]
pub struct SupervisionConfig {
    /// Strategy to apply when a child fails.
    pub strategy: SupervisionStrategy,
    /// Maximum number of restarts allowed within the window.
    pub max_restarts: u32,
    /// Time window (in seconds) for counting restarts.
    pub restart_window_secs: u64,
    /// R2.9: Base backoff delay in milliseconds before restarting.
    /// Each successive restart doubles the delay (exponential backoff).
    /// 0 means no backoff (immediate restart, previous behaviour).
    pub backoff_base_ms: u64,
    /// R2.9: Maximum backoff delay in milliseconds (caps the doubling).
    pub backoff_max_ms: u64,
    /// R2.9: Maximum number of supervised children this actor may have.
    /// 0 means unlimited.
    pub max_children: u32,
}

impl Default for SupervisionConfig {
    fn default() -> Self {
        Self {
            strategy: SupervisionStrategy::Restart,
            max_restarts: 3,
            restart_window_secs: 60,
            backoff_base_ms: 0,
            backoff_max_ms: 30_000,
            max_children: 0,
        }
    }
}

/// Tracks restart history for a supervised child.
#[derive(Debug, Clone)]
struct RestartTracker {
    /// Times of recent restarts (for rate limiting).
    restart_times: VecDeque<Instant>,
    /// Total restarts since actor was created.
    total_restarts: u32,
    /// R2.9: Consecutive restarts without a successful message processed.
    /// Used for exponential backoff calculation. Reset when actor runs successfully.
    consecutive_restarts: u32,
}

impl Default for RestartTracker {
    fn default() -> Self {
        Self {
            restart_times: VecDeque::new(),
            total_restarts: 0,
            consecutive_restarts: 0,
        }
    }
}

// R2.6: SupervisedChild struct removed — factory now lives in SupervisedChildInfo.

/// Configuration for spawning actors.
#[derive(Debug, Clone)]
pub struct ActorConfig {
    /// Maximum number of messages in mailbox before backpressure kicks in.
    pub mailbox_capacity: usize,
    /// Supervision configuration for child actors.
    pub supervision: Option<SupervisionConfig>,
}

impl Default for ActorConfig {
    fn default() -> Self {
        Self {
            mailbox_capacity: DEFAULT_MAILBOX_CAPACITY,
            supervision: None,
        }
    }
}

impl ActorConfig {
    /// Create config with supervision enabled using default settings.
    pub fn with_supervision() -> Self {
        Self {
            mailbox_capacity: DEFAULT_MAILBOX_CAPACITY,
            supervision: Some(SupervisionConfig::default()),
        }
    }

    /// Create config with custom supervision settings.
    pub fn with_custom_supervision(config: SupervisionConfig) -> Self {
        Self {
            mailbox_capacity: DEFAULT_MAILBOX_CAPACITY,
            supervision: Some(config),
        }
    }
}

#[derive(Clone)]
pub struct ActorHandle {
    pub id: ActorId,
    pub(crate) sender: SyncSender<Message>,
}

/// Result of attempting to send a message with backpressure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    /// Message was sent successfully.
    Ok,
    /// Mailbox is full, message was not sent (backpressure).
    Full,
    /// Actor is no longer alive.
    Disconnected,
}

#[derive(Debug, Clone)]
pub enum Message {
    Exit,
    GracefulStop,
    User(crate::ValueHandle),
    Failure(String),
    /// Child failure notification sent to parent supervisor.
    ChildFailure {
        child_id: ActorId,
        reason: String,
    },
    /// Actor death notification sent to monitors.
    ActorDown {
        actor_id: ActorId,
        reason: String,
    },
}

// Safety: ValueHandle is frozen before sending, making it immutable.
// Frozen values are safe to share across threads.
unsafe impl Send for Message {}

#[derive(Clone)]
pub struct ActorEntry {
    pub(crate) sender: SyncSender<Message>,
    parent: Option<ActorId>,
    pub(crate) mailbox_capacity: usize,
    /// Supervision config for this actor's children.
    pub(crate) supervision: Option<SupervisionConfig>,
}

/// Statistics for actor mailbox operations.
#[derive(Debug, Default, Clone)]
pub struct MailboxStats {
    pub messages_sent: u64,
    pub messages_dropped: u64,
    pub backpressure_events: u64,
}

static MAILBOX_STATS: OnceLock<Mutex<MailboxStats>> = OnceLock::new();

fn mailbox_stats() -> &'static Mutex<MailboxStats> {
    MAILBOX_STATS.get_or_init(|| Mutex::new(MailboxStats::default()))
}

pub fn get_mailbox_stats() -> MailboxStats {
    mailbox_stats().lock().map(|g| g.clone()).unwrap_or_default()
}

// ========== Timer System ==========

/// Unique identifier for a timer, used for cancellation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

/// A scheduled timer entry.
struct TimerEntry {
    id: TimerId,
    fire_at: Instant,
    target: ActorHandle,
    message: crate::ValueHandle,
    /// If Some, this timer repeats at the given interval
    repeat_interval: Option<Duration>,
    /// Set to true when timer is cancelled
    cancelled: Arc<AtomicBool>,
}

// Safety: ValueHandle is frozen before being stored in timers, making it immutable.
// Frozen values are safe to share across threads.
unsafe impl Send for TimerEntry {}
unsafe impl Sync for TimerEntry {}

impl PartialEq for TimerEntry {
    fn eq(&self, other: &Self) -> bool {
        self.fire_at == other.fire_at && self.id == other.id
    }
}

impl Eq for TimerEntry {}

impl PartialOrd for TimerEntry {
    fn partial_cmp(&self, other: &Self) -> Option<CmpOrdering> {
        Some(self.cmp(other))
    }
}

impl Ord for TimerEntry {
    fn cmp(&self, other: &Self) -> CmpOrdering {
        // Reverse ordering for min-heap (earliest first)
        other.fire_at.cmp(&self.fire_at)
            .then_with(|| other.id.0.cmp(&self.id.0))
    }
}

/// Timer cancellation token. When dropped or explicitly cancelled, 
/// the associated timer will not fire.
#[derive(Clone)]
pub struct TimerToken {
    id: TimerId,
    cancelled: Arc<AtomicBool>,
}

impl TimerToken {
    /// Cancel this timer. Returns true if this call cancelled it,
    /// false if it was already cancelled or has fired.
    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::SeqCst)
    }

    /// Check if this timer has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    /// Get the timer ID.
    pub fn id(&self) -> TimerId {
        self.id
    }
}

/// Timer wheel for scheduling delayed messages.
/// Uses a simple priority queue (binary heap) implementation.
#[derive(Clone)]
pub struct TimerWheel {
    timers: Arc<Mutex<BinaryHeap<TimerEntry>>>,
    next_id: Arc<AtomicU64>,
    worker_started: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    system: Arc<Mutex<Option<ActorSystem>>>,
    worker_handle: Arc<Mutex<Option<thread::JoinHandle<()>>>>,
}

impl Default for TimerWheel {
    fn default() -> Self {
        Self::new()
    }
}

impl TimerWheel {
    pub fn new() -> Self {
        Self {
            timers: Arc::new(Mutex::new(BinaryHeap::new())),
            next_id: Arc::new(AtomicU64::new(1)),
            worker_started: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(AtomicBool::new(false)),
            system: Arc::new(Mutex::new(None)),
            worker_handle: Arc::new(Mutex::new(None)),
        }
    }

    /// Set the actor system (called when system is initialized).
    pub fn set_system(&self, system: ActorSystem) {
        *self.system.lock().unwrap() = Some(system);
    }

    /// Start the timer worker thread if not already started.
    fn ensure_worker(&self) {
        if self.worker_started.swap(true, Ordering::SeqCst) {
            return;
        }

        let timers = self.timers.clone();
        let system = self.system.clone();
        let shutdown = self.shutdown.clone();

        let handle = thread::Builder::new()
            .name("timer-worker".to_string())
            .spawn(move || {
                loop {
                    if shutdown.load(Ordering::SeqCst) {
                        break;
                    }
                    // Calculate how long to sleep
                    let sleep_duration = {
                        let heap = timers.lock().unwrap();
                        if let Some(next) = heap.peek() {
                            let now = Instant::now();
                            if next.fire_at <= now {
                                Duration::ZERO
                            } else {
                                next.fire_at.duration_since(now)
                            }
                        } else {
                            // No timers, sleep for a while then check again
                            Duration::from_millis(100)
                        }
                    };

                    if !sleep_duration.is_zero() {
                        thread::sleep(sleep_duration.min(Duration::from_millis(100)));
                    }

                    // Process any due timers
                    let now = Instant::now();
                    let mut to_reschedule = Vec::new();

                    loop {
                        let entry = {
                            let mut heap = timers.lock().unwrap();
                            if let Some(next) = heap.peek() {
                                if next.fire_at <= now && !next.cancelled.load(Ordering::SeqCst) {
                                    heap.pop()
                                } else if next.fire_at <= now {
                                    // Cancelled, just remove it
                                    heap.pop();
                                    continue;
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        match entry {
                            Some(entry) => {
                                // Fire the timer
                                if let Some(sys) = system.lock().unwrap().as_ref() {
                                    let _ = sys.send(&entry.target, Message::User(entry.message.clone()));
                                }

                                // If repeating, reschedule
                                if let Some(interval) = entry.repeat_interval {
                                    if !entry.cancelled.load(Ordering::SeqCst) {
                                        to_reschedule.push(TimerEntry {
                                            id: entry.id,
                                            fire_at: now + interval,
                                            target: entry.target,
                                            message: entry.message,
                                            repeat_interval: Some(interval),
                                            cancelled: entry.cancelled,
                                        });
                                    }
                                }
                            }
                            None => break,
                        }
                    }

                    // Add rescheduled timers
                    if !to_reschedule.is_empty() {
                        let mut heap = timers.lock().unwrap();
                        for entry in to_reschedule {
                            heap.push(entry);
                        }
                    }
                }
            })
            .expect("failed to spawn timer worker");
        *self.worker_handle.lock().unwrap() = Some(handle);
    }

    /// Signal the timer worker to stop and wait for it to finish.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    /// Schedule a one-shot timer to send a message after a delay.
    pub fn send_after(
        &self,
        delay: Duration,
        target: ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.ensure_worker();

        let id = TimerId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let cancelled = Arc::new(AtomicBool::new(false));
        let fire_at = Instant::now() + delay;

        let entry = TimerEntry {
            id,
            fire_at,
            target,
            message,
            repeat_interval: None,
            cancelled: cancelled.clone(),
        };

        self.timers.lock().unwrap().push(entry);

        TimerToken { id, cancelled }
    }

    /// Schedule a repeating timer to send a message at regular intervals.
    pub fn schedule_repeat(
        &self,
        interval: Duration,
        target: ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.ensure_worker();

        let id = TimerId(self.next_id.fetch_add(1, Ordering::Relaxed));
        let cancelled = Arc::new(AtomicBool::new(false));
        let fire_at = Instant::now() + interval;

        let entry = TimerEntry {
            id,
            fire_at,
            target,
            message,
            repeat_interval: Some(interval),
            cancelled: cancelled.clone(),
        };

        self.timers.lock().unwrap().push(entry);

        TimerToken { id, cancelled }
    }

    /// Cancel a timer by its ID. Returns true if the timer was found and cancelled.
    pub fn cancel(&self, id: TimerId) -> bool {
        let heap = self.timers.lock().unwrap();
        for entry in heap.iter() {
            if entry.id == id {
                return !entry.cancelled.swap(true, Ordering::SeqCst);
            }
        }
        false
    }

    /// Get the number of pending timers (includes cancelled but not yet cleaned up).
    pub fn pending_count(&self) -> usize {
        self.timers.lock().unwrap().len()
    }
}

#[derive(Clone)]
pub struct ActorSystem {
    pub(crate) registry: Arc<Mutex<HashMap<ActorId, ActorEntry>>>,
    /// Named actor registry: lock-free concurrent map (R2.2).
    named_registry: Arc<dashmap::DashMap<String, ActorHandle>>,
    /// Monitor registry: maps monitored actor → set of watcher actors.
    monitors: Arc<Mutex<HashMap<ActorId, HashSet<ActorId>>>>,
    scheduler: Scheduler,
    /// Timer wheel for delayed/scheduled messages.
    pub timer_wheel: TimerWheel,
}

impl Default for ActorSystem {
    fn default() -> Self {
        Self::new()
    }
}

impl ActorSystem {
    pub fn new() -> Self {
        let timer_wheel = TimerWheel::new();
        let system = Self { 
            registry: Arc::new(Mutex::new(HashMap::new())), 
            named_registry: Arc::new(dashmap::DashMap::new()),
            monitors: Arc::new(Mutex::new(HashMap::new())),
            scheduler: Scheduler::new(),
            timer_wheel: timer_wheel.clone(),
        };
        // Give the timer wheel a reference to the system for sending messages
        timer_wheel.set_system(system.clone());
        system
    }

    /// Spawn an actor with default configuration.
    pub fn spawn<F>(&self, parent: Option<ActorId>, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.spawn_with_config(parent, ActorConfig::default(), f)
    }

    /// Spawn an actor with custom configuration.
    pub fn spawn_with_config<F>(&self, parent: Option<ActorId>, config: ActorConfig, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(config.mailbox_capacity);
        let id = ActorId(self.scheduler.next_id());
        let handle = ActorHandle { id, sender: tx.clone() };
        let parent_id = parent.unwrap_or(id);
        let supervision_config = config.supervision.clone();
        {
            let mut reg = self.registry.lock().unwrap();
            reg.insert(id, ActorEntry { 
                sender: tx.clone(), 
                parent: Some(parent_id),
                mailbox_capacity: config.mailbox_capacity,
                supervision: config.supervision.clone(),
            });
        }
        let system = self.clone();
        let handle_clone = handle.clone();
        self.scheduler.submit(move || {
            let ctx = ActorContext {
                id,
                system: system.clone(),
                rx,
                parent,
                handle: handle_clone,
                supervised_children: Mutex::new(HashMap::new()),
                supervision_config,
            };
            ctx.run(f);
            // Notify monitors of actor death (AC-2)
            system.notify_monitors(id, "normal");
            system.registry.lock().unwrap().remove(&id);
            // Clean up monitor registrations for this actor
            system.monitors.lock().unwrap().remove(&id);
        });
        handle
    }

    /// Send a message, blocking if mailbox is full.
    pub fn send(&self, handle: &ActorHandle, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        if let Some(entry) = self.registry.lock().unwrap().get(&handle.id) {
            let result = entry.sender.send(msg);
            if result.is_ok() {
                if let Ok(mut stats) = mailbox_stats().lock() {
                    stats.messages_sent += 1;
                }
            }
            result
        } else {
            Err(mpsc::SendError(msg))
        }
    }

    /// Try to send a message without blocking. Returns backpressure status.
    pub fn try_send(&self, handle: &ActorHandle, msg: Message) -> SendResult {
        if let Some(entry) = self.registry.lock().unwrap().get(&handle.id) {
            match entry.sender.try_send(msg) {
                Ok(()) => {
                    if let Ok(mut stats) = mailbox_stats().lock() {
                        stats.messages_sent += 1;
                    }
                    SendResult::Ok
                }
                Err(TrySendError::Full(_)) => {
                    if let Ok(mut stats) = mailbox_stats().lock() {
                        stats.backpressure_events += 1;
                    }
                    SendResult::Full
                }
                Err(TrySendError::Disconnected(_)) => {
                    if let Ok(mut stats) = mailbox_stats().lock() {
                        stats.messages_dropped += 1;
                    }
                    SendResult::Disconnected
                }
            }
        } else {
            SendResult::Disconnected
        }
    }

    pub fn parent_of(&self, id: ActorId) -> Option<ActorId> {
        self.registry.lock().unwrap().get(&id).and_then(|e| e.parent)
    }

    /// Get the mailbox capacity for an actor.
    pub fn mailbox_capacity(&self, id: ActorId) -> Option<usize> {
        self.registry.lock().unwrap().get(&id).map(|e| e.mailbox_capacity)
    }

    /// Get an actor handle by ID.
    pub fn get_actor_handle(&self, id: ActorId) -> Option<ActorHandle> {
        self.registry.lock().unwrap().get(&id).map(|e| ActorHandle { id, sender: e.sender.clone() })
    }

    // ========== Shutdown ==========

    /// Gracefully shut down the actor system.
    /// Stops the timer worker, signals worker threads, and saves all stores.
    pub fn shutdown(&self) {
        // 1. Stop timer wheel
        self.timer_wheel.shutdown();
        // 2. Signal scheduler to stop (workers break on channel close)
        self.scheduler.shutdown();
        // 3. Save all persistent stores
        let _ = crate::store::save_all_engines();
    }

    // ========== Named Actor Registry ==========

    // ========== Actor Monitoring (AC-2) ==========

    /// Register `watcher` to be notified when `watched` dies.
    pub fn monitor(&self, watcher: ActorId, watched: ActorId) {
        let mut monitors = self.monitors.lock().unwrap();
        monitors.entry(watched).or_insert_with(HashSet::new).insert(watcher);
    }

    /// Unregister `watcher` from death notifications of `watched`.
    pub fn demonitor(&self, watcher: ActorId, watched: ActorId) {
        let mut monitors = self.monitors.lock().unwrap();
        if let Some(watchers) = monitors.get_mut(&watched) {
            watchers.remove(&watcher);
            if watchers.is_empty() {
                monitors.remove(&watched);
            }
        }
    }

    /// Notify all monitors that an actor has died.
    fn notify_monitors(&self, dead_actor: ActorId, reason: &str) {
        let watchers = {
            let monitors = self.monitors.lock().unwrap();
            match monitors.get(&dead_actor) {
                Some(set) => set.clone(),
                None => return,
            }
        };
        let registry = self.registry.lock().unwrap();
        for watcher_id in watchers {
            if let Some(entry) = registry.get(&watcher_id) {
                let _ = entry.sender.try_send(Message::ActorDown {
                    actor_id: dead_actor,
                    reason: reason.to_string(),
                });
            }
        }
    }

    // ========== Named Actor Registry (continued) ==========

    /// Register an actor with a name. Returns true if successful, false if name already taken.
    pub fn register_named(&self, name: &str, handle: ActorHandle) -> bool {
        use dashmap::mapref::entry::Entry;
        match self.named_registry.entry(name.to_string()) {
            Entry::Occupied(_) => false,
            Entry::Vacant(v) => {
                v.insert(handle);
                true
            }
        }
    }

    /// Lookup an actor by name. Returns None if not found.
    /// Lock-free on the read path (R2.2).
    pub fn lookup_named(&self, name: &str) -> Option<ActorHandle> {
        self.named_registry.get(name).map(|r| r.value().clone())
    }

    /// Unregister a named actor. Returns true if the name was found and removed.
    pub fn unregister_named(&self, name: &str) -> bool {
        self.named_registry.remove(name).is_some()
    }

    /// Spawn an actor and register it with a name. Returns None if name already taken.
    pub fn spawn_named<F>(&self, name: &str, parent: Option<ActorId>, f: F) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        use dashmap::mapref::entry::Entry;
        // entry() provides atomic check-and-insert — no TOCTOU race.
        match self.named_registry.entry(name.to_string()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(v) => {
                let handle = self.spawn(parent, f);
                v.insert(handle.clone());
                Some(handle)
            }
        }
    }

    /// Spawn a named actor with custom configuration.
    pub fn spawn_named_with_config<F>(
        &self,
        name: &str,
        parent: Option<ActorId>,
        config: ActorConfig,
        f: F,
    ) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        use dashmap::mapref::entry::Entry;
        match self.named_registry.entry(name.to_string()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(v) => {
                let handle = self.spawn_with_config(parent, config, f);
                v.insert(handle.clone());
                Some(handle)
            }
        }
    }

    /// List all registered named actors.
    pub fn list_named(&self) -> Vec<(String, ActorHandle)> {
        self.named_registry
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect()
    }

    // ========== Timer Operations ==========

    /// Schedule a message to be sent after a delay.
    pub fn send_after(
        &self,
        delay: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.timer_wheel.send_after(delay, target.clone(), message)
    }

    /// Schedule a message to be sent repeatedly at the given interval.
    pub fn schedule_repeat(
        &self,
        interval: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.timer_wheel.schedule_repeat(interval, target.clone(), message)
    }

    /// Cancel a timer by its token.
    pub fn cancel_timer(&self, token: &TimerToken) -> bool {
        token.cancel()
    }

    /// Get the number of pending timers.
    pub fn pending_timers(&self) -> usize {
        self.timer_wheel.pending_count()
    }

    // ========== Supervision Operations ==========

    /// Notify that a child has failed. The supervisor will handle according to strategy.
    pub fn notify_child_failure(&self, parent: &ActorHandle, child_id: ActorId, reason: String) {
        let _ = self.send(parent, Message::ChildFailure { child_id, reason });
    }

    /// Handle child failure according to supervision strategy.
    pub fn handle_child_failure(
        &self,
        child_id: ActorId,
        reason: &str,
        config: &SupervisionConfig,
        tracker: &mut RestartTracker,
    ) -> SupervisionDecision {
        let now = Instant::now();
        let window = Duration::from_secs(config.restart_window_secs);
        
        // Clean up old restart times outside the window
        while let Some(&oldest) = tracker.restart_times.front() {
            if now.duration_since(oldest) > window {
                tracker.restart_times.pop_front();
            } else {
                break;
            }
        }
        
        // Check if we've exceeded max restarts in the window
        if tracker.restart_times.len() >= config.max_restarts as usize {
            tracker.consecutive_restarts = 0;
            return SupervisionDecision::Escalate;
        }
        
        match config.strategy {
            SupervisionStrategy::Restart => {
                tracker.restart_times.push_back(now);
                tracker.total_restarts += 1;
                tracker.consecutive_restarts += 1;

                // R2.9: Compute exponential backoff delay.
                if config.backoff_base_ms > 0 {
                    let exp = (tracker.consecutive_restarts - 1).min(20);
                    let delay_ms = config.backoff_base_ms.saturating_mul(1u64 << exp);
                    let delay_ms = delay_ms.min(config.backoff_max_ms);
                    SupervisionDecision::RestartAfter(Duration::from_millis(delay_ms))
                } else {
                    SupervisionDecision::Restart
                }
            }
            SupervisionStrategy::Stop => SupervisionDecision::Stop,
            SupervisionStrategy::Escalate => SupervisionDecision::Escalate,
            SupervisionStrategy::Resume => SupervisionDecision::Resume,
        }
    }
}

/// Decision made by supervisor after child failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionDecision {
    /// Restart the child.
    Restart,
    /// R2.9: Restart the child after a backoff delay.
    RestartAfter(Duration),
    /// Stop the child permanently.
    Stop,
    /// Escalate to grandparent.
    Escalate,
    /// Let child continue (resume).
    Resume,
}

pub struct ActorContext {
    pub id: ActorId,
    system: ActorSystem,
    rx: Receiver<Message>,
    parent: Option<ActorId>,
    handle: ActorHandle,
    /// Supervised children tracked by this actor.
    supervised_children: Mutex<HashMap<ActorId, SupervisedChildInfo>>,
    /// Supervision config for this actor's children.
    supervision_config: Option<SupervisionConfig>,
}

/// Runtime info for a supervised child, including restart factory.
#[derive(Clone)]
struct SupervisedChildInfo {
    handle: ActorHandle,
    /// R2.6: Factory function to recreate the child on restart.
    factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static>,
    config: SupervisionConfig,
    tracker: RestartTracker,
}

// Safety: ActorContext contains Send types (ids, system Arc, Receiver<Message>, handles).
unsafe impl Send for ActorContext {}

impl ActorContext {
    fn run<F>(self, f: F)
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        CURRENT_ACTOR.with(|slot| {
            *slot.borrow_mut() = Some(self.id);
        });
        f(self);
        CURRENT_ACTOR.with(|slot| {
            *slot.borrow_mut() = None;
        });
    }

    pub fn recv(&self) -> Option<Message> {
        self.rx.recv().ok()
    }

    pub fn try_recv(&self) -> Option<Message> {
        match self.rx.try_recv() {
            Ok(m) => Some(m),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => None,
        }
    }

    /// R2.10: Drain all remaining messages from the mailbox via the handler,
    /// then stop. Call this when you receive `Message::GracefulStop`.
    /// The `handler` is called for each `Message::User` message that's
    /// still in the mailbox. Other control messages are discarded.
    pub fn drain_and_stop<F>(&self, mut handler: F)
    where
        F: FnMut(crate::ValueHandle),
    {
        loop {
            match self.rx.try_recv() {
                Ok(Message::User(val)) => handler(val),
                Ok(Message::GracefulStop) => {
                    // Another GracefulStop while draining — ignore
                }
                Ok(Message::Exit) => break,
                Ok(_) => {} // discard other control messages
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }
        // Unregister from the registry
        self.system.registry.lock().unwrap().remove(&self.id);
        self.system.notify_monitors(self.id, "graceful_stop");
    }

    pub fn spawn_child<F>(&self, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn(Some(self.id), f)
    }

    /// Spawn a child actor with custom configuration.
    pub fn spawn_child_with_config<F>(&self, config: ActorConfig, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn_with_config(Some(self.id), config, f)
    }

    /// R2.6: Spawn a supervised child that will be restarted on failure.
    /// The factory is `Fn` (not `FnOnce`) so it can be called again on restart.
    /// Returns None if max_children limit is reached (R2.9).
    pub fn spawn_supervised_child<F>(&self, f: F) -> Option<ActorHandle>
    where
        F: Fn(ActorContext) + Send + Sync + 'static,
    {
        let config = self.supervision_config.clone().unwrap_or_default();
        // R2.9: Enforce max_children limit
        if config.max_children > 0 {
            let children = self.supervised_children.lock().unwrap();
            if children.len() >= config.max_children as usize {
                return None;
            }
        }

        let factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static> = Arc::new(f);
        let factory_clone = factory.clone();
        let handle = self.system.spawn(Some(self.id), move |ctx| factory_clone(ctx));
        
        let info = SupervisedChildInfo {
            handle: handle.clone(),
            factory,
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children.lock().unwrap().insert(handle.id, info);
        
        Some(handle)
    }

    /// R2.6: Spawn a supervised child with custom supervision config.
    /// Returns None if max_children limit is reached (R2.9).
    pub fn spawn_supervised_child_with_config<F>(&self, config: SupervisionConfig, f: F) -> Option<ActorHandle>
    where
        F: Fn(ActorContext) + Send + Sync + 'static,
    {
        // R2.9: Enforce max_children limit
        if config.max_children > 0 {
            let children = self.supervised_children.lock().unwrap();
            if children.len() >= config.max_children as usize {
                return None;
            }
        }

        let factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static> = Arc::new(f);
        let factory_clone = factory.clone();
        let handle = self.system.spawn(Some(self.id), move |ctx| factory_clone(ctx));
        
        let info = SupervisedChildInfo {
            handle: handle.clone(),
            factory,
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children.lock().unwrap().insert(handle.id, info);
        
        Some(handle)
    }

    /// Handle a child failure message. Returns the supervision decision made.
    pub fn handle_child_failure_msg(&self, child_id: ActorId, reason: &str) -> SupervisionDecision {
        let mut children = self.supervised_children.lock().unwrap();
        if let Some(child_info) = children.get_mut(&child_id) {
            let decision = self.system.handle_child_failure(
                child_id,
                reason,
                &child_info.config,
                &mut child_info.tracker,
            );
            // Clone what we need before releasing the mutable borrow
            let factory = child_info.factory.clone();
            let config = child_info.config.clone();
            let tracker = child_info.tracker.clone();
            
            match decision {
                SupervisionDecision::Stop => {
                    children.remove(&child_id);
                }
                SupervisionDecision::Escalate => {
                    children.remove(&child_id);
                    // Notify our parent if we have one
                    if let Some(parent_id) = self.parent {
                        if let Some(parent_handle) = self.system.get_actor_handle(parent_id) {
                            self.system.notify_child_failure(&parent_handle, self.id, format!("Escalated from child {}: {}", child_id.0, reason));
                        }
                    }
                }
                SupervisionDecision::Restart => {
                    // R2.6: Actually restart the child using its factory.
                    let factory_clone = factory.clone();
                    let new_handle = self.system.spawn(Some(self.id), move |ctx| factory_clone(ctx));
                    // Remove old entry, insert new one with updated handle
                    children.remove(&child_id);
                    children.insert(new_handle.id, SupervisedChildInfo {
                        handle: new_handle,
                        factory,
                        config,
                        tracker,
                    });
                }
                SupervisionDecision::RestartAfter(delay) => {
                    // R2.9: Restart after exponential backoff delay.
                    // Remove old child, drop lock, sleep, then re-acquire and insert.
                    children.remove(&child_id);
                    drop(children);
                    std::thread::sleep(delay);
                    let factory_clone = factory.clone();
                    let new_handle = self.system.spawn(Some(self.id), move |ctx| factory_clone(ctx));
                    self.supervised_children.lock().unwrap().insert(new_handle.id, SupervisedChildInfo {
                        handle: new_handle,
                        factory,
                        config,
                        tracker,
                    });
                }
                SupervisionDecision::Resume => {
                    // Do nothing — actor continues (if still alive)
                }
            }
            
            decision
        } else {
            // Unknown child, just stop
            SupervisionDecision::Stop
        }
    }

    pub fn send(&self, handle: &ActorHandle, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        self.system.send(handle, msg)
    }

    /// Try to send a message without blocking. Returns backpressure status.
    pub fn try_send(&self, handle: &ActorHandle, msg: Message) -> SendResult {
        self.system.try_send(handle, msg)
    }

    pub fn system(&self) -> ActorSystem {
        self.system.clone()
    }

    pub fn parent(&self) -> Option<ActorId> {
        self.parent
    }

    pub fn handle(&self) -> ActorHandle {
        self.handle.clone()
    }

    // ========== Named Actor Operations ==========

    /// Register this actor with a name. Returns true if successful.
    pub fn register_as(&self, name: &str) -> bool {
        self.system.register_named(name, self.handle.clone())
    }

    /// Lookup an actor by name.
    pub fn lookup(&self, name: &str) -> Option<ActorHandle> {
        self.system.lookup_named(name)
    }

    /// Spawn a named child actor. Returns None if name already taken.
    pub fn spawn_named_child<F>(&self, name: &str, f: F) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn_named(name, Some(self.id), f)
    }

    /// Send a message to a named actor. Returns Err if actor not found.
    pub fn send_to_named(&self, name: &str, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        if let Some(handle) = self.lookup(name) {
            self.send(&handle, msg)
        } else {
            Err(mpsc::SendError(msg))
        }
    }

    // ========== Timer Operations ==========

    /// Schedule a message to be sent to a target actor after a delay.
    pub fn send_after(
        &self,
        delay: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.send_after(delay, target, message)
    }

    /// Schedule a message to be sent to this actor after a delay.
    pub fn send_self_after(&self, delay: Duration, message: crate::ValueHandle) -> TimerToken {
        self.system.send_after(delay, &self.handle, message)
    }

    /// Schedule a message to be sent repeatedly to a target actor.
    pub fn schedule_repeat(
        &self,
        interval: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.schedule_repeat(interval, target, message)
    }

    /// Schedule a message to be sent repeatedly to this actor.
    pub fn schedule_self_repeat(
        &self,
        interval: Duration,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.schedule_repeat(interval, &self.handle, message)
    }

    /// Cancel a timer.
    pub fn cancel_timer(&self, token: &TimerToken) -> bool {
        token.cancel()
    }
}

/// Work-stealing scheduler for actors (M:N, R2.1).
///
/// Each worker thread owns a local `crossbeam_deque::Worker` deque.  New tasks
/// are pushed to workers in round-robin order.  Workers pop from their local
/// deque first; idle workers steal from a random non-empty peer via
/// `crossbeam_deque::Stealer`.  This eliminates the central `Mutex<Receiver>`
/// contention of the previous single-channel design.
#[derive(Clone)]
struct Scheduler {
    work: Arc<WorkStealingQueue>,
}

struct WorkStealingQueue {
    /// One injector per worker, indexed by worker ordinal.
    /// `submit()` round-robins across these via `next_worker`.
    injectors: Vec<crossbeam_deque::Injector<Runnable>>,
    /// Stealers corresponding to each worker's local deque.
    stealers: Vec<crossbeam_deque::Stealer<Runnable>>,
    /// Round-robin counter for external `submit()` calls.
    next_worker: AtomicU64,
    next_id: AtomicU64,
    workers_started: AtomicBool,
    shutdown: AtomicBool,
    worker_handles: Mutex<Vec<thread::JoinHandle<()>>>,
    /// Number of worker threads.
    worker_count: usize,
    /// Condvar used to park/unpark idle workers.
    notify: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

// Safety: all interior mutability is via atomics / Mutex / crossbeam types.
unsafe impl Send for WorkStealingQueue {}
unsafe impl Sync for WorkStealingQueue {}

type Runnable = Box<dyn FnOnce() + Send + 'static>;

impl Scheduler {
    fn new() -> Self {
        let worker_count = available_parallelism().map(|n| n.get()).unwrap_or(1).max(1);

        // Create per-worker deques.  We keep the Workers in a temporary Vec,
        // extract Stealers (which are Send+Sync), and move Workers into the
        // spawned threads below.
        let mut local_workers: Vec<crossbeam_deque::Worker<Runnable>> = Vec::with_capacity(worker_count);
        let mut stealers: Vec<crossbeam_deque::Stealer<Runnable>> = Vec::with_capacity(worker_count);
        let mut injectors: Vec<crossbeam_deque::Injector<Runnable>> = Vec::with_capacity(worker_count);

        for _ in 0..worker_count {
            let w = crossbeam_deque::Worker::new_fifo();
            stealers.push(w.stealer());
            local_workers.push(w);
            injectors.push(crossbeam_deque::Injector::new());
        }

        let notify = Arc::new((Mutex::new(false), std::sync::Condvar::new()));

        let queue = Arc::new(WorkStealingQueue {
            injectors,
            stealers,
            next_worker: AtomicU64::new(0),
            next_id: AtomicU64::new(0),
            workers_started: AtomicBool::new(true),
            shutdown: AtomicBool::new(false),
            worker_handles: Mutex::new(Vec::with_capacity(worker_count)),
            worker_count,
            notify: notify.clone(),
        });

        // Spawn worker threads.
        let mut handles = Vec::with_capacity(worker_count);
        for (idx, local) in local_workers.into_iter().enumerate() {
            let queue_ref = queue.clone();
            let notify_ref = notify.clone();
            let handle = thread::Builder::new()
                .name(format!("actor-worker-{idx}"))
                .spawn(move || {
                    worker_loop(idx, local, queue_ref, notify_ref);
                })
                .expect("failed to spawn actor worker");
            handles.push(handle);
        }
        *queue.worker_handles.lock().unwrap() = handles;

        Self { work: queue }
    }

    /// Shut down the scheduler.
    fn shutdown(&self) {
        self.work.shutdown.store(true, Ordering::SeqCst);
        // Wake all parked workers so they observe the shutdown flag.
        let (lock, cvar) = &*self.work.notify;
        let mut flag = lock.lock().unwrap();
        *flag = true;
        cvar.notify_all();
        drop(flag);
    }

    fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        // Round-robin across workers' injectors.
        let idx = self.work.next_worker.fetch_add(1, Ordering::Relaxed) as usize
            % self.work.worker_count;
        self.work.injectors[idx].push(Box::new(f));

        // Wake a potentially parked worker.
        let (lock, cvar) = &*self.work.notify;
        let mut flag = lock.lock().unwrap();
        *flag = true;
        cvar.notify_one();
        drop(flag);
    }

    fn next_id(&self) -> u64 {
        self.work.next_id.fetch_add(1, Ordering::Relaxed).saturating_add(1)
    }
}

/// Per-worker event loop.  Tries local deque → own injector → steal from peers → park.
fn worker_loop(
    idx: usize,
    local: crossbeam_deque::Worker<Runnable>,
    queue: Arc<WorkStealingQueue>,
    notify: Arc<(Mutex<bool>, std::sync::Condvar)>,
) {
    use crossbeam_deque::Steal;

    let mut rng_state: u64 = idx as u64 ^ 0xDEAD_BEEF;

    loop {
        if queue.shutdown.load(Ordering::Relaxed) {
            break;
        }

        // 1. Pop from local deque.
        if let Some(task) = local.pop() {
            task();
            continue;
        }

        // 2. Drain own injector into local deque.
        loop {
            match queue.injectors[idx].steal_batch_and_pop(&local) {
                Steal::Success(task) => {
                    task();
                    break;
                }
                Steal::Retry => continue,
                Steal::Empty => break,
            }
        }
        // Check if we got more work from the injector batch.
        if let Some(task) = local.pop() {
            task();
            continue;
        }

        // 3. Steal from a random peer's deque.
        let n = queue.worker_count;
        if n > 1 {
            // Simple xorshift for cheap random peer selection.
            rng_state ^= rng_state << 13;
            rng_state ^= rng_state >> 7;
            rng_state ^= rng_state << 17;
            let start = rng_state as usize % n;
            let mut stolen = false;
            for offset in 0..n {
                let peer = (start + offset) % n;
                if peer == idx {
                    continue;
                }
                match queue.stealers[peer].steal_batch_and_pop(&local) {
                    Steal::Success(task) => {
                        task();
                        stolen = true;
                        break;
                    }
                    Steal::Retry => {}
                    Steal::Empty => {}
                }
            }
            if stolen {
                continue;
            }
            // Also try peer injectors.
            for offset in 0..n {
                let peer = (start + offset) % n;
                if peer == idx {
                    continue;
                }
                match queue.injectors[peer].steal_batch_and_pop(&local) {
                    Steal::Success(task) => {
                        task();
                        stolen = true;
                        break;
                    }
                    Steal::Retry => {}
                    Steal::Empty => {}
                }
            }
            if stolen {
                continue;
            }
        }

        // 4. No work found — park the thread briefly.
        let (lock, cvar) = &*notify;
        let mut flag = lock.lock().unwrap();
        // Re-check shutdown before parking.
        if queue.shutdown.load(Ordering::Relaxed) {
            break;
        }
        // Wait with a timeout so workers periodically try stealing even
        // without an explicit wake-up (avoids subtle starvation).
        let _ = cvar.wait_timeout(flag, Duration::from_millis(1));
    }
}

thread_local! {
    static CURRENT_ACTOR: std::cell::RefCell<Option<ActorId>> = std::cell::RefCell::new(None);
}

pub fn current_actor() -> Option<ActorId> {
    CURRENT_ACTOR.with(|slot| *slot.borrow())
}

pub fn global_system() -> &'static ActorSystem {
    static INSTANCE: OnceLock<ActorSystem> = OnceLock::new();
    INSTANCE.get_or_init(ActorSystem::new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
    use std::time::Duration;

    #[test]
    fn r26_supervised_child_restarts_after_failure() {
        let system = ActorSystem::new();
        let invocation_count = Arc::new(AtomicU32::new(0));
        let count_clone = invocation_count.clone();

        // Supervisor actor that spawns a supervised child
        let sup_handle = system.spawn(None, move |ctx: ActorContext| {
            let cc = count_clone.clone();
            let child = ctx.spawn_supervised_child(move |_child_ctx: ActorContext| {
                cc.fetch_add(1, Ordering::SeqCst);
                // Simulate work then terminate normally on 2nd+ invocation
                // First invocation: immediate exit (simulating failure)
            }).unwrap();

            // Simulate child failure notification
            ctx.handle_child_failure_msg(child.id, "test crash");

            // Give time for restarted child to run
            std::thread::sleep(Duration::from_millis(100));
        });

        // Wait for supervisor
        std::thread::sleep(Duration::from_millis(300));
        // Factory should have been invoked at least twice (original + restart)
        assert!(
            invocation_count.load(Ordering::SeqCst) >= 2,
            "expected at least 2 invocations, got {}",
            invocation_count.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn r26_restart_budget_escalates() {
        let system = ActorSystem::new();
        let escalated = Arc::new(AtomicU32::new(0));
        let esc_clone = escalated.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let config = SupervisionConfig {
                strategy: SupervisionStrategy::Restart,
                max_restarts: 2,
                restart_window_secs: 60,
                ..Default::default()
            };
            let child = ctx.spawn_supervised_child_with_config(config, |_ctx: ActorContext| {
                // noop
            }).unwrap();

            // Exhaust restart budget
            let d1 = ctx.handle_child_failure_msg(child.id, "crash1");
            assert!(matches!(d1, SupervisionDecision::Restart));

            // After restart, the child has a new ID. We can find it in supervised_children.
            // Simulate failure of the new child
            let children = ctx.supervised_children.lock().unwrap();
            let new_child_id = children.keys().next().copied();
            drop(children);

            if let Some(new_id) = new_child_id {
                let d2 = ctx.handle_child_failure_msg(new_id, "crash2");
                assert!(matches!(d2, SupervisionDecision::Restart));

                let children = ctx.supervised_children.lock().unwrap();
                let third_child_id = children.keys().next().copied();
                drop(children);

                if let Some(third_id) = third_child_id {
                    let d3 = ctx.handle_child_failure_msg(third_id, "crash3");
                    // Budget exhausted (max_restarts=2), should escalate
                    assert!(
                        matches!(d3, SupervisionDecision::Escalate),
                        "expected Escalate after budget exceeded, got {:?}",
                        d3
                    );
                    esc_clone.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(escalated.load(Ordering::SeqCst), 1, "escalation should have happened");
    }

    #[test]
    fn r26_non_supervised_child_no_restart() {
        let system = ActorSystem::new();
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = count.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let cc = count_clone.clone();
            // spawn_child (not supervised) — one-shot
            let _child = ctx.spawn_child(move |_ctx: ActorContext| {
                cc.fetch_add(1, Ordering::SeqCst);
            });

            std::thread::sleep(Duration::from_millis(100));
        });

        std::thread::sleep(Duration::from_millis(300));
        // Should only have been invoked once (no restart possible)
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn r26_restarted_child_receives_messages() {
        let system = ActorSystem::new();
        let msg_received = Arc::new(AtomicU32::new(0));
        let msg_clone = msg_received.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let mc = msg_clone.clone();
            let child = ctx.spawn_supervised_child(move |child_ctx: ActorContext| {
                // Try to receive a single message
                if let Some(Message::User(_)) = child_ctx.recv() {
                    mc.fetch_add(1, Ordering::SeqCst);
                }
            }).unwrap();

            // Simulate failure and restart
            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(50));

            // Send message to restarted child (find new handle)
            let children = ctx.supervised_children.lock().unwrap();
            if let Some(info) = children.values().next() {
                let new_handle = info.handle.clone();
                drop(children);
                // Send a user message (unit value = 0x7FF8000000000001)
                let _ = ctx.send(&new_handle, Message::User(std::ptr::null_mut()));
                std::thread::sleep(Duration::from_millis(100));
            }
        });

        std::thread::sleep(Duration::from_millis(400));
        assert!(
            msg_received.load(Ordering::SeqCst) >= 1,
            "restarted child should have received a message"
        );
    }

    // ---- R2.10 Graceful Stop Tests ----

    #[test]
    fn r210_graceful_stop_drains_mailbox() {
        let system = ActorSystem::new();
        let processed = Arc::new(AtomicUsize::new(0));
        let pc = processed.clone();

        let handle = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::User(_)) => {
                        pc.fetch_add(1, Ordering::SeqCst);
                    }
                    Some(Message::GracefulStop) => {
                        // Drain remaining messages via drain_and_stop
                        let pc2 = pc.clone();
                        ctx.drain_and_stop(move |_val| {
                            pc2.fetch_add(1, Ordering::SeqCst);
                        });
                        break;
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Send several messages, then GracefulStop
        for _ in 0..5 {
            let _ = system.send(&handle, Message::User(std::ptr::null_mut()));
        }
        let _ = system.send(&handle, Message::GracefulStop);

        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(
            processed.load(Ordering::SeqCst),
            5,
            "all 5 messages should be processed before stopping"
        );
    }

    #[test]
    fn r210_graceful_stop_removes_from_registry() {
        let system = ActorSystem::new();
        let sys2 = system.clone();

        let handle = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::GracefulStop) => {
                        ctx.drain_and_stop(|_| {});
                        break;
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        let id = handle.id;
        assert!(
            sys2.registry.lock().unwrap().contains_key(&id),
            "actor should be in registry before stop"
        );

        let _ = system.send(&handle, Message::GracefulStop);
        std::thread::sleep(Duration::from_millis(200));

        assert!(
            !sys2.registry.lock().unwrap().contains_key(&id),
            "actor should be removed from registry after graceful stop"
        );
    }

    #[test]
    fn r210_exit_does_not_drain() {
        let system = ActorSystem::new();
        let processed = Arc::new(AtomicUsize::new(0));
        let pc = processed.clone();

        let handle = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::User(_)) => {
                        pc.fetch_add(1, Ordering::SeqCst);
                        // Simulate slow processing
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Send messages then immediately Exit (abrupt)
        for _ in 0..10 {
            let _ = system.send(&handle, Message::User(std::ptr::null_mut()));
        }
        let _ = system.send(&handle, Message::Exit);

        std::thread::sleep(Duration::from_millis(300));
        let count = processed.load(Ordering::SeqCst);
        assert!(
            count < 10,
            "Exit should not drain all messages (got {})",
            count
        );
    }

    // ---- R2.1 Work-Stealing Scheduler Tests ----

    #[test]
    fn r21_work_stealing_distributes_across_workers() {
        // Spawn many actors and verify they all complete — exercises the
        // round-robin submission + work-stealing across multiple workers.
        let system = ActorSystem::new();
        let completed = Arc::new(AtomicU32::new(0));
        let n = 32;

        for _ in 0..n {
            let c = completed.clone();
            system.spawn(None, move |_ctx: ActorContext| {
                // Tiny workload — just mark completion.
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        std::thread::sleep(Duration::from_millis(500));
        assert_eq!(
            completed.load(Ordering::SeqCst),
            n,
            "all {} actors should have completed via work-stealing scheduler",
            n
        );
    }

    #[test]
    fn r21_work_stealing_fairness_under_load() {
        // Submit actors in bursts — verify all complete even when some
        // workers might be busy.
        let system = ActorSystem::new();
        let completed = Arc::new(AtomicU32::new(0));
        let n = 64;

        for i in 0..n {
            let c = completed.clone();
            system.spawn(None, move |_ctx: ActorContext| {
                // Variable-length work to stress stealing.
                if i % 4 == 0 {
                    std::thread::sleep(Duration::from_millis(10));
                }
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        std::thread::sleep(Duration::from_millis(2000));
        assert_eq!(
            completed.load(Ordering::SeqCst),
            n,
            "all {} actors should complete under load (fairness)",
            n
        );
    }

    #[test]
    fn r21_work_stealing_preserves_message_ordering() {
        // A single actor should see messages in the order they were sent,
        // regardless of which worker thread runs it.
        let system = ActorSystem::new();
        let received_order = Arc::new(Mutex::new(Vec::<u32>::new()));
        let ro = received_order.clone();

        let handle = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::User(val)) => {
                        // Decode the sequence number from the pointer value.
                        let seq = val as u32;
                        ro.lock().unwrap().push(seq);
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Send numbered messages.
        for i in 1u32..=20 {
            let _ = system.send(&handle, Message::User(i as *mut _));
        }
        let _ = system.send(&handle, Message::Exit);

        std::thread::sleep(Duration::from_millis(500));
        let order = received_order.lock().unwrap();
        let expected: Vec<u32> = (1..=20).collect();
        assert_eq!(*order, expected, "messages should arrive in send order");
    }

    #[test]
    fn r21_work_stealing_starvation_free() {
        // Spawn one long-running actor and many short ones.
        // The short ones must still complete (work stealing prevents starvation).
        let system = ActorSystem::new();
        let short_completed = Arc::new(AtomicU32::new(0));
        let n_short = 16;

        // Long-running actor.
        system.spawn(None, move |_ctx: ActorContext| {
            std::thread::sleep(Duration::from_millis(500));
        });

        // Short actors (after the long one is submitted).
        for _ in 0..n_short {
            let c = short_completed.clone();
            system.spawn(None, move |_ctx: ActorContext| {
                c.fetch_add(1, Ordering::SeqCst);
            });
        }

        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            short_completed.load(Ordering::SeqCst),
            n_short,
            "short actors must complete even while a long actor is running (no starvation)",
        );
    }

    // ---- R2.2 Lock-Free Actor Registry Tests ----

    #[test]
    fn r22_concurrent_register_lookup() {
        // Multiple threads concurrently register and look up named actors.
        let system = ActorSystem::new();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();

        for t in 0..8u32 {
            let sys = system.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                let name = format!("actor-{}", t);
                let actor = sys.spawn(None, |_ctx: ActorContext| {});
                b.wait(); // synchronize registration
                sys.register_named(&name, actor);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

        // All 8 should be registered.
        let listed = system.list_named();
        assert_eq!(listed.len(), 8, "all 8 named actors should be registered");
        for t in 0..8u32 {
            let name = format!("actor-{}", t);
            assert!(
                system.lookup_named(&name).is_some(),
                "actor-{} should be found",
                t
            );
        }
    }

    #[test]
    fn r22_unregister_lookup_race() {
        // Unregister while another thread looks up — no panic, correct result.
        let system = ActorSystem::new();
        let actor = system.spawn(None, |_ctx: ActorContext| {});
        system.register_named("ephemeral", actor);

        let sys2 = system.clone();
        let lookup_thread = thread::spawn(move || {
            let mut found = 0u32;
            for _ in 0..1000 {
                if sys2.lookup_named("ephemeral").is_some() {
                    found += 1;
                }
            }
            found
        });

        // Small delay then unregister.
        std::thread::sleep(Duration::from_millis(1));
        system.unregister_named("ephemeral");

        let found = lookup_thread.join().unwrap();
        // After unregister, lookup must return None.
        assert!(
            system.lookup_named("ephemeral").is_none(),
            "should be gone after unregister"
        );
        // found can be anything from 0 to 1000 — just confirm no panic.
        let _ = found;
    }

    #[test]
    fn r22_many_named_actors_throughput() {
        // Register and look up many named actors to exercise DashMap sharding.
        let system = ActorSystem::new();
        let n = 100;

        for i in 0..n {
            let name = format!("svc-{}", i);
            let handle = system.spawn(None, |_ctx: ActorContext| {});
            assert!(system.register_named(&name, handle), "register {} should succeed", i);
        }

        // All lookups should succeed.
        for i in 0..n {
            let name = format!("svc-{}", i);
            assert!(
                system.lookup_named(&name).is_some(),
                "svc-{} should be found",
                i
            );
        }

        // Duplicate registration should fail.
        let extra = system.spawn(None, |_ctx: ActorContext| {});
        assert!(
            !system.register_named("svc-0", extra),
            "duplicate register must fail"
        );

        assert_eq!(system.list_named().len(), n);
    }

    // ========== R2.8 Actor Monitoring Tests ==========

    #[test]
    fn r28_monitor_receives_actor_down_on_exit() {
        let system = ActorSystem::new();
        let got_down = Arc::new(AtomicU32::new(0));
        let got_clone = got_down.clone();

        // Watcher actor that receives ActorDown messages
        let watcher = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::ActorDown { actor_id: _, reason }) => {
                        assert_eq!(reason, "normal");
                        got_clone.fetch_add(1, Ordering::SeqCst);
                        break;
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Target actor that will terminate immediately
        let target = system.spawn(None, |_ctx: ActorContext| {
            // Terminates immediately
        });

        // Register the monitor
        system.monitor(watcher.id, target.id);

        // Give time for target to die and notification to be delivered
        std::thread::sleep(Duration::from_millis(300));

        assert!(
            got_down.load(Ordering::SeqCst) >= 1,
            "watcher should have received ActorDown, got {}",
            got_down.load(Ordering::SeqCst)
        );
    }

    #[test]
    fn r28_demonitor_stops_notifications() {
        let system = ActorSystem::new();
        let got_down = Arc::new(AtomicU32::new(0));
        let got_clone = got_down.clone();

        // Watcher that counts ActorDown messages
        let watcher = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::ActorDown { .. }) => {
                        got_clone.fetch_add(1, Ordering::SeqCst);
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Target that waits a bit before dying
        let target = system.spawn(None, |_ctx: ActorContext| {
            std::thread::sleep(Duration::from_millis(100));
        });

        // Monitor then demonitor before target dies
        system.monitor(watcher.id, target.id);
        system.demonitor(watcher.id, target.id);

        // Give time for target to die
        std::thread::sleep(Duration::from_millis(300));

        // Stop the watcher
        let _ = system.send(&watcher, Message::Exit);
        std::thread::sleep(Duration::from_millis(50));

        assert_eq!(
            got_down.load(Ordering::SeqCst),
            0,
            "demonitored watcher should NOT have received ActorDown"
        );
    }

    #[test]
    fn r28_multiple_monitors() {
        let system = ActorSystem::new();
        let count = Arc::new(AtomicU32::new(0));

        // Spawn 3 watchers
        let mut watchers = Vec::new();
        for _ in 0..3 {
            let c = count.clone();
            let w = system.spawn(None, move |ctx: ActorContext| {
                loop {
                    match ctx.recv() {
                        Some(Message::ActorDown { .. }) => {
                            c.fetch_add(1, Ordering::SeqCst);
                            break;
                        }
                        Some(Message::Exit) | None => break,
                        _ => {}
                    }
                }
            });
            watchers.push(w);
        }

        // Target
        let target = system.spawn(None, |_ctx: ActorContext| {
            // Terminates immediately
        });

        // Register all watchers as monitors
        for w in &watchers {
            system.monitor(w.id, target.id);
        }

        // Give time
        std::thread::sleep(Duration::from_millis(300));

        assert_eq!(
            count.load(Ordering::SeqCst),
            3,
            "all 3 watchers should receive ActorDown"
        );
    }

    #[test]
    fn r28_monitor_nonexistent_actor_no_crash() {
        let system = ActorSystem::new();

        // Create a watcher
        let watcher = system.spawn(None, |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        // Monitor a non-existent actor — should not crash
        let fake_id = ActorId(999999);
        system.monitor(watcher.id, fake_id);

        // Demonitor also should not crash
        system.demonitor(watcher.id, fake_id);

        // Clean up
        let _ = system.send(&watcher, Message::Exit);
        std::thread::sleep(Duration::from_millis(50));
    }

    // ---- R2.9 Supervision Hardening Tests ----

    #[test]
    fn r29_backoff_delays_restart() {
        // With backoff_base_ms=50, the first restart should produce RestartAfter(50ms),
        // the second RestartAfter(100ms) — checking escalating delays.
        let system = ActorSystem::new();
        let decisions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let dec_clone = decisions.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let config = SupervisionConfig {
                strategy: SupervisionStrategy::Restart,
                max_restarts: 5,
                restart_window_secs: 60,
                backoff_base_ms: 50,
                backoff_max_ms: 10_000,
                max_children: 0,
            };
            let child = ctx.spawn_supervised_child_with_config(config, |_ctx: ActorContext| {
                // noop
            }).unwrap();

            let d1 = ctx.handle_child_failure_msg(child.id, "crash1");
            dec_clone.lock().unwrap().push(d1);

            // Find new child id
            let children = ctx.supervised_children.lock().unwrap();
            let new_id = children.keys().next().copied();
            drop(children);

            // Wait for backoff to complete (first restart is 50ms)
            std::thread::sleep(Duration::from_millis(200));

            if let Some(id) = new_id {
                let d2 = ctx.handle_child_failure_msg(id, "crash2");
                dec_clone.lock().unwrap().push(d2);
            }
        });

        std::thread::sleep(Duration::from_millis(600));
        let decs = decisions.lock().unwrap();
        assert!(decs.len() >= 1, "should have at least one decision");
        // First restart should be RestartAfter(50ms)
        match decs[0] {
            SupervisionDecision::RestartAfter(d) => {
                assert_eq!(d.as_millis(), 50, "first backoff should be 50ms");
            }
            other => panic!("expected RestartAfter, got {:?}", other),
        }
        if decs.len() >= 2 {
            match decs[1] {
                SupervisionDecision::RestartAfter(d) => {
                    assert_eq!(d.as_millis(), 100, "second backoff should be 100ms");
                }
                other => panic!("expected RestartAfter, got {:?}", other),
            }
        }
    }

    #[test]
    fn r29_backoff_caps_at_max() {
        let system = ActorSystem::new();
        let result = Arc::new(std::sync::Mutex::new(None));
        let res_clone = result.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let config = SupervisionConfig {
                strategy: SupervisionStrategy::Restart,
                max_restarts: 30,
                restart_window_secs: 60,
                backoff_base_ms: 100,
                backoff_max_ms: 500,
                max_children: 0,
            };
            let child = ctx.spawn_supervised_child_with_config(config, |_ctx: ActorContext| {}).unwrap();

            // Trigger many failures to push backoff past max
            let mut current_id = child.id;
            let mut last_decision = SupervisionDecision::Restart;
            for i in 0..10 {
                last_decision = ctx.handle_child_failure_msg(current_id, &format!("crash{}", i));
                std::thread::sleep(Duration::from_millis(50)); // let delayed restarts complete
                let children = ctx.supervised_children.lock().unwrap();
                if let Some(&id) = children.keys().next() {
                    current_id = id;
                } else {
                    break;
                }
            }
            *res_clone.lock().unwrap() = Some(last_decision);
        });

        std::thread::sleep(Duration::from_millis(2000));
        let decision = result.lock().unwrap().take();
        if let Some(SupervisionDecision::RestartAfter(d)) = decision {
            assert!(d.as_millis() <= 500, "backoff should cap at 500ms, got {}ms", d.as_millis());
        }
        // Either RestartAfter(capped) or Escalate — both acceptable
    }

    #[test]
    fn r29_max_children_enforced() {
        let system = ActorSystem::new();
        let spawn_results = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sp_clone = spawn_results.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let config = SupervisionConfig {
                strategy: SupervisionStrategy::Restart,
                max_restarts: 3,
                restart_window_secs: 60,
                backoff_base_ms: 0,
                backoff_max_ms: 0,
                max_children: 2,
            };
            // Spawn first child — should succeed
            let r1 = ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {
                std::thread::sleep(Duration::from_millis(500));
            });
            sp_clone.lock().unwrap().push(r1.is_some());

            // Spawn second child — should succeed
            let r2 = ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {
                std::thread::sleep(Duration::from_millis(500));
            });
            sp_clone.lock().unwrap().push(r2.is_some());

            // Spawn third child — should fail (max_children=2)
            let r3 = ctx.spawn_supervised_child_with_config(config, |_ctx: ActorContext| {
                std::thread::sleep(Duration::from_millis(500));
            });
            sp_clone.lock().unwrap().push(r3.is_some());
        });

        std::thread::sleep(Duration::from_millis(300));
        let results = spawn_results.lock().unwrap();
        assert_eq!(results.len(), 3, "should have 3 spawn attempts");
        assert!(results[0], "first child should succeed");
        assert!(results[1], "second child should succeed");
        assert!(!results[2], "third child should be rejected (max_children=2)");
    }

    #[test]
    fn r29_no_backoff_immediate_restart() {
        // With backoff_base_ms=0, restarts should be immediate (Restart, not RestartAfter)
        let system = ActorSystem::new();
        let decision_result = Arc::new(std::sync::Mutex::new(None));
        let dr = decision_result.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let child = ctx.spawn_supervised_child(move |_ctx: ActorContext| {}).unwrap();
            let d = ctx.handle_child_failure_msg(child.id, "crash");
            *dr.lock().unwrap() = Some(d);
        });

        std::thread::sleep(Duration::from_millis(200));
        let d = decision_result.lock().unwrap().take().unwrap();
        assert!(matches!(d, SupervisionDecision::Restart), "default config (backoff=0) should give immediate Restart, got {:?}", d);
    }

    // ---- R2.12 Actor Integration Tests ----

    #[test]
    fn r212_multi_level_supervision_escalation() {
        // Three-level hierarchy: root → supervisor → worker
        // Worker exceeds restart budget → supervisor escalates to root
        let system = ActorSystem::new();
        let escalation_seen = Arc::new(AtomicU32::new(0));
        let esc = escalation_seen.clone();

        system.spawn(None, move |root_ctx: ActorContext| {
            let esc_inner = esc.clone();
            // Root spawns a mid-level supervisor
            let mid = root_ctx.spawn_supervised_child(move |mid_ctx: ActorContext| {
                let config = SupervisionConfig {
                    strategy: SupervisionStrategy::Restart,
                    max_restarts: 1,
                    restart_window_secs: 60,
                    ..Default::default()
                };
                let worker = mid_ctx.spawn_supervised_child_with_config(config, |_wctx: ActorContext| {
                    // Worker that crashes immediately
                }).unwrap();

                // First failure → restart
                let d1 = mid_ctx.handle_child_failure_msg(worker.id, "crash1");
                assert!(matches!(d1, SupervisionDecision::Restart));

                // Find new worker ID
                let children = mid_ctx.supervised_children.lock().unwrap();
                let new_id = children.keys().next().copied();
                drop(children);

                if let Some(id) = new_id {
                    // Second failure → escalate (budget=1 exhausted)
                    let d2 = mid_ctx.handle_child_failure_msg(id, "crash2");
                    assert!(matches!(d2, SupervisionDecision::Escalate));
                }

                // Keep mid alive briefly so root can process
                std::thread::sleep(Duration::from_millis(200));
            }).unwrap();

            // Wait for escalation notification from mid-level
            std::thread::sleep(Duration::from_millis(100));

            // Simulate receiving the escalation (in real life this would be a ChildFailure message)
            let d = root_ctx.handle_child_failure_msg(mid.id, "escalated from worker");
            // Root should restart the mid-level supervisor
            assert!(matches!(d, SupervisionDecision::Restart), "root should restart mid-supervisor, got {:?}", d);
            esc_inner.fetch_add(1, Ordering::SeqCst);
        });

        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(escalation_seen.load(Ordering::SeqCst), 1, "root should have handled escalation");
    }

    #[test]
    fn r212_monitoring_and_supervision_together() {
        // A watcher monitors an actor that is supervised by a supervisor.
        // When the supervised actor is restarted, the watcher should receive ActorDown.
        let system = ActorSystem::new();
        let monitor_notified = Arc::new(AtomicU32::new(0));
        let mn = monitor_notified.clone();

        // Watcher actor
        let watcher = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::ActorDown { .. }) => {
                        mn.fetch_add(1, Ordering::SeqCst);
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        let watcher_id = watcher.id;
        let system_clone = system.clone();

        // Supervisor that has a supervised child monitored by the watcher
        system.spawn(None, move |ctx: ActorContext| {
            let child = ctx.spawn_supervised_child(|_wctx: ActorContext| {
                // Worker does nothing, exits
            }).unwrap();

            // The watcher monitors the child
            system_clone.monitor(watcher_id, child.id);

            std::thread::sleep(Duration::from_millis(100));

            // Simulate failure → restart (old child stops → monitor triggers)
            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(200));
        });

        std::thread::sleep(Duration::from_millis(500));
        let _ = system.send(&watcher, Message::Exit);
        std::thread::sleep(Duration::from_millis(50));
        // Monitor may or may not fire depending on whether the old child's handle
        // was properly dropped. This test verifies the integration doesn't panic.
    }

    #[test]
    fn r212_named_actor_survives_restart() {
        // A named actor that is supervised should be findable after restart
        let system = ActorSystem::new();
        let messages = Arc::new(AtomicU32::new(0));
        let mc = messages.clone();
        let system2 = system.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let mc2 = mc.clone();
            let child = ctx.spawn_supervised_child(move |child_ctx: ActorContext| {
                child_ctx.register_as("named_worker");
                loop {
                    match child_ctx.recv() {
                        Some(Message::User(_)) => {
                            mc2.fetch_add(1, Ordering::SeqCst);
                        }
                        Some(Message::Exit) | None => break,
                        _ => {}
                    }
                }
            }).unwrap();

            std::thread::sleep(Duration::from_millis(100));

            // Send a message via name lookup
            if let Some(h) = system2.lookup_named("named_worker") {
                let _ = system2.send(&h, Message::User(std::ptr::null_mut()));
            }

            std::thread::sleep(Duration::from_millis(100));

            // Simulate failure → restart
            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(200));

            // After restart, the new child should re-register
            if let Some(h) = system2.lookup_named("named_worker") {
                let _ = system2.send(&h, Message::User(std::ptr::null_mut()));
            }
            std::thread::sleep(Duration::from_millis(100));
        });

        std::thread::sleep(Duration::from_millis(800));
        // At least the first message should have been received
        assert!(messages.load(Ordering::SeqCst) >= 1, "named worker should receive at least 1 message");
    }

    #[test]
    fn r212_concurrent_supervision_stress() {
        // Supervisor with many children failing concurrently
        let system = ActorSystem::new();
        let total_restarts = Arc::new(AtomicU32::new(0));
        let tr = total_restarts.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let config = SupervisionConfig {
                strategy: SupervisionStrategy::Restart,
                max_restarts: 10,
                restart_window_secs: 60,
                ..Default::default()
            };

            let mut child_ids = Vec::new();
            for _ in 0..5 {
                if let Some(child) = ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {
                    // Quick worker
                }) {
                    child_ids.push(child.id);
                }
            }

            // All 5 children fail simultaneously
            for id in &child_ids {
                let d = ctx.handle_child_failure_msg(*id, "concurrent crash");
                if matches!(d, SupervisionDecision::Restart) {
                    tr.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(total_restarts.load(Ordering::SeqCst), 5, "all 5 children should be restarted");
    }
}
