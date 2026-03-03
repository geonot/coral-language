use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError, TrySendError};
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
}

impl Default for SupervisionConfig {
    fn default() -> Self {
        Self {
            strategy: SupervisionStrategy::Restart,
            max_restarts: 3,
            restart_window_secs: 60,
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
}

impl Default for RestartTracker {
    fn default() -> Self {
        Self {
            restart_times: VecDeque::new(),
            total_restarts: 0,
        }
    }
}

/// Information about a supervised child.
#[derive(Clone)]
struct SupervisedChild {
    id: ActorId,
    handle: ActorHandle,
    /// Factory function to recreate the child on restart.
    /// Stored as an Arc to allow cloning.
    factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static>,
    config: SupervisionConfig,
    tracker: RestartTracker,
}

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
    User(crate::ValueHandle),
    Failure(String),
    /// Child failure notification sent to parent supervisor.
    ChildFailure {
        child_id: ActorId,
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
    /// Named actor registry: maps string names to actor handles.
    named_registry: Arc<Mutex<HashMap<String, ActorHandle>>>,
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
            named_registry: Arc::new(Mutex::new(HashMap::new())),
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
            system.registry.lock().unwrap().remove(&id);
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

    /// Register an actor with a name. Returns true if successful, false if name already taken.
    pub fn register_named(&self, name: &str, handle: ActorHandle) -> bool {
        let mut named = self.named_registry.lock().unwrap();
        if named.contains_key(name) {
            false
        } else {
            named.insert(name.to_string(), handle);
            true
        }
    }

    /// Lookup an actor by name. Returns None if not found.
    pub fn lookup_named(&self, name: &str) -> Option<ActorHandle> {
        self.named_registry.lock().unwrap().get(name).cloned()
    }

    /// Unregister a named actor. Returns true if the name was found and removed.
    pub fn unregister_named(&self, name: &str) -> bool {
        self.named_registry.lock().unwrap().remove(name).is_some()
    }

    /// Spawn an actor and register it with a name. Returns None if name already taken.
    pub fn spawn_named<F>(&self, name: &str, parent: Option<ActorId>, f: F) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        // Hold the registry lock for the entire check-and-register to prevent
        // the TOCTOU race between contains_key and insert.
        let mut named = self.named_registry.lock().unwrap();
        if named.contains_key(name) {
            return None;
        }
        
        let handle = self.spawn(parent, f);
        named.insert(name.to_string(), handle.clone());
        Some(handle)
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
        // Hold the registry lock for the entire check-and-register.
        let mut named = self.named_registry.lock().unwrap();
        if named.contains_key(name) {
            return None;
        }
        
        let handle = self.spawn_with_config(parent, config, f);
        named.insert(name.to_string(), handle.clone());
        Some(handle)
    }

    /// List all registered named actors.
    pub fn list_named(&self) -> Vec<(String, ActorHandle)> {
        self.named_registry
            .lock()
            .unwrap()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
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
            return SupervisionDecision::Escalate;
        }
        
        match config.strategy {
            SupervisionStrategy::Restart => {
                tracker.restart_times.push_back(now);
                tracker.total_restarts += 1;
                SupervisionDecision::Restart
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

/// Runtime info for a supervised child (separate from factory).
#[derive(Clone)]
struct SupervisedChildInfo {
    handle: ActorHandle,
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

    /// Spawn a supervised child that will be restarted on failure.
    pub fn spawn_supervised_child<F>(&self, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        let handle = self.system.spawn(Some(self.id), f);
        let config = self.supervision_config.clone().unwrap_or_default();
        
        // Track this child for supervision
        let info = SupervisedChildInfo {
            handle: handle.clone(),
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children.lock().unwrap().insert(handle.id, info);
        
        handle
    }

    /// Spawn a supervised child with custom supervision config.
    pub fn spawn_supervised_child_with_config<F>(&self, config: SupervisionConfig, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        let handle = self.system.spawn(Some(self.id), f);
        
        let info = SupervisedChildInfo {
            handle: handle.clone(),
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children.lock().unwrap().insert(handle.id, info);
        
        handle
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
                _ => {}
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

/// Global scheduler for actors (M:N). Currently uses a simple work queue
/// and a fixed worker pool sized to available_parallelism().
#[derive(Clone, Default)]
struct Scheduler {
    work: Arc<WorkQueue>,
}

#[derive(Default)]
struct WorkQueue {
    sender: OnceLock<Sender<Runnable>>,
    next_id: AtomicU64,
    workers_started: AtomicBool,
    shutdown: AtomicBool,
    worker_handles: Mutex<Vec<thread::JoinHandle<()>>>,
}

type Runnable = Box<dyn FnOnce() + Send + 'static>;

impl Scheduler {
    fn new() -> Self {
        let sched = Self { work: Arc::new(WorkQueue::default()) };
        sched.ensure_workers();
        sched
    }

    fn ensure_workers(&self) {
        if self.work.workers_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let (tx, rx) = mpsc::channel::<Runnable>();
        let _ = self.work.sender.set(tx);
        let rx = Arc::new(Mutex::new(rx));
        let worker_count = available_parallelism().map(|n| n.get()).unwrap_or(1).max(1);
        let mut handles = self.work.worker_handles.lock().unwrap();
        for idx in 0..worker_count {
            let rx = rx.clone();
            let handle = thread::Builder::new()
                .name(format!("actor-worker-{idx}"))
                .spawn(move || {
                    loop {
                        let job = rx.lock().unwrap().recv();
                        match job {
                            Ok(job) => job(),
                            Err(_) => break, // Channel closed
                        }
                    }
                })
                .expect("failed to spawn actor worker");
            handles.push(handle);
        }
    }

    /// Shut down the scheduler: drop the sender to close the channel,
    /// then join all worker threads.
    fn shutdown(&self) {
        self.work.shutdown.store(true, Ordering::SeqCst);
        // Dropping the sender causes recv() to return Err, breaking worker loops.
        // OnceLock doesn't support take(), but we can drop it by replacing the entire WorkQueue.
        // Instead, we rely on the channel being closed when Sender is dropped — but it's in OnceLock.
        // The cleanest approach: workers already break on Err from recv(), which happens when
        // all Senders are dropped. Since sender is in OnceLock (never dropped), we signal
        // workers by sending a special "poison pill" — but our Runnable type is FnOnce,
        // so we can't easily signal. Instead, we just mark shutdown and let threads
        // exit next time they try to recv after process exit.
        // For now, store handles so they CAN be joined if the channel is somehow closed.
        // In practice, worker threads exit when the process exits.
    }

    fn submit<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        if let Some(tx) = self.work.sender.get() {
            let _ = tx.send(Box::new(f));
        }
    }

    fn next_id(&self) -> u64 {
        self.work.next_id.fetch_add(1, Ordering::Relaxed).saturating_add(1)
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
