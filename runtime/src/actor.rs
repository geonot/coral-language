use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::thread::available_parallelism;

/// Actor identifier (monotonic increasing, never reused within a process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActorId(pub u64);

#[derive(Clone)]
pub struct ActorHandle {
    pub id: ActorId,
    pub(crate) sender: Sender<Message>,
}

#[derive(Debug, Clone)]
pub enum Message {
    Exit,
    User(crate::ValueHandle),
    Failure(String),
}

// Safety: ValueHandle is frozen before sending, making it immutable.
// Frozen values are safe to share across threads.
unsafe impl Send for Message {}

#[derive(Clone)]
pub struct ActorEntry {
    pub(crate) sender: Sender<Message>,
    parent: Option<ActorId>,
}

#[derive(Clone, Default)]
pub struct ActorSystem {
    pub(crate) registry: Arc<Mutex<HashMap<ActorId, ActorEntry>>>,
    scheduler: Scheduler,
}

impl ActorSystem {
    pub fn new() -> Self {
        Self { registry: Arc::new(Mutex::new(HashMap::new())), scheduler: Scheduler::new() }
    }

    pub fn spawn<F>(&self, parent: Option<ActorId>, f: F) -> ActorHandle
    where
        F: FnOnce(ActorContext) + Send + 'static,
    {
        let (tx, rx) = mpsc::channel();
        let id = ActorId(self.scheduler.next_id());
        let handle = ActorHandle { id, sender: tx.clone() };
        let parent_id = parent.unwrap_or(id);
        {
            let mut reg = self.registry.lock().unwrap();
            reg.insert(id, ActorEntry { sender: tx.clone(), parent: Some(parent_id) });
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
            };
            ctx.run(f);
            system.registry.lock().unwrap().remove(&id);
        });
        handle
    }

    pub fn send(&self, handle: &ActorHandle, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        if let Some(entry) = self.registry.lock().unwrap().get(&handle.id) {
            entry.sender.send(msg)
        } else {
            Err(mpsc::SendError(msg))
        }
    }

    pub fn parent_of(&self, id: ActorId) -> Option<ActorId> {
        self.registry.lock().unwrap().get(&id).and_then(|e| e.parent)
    }
}

pub struct ActorContext {
    pub id: ActorId,
    system: ActorSystem,
    rx: Receiver<Message>,
    parent: Option<ActorId>,
    handle: ActorHandle,
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

    pub fn send(&self, handle: &ActorHandle, msg: Message) -> Result<(), mpsc::SendError<Message>> {
        self.system.send(handle, msg)
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
}

/// Global scheduler for actors (M:N). Currently uses a simple work queue
/// and a fixed worker pool sized to available_parallelism().
#[derive(Clone, Default)]
struct Scheduler {
    work: Arc<WorkQueue>,
}

#[derive(Default)]
struct WorkQueue {
    sender: OnceLock<Sender<Runnable>>, // initialized once
    next_id: AtomicU64,
    workers_started: AtomicBool,
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
        for idx in 0..worker_count {
            let rx = rx.clone();
            thread::Builder::new()
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
        }
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
