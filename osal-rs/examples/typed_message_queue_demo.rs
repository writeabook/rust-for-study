//! Typed Message Queue Pipeline Demo
//!
//! This demo is a direct copy of `portable_osal_integration_demo.rs` with
//! exactly one change: the raw byte-packing `Queue` (`[u8; 16]`) is replaced
//! by `QueueStreamed<SensorPacket>` with `osal-rs-serde` serialisation.
//! All other logic — multi-producer / multi-consumer, supervisor lifecycle,
//! timer-to-monitor notification, head-start phase, mid-demo period change,
//! graceful shutdown — is left unchanged.
//!
//! ```text
//!   Producers (×2)  ──post──>  QueueStreamed<SensorPacket>  ──fetch──>  Consumers (×3)
//!                                                                             │
//!                                                                             ▼
//!                                                                      Shared Stats (Mutex)
//!
//!   Timer ──notify──> Monitor ──reads──> Stats
//!   Supervisor controls START / STOP via EventGroup
//! ```
//!
//! # Build & Run
//!
//! ```bash
//! # Linux backend  — `linux` implies `std`, no extra `std` needed.
//! cargo run -p osal-rs --example typed_message_queue_demo \
//!     --no-default-features --features "linux serde"
//!
//! # POSIX backend — `posix` is no_std and provides its own panic handler /
//! # allocator.  The example binary uses std as its runtime, so `std` must be
//! # enabled to hand those responsibilities back to the host toolchain and
//! # avoid a duplicate `panic_impl` lang-item conflict.
//! cargo run -p osal-rs --example typed_message_queue_demo \
//!     --no-default-features --features "posix std serde"
//! ```

extern crate alloc;

use core::time::Duration;

use alloc::sync::Arc;

use osal_rs::os::types::{EventBits, StackType, TickType, UBaseType};
use osal_rs::os::*;
use osal_rs::utils::Result;
use osal_rs_serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Constants  (identical to portable_osal_integration_demo.rs)
// ---------------------------------------------------------------------------

const MESSAGE_SIZE: usize = 20; // SensorPacket = 5 × u32
const QUEUE_CAPACITY: usize = 128;
const PRODUCER_COUNT: u32 = 2;
const CONSUMER_COUNT: u32 = 3;
const TOTAL_READY_TASKS: u32 = PRODUCER_COUNT + CONSUMER_COUNT + 1; // +1 = Monitor
const PRODUCER_HEAD_START_TICKS: TickType = 1000;
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
// SensorPacket — replaces the old `[u8; 16]` + manual pack/verify helpers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default, Debug, PartialEq, Serialize, Deserialize)]
struct SensorPacket {
    producer_id: u32,
    sequence_id: u32,
    tick: u32,
    value: u32,
    checksum: u32,
}

impl BytesHasLen for SensorPacket {
    fn len(&self) -> usize {
        MESSAGE_SIZE
    }
}

impl SensorPacket {
    fn new(producer_id: u32, sequence_id: u32, tick: u32) -> Self {
        let value = tick.wrapping_mul(producer_id.wrapping_add(1));
        let checksum = producer_id ^ sequence_id ^ tick ^ value;
        Self {
            producer_id,
            sequence_id,
            tick,
            value,
            checksum,
        }
    }

    fn is_valid(&self) -> bool {
        let expected =
            self.producer_id ^ self.sequence_id ^ self.tick ^ self.value;
        self.checksum == expected
    }
}

// ---------------------------------------------------------------------------
// Shared statistics  (same fields as original)
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
// Resource bundle  — only change: Queue → QueueStreamed<SensorPacket>
// ---------------------------------------------------------------------------

struct DemoResources {
    queue: Arc<QueueStreamed<SensorPacket>>,
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
// Producer task  — only change: build_packet → SensorPacket::new
// ---------------------------------------------------------------------------

fn producer_task(id: u32, res: Arc<DemoResources>) {
    res.ready_sem.signal();

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
        let packet = SensorPacket::new(id, seq, tick);

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
        let packet = SensorPacket::new(id, seq, tick);

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
// Consumer task  — only change: verify_packet → SensorPacket::is_valid
// ---------------------------------------------------------------------------

fn consumer_task(res: Arc<DemoResources>) {
    res.ready_sem.signal();

    if res.events.wait(START_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    // Wait until producers finish their head-start.
    if res.events.wait(CONSUMER_GO_BIT | STOP_BIT, TickType::MAX) & STOP_BIT != 0 {
        return;
    }

    let mut packet = SensorPacket::default();

    loop {
        if res.events.get() & STOP_BIT != 0 {
            break;
        }

        match res.queue.fetch(&mut packet, QUEUE_FETCH_TIMEOUT_TICKS) {
            Ok(_) => {
                let valid = packet.is_valid();

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
// Monitor task  — identical to original
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
// Timer callback  — identical to original
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
// Supervisor task  — identical to original
// ---------------------------------------------------------------------------

fn supervisor_task(res: Arc<DemoResources>, heartbeat: Arc<Timer>) {
    for _ in 0..TOTAL_READY_TASKS {
        res.ready_sem.wait(TickType::MAX);
    }

    demo_log!("[supervisor] all tasks ready, set START_BIT");

    res.events.set(START_BIT);
    heartbeat.start(0);

    System::delay(PRODUCER_HEAD_START_TICKS);
    System::delay(DEMO_FIRST_PHASE_TICKS);

    heartbeat.change_period(TIMER_PERIOD_TICKS / 2, 0);
    heartbeat.reset(0);

    System::delay(DEMO_SECOND_PHASE_TICKS);

    demo_log!("[supervisor] set STOP_BIT");
    res.events.set(STOP_BIT);

    heartbeat.stop(0);

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
// Spawn helpers  — identical to original
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
// Timer creation  — identical to original
// ---------------------------------------------------------------------------

fn create_heartbeat_timer(monitor: &Thread) -> Result<Arc<Timer>> {
    let monitor_handle = monitor.clone();
    let timer_param: TimerParam = Arc::new(monitor_handle);

    let timer = Timer::new_with_to_tick(
        "heartbeat",
        Duration::from_millis(TIMER_PERIOD_TICKS as u64),
        true,
        Some(timer_param),
        timer_callback,
    )?;

    Ok(Arc::new(timer))
}

// ---------------------------------------------------------------------------
// Portable startup  — only change: Queue → QueueStreamed<SensorPacket>
// ---------------------------------------------------------------------------

pub fn demo_startup() -> Result<DemoApp> {
    demo_log!("[init] Typed Message Queue Pipeline Demo");

    let queue = Arc::new(QueueStreamed::<SensorPacket>::new(
        QUEUE_CAPACITY as UBaseType,
        MESSAGE_SIZE as UBaseType,
    )?);
    demo_log!(
        "[init] QueueStreamed<SensorPacket> capacity={} message_size={}",
        QUEUE_CAPACITY,
        MESSAGE_SIZE
    );

    let stats = Arc::new(Mutex::new(Stats::default()));
    demo_log!("[init] stats mutex");

    let ready_sem = Arc::new(Semaphore::new(TOTAL_READY_TASKS, 0)?);
    demo_log!("[init] ready semaphore max_count={}", TOTAL_READY_TASKS);

    let events = Arc::new(EventGroup::new()?);
    demo_log!("[init] event group");

    let resources = Arc::new(DemoResources {
        queue,
        stats,
        ready_sem,
        events,
    });

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

    let heartbeat = create_heartbeat_timer(&monitor)?;
    demo_log!("[init] heartbeat timer period={}ms", TIMER_PERIOD_TICKS);

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

/// Host runner — identical to original.
#[cfg(any(feature = "linux", feature = "posix"))]
fn main() -> Result<()> {
    #[cfg(feature = "linux")]
    demo_log!("[main] run typed message queue demo on linux backend");

    #[cfg(feature = "posix")]
    demo_log!("[main] run typed message queue demo on posix backend");

    let app = demo_startup()?;

    app.resources.events.wait(DONE_BIT, TickType::MAX);

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

/// FreeRTOS entry point — identical to original.
#[cfg(feature = "freertos")]
pub fn freertos_demo_entry() -> Result<()> {
    demo_log!("[main] run typed message queue demo on freertos backend");

    let _app = demo_startup()?;

    System::start();

    Ok(())
}
