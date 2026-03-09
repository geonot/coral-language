# Coral Actor System Completion Plan

_Created: 2026-01-06_

## 1. Current State

### 1.1 What's Working

| Feature | Status | Notes |
|---------|--------|-------|
| Actor spawn | ✅ Working | `coral_actor_spawn` FFI |
| Actor send | ✅ Working | `coral_actor_send` FFI |
| M:N scheduler | ✅ Working | Worker thread pool with shutdown |
| Bounded mailboxes | ✅ Working | Default 1024 capacity |
| Backpressure | ✅ Basic | Returns `Full` status |
| Actor state | ✅ Working | `self.field` access in handlers |
| State from closure | ✅ Working | State passed via environment |
| Failure propagation | ✅ Basic | Failure message to parent |
| @handler syntax | ✅ Working | Compile-time arity check |
| Named actors | ✅ Working | Global registry, lookup by name, singleton patterns |
| Actor supervision | ✅ Working | Restart strategies, parent notification |
| Actor timers | ✅ Working | Timer wheel, periodic/one-shot timers |
| Clean shutdown | ✅ Working | AtomicBool + JoinHandle for timer/worker/scheduler |
| Thread-local pools | ✅ Working | LOCAL_VALUE_POOL with overflow to global |
| CAS-based refcount | ✅ Working | compare_exchange_weak eliminates TOCTOU |

### 1.2 What's Not Yet Implemented

| Feature | Priority | Complexity |
|---------|----------|------------|
| Typed message contracts | P1 | Medium |
| Actor monitoring (watch/unwatch) | P2 | Low |
| Remote actors (networking) | P2 | High |
| Location transparency | P2 | High |
| Work-stealing scheduler | P2 | Medium |

---

## 2. Named Actors

### 2.1 Design

Named actors can be registered globally and looked up by name. This enables:
- Service discovery (find "DatabaseActor" by name)
- Singleton patterns (ensure only one instance)
- Cross-module communication without passing handles

### 2.2 Syntax

```coral
// Register actor with name
actor DatabaseService
    @init
        register_as("database")
    
    @query(msg)
        // handle query

// Lookup by name
db is lookup_actor("database")
send(db, make_Query("SELECT * FROM users"))

// Or shorthand syntax
send_to "database", make_Query("SELECT * FROM users")
```

### 2.3 Implementation

```rust
// runtime/src/actor.rs additions
pub struct ActorRegistry {
    names: HashMap<String, ActorId>,
    ids: HashMap<ActorId, String>,
}

impl ActorSystem {
    pub fn register(&self, name: &str, id: ActorId) -> Result<(), RegistryError> {
        let mut reg = self.registry.lock().unwrap();
        if reg.names.contains_key(name) {
            return Err(RegistryError::NameExists);
        }
        reg.names.insert(name.to_string(), id);
        reg.ids.insert(id, name.to_string());
        Ok(())
    }
    
    pub fn lookup(&self, name: &str) -> Option<ActorHandle> {
        let reg = self.registry.lock().unwrap();
        reg.names.get(name).and_then(|id| {
            self.get_handle(*id)
        })
    }
    
    pub fn unregister(&self, name: &str) {
        let mut reg = self.registry.lock().unwrap();
        if let Some(id) = reg.names.remove(name) {
            reg.ids.remove(&id);
        }
    }
}
```

### 2.4 Tasks

- [ ] 2.1 Add `ActorRegistry` to runtime
- [ ] 2.2 Implement `register_as` FFI function
- [ ] 2.3 Implement `lookup_actor` FFI function
- [ ] 2.4 Add `send_to` syntax sugar in parser
- [ ] 2.5 Handle name conflicts (error or replace?)
- [ ] 2.6 Auto-unregister on actor death
- [ ] 2.7 Test named actor lookup across modules

---

## 3. Actor Supervision

### 3.1 Design

Supervision allows parent actors to handle child failures systematically. Based on Erlang/Akka patterns:

| Strategy | Behavior |
|----------|----------|
| `restart` | Restart failed child with initial state |
| `stop` | Stop failed child permanently |
| `escalate` | Propagate failure to grandparent |
| `resume` | Ignore failure, continue with current state |

### 3.2 Syntax

```coral
actor Supervisor
    @supervision
        strategy is restart
        max_restarts is 3
        window is 60  // seconds
    
    @init
        // Spawn children
        worker is spawn_child(Worker)
    
    @child_failure(msg)
        // Custom failure handling (optional)
        log("Child ${msg.child_id} failed: ${msg.reason}")
        // Default strategy applied after this handler

actor Worker
    @work(msg)
        // May fail
        risky_operation()
```

### 3.3 Implementation

```rust
// runtime/src/actor.rs additions

#[derive(Clone, Copy, Debug)]
pub enum SupervisionStrategy {
    Restart,
    Stop,
    Escalate,
    Resume,
}

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    pub strategy: SupervisionStrategy,
    pub max_restarts: u32,
    pub window_secs: u64,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            strategy: SupervisionStrategy::Restart,
            max_restarts: 3,
            window_secs: 60,
        }
    }
}

pub struct ChildTracker {
    pub child_id: ActorId,
    pub restart_count: u32,
    pub restart_times: VecDeque<Instant>,
    pub initial_state: ValueHandle,  // For restart
}

impl ActorContext {
    pub fn spawn_supervised<F>(&self, config: SupervisorConfig, f: F) -> ActorHandle
    where F: FnOnce(ActorContext) + Send + Clone + 'static
    {
        // Store f for potential restarts
        // Track child with config
        // ...
    }
    
    fn handle_child_failure(&self, child_id: ActorId, reason: String) {
        let tracker = self.get_child_tracker(child_id);
        
        // Check restart budget
        let now = Instant::now();
        tracker.restart_times.retain(|t| {
            now.duration_since(*t).as_secs() < self.supervisor_config.window_secs
        });
        
        if tracker.restart_times.len() >= self.supervisor_config.max_restarts as usize {
            // Budget exhausted - escalate
            self.escalate_failure(reason);
            return;
        }
        
        match self.supervisor_config.strategy {
            SupervisionStrategy::Restart => {
                tracker.restart_count += 1;
                tracker.restart_times.push_back(now);
                self.restart_child(child_id);
            }
            SupervisionStrategy::Stop => {
                self.stop_child(child_id);
            }
            SupervisionStrategy::Escalate => {
                self.escalate_failure(reason);
            }
            SupervisionStrategy::Resume => {
                // Do nothing, child continues
            }
        }
    }
}
```

### 3.4 Tasks

- [ ] 3.1 Add `SupervisorConfig` to runtime
- [ ] 3.2 Implement child tracking
- [ ] 3.3 Implement restart logic with state reset
- [ ] 3.4 Implement restart budget (max restarts in window)
- [ ] 3.5 Add `@supervision` annotation parsing
- [ ] 3.6 Add `@child_failure` handler support
- [ ] 3.7 Implement escalation chain
- [ ] 3.8 Test supervision scenarios

---

## 4. Remote Actors (Networking)

### 4.1 Design

Remote actors enable distributed systems. Messages are serialized and sent over the network transparently.

### 4.2 Syntax

```coral
// Local actor creation
local_worker is spawn(Worker)

// Remote actor connection
remote_worker is connect_actor("tcp://192.168.1.100:9000/worker")

// Send works the same way
send(remote_worker, make_Task(data))

// Spawn on remote node
remote_actor is spawn_remote("tcp://192.168.1.100:9000", Worker)

// Listen for incoming connections
listen_actors("tcp://0.0.0.0:9000")
```

### 4.3 Architecture

```
┌─────────────────┐          Network          ┌─────────────────┐
│    Node A       │                           │    Node B       │
├─────────────────┤                           ├─────────────────┤
│                 │                           │                 │
│  ┌───────────┐  │     Serialized Message    │  ┌───────────┐  │
│  │  Actor A  │──┼──────────────────────────▶│  │  Actor B  │  │
│  └───────────┘  │                           │  └───────────┘  │
│        │        │                           │                 │
│        │ send() │                           │                 │
│        ▼        │                           │                 │
│  ┌───────────┐  │                           │  ┌───────────┐  │
│  │  Network  │  │                           │  │  Network  │  │
│  │  Layer    │  │◀─────────────────────────▶│  │  Layer    │  │
│  └───────────┘  │                           │  └───────────┘  │
│                 │                           │                 │
└─────────────────┘                           └─────────────────┘
```

### 4.4 Protocol

```
Message Format:
┌─────────────────────────────────────────────────────────────┐
│  Header (16 bytes)                                          │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Magic: "CACT" (4 bytes)                                 ││
│  │ Version: u16                                            ││
│  │ Type: u16 (Send, Spawn, Lookup, etc.)                   ││
│  │ Message ID: u64                                         ││
│  └─────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────┤
│  Target (variable)                                          │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Actor ID: u64                                           ││
│  │ Or Actor Name Length + Name                             ││
│  └─────────────────────────────────────────────────────────┘│
├─────────────────────────────────────────────────────────────┤
│  Payload (variable)                                         │
│  ┌─────────────────────────────────────────────────────────┐│
│  │ Serialized Coral Value                                  ││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

### 4.5 Tasks

- [ ] 4.1 Design message serialization format
- [ ] 4.2 Implement TCP transport layer
- [ ] 4.3 Implement `connect_actor` function
- [ ] 4.4 Implement `spawn_remote` function
- [ ] 4.5 Implement `listen_actors` function
- [ ] 4.6 Handle network failures gracefully
- [ ] 4.7 Implement connection pooling
- [ ] 4.8 Add TLS support (optional)
- [ ] 4.9 Test cross-node communication

---

## 5. Typed Message Contracts

### 5.1 Design

Currently all messages are `Any`. Typed contracts enable compile-time verification of message types.

### 5.2 Syntax

```coral
// Define message types
type DatabaseMessage
    | Query(sql: String, reply_to: Actor)
    | Insert(table: String, data: Map)
    | Update(table: String, id: Int, data: Map)
    | Delete(table: String, id: Int)

actor DatabaseActor
    @messages(DatabaseMessage)  // Declare accepted messages
    
    @query(msg: Query)
        result is execute_sql(msg.sql)
        send(msg.reply_to, result)
    
    @insert(msg: Insert)
        // ...

// Compile-time error if wrong message type sent
db is spawn(DatabaseActor)
send(db, make_Query("SELECT *", self()))  // OK
send(db, "hello")                          // Compile error: expected DatabaseMessage
```

### 5.3 Implementation

Add message type checking in semantic analysis:

```rust
// src/semantic.rs additions
fn check_actor_send(
    callee: &Expression,
    args: &[Expression],
    actor_types: &HashMap<String, TypeId>,
) -> Result<(), Diagnostic> {
    if let Expression::Identifier(name, span) = callee {
        if name == "send" && args.len() == 2 {
            let actor_expr = &args[0];
            let msg_expr = &args[1];
            
            // Look up actor's message type
            if let Some(actor_type) = get_actor_type(actor_expr, actor_types) {
                let msg_type = infer_type(msg_expr);
                if !is_subtype(&msg_type, &actor_type.message_type) {
                    return Err(Diagnostic::new(
                        format!("actor expects {}, got {}", 
                            actor_type.message_type, msg_type),
                        msg_expr.span()
                    ));
                }
            }
        }
    }
    Ok(())
}
```

### 5.4 Tasks

- [ ] 5.1 Add `@messages` annotation parsing
- [ ] 5.2 Track actor message types in semantic model
- [ ] 5.3 Implement message type checking at send sites
- [ ] 5.4 Generate helpful error messages
- [ ] 5.5 Handle union message types
- [ ] 5.6 Test type errors

---

## 6. Actor Timers

### 6.1 Design

Timers allow actors to receive messages at specified intervals or after delays.

### 6.2 Syntax

```coral
actor HealthChecker
    @init
        // Send message to self every 30 seconds
        schedule_interval(self(), make_Check(), 30.seconds)
        
        // Send message after 5 second delay
        schedule_once(self(), make_Warmup(), 5.seconds)
    
    @check(msg)
        status is check_health()
        log("Health: ${status}")
    
    @warmup(msg)
        prepare_caches()
```

### 6.3 Implementation

```rust
// runtime/src/actor.rs additions

pub struct Timer {
    pub id: u64,
    pub target: ActorHandle,
    pub message: ValueHandle,
    pub interval_ms: Option<u64>,  // None for one-shot
    pub next_fire: Instant,
}

pub struct TimerWheel {
    timers: BinaryHeap<Timer>,
    next_id: AtomicU64,
}

impl TimerWheel {
    pub fn schedule_once(&self, target: ActorHandle, msg: ValueHandle, delay: Duration) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let timer = Timer {
            id,
            target,
            message: msg,
            interval_ms: None,
            next_fire: Instant::now() + delay,
        };
        self.timers.push(timer);
        id
    }
    
    pub fn schedule_interval(&self, target: ActorHandle, msg: ValueHandle, interval: Duration) -> u64 {
        // Similar, but with interval_ms set
    }
    
    pub fn cancel(&self, timer_id: u64) {
        // Remove timer from wheel
    }
    
    pub fn tick(&self) {
        let now = Instant::now();
        while let Some(timer) = self.timers.peek() {
            if timer.next_fire > now {
                break;
            }
            let timer = self.timers.pop().unwrap();
            // Send message
            if let Some(interval) = timer.interval_ms {
                // Reschedule for next interval
                let mut next_timer = timer.clone();
                next_timer.next_fire = now + Duration::from_millis(interval);
                self.timers.push(next_timer);
            }
        }
    }
}
```

### 6.4 Tasks

- [ ] 6.1 Implement `TimerWheel` structure
- [ ] 6.2 Add timer thread to scheduler
- [ ] 6.3 Implement `schedule_once` FFI
- [ ] 6.4 Implement `schedule_interval` FFI
- [ ] 6.5 Implement `cancel_timer` FFI
- [ ] 6.6 Handle actor death (cancel timers)
- [ ] 6.7 Test timer accuracy

---

## 7. Actor Monitoring

### 7.1 Design

Monitoring allows actors to be notified when other actors terminate.

### 7.2 Syntax

```coral
actor Watcher
    @init
        worker is spawn(Worker)
        monitor(worker)
    
    @down(msg: ActorDown)
        log("Actor ${msg.actor_id} terminated: ${msg.reason}")
        // Decide what to do: restart, escalate, etc.
```

### 7.3 Tasks

- [ ] 7.1 Implement `monitor` function
- [ ] 7.2 Track monitors per actor
- [ ] 7.3 Send `ActorDown` message on termination
- [ ] 7.4 Support `demonitor` to stop watching
- [ ] 7.5 Test monitoring scenarios

---

## 8. Implementation Roadmap

### Phase 1: Named Actors (Week 1)
- [ ] Actor registry implementation
- [ ] `register_as` / `lookup_actor` FFI
- [ ] `send_to` syntax sugar
- [ ] Tests

### Phase 2: Supervision (Weeks 2-3)
- [ ] Supervisor config
- [ ] Child tracking
- [ ] Restart logic
- [ ] Budget enforcement
- [ ] Escalation
- [ ] Tests

### Phase 3: Typed Messages (Week 4)
- [ ] `@messages` annotation
- [ ] Type checking at send
- [ ] Error messages
- [ ] Tests

### Phase 4: Timers (Week 5)
- [ ] Timer wheel
- [ ] Timer thread
- [ ] FFI functions
- [ ] Tests

### Phase 5: Monitoring (Week 6)
- [ ] Monitor tracking
- [ ] Down messages
- [ ] Tests

### Phase 6: Networking (Weeks 7-10)
- [ ] Serialization format
- [ ] TCP transport
- [ ] Remote spawn/connect
- [ ] Failure handling
- [ ] Tests

---

## 9. Testing Strategy

### 9.1 Unit Tests

```coral
*test_named_actor_registration()
    actor is spawn(Echo)
    register_as("echo", actor)
    
    found is lookup_actor("echo")
    assert_not_none(found)
    assert_eq(actor.id, found.id)

*test_supervision_restart()
    supervisor is spawn(TestSupervisor)
    // ... trigger child failure
    // ... verify restart happened
```

### 9.2 Stress Tests

```coral
*test_many_actors()
    actors is []
    for i in 0..10000
        a is spawn(Counter)
        actors.push(a)
    
    for a in actors
        send(a, make_Increment())
    
    // Wait for completion
    // Verify all counters at 1
```

### 9.3 Network Tests

```coral
*test_remote_send()
    // Start test server
    listener is listen_actors("tcp://127.0.0.1:0")
    port is listener.port
    
    // Connect and send
    remote is connect_actor("tcp://127.0.0.1:${port}/echo")
    send(remote, "hello")
    
    // Verify echo received
```

---

## 10. Success Criteria

| Feature | Metric |
|---------|--------|
| Named Actors | Register/lookup < 1μs |
| Supervision | Restart time < 10ms |
| Typed Messages | Compile-time error for wrong type |
| Timers | Accuracy within 10ms |
| Monitoring | Down message within 1ms of death |
| Networking | 10k messages/sec over localhost |
