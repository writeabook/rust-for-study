//! Portable OSAL Multi-Task Pipeline Integration Demo
//!
//! This demo verifies that the seven core OSAL modules (Thread, Queue, Mutex,
//! Semaphore, EventGroup, Timer, System) can work collaboratively under a
//! unified interface.  It simulates an embedded data-processing pipeline:
//!
//! ```text
//!   Producers (×2)  ──post──>  Queue  ──fetch──>  Consumers (×3)
//!                                                     │
//!                                                     ▼
//!                                              Shared Stats (Mutex)
//!
//!   Timer ──notify──> Monitor ──reads──> Stats
//!   Supervisor controls START / STOP via EventGroup
//! ```
//!
//! # Portability
//!
//! The demo separates the **portable core** (task logic, resource creation,
//! synchronisation primitives) from the **platform runner** (entry point,
//! cleanup).  The core uses only `osal_rs::os::*`; no backend-specific
//! APIs appear in the business logic.
//!
//! - **Host runner** (`main` with `cfg(feature = "posix")`):
//!   calls `demo_startup()`, waits for `DONE_BIT`, joins threads, exits.
//! - **FreeRTOS runner** (`freertos_demo_entry` with `cfg(feature = "freertos")`):
//!   calls `demo_startup()`, then `System::start()`.
//!
//! # Build & Run
//!
//! ```bash
//! cargo run --example portable_osal_integration_demo \
//!     --no-default-features --features "posix std"
//! ```

extern crate alloc;

use core::time::Duration;

use alloc::sync::Arc;

use osal_rs::os::types::{EventBits, StackType, TickType, UBaseType};
use osal_rs::os::*;
use osal_rs::utils::Result;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PACKET_SIZE: usize = 16;
const QUEUE_CAPACITY: usize = 128; // large enough for producer head-start
const PRODUCER_COUNT: u32 = 2;
const CONSUMER_COUNT: u32 = 3;
/// Number of tasks that signal the ready semaphore (all except Supervisor).
const TOTAL_READY_TASKS: u32 = PRODUCER_COUNT + CONSUMER_COUNT + 1; // +1 = Monitor
const PRODUCER_HEAD_START_TICKS: TickType = 1000; // producer runs alone for this many ticks
const PRODUCER_PERIOD_TICKS: TickType = 25;
const CONSUMER_PROCESS_TICKS: TickType = 30;
const QUEUE_FETCH_TIMEOUT_TICKS: TickType = 100;
const QUEUE_POST_TIMEOUT_TICKS: TickType = 100;
const DEMO_FIRST_PHASE_TICKS: TickType = 2001;
const DEMO_SECOND_PHASE_TICKS: TickType = 3001;
const MONITOR_WAIT_TICKS: TickType = 2000;
const TIMER_PERIOD_TICKS: TickType = 1000;
const STACK_SIZE: StackType = 1024;

// ---------------------------------------------------------------------------
// Event / notification bits
// ---------------------------------------------------------------------------

const START_BIT: EventBits = 1 << 0;
const STOP_BIT: EventBits = 1 << 1;
const ERROR_BIT: EventBits = 1 << 2;
const DONE_BIT: EventBits = 1 << 3;
const CONSUMER_GO_BIT: EventBits = 1 << 4;
const TIMER_TICK_BIT: u32 = 1 << 0;

// ---------------------------------------------------------------------------
// Portable log macro
// ---------------------------------------------------------------------------

macro_rules! demo_log {
    ($($arg:tt)*) => {
        osal_rs::println!($($arg)*)
    };
}

// ---------------------------------------------------------------------------
// Packet helpers  (16-byte fixed-length message)
// ---------------------------------------------------------------------------

fn build_packet(producer_id: u32, sequence_id: u32, tick: u32) -> [u8; PACKET_SIZE] {
    let checksum = producer_id ^ sequence_id ^ tick;
    let mut buf = [0u8; PACKET_SIZE];
    buf[0..4].copy_from_slice(&producer_id.to_le_bytes());
    buf[4..8].copy_from_slice(&sequence_id.to_le_bytes());
    buf[8..12].copy_from_slice(&tick.to_le_bytes());
    buf[12..16].copy_from_slice(&checksum.to_le_bytes());
    buf
}

fn verify_packet(buf: &[u8; PACKET_SIZE]) -> bool {
    let pid = u32::from_le_bytes(buf[..4].try_into().unwrap());
    let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    let tick = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    let cksum = u32::from_le_bytes(buf[12..16].try_into().unwrap());
    cksum == pid ^ seq ^ tick
}

// ---------------------------------------------------------------------------
// Shared statistics
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    produced: u32,
    consumed: u32,
    dropped: u32,
    checksum_error: u32,
    queue_timeout: u32,
}

// ---------------------------------------------------------------------------
// Resource bundle (portable — shared via Arc)
// ---------------------------------------------------------------------------

struct DemoResources {
    queue: Arc<Queue>,
    stats: Arc<Mutex<Stats>>,
    ready_sem: Arc<Semaphore>,
    events: Arc<EventGroup>,
}

// ---------------------------------------------------------------------------
// DemoApp — handles returned to the platform runner
// ---------------------------------------------------------------------------

pub struct DemoApp {
    pub resources: Arc<DemoResources>,
    pub producer0: Thread,
    pub producer1: Thread,
    pub consumer0: Thread,
    pub consumer1: Thread,
    pub consumer2: Thread,
    pub monitor: Thread,
    pub supervisor: Thread,
    pub heartbeat: Arc<Timer>,
}

// ---------------------------------------------------------------------------
// Producer task
// ---------------------------------------------------------------------------

fn producer_task(id: u32, res: Arc<DemoResources>) {
    res.ready_sem.signal();

    // Wait for START (or immediate STOP).
    if res.events.wait(START_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    let mut seq = 0u32;
    let mut last_wake = System::get_tick_count();

    // — Head-start: produce alone for PRODUCER_HEAD_START_TICKS ticks.
    let start_tick = System::get_tick_count();
    while System::get_tick_count().wrapping_sub(start_tick) < PRODUCER_HEAD_START_TICKS {
        if res.events.get() & STOP_BIT != 0 {
            return;
        }

        let tick = System::get_tick_count();
        let packet = build_packet(id, seq, tick);

        match res.queue.post(&packet, QUEUE_POST_TIMEOUT_TICKS) {
            Ok(_) => {
                let mut s = res.stats.lock().unwrap();
                s.produced += 1;
            }
            Err(_) => {
                let mut s = res.stats.lock().unwrap();
                s.dropped += 1;
            }
        }

        seq = seq.wrapping_add(1);
        System::delay_until(&mut last_wake, PRODUCER_PERIOD_TICKS);
    }

    // Signal consumers that they can start.
    res.events.set(CONSUMER_GO_BIT);

    // — Main loop: keep producing until STOP_BIT is set.
    loop {
        if res.events.get() & STOP_BIT != 0 {
            break;
        }

        let tick = System::get_tick_count();
        let packet = build_packet(id, seq, tick);

        match res.queue.post(&packet, QUEUE_POST_TIMEOUT_TICKS) {
            Ok(_) => {
                let mut s = res.stats.lock().unwrap();
                s.produced += 1;
            }
            Err(_) => {
                let mut s = res.stats.lock().unwrap();
                s.dropped += 1;
            }
        }

        seq = seq.wrapping_add(1);
        System::delay_until(&mut last_wake, PRODUCER_PERIOD_TICKS);
    }
}

// ---------------------------------------------------------------------------
// Consumer task
// ---------------------------------------------------------------------------

fn consumer_task(res: Arc<DemoResources>) {
    res.ready_sem.signal();

    // Wait for START, and then for CONSUMER_GO (or immediate STOP).
    if res.events.wait(START_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    // Wait until producers finish their head-start.
    if res.events.wait(CONSUMER_GO_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    let mut packet = [0u8; PACKET_SIZE];

    loop {
        if res.events.get() & STOP_BIT != 0 {
            break;
        }

        match res.queue.fetch(&mut packet, QUEUE_FETCH_TIMEOUT_TICKS) {
            Ok(_) => {
                let valid = verify_packet(&packet);

                let mut s = res.stats.lock().unwrap();
                s.consumed += 1;

                if !valid {
                    s.checksum_error += 1;
                    res.events.set(ERROR_BIT);
                }
            }
            Err(_) => {
                let mut s = res.stats.lock().unwrap();
                s.queue_timeout += 1;
            }
        }

        System::delay(CONSUMER_PROCESS_TICKS);
    }
}

// ---------------------------------------------------------------------------
// Monitor task  — receives Timer notifications via wait_notification
// ---------------------------------------------------------------------------

fn monitor_task(res: Arc<DemoResources>) {
    res.ready_sem.signal();

    if res.events.wait(START_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    let current = Thread::get_current();

    loop {
        if res.events.get() & STOP_BIT != 0 {
            break;
        }

        let notified = current
            .wait_notification(0, TIMER_TICK_BIT, MONITOR_WAIT_TICKS)
            .unwrap_or(0);

        if notified & TIMER_TICK_BIT != 0 {
            let s = res.stats.lock().unwrap();

            System::enter_critical();
            demo_log!(
                "[monitor] tick={:5} produced={} consumed={} dropped={} timeout={} checksum_error={}",
                System::get_tick_count(),
                s.produced,
                s.consumed,
                s.dropped,
                s.queue_timeout,
                s.checksum_error,
            );
            System::exit_critical();
        }
    }
}

// ---------------------------------------------------------------------------
// Timer callback  — only notifies the Monitor, never blocks
// ---------------------------------------------------------------------------

fn timer_callback(_timer: Box<dyn TimerFn>, param: Option<TimerParam>) -> Result<TimerParam> {
    if let Some(p) = param {
        if let Some(thread) = p.downcast_ref::<Thread>() {
            let _ = thread.notify(ThreadNotification::SetBits(TIMER_TICK_BIT));
        }
        Ok(p)
    } else {
        Ok(Arc::new(()))
    }
}

// ---------------------------------------------------------------------------
// Supervisor task  — lifecycle controller
// ---------------------------------------------------------------------------

fn supervisor_task(res: Arc<DemoResources>, heartbeat: Arc<Timer>) {
    // Phase 0 — wait for all tasks to signal ready.
    for _ in 0..TOTAL_READY_TASKS {
        res.ready_sem.wait(TickType::MAX);
    }

    demo_log!("[supervisor] all tasks ready, set START_BIT");

    res.events.set(START_BIT);
    heartbeat.start(0);

    // Phase 1 — producer head-start.
    System::delay(PRODUCER_HEAD_START_TICKS);

    // Phase 2 — default timer period.
    System::delay(DEMO_FIRST_PHASE_TICKS);

    // Change period mid-demo.
    heartbeat.change_period(TIMER_PERIOD_TICKS / 2, 0);
    heartbeat.reset(0);

    // Phase 3 — faster timer period.
    System::delay(DEMO_SECOND_PHASE_TICKS);

    // Phase 4 — graceful shutdown.
    demo_log!("[supervisor] set STOP_BIT");
    res.events.set(STOP_BIT);

    heartbeat.stop(0);

    // Print final summary.
    {
        let s = res.stats.lock().unwrap();
        demo_log!(
            "[summary] produced={} consumed={} dropped={} timeout={} checksum_error={}",
            s.produced,
            s.consumed,
            s.dropped,
            s.queue_timeout,
            s.checksum_error
        );
        demo_log!("[summary] demo finished");
    }

    res.events.set(DONE_BIT);
}

// ---------------------------------------------------------------------------
// Spawn helpers
// ---------------------------------------------------------------------------

fn spawn_producer(id: u32, res: Arc<DemoResources>) -> Result<Thread> {
    let r = Arc::clone(&res);
    let mut t = Thread::new(&alloc::format!("prod-{}", id), STACK_SIZE, 3);
    let r2 = Arc::clone(&r);
    let spawned = t.spawn_simple(move || {
        let r3 = Arc::clone(&r2);
        producer_task(id, r3)
    })?;
    Ok(spawned)
}

fn spawn_consumer(id: u32, res: Arc<DemoResources>) -> Result<Thread> {
    let r = Arc::clone(&res);
    let mut t = Thread::new(&alloc::format!("cons-{}", id), STACK_SIZE, 3);
    let r2 = Arc::clone(&r);
    let spawned = t.spawn_simple(move || {
        let r3 = Arc::clone(&r2);
        consumer_task(r3)
    })?;
    Ok(spawned)
}

fn spawn_monitor(res: Arc<DemoResources>) -> Result<Thread> {
    let r = Arc::clone(&res);
    let mut t = Thread::new("monitor", STACK_SIZE * 2, 2);
    let r2 = Arc::clone(&r);
    let spawned = t.spawn_simple(move || {
        let r3 = Arc::clone(&r2);
        monitor_task(r3)
    })?;
    Ok(spawned)
}

fn spawn_supervisor(res: Arc<DemoResources>, heartbeat: Arc<Timer>) -> Result<Thread> {
    let r = Arc::clone(&res);
    let h = Arc::clone(&heartbeat);
    let mut t = Thread::new("supervisor", STACK_SIZE, 1);
    let spawned = t.spawn_simple(move || {
        let r3 = Arc::clone(&r);
        let h3 = Arc::clone(&h);
        supervisor_task(r3, h3)
    })?;
    Ok(spawned)
}

// ---------------------------------------------------------------------------
// Timer creation
// ---------------------------------------------------------------------------

fn create_heartbeat_timer(monitor: &Thread) -> Result<Arc<Timer>> {
    let monitor_handle = monitor.clone();
    let timer_param: TimerParam = Arc::new(monitor_handle);

    let timer = Timer::new_with_to_tick(
        "heartbeat",
        Duration::from_millis(TIMER_PERIOD_TICKS as u64),
        true, // auto-reload
        Some(timer_param),
        timer_callback,
    )?;

    Ok(Arc::new(timer))
}

// ---------------------------------------------------------------------------
// Portable startup — called by both POSIX host and FreeRTOS runners
// ---------------------------------------------------------------------------

pub fn demo_startup() -> Result<DemoApp> {
    demo_log!("[init] Portable OSAL Integration Demo");

    // — 1. Create OSAL resources --------------------------------------------

    let queue = Arc::new(
        Queue::new(QUEUE_CAPACITY as UBaseType, PACKET_SIZE as UBaseType)
            .map_err(|_| osal_rs::utils::Error::OutOfMemory)?,
    );
    demo_log!(
        "[init] queue capacity={} message_size={}",
        QUEUE_CAPACITY,
        PACKET_SIZE
    );

    let stats = Arc::new(Mutex::new(Stats::default()));
    demo_log!("[init] stats mutex");

    let ready_sem = Arc::new(
        Semaphore::new(TOTAL_READY_TASKS, 0).map_err(|_| osal_rs::utils::Error::OutOfMemory)?,
    );
    demo_log!("[init] ready semaphore max_count={}", TOTAL_READY_TASKS);

    let events = Arc::new(EventGroup::new().map_err(|_| osal_rs::utils::Error::OutOfMemory)?);
    demo_log!("[init] event group");

    let resources = Arc::new(DemoResources {
        queue,
        stats,
        ready_sem,
        events,
    });

    // — 2. Spawn workers ----------------------------------------------------

    let producer0 = spawn_producer(0, Arc::clone(&resources))?;
    demo_log!("[init] producer-0 spawned");
    let producer1 = spawn_producer(1, Arc::clone(&resources))?;
    demo_log!("[init] producer-1 spawned");

    let consumer0 = spawn_consumer(0, Arc::clone(&resources))?;
    demo_log!("[init] consumer-0 spawned");
    let consumer1 = spawn_consumer(1, Arc::clone(&resources))?;
    demo_log!("[init] consumer-1 spawned");
    let consumer2 = spawn_consumer(2, Arc::clone(&resources))?;
    demo_log!("[init] consumer-2 spawned");

    let monitor = spawn_monitor(Arc::clone(&resources))?;
    demo_log!("[init] monitor spawned");

    // — 3. Create heartbeat timer — notifies monitor via TimerParam ----------

    let heartbeat = create_heartbeat_timer(&monitor)?;
    demo_log!("[init] heartbeat timer period={}ms", TIMER_PERIOD_TICKS);

    // — 4. Spawn Supervisor (OSAL Thread, not main-thread function) ----------

    let supervisor = spawn_supervisor(Arc::clone(&resources), Arc::clone(&heartbeat))?;
    demo_log!("[init] supervisor spawned");

    Ok(DemoApp {
        resources,
        producer0,
        producer1,
        consumer0,
        consumer1,
        consumer2,
        monitor,
        supervisor,
        heartbeat,
    })
}

// ===========================================================================
// Platform runners
// ===========================================================================

/// Host runner (POSIX backend on Linux host) — calls `demo_startup()`,
/// waits for `DONE_BIT`, joins all threads, and exits cleanly.
#[cfg(feature = "posix")]
fn main() -> Result<()> {
    demo_log!("[main] run portable demo on posix backend");

    let app = demo_startup()?;

    // Wait for the Supervisor to signal demo completion.
    app.resources.events.wait(DONE_BIT, TickType::MAX);

    // POSIX host threads require join() to reclaim thread resources.
    app.producer0.join(core::ptr::null_mut())?;
    app.producer1.join(core::ptr::null_mut())?;
    app.consumer0.join(core::ptr::null_mut())?;
    app.consumer1.join(core::ptr::null_mut())?;
    app.consumer2.join(core::ptr::null_mut())?;
    app.monitor.join(core::ptr::null_mut())?;
    app.supervisor.join(core::ptr::null_mut())?;

    demo_log!("[main] demo completed successfully");

    Ok(())
}

/// FreeRTOS entry point — calls `demo_startup()`, then starts the
/// scheduler.  Under FreeRTOS `System::start()` does not return.
#[cfg(feature = "freertos")]
pub fn freertos_demo_entry() -> Result<()> {
    demo_log!("[main] run portable demo on freertos backend");

    let _app = demo_startup()?;

    // Start the FreeRTOS scheduler.  This call never returns.
    System::start();

    Ok(())
}
