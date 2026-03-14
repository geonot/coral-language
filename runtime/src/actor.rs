use std::cmp::Ordering as CmpOrdering;
use std::collections::{BinaryHeap, VecDeque};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::available_parallelism;
use std::time::{Duration, Instant};

pub const DEFAULT_MAILBOX_CAPACITY: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionStrategy {
    Restart,

    Stop,

    Escalate,

    Resume,
}

impl Default for SupervisionStrategy {
    fn default() -> Self {
        Self::Restart
    }
}

#[derive(Debug, Clone)]
pub struct SupervisionConfig {
    pub strategy: SupervisionStrategy,

    pub max_restarts: u32,

    pub restart_window_secs: u64,

    pub backoff_base_ms: u64,

    pub backoff_max_ms: u64,

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

#[derive(Debug, Clone)]
struct RestartTracker {
    restart_times: VecDeque<Instant>,

    total_restarts: u32,

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

#[derive(Debug, Clone)]
pub struct ActorConfig {
    pub mailbox_capacity: usize,

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
    pub fn with_supervision() -> Self {
        Self {
            mailbox_capacity: DEFAULT_MAILBOX_CAPACITY,
            supervision: Some(SupervisionConfig::default()),
        }
    }

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendResult {
    Ok,

    Full,

    Disconnected,
}

#[derive(Debug, Clone)]
pub enum Message {
    Exit,
    GracefulStop,
    User(crate::ValueHandle),
    Failure(String),

    ChildFailure { child_id: ActorId, reason: String },

    ActorDown { actor_id: ActorId, reason: String },
}

unsafe impl Send for Message {}

#[derive(Clone)]
pub struct ActorEntry {
    pub(crate) sender: SyncSender<Message>,
    parent: Option<ActorId>,
    pub(crate) mailbox_capacity: usize,

    pub(crate) supervision: Option<SupervisionConfig>,

    pub(crate) preferred_worker: usize,
}

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
    mailbox_stats()
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TimerId(pub u64);

struct TimerEntry {
    id: TimerId,
    fire_at: Instant,
    target: ActorHandle,
    message: crate::ValueHandle,

    repeat_interval: Option<Duration>,

    cancelled: Arc<AtomicBool>,
}

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
        other
            .fire_at
            .cmp(&self.fire_at)
            .then_with(|| other.id.0.cmp(&self.id.0))
    }
}

#[derive(Clone)]
pub struct TimerToken {
    id: TimerId,
    cancelled: Arc<AtomicBool>,
}

impl TimerToken {
    pub fn cancel(&self) -> bool {
        !self.cancelled.swap(true, Ordering::SeqCst)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    pub fn id(&self) -> TimerId {
        self.id
    }
}

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

    pub fn set_system(&self, system: ActorSystem) {
        *self.system.lock().unwrap() = Some(system);
    }

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
                            Duration::from_millis(100)
                        }
                    };

                    if !sleep_duration.is_zero() {
                        thread::sleep(sleep_duration.min(Duration::from_millis(100)));
                    }

                    let now = Instant::now();
                    let mut to_reschedule = Vec::new();

                    loop {
                        let entry = {
                            let mut heap = timers.lock().unwrap();
                            if let Some(next) = heap.peek() {
                                if next.fire_at <= now {
                                    heap.pop()
                                } else {
                                    None
                                }
                            } else {
                                None
                            }
                        };

                        match entry {
                            Some(entry) => {
                                // Check cancelled AFTER pop to avoid TOCTOU race
                                if entry.cancelled.load(Ordering::SeqCst) {
                                    continue;
                                }
                                if let Some(sys) = system.lock().unwrap().as_ref() {
                                    let _ = sys
                                        .send(&entry.target, Message::User(entry.message.clone()));
                                }

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

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        if let Some(handle) = self.worker_handle.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

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

    pub fn cancel(&self, id: TimerId) -> bool {
        let heap = self.timers.lock().unwrap();
        for entry in heap.iter() {
            if entry.id == id {
                return !entry.cancelled.swap(true, Ordering::SeqCst);
            }
        }
        false
    }

    pub fn pending_count(&self) -> usize {
        self.timers.lock().unwrap().len()
    }
}

#[derive(Clone)]
pub struct ActorSystem {
    pub(crate) registry: Arc<Mutex<HashMap<ActorId, ActorEntry>>>,

    named_registry: Arc<dashmap::DashMap<String, ActorHandle>>,

    monitors: Arc<Mutex<HashMap<ActorId, HashSet<ActorId>>>>,
    scheduler: Scheduler,

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

        timer_wheel.set_system(system.clone());
        system
    }

    pub fn spawn<F>(&self, parent: Option<ActorId>, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.spawn_with_config(parent, ActorConfig::default(), f)
    }

    pub fn spawn_with_config<F>(
        &self,
        parent: Option<ActorId>,
        config: ActorConfig,
        f: F,
    ) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(config.mailbox_capacity);
        let id = ActorId(self.scheduler.next_id());
        let handle = ActorHandle {
            id,
            sender: tx.clone(),
        };
        let parent_id = parent.unwrap_or(id);
        let supervision_config = config.supervision.clone();
        let preferred = self.scheduler.assign_worker();
        {
            let mut reg = self.registry.lock().unwrap();
            reg.insert(
                id,
                ActorEntry {
                    sender: tx.clone(),
                    parent: Some(parent_id),
                    mailbox_capacity: config.mailbox_capacity,
                    supervision: config.supervision.clone(),
                    preferred_worker: preferred,
                },
            );
        }
        let system = self.clone();
        let handle_clone = handle.clone();
        self.scheduler.submit_pinned(preferred, move || {
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

            system.notify_monitors(id, "normal");
            system.registry.lock().unwrap().remove(&id);

            system.monitors.lock().unwrap().remove(&id);
        });
        handle
    }

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
        self.registry
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|e| e.parent)
    }

    pub fn mailbox_capacity(&self, id: ActorId) -> Option<usize> {
        self.registry
            .lock()
            .unwrap()
            .get(&id)
            .map(|e| e.mailbox_capacity)
    }

    pub fn get_actor_handle(&self, id: ActorId) -> Option<ActorHandle> {
        self.registry.lock().unwrap().get(&id).map(|e| ActorHandle {
            id,
            sender: e.sender.clone(),
        })
    }

    pub fn shutdown(&self) {
        self.timer_wheel.shutdown();

        self.scheduler.shutdown();

        let _ = crate::store::save_all_engines();
    }

    pub fn monitor(&self, watcher: ActorId, watched: ActorId) {
        let mut monitors = self.monitors.lock().unwrap();
        monitors
            .entry(watched)
            .or_insert_with(HashSet::new)
            .insert(watcher);
    }

    pub fn demonitor(&self, watcher: ActorId, watched: ActorId) {
        let mut monitors = self.monitors.lock().unwrap();
        if let Some(watchers) = monitors.get_mut(&watched) {
            watchers.remove(&watcher);
            if watchers.is_empty() {
                monitors.remove(&watched);
            }
        }
    }

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

    pub fn lookup_named(&self, name: &str) -> Option<ActorHandle> {
        self.named_registry.get(name).map(|r| r.value().clone())
    }

    pub fn unregister_named(&self, name: &str) -> bool {
        self.named_registry.remove(name).is_some()
    }

    pub fn spawn_named<F>(&self, name: &str, parent: Option<ActorId>, f: F) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        use dashmap::mapref::entry::Entry;

        match self.named_registry.entry(name.to_string()) {
            Entry::Occupied(_) => None,
            Entry::Vacant(v) => {
                let handle = self.spawn(parent, f);
                v.insert(handle.clone());
                Some(handle)
            }
        }
    }

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

    pub fn list_named(&self) -> Vec<(String, ActorHandle)> {
        self.named_registry
            .iter()
            .map(|r| (r.key().clone(), r.value().clone()))
            .collect()
    }

    pub fn send_after(
        &self,
        delay: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.timer_wheel.send_after(delay, target.clone(), message)
    }

    pub fn schedule_repeat(
        &self,
        interval: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.timer_wheel
            .schedule_repeat(interval, target.clone(), message)
    }

    pub fn cancel_timer(&self, token: &TimerToken) -> bool {
        token.cancel()
    }

    pub fn pending_timers(&self) -> usize {
        self.timer_wheel.pending_count()
    }

    pub fn notify_child_failure(&self, parent: &ActorHandle, child_id: ActorId, reason: String) {
        let _ = self.send(parent, Message::ChildFailure { child_id, reason });
    }

    pub fn handle_child_failure(
        &self,
        child_id: ActorId,
        reason: &str,
        config: &SupervisionConfig,
        tracker: &mut RestartTracker,
    ) -> SupervisionDecision {
        let now = Instant::now();
        let window = Duration::from_secs(config.restart_window_secs);

        while let Some(&oldest) = tracker.restart_times.front() {
            if now.duration_since(oldest) > window {
                tracker.restart_times.pop_front();
            } else {
                break;
            }
        }

        if tracker.restart_times.len() >= config.max_restarts as usize {
            tracker.consecutive_restarts = 0;
            return SupervisionDecision::Escalate;
        }

        match config.strategy {
            SupervisionStrategy::Restart => {
                tracker.restart_times.push_back(now);
                tracker.total_restarts += 1;
                tracker.consecutive_restarts += 1;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisionDecision {
    Restart,

    RestartAfter(Duration),

    Stop,

    Escalate,

    Resume,
}

pub struct ActorContext {
    pub id: ActorId,
    system: ActorSystem,
    rx: Receiver<Message>,
    parent: Option<ActorId>,
    handle: ActorHandle,

    supervised_children: Mutex<HashMap<ActorId, SupervisedChildInfo>>,

    supervision_config: Option<SupervisionConfig>,
}

#[derive(Clone)]
struct SupervisedChildInfo {
    handle: ActorHandle,

    factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static>,
    config: SupervisionConfig,
    tracker: RestartTracker,
}

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

    pub fn drain_and_stop<F>(&self, mut handler: F)
    where
        F: FnMut(crate::ValueHandle),
    {
        loop {
            match self.rx.try_recv() {
                Ok(Message::User(val)) => handler(val),
                Ok(Message::GracefulStop) => {}
                Ok(Message::Exit) => break,
                Ok(_) => {}
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        self.system.registry.lock().unwrap().remove(&self.id);
        self.system.notify_monitors(self.id, "graceful_stop");
    }

    pub fn spawn_child<F>(&self, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn(Some(self.id), f)
    }

    pub fn spawn_child_with_config<F>(&self, config: ActorConfig, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn_with_config(Some(self.id), config, f)
    }

    pub fn spawn_supervised_child<F>(&self, f: F) -> Option<ActorHandle>
    where
        F: Fn(ActorContext) + Send + Sync + 'static,
    {
        let config = self.supervision_config.clone().unwrap_or_default();

        if config.max_children > 0 {
            let children = self.supervised_children.lock().unwrap();
            if children.len() >= config.max_children as usize {
                return None;
            }
        }

        let factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static> = Arc::new(f);
        let factory_clone = factory.clone();
        let handle = self
            .system
            .spawn(Some(self.id), move |ctx| factory_clone(ctx));

        let info = SupervisedChildInfo {
            handle: handle.clone(),
            factory,
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children
            .lock()
            .unwrap()
            .insert(handle.id, info);

        Some(handle)
    }

    pub fn spawn_supervised_child_with_config<F>(
        &self,
        config: SupervisionConfig,
        f: F,
    ) -> Option<ActorHandle>
    where
        F: Fn(ActorContext) + Send + Sync + 'static,
    {
        if config.max_children > 0 {
            let children = self.supervised_children.lock().unwrap();
            if children.len() >= config.max_children as usize {
                return None;
            }
        }

        let factory: Arc<dyn Fn(ActorContext) + Send + Sync + 'static> = Arc::new(f);
        let factory_clone = factory.clone();
        let handle = self
            .system
            .spawn(Some(self.id), move |ctx| factory_clone(ctx));

        let info = SupervisedChildInfo {
            handle: handle.clone(),
            factory,
            config,
            tracker: RestartTracker::default(),
        };
        self.supervised_children
            .lock()
            .unwrap()
            .insert(handle.id, info);

        Some(handle)
    }

    pub fn handle_child_failure_msg(&self, child_id: ActorId, reason: &str) -> SupervisionDecision {
        let mut children = self.supervised_children.lock().unwrap();
        if let Some(child_info) = children.get_mut(&child_id) {
            let decision = self.system.handle_child_failure(
                child_id,
                reason,
                &child_info.config,
                &mut child_info.tracker,
            );

            let factory = child_info.factory.clone();
            let config = child_info.config.clone();
            let tracker = child_info.tracker.clone();

            match decision {
                SupervisionDecision::Stop => {
                    children.remove(&child_id);
                }
                SupervisionDecision::Escalate => {
                    children.remove(&child_id);

                    if let Some(parent_id) = self.parent {
                        if let Some(parent_handle) = self.system.get_actor_handle(parent_id) {
                            self.system.notify_child_failure(
                                &parent_handle,
                                self.id,
                                format!("Escalated from child {}: {}", child_id.0, reason),
                            );
                        }
                    }
                }
                SupervisionDecision::Restart => {
                    let factory_clone = factory.clone();
                    let new_handle = self
                        .system
                        .spawn(Some(self.id), move |ctx| factory_clone(ctx));

                    children.remove(&child_id);
                    children.insert(
                        new_handle.id,
                        SupervisedChildInfo {
                            handle: new_handle,
                            factory,
                            config,
                            tracker,
                        },
                    );
                }
                SupervisionDecision::RestartAfter(delay) => {
                    children.remove(&child_id);
                    drop(children);
                    std::thread::sleep(delay);
                    let factory_clone = factory.clone();
                    let new_handle = self
                        .system
                        .spawn(Some(self.id), move |ctx| factory_clone(ctx));
                    self.supervised_children.lock().unwrap().insert(
                        new_handle.id,
                        SupervisedChildInfo {
                            handle: new_handle,
                            factory,
                            config,
                            tracker,
                        },
                    );
                }
                SupervisionDecision::Resume => {}
            }

            decision
        } else {
            SupervisionDecision::Stop
        }
    }

    pub fn send(&self, handle: &ActorHandle, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        self.system.send(handle, msg)
    }

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

    pub fn register_as(&self, name: &str) -> bool {
        self.system.register_named(name, self.handle.clone())
    }

    pub fn lookup(&self, name: &str) -> Option<ActorHandle> {
        self.system.lookup_named(name)
    }

    pub fn spawn_named_child<F>(&self, name: &str, f: F) -> Option<ActorHandle>
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        self.system.spawn_named(name, Some(self.id), f)
    }

    pub fn send_to_named(&self, name: &str, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        if let Some(handle) = self.lookup(name) {
            self.send(&handle, msg)
        } else {
            Err(mpsc::SendError(msg))
        }
    }

    pub fn send_after(
        &self,
        delay: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.send_after(delay, target, message)
    }

    pub fn send_self_after(&self, delay: Duration, message: crate::ValueHandle) -> TimerToken {
        self.system.send_after(delay, &self.handle, message)
    }

    pub fn schedule_repeat(
        &self,
        interval: Duration,
        target: &ActorHandle,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.schedule_repeat(interval, target, message)
    }

    pub fn schedule_self_repeat(
        &self,
        interval: Duration,
        message: crate::ValueHandle,
    ) -> TimerToken {
        self.system.schedule_repeat(interval, &self.handle, message)
    }

    pub fn cancel_timer(&self, token: &TimerToken) -> bool {
        token.cancel()
    }
}

#[derive(Clone)]
struct Scheduler {
    work: Arc<WorkStealingQueue>,
}

struct WorkStealingQueue {
    injectors: Vec<crossbeam_deque::Injector<Runnable>>,

    stealers: Vec<crossbeam_deque::Stealer<Runnable>>,

    next_worker: AtomicU64,
    next_id: AtomicU64,
    workers_started: AtomicBool,
    shutdown: AtomicBool,
    worker_handles: Mutex<Vec<thread::JoinHandle<()>>>,

    worker_count: usize,

    notify: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

unsafe impl Send for WorkStealingQueue {}
unsafe impl Sync for WorkStealingQueue {}

type Runnable = Box<dyn FnOnce() + Send + 'static>;

impl Scheduler {
    fn new() -> Self {
        let worker_count = available_parallelism().map(|n| n.get()).unwrap_or(1).max(1);

        let mut local_workers: Vec<crossbeam_deque::Worker<Runnable>> =
            Vec::with_capacity(worker_count);
        let mut stealers: Vec<crossbeam_deque::Stealer<Runnable>> =
            Vec::with_capacity(worker_count);
        let mut injectors: Vec<crossbeam_deque::Injector<Runnable>> =
            Vec::with_capacity(worker_count);

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

    fn shutdown(&self) {
        self.work.shutdown.store(true, Ordering::SeqCst);

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
        let idx =
            self.work.next_worker.fetch_add(1, Ordering::Relaxed) as usize % self.work.worker_count;
        self.work.injectors[idx].push(Box::new(f));

        let (lock, cvar) = &*self.work.notify;
        let mut flag = lock.lock().unwrap();
        *flag = true;
        cvar.notify_one();
        drop(flag);
    }

    fn submit_pinned<F>(&self, worker_idx: usize, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let idx = worker_idx % self.work.worker_count;
        self.work.injectors[idx].push(Box::new(f));

        let (lock, cvar) = &*self.work.notify;
        let mut flag = lock.lock().unwrap();
        *flag = true;
        cvar.notify_one();
        drop(flag);
    }

    fn assign_worker(&self) -> usize {
        self.work.next_worker.fetch_add(1, Ordering::Relaxed) as usize % self.work.worker_count
    }

    fn next_id(&self) -> u64 {
        self.work
            .next_id
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1)
    }
}

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

        if let Some(task) = local.pop() {
            task();
            continue;
        }

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

        if let Some(task) = local.pop() {
            task();
            continue;
        }

        let n = queue.worker_count;
        if n > 1 {
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

        let (lock, cvar) = &*notify;
        let mut flag = lock.lock().unwrap();

        if queue.shutdown.load(Ordering::Relaxed) {
            break;
        }

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

        let sup_handle = system.spawn(None, move |ctx: ActorContext| {
            let cc = count_clone.clone();
            let child = ctx
                .spawn_supervised_child(move |_child_ctx: ActorContext| {
                    cc.fetch_add(1, Ordering::SeqCst);
                })
                .unwrap();

            ctx.handle_child_failure_msg(child.id, "test crash");

            std::thread::sleep(Duration::from_millis(100));
        });

        std::thread::sleep(Duration::from_millis(300));

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
            let child = ctx
                .spawn_supervised_child_with_config(config, |_ctx: ActorContext| {})
                .unwrap();

            let d1 = ctx.handle_child_failure_msg(child.id, "crash1");
            assert!(matches!(d1, SupervisionDecision::Restart));

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
        assert_eq!(
            escalated.load(Ordering::SeqCst),
            1,
            "escalation should have happened"
        );
    }

    #[test]
    fn r26_non_supervised_child_no_restart() {
        let system = ActorSystem::new();
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = count.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let cc = count_clone.clone();

            let _child = ctx.spawn_child(move |_ctx: ActorContext| {
                cc.fetch_add(1, Ordering::SeqCst);
            });

            std::thread::sleep(Duration::from_millis(100));
        });

        std::thread::sleep(Duration::from_millis(300));

        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn r26_restarted_child_receives_messages() {
        let system = ActorSystem::new();
        let msg_received = Arc::new(AtomicU32::new(0));
        let msg_clone = msg_received.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let mc = msg_clone.clone();
            let child = ctx
                .spawn_supervised_child(move |child_ctx: ActorContext| {
                    if let Some(Message::User(_)) = child_ctx.recv() {
                        mc.fetch_add(1, Ordering::SeqCst);
                    }
                })
                .unwrap();

            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(50));

            let children = ctx.supervised_children.lock().unwrap();
            if let Some(info) = children.values().next() {
                let new_handle = info.handle.clone();
                drop(children);

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

                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

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

    #[test]
    fn r21_work_stealing_distributes_across_workers() {
        let system = ActorSystem::new();
        let completed = Arc::new(AtomicU32::new(0));
        let n = 32;

        for _ in 0..n {
            let c = completed.clone();
            system.spawn(None, move |_ctx: ActorContext| {
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
        let system = ActorSystem::new();
        let completed = Arc::new(AtomicU32::new(0));
        let n = 64;

        for i in 0..n {
            let c = completed.clone();
            system.spawn(None, move |_ctx: ActorContext| {
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
        let system = ActorSystem::new();
        let received_order = Arc::new(Mutex::new(Vec::<u32>::new()));
        let ro = received_order.clone();

        let handle = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::User(val)) => {
                        let seq = val as u32;
                        ro.lock().unwrap().push(seq);
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

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
        let system = ActorSystem::new();
        let short_completed = Arc::new(AtomicU32::new(0));
        let n_short = 16;

        system.spawn(None, move |_ctx: ActorContext| {
            std::thread::sleep(Duration::from_millis(500));
        });

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

    #[test]
    fn r22_concurrent_register_lookup() {
        let system = ActorSystem::new();
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let mut handles = Vec::new();

        for t in 0..8u32 {
            let sys = system.clone();
            let b = barrier.clone();
            handles.push(thread::spawn(move || {
                let name = format!("actor-{}", t);
                let actor = sys.spawn(None, |_ctx: ActorContext| {});
                b.wait();
                sys.register_named(&name, actor);
            }));
        }

        for h in handles {
            h.join().unwrap();
        }

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

        std::thread::sleep(Duration::from_millis(1));
        system.unregister_named("ephemeral");

        let found = lookup_thread.join().unwrap();

        assert!(
            system.lookup_named("ephemeral").is_none(),
            "should be gone after unregister"
        );

        let _ = found;
    }

    #[test]
    fn r22_many_named_actors_throughput() {
        let system = ActorSystem::new();
        let n = 100;

        for i in 0..n {
            let name = format!("svc-{}", i);
            let handle = system.spawn(None, |_ctx: ActorContext| {});
            assert!(
                system.register_named(&name, handle),
                "register {} should succeed",
                i
            );
        }

        for i in 0..n {
            let name = format!("svc-{}", i);
            assert!(
                system.lookup_named(&name).is_some(),
                "svc-{} should be found",
                i
            );
        }

        let extra = system.spawn(None, |_ctx: ActorContext| {});
        assert!(
            !system.register_named("svc-0", extra),
            "duplicate register must fail"
        );

        assert_eq!(system.list_named().len(), n);
    }

    #[test]
    fn r28_monitor_receives_actor_down_on_exit() {
        let system = ActorSystem::new();
        let got_down = Arc::new(AtomicU32::new(0));
        let got_clone = got_down.clone();

        let watcher = system.spawn(None, move |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::ActorDown {
                        actor_id: _,
                        reason,
                    }) => {
                        assert_eq!(reason, "normal");
                        got_clone.fetch_add(1, Ordering::SeqCst);
                        break;
                    }
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        let target = system.spawn(None, |_ctx: ActorContext| {});

        system.monitor(watcher.id, target.id);

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

        let target = system.spawn(None, |_ctx: ActorContext| {
            std::thread::sleep(Duration::from_millis(100));
        });

        system.monitor(watcher.id, target.id);
        system.demonitor(watcher.id, target.id);

        std::thread::sleep(Duration::from_millis(300));

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

        let target = system.spawn(None, |_ctx: ActorContext| {});

        for w in &watchers {
            system.monitor(w.id, target.id);
        }

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

        let watcher = system.spawn(None, |ctx: ActorContext| {
            loop {
                match ctx.recv() {
                    Some(Message::Exit) | None => break,
                    _ => {}
                }
            }
        });

        let fake_id = ActorId(999999);
        system.monitor(watcher.id, fake_id);

        system.demonitor(watcher.id, fake_id);

        let _ = system.send(&watcher, Message::Exit);
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn r29_backoff_delays_restart() {
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
            let child = ctx
                .spawn_supervised_child_with_config(config, |_ctx: ActorContext| {})
                .unwrap();

            let d1 = ctx.handle_child_failure_msg(child.id, "crash1");
            dec_clone.lock().unwrap().push(d1);

            let children = ctx.supervised_children.lock().unwrap();
            let new_id = children.keys().next().copied();
            drop(children);

            std::thread::sleep(Duration::from_millis(200));

            if let Some(id) = new_id {
                let d2 = ctx.handle_child_failure_msg(id, "crash2");
                dec_clone.lock().unwrap().push(d2);
            }
        });

        std::thread::sleep(Duration::from_millis(600));
        let decs = decisions.lock().unwrap();
        assert!(decs.len() >= 1, "should have at least one decision");

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
            let child = ctx
                .spawn_supervised_child_with_config(config, |_ctx: ActorContext| {})
                .unwrap();

            let mut current_id = child.id;
            let mut last_decision = SupervisionDecision::Restart;
            for i in 0..10 {
                last_decision = ctx.handle_child_failure_msg(current_id, &format!("crash{}", i));
                std::thread::sleep(Duration::from_millis(50));
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
            assert!(
                d.as_millis() <= 500,
                "backoff should cap at 500ms, got {}ms",
                d.as_millis()
            );
        }
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

            let r1 =
                ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {
                    std::thread::sleep(Duration::from_millis(500));
                });
            sp_clone.lock().unwrap().push(r1.is_some());

            let r2 =
                ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {
                    std::thread::sleep(Duration::from_millis(500));
                });
            sp_clone.lock().unwrap().push(r2.is_some());

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
        assert!(
            !results[2],
            "third child should be rejected (max_children=2)"
        );
    }

    #[test]
    fn r29_no_backoff_immediate_restart() {
        let system = ActorSystem::new();
        let decision_result = Arc::new(std::sync::Mutex::new(None));
        let dr = decision_result.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let child = ctx
                .spawn_supervised_child(move |_ctx: ActorContext| {})
                .unwrap();
            let d = ctx.handle_child_failure_msg(child.id, "crash");
            *dr.lock().unwrap() = Some(d);
        });

        std::thread::sleep(Duration::from_millis(200));
        let d = decision_result.lock().unwrap().take().unwrap();
        assert!(
            matches!(d, SupervisionDecision::Restart),
            "default config (backoff=0) should give immediate Restart, got {:?}",
            d
        );
    }

    #[test]
    fn r212_multi_level_supervision_escalation() {
        let system = ActorSystem::new();
        let escalation_seen = Arc::new(AtomicU32::new(0));
        let esc = escalation_seen.clone();

        system.spawn(None, move |root_ctx: ActorContext| {
            let esc_inner = esc.clone();

            let mid = root_ctx
                .spawn_supervised_child(move |mid_ctx: ActorContext| {
                    let config = SupervisionConfig {
                        strategy: SupervisionStrategy::Restart,
                        max_restarts: 1,
                        restart_window_secs: 60,
                        ..Default::default()
                    };
                    let worker = mid_ctx
                        .spawn_supervised_child_with_config(config, |_wctx: ActorContext| {})
                        .unwrap();

                    let d1 = mid_ctx.handle_child_failure_msg(worker.id, "crash1");
                    assert!(matches!(d1, SupervisionDecision::Restart));

                    let children = mid_ctx.supervised_children.lock().unwrap();
                    let new_id = children.keys().next().copied();
                    drop(children);

                    if let Some(id) = new_id {
                        let d2 = mid_ctx.handle_child_failure_msg(id, "crash2");
                        assert!(matches!(d2, SupervisionDecision::Escalate));
                    }

                    std::thread::sleep(Duration::from_millis(200));
                })
                .unwrap();

            std::thread::sleep(Duration::from_millis(100));

            let d = root_ctx.handle_child_failure_msg(mid.id, "escalated from worker");

            assert!(
                matches!(d, SupervisionDecision::Restart),
                "root should restart mid-supervisor, got {:?}",
                d
            );
            esc_inner.fetch_add(1, Ordering::SeqCst);
        });

        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(
            escalation_seen.load(Ordering::SeqCst),
            1,
            "root should have handled escalation"
        );
    }

    #[test]
    fn r212_monitoring_and_supervision_together() {
        let system = ActorSystem::new();
        let monitor_notified = Arc::new(AtomicU32::new(0));
        let mn = monitor_notified.clone();

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

        system.spawn(None, move |ctx: ActorContext| {
            let child = ctx
                .spawn_supervised_child(|_wctx: ActorContext| {})
                .unwrap();

            system_clone.monitor(watcher_id, child.id);

            std::thread::sleep(Duration::from_millis(100));

            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(200));
        });

        std::thread::sleep(Duration::from_millis(500));
        let _ = system.send(&watcher, Message::Exit);
        std::thread::sleep(Duration::from_millis(50));
    }

    #[test]
    fn r212_named_actor_survives_restart() {
        let system = ActorSystem::new();
        let messages = Arc::new(AtomicU32::new(0));
        let mc = messages.clone();
        let system2 = system.clone();

        system.spawn(None, move |ctx: ActorContext| {
            let mc2 = mc.clone();
            let child = ctx
                .spawn_supervised_child(move |child_ctx: ActorContext| {
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
                })
                .unwrap();

            std::thread::sleep(Duration::from_millis(100));

            if let Some(h) = system2.lookup_named("named_worker") {
                let _ = system2.send(&h, Message::User(std::ptr::null_mut()));
            }

            std::thread::sleep(Duration::from_millis(100));

            ctx.handle_child_failure_msg(child.id, "crash");
            std::thread::sleep(Duration::from_millis(200));

            if let Some(h) = system2.lookup_named("named_worker") {
                let _ = system2.send(&h, Message::User(std::ptr::null_mut()));
            }
            std::thread::sleep(Duration::from_millis(100));
        });

        std::thread::sleep(Duration::from_millis(800));

        assert!(
            messages.load(Ordering::SeqCst) >= 1,
            "named worker should receive at least 1 message"
        );
    }

    #[test]
    fn r212_concurrent_supervision_stress() {
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
                if let Some(child) =
                    ctx.spawn_supervised_child_with_config(config.clone(), |_ctx: ActorContext| {})
                {
                    child_ids.push(child.id);
                }
            }

            for id in &child_ids {
                let d = ctx.handle_child_failure_msg(*id, "concurrent crash");
                if matches!(d, SupervisionDecision::Restart) {
                    tr.fetch_add(1, Ordering::SeqCst);
                }
            }
        });

        std::thread::sleep(Duration::from_millis(400));
        assert_eq!(
            total_restarts.load(Ordering::SeqCst),
            5,
            "all 5 children should be restarted"
        );
    }
}
