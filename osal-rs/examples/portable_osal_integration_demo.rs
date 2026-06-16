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
//! The demo uses only `osal_rs::os::*` and avoids Linux-specific APIs
//! (`queue.close()`, `thread.join()` in core logic).  It can be compiled
//! for either the Linux or FreeRTOS backend by toggling the feature flag.
//! The few platform-specific bits (e.g. thread join on Linux) are guarded
//! with `#[cfg(feature = "linux")]`.
//!
//! # Build & Run
//!
//! ```bash
//! cargo run --example portable_osal_integration_demo \
//!     --no-default-features --features linux
//! ```

extern crate alloc;

use core::time::Duration;

use alloc::sync::Arc;

use osal_rs::os::*;
use osal_rs::os::types::{EventBits, TickType, UBaseType};
use osal_rs::utils::Result;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const PACKET_SIZE: usize = 16;
const QUEUE_CAPACITY: usize = 16;
const PRODUCER_COUNT: u32 = 2;
const CONSUMER_COUNT: u32 = 3;
/// Number of tasks that signal the ready semaphore (all except Supervisor).
const TOTAL_READY_TASKS: u32 = PRODUCER_COUNT + CONSUMER_COUNT + 1; // +1 = Monitor
const PRODUCER_PERIOD_TICKS: TickType = 50;
const CONSUMER_PROCESS_TICKS: TickType = 30;
const QUEUE_FETCH_TIMEOUT_TICKS: TickType = 100;
const QUEUE_POST_TIMEOUT_TICKS: TickType = 100;
const DEMO_FIRST_PHASE_TICKS: TickType = 2000;
const DEMO_SECOND_PHASE_TICKS: TickType = 3000;
const MONITOR_WAIT_TICKS: TickType = 2000;
const TIMER_PERIOD_TICKS: TickType = 1000;

// ---------------------------------------------------------------------------
// Event / notification bits
// ---------------------------------------------------------------------------

const START_BIT: EventBits = 1 << 0;
const STOP_BIT: EventBits = 1 << 1;
const ERROR_BIT: EventBits = 1 << 2;
const DONE_BIT: EventBits = 1 << 3;
const TIMER_TICK_BIT: u32 = 1 << 0;

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

struct Stats {
    produced: u32,
    consumed: u32,
    dropped: u32,
    checksum_error: u32,
    queue_timeout: u32,
}

// ---------------------------------------------------------------------------
// Resource bundle passed to every task function
// ---------------------------------------------------------------------------

struct DemoResources {
    queue: Arc<Queue>,
    stats: Arc<Mutex<Stats>>,
    ready_sem: Arc<Semaphore>,
    events: Arc<EventGroup>,
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

fn consumer_task(id: u32, res: Arc<DemoResources>) {
    res.ready_sem.signal();

    if res.events.wait(START_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
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

    // Get our own thread handle to wait for notifications.
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
            println!(
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
// Timer callback
// ---------------------------------------------------------------------------

fn timer_callback(_timer: Box<dyn TimerFn>, param: Option<TimerParam>) -> Result<TimerParam> {
    if let Some(p) = param {
        if let Some(thread) = p.downcast_ref::<Thread>() {
            let _ = thread.notify(ThreadNotification::SetBits(TIMER_TICK_BIT));
        }
        Ok(p)
    } else {
        Ok(alloc::sync::Arc::new(()))
    }
}

// ---------------------------------------------------------------------------
// Supervisor task  — lifecycle controller
// ---------------------------------------------------------------------------

fn supervisor_task(
    res: Arc<DemoResources>,
    heartbeat: Arc<Timer>,
    spawned: Vec<Thread>,
) {
    // Phase 0 — wait for all tasks to signal ready.
    for _ in 0..TOTAL_READY_TASKS {
        res.ready_sem.wait(TickType::MAX);
    }
    println!("[supervisor] all tasks ready, set START_BIT");

    res.events.set(START_BIT);
    heartbeat.start(0);

    // Phase 1 — default timer period (1 000 ms).
    System::delay(DEMO_FIRST_PHASE_TICKS);

    // Change period mid-demo.
    heartbeat.change_period(TIMER_PERIOD_TICKS / 2, 0);
    heartbeat.reset(0);

    // Phase 2 — faster timer period (500 ms).
    System::delay(DEMO_SECOND_PHASE_TICKS);

    // Phase 3 — graceful shutdown.
    println!("[supervisor] set STOP_BIT");
    res.events.set(STOP_BIT);

    heartbeat.stop(0);

    // Print final summary.
    {
        let s = res.stats.lock().unwrap();
        println!("[summary] produced={} consumed={} dropped={} timeout={} checksum_error={}",
            s.produced, s.consumed, s.dropped, s.queue_timeout, s.checksum_error);
        println!("[summary] demo finished");
    }

    res.events.set(DONE_BIT);

    // Linux: join all OS threads for a clean process exit.
    #[cfg(feature = "linux")]
    {
        for h in &spawned {
            let _ = h.join(core::ptr::null_mut());
        }
    }
}

// ---------------------------------------------------------------------------
// main — resource creation, task spawning, then hand-off to Supervisor
// ---------------------------------------------------------------------------

fn main() {
    println!("[init] Portable OSAL Integration Demo (linux backend)");

    // — 1. Create OSAL resources --------------------------------------------

    let queue = Arc::new(
        Queue::new(QUEUE_CAPACITY as UBaseType, PACKET_SIZE as UBaseType)
            .expect("create queue"),
    );
    println!("[init] queue capacity={} message_size={}", QUEUE_CAPACITY, PACKET_SIZE);

    let stats = Arc::new(Mutex::new(Stats {
        produced: 0,
        consumed: 0,
        dropped: 0,
        checksum_error: 0,
        queue_timeout: 0,
    }));
    println!("[init] stats mutex");

    let ready_sem = Arc::new(
        Semaphore::new(TOTAL_READY_TASKS, 0).expect("create ready sem"),
    );
    println!("[init] ready semaphore max_count={}", TOTAL_READY_TASKS);

    let events = Arc::new(EventGroup::new().expect("create event group"));
    println!("[init] event group");

    let res = Arc::new(DemoResources {
        queue,
        stats,
        ready_sem,
        events,
    });

    // — 2. Spawn workers ----------------------------------------------------

    let mut spawned: Vec<Thread> = Vec::new();

    // Producers
    for id in 0..PRODUCER_COUNT {
        let r = Arc::clone(&res);
        let mut t = Thread::new(&alloc::format!("prod-{}", id), 1024, 3);
        let r2 = Arc::clone(&r);
        let s = t
            .spawn_simple(move || {
                let r3 = Arc::clone(&r2);
                producer_task(id, r3)
            })
            .expect("spawn producer");
        println!("[init] producer-{} spawned", id);
        spawned.push(s);
    }

    // Consumers
    for id in 0..CONSUMER_COUNT {
        let r = Arc::clone(&res);
        let mut t = Thread::new(&alloc::format!("cons-{}", id), 1024, 3);
        let r2 = Arc::clone(&r);
        let s = t
            .spawn_simple(move || {
                let r3 = Arc::clone(&r2);
                consumer_task(id, r3)
            })
            .expect("spawn consumer");
        println!("[init] consumer-{} spawned", id);
        spawned.push(s);
    }

    // Monitor
    let r = Arc::clone(&res);
    let mut monitor = Thread::new("monitor", 2048, 2);
    let r2 = Arc::clone(&r);
    let spawned_monitor = monitor
        .spawn_simple(move || {
            let r3 = Arc::clone(&r2);
            monitor_task(r3)
        })
        .expect("spawn monitor");
    println!("[init] monitor spawned");
    spawned.push(spawned_monitor.clone());

    // — 3. Create heartbeat timer — notifies monitor via TimerParam ----------

    let timer_param: TimerParam = Arc::new(spawned_monitor);

    let heartbeat = Arc::new(
        Timer::new_with_to_tick(
            "heartbeat",
            Duration::from_millis(TIMER_PERIOD_TICKS as u64),
            true, // auto-reload
            Some(timer_param),
            timer_callback,
        )
        .expect("create timer"),
    );
    println!("[init] heartbeat timer period={}ms", TIMER_PERIOD_TICKS);

    // — 4. Run supervisor on the main thread --------------------------------

    supervisor_task(res, heartbeat, spawned);

    // Threads are joined inside supervisor_task on Linux.
    println!("[main] demo completed successfully");
}