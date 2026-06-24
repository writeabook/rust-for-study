//! Linux legacy backend smoke tests.
//!
//! These tests exercise Linux-backend-specific behaviours that are not
//! part of the portable OSAL API contract. They are transitional and may
//! be removed or consolidated once the POSIX backend fully covers host
//! functionality.
//!
//! Each section was migrated from the previous `linux/` submodule files.

use osal_rs::os::*;
use osal_rs::utils::Result;


mod linux_legacy_event_group_tests {
//! Linux-specific event group tests.
//!
//! These tests supplement the common event group test suite with tests
//! that exercise Linux-backend-specific behaviours: unique handles,
//! non-blocking ISR paths, infinite wait, reserved-bit masking,
//! and signal-wake semantics.

use osal_rs::os::types::TickType;
use osal_rs::os::*;
use osal_rs::utils::Result;

/// Entry-point called from `mod.rs` to run all Linux-specific
/// event group tests.
pub fn run_all_tests() -> Result<()> {
    event_group_handles_are_unique()?;
    event_group_wait_zero_is_non_blocking()?;
    event_group_wait_max_blocks_until_set()?;
    event_group_finite_wait_wakes_before_timeout()?;
    event_group_isr_paths_are_non_blocking()?;
    event_group_reserved_bits_are_masked()?;
    event_group_clear_reserved_bits_is_noop()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handle uniqueness
// ---------------------------------------------------------------------------

/// Two new event group objects must have distinct handles.
fn event_group_handles_are_unique() -> Result<()> {
    let e1 = EventGroup::new()?;
    let e2 = EventGroup::new()?;

    assert_ne!(*e1, *e2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-blocking wait(0)
// ---------------------------------------------------------------------------

/// `wait(mask, 0)` must return immediately when no bits are set.
fn event_group_wait_zero_is_non_blocking() -> Result<()> {
    let events = EventGroup::new()?;

    let result = events.wait(0b0001, 0 as TickType);

    // No bits were set; bit 0 should be 0.
    assert_eq!(result & 0b0001, 0);
    Ok(())
}

// ---------------------------------------------------------------------------
// Infinite wait with wake-up
// ---------------------------------------------------------------------------

/// Thread A blocks on `wait(mask, TickType::MAX)`.  Thread B sets a
/// matching bit and Thread A wakes up successfully.
///
/// Uses a `Barrier` to ensure the waiter thread has truly begun
/// blocking before the signal is sent, making the test rigorous.
fn event_group_wait_max_blocks_until_set() -> Result<()> {
    use std::sync::{Arc, Barrier};

    let barrier = Arc::new(Barrier::new(2));
    let events = Arc::new(EventGroup::new()?);
    let waiter_events = Arc::clone(&events);
    let waiter_barrier = Arc::clone(&barrier);

    let handle = std::thread::spawn(move || {
        waiter_barrier.wait(); // signal: waiter about to enter wait()
        waiter_events.wait(0b0010, TickType::MAX)
    });

    barrier.wait(); // both threads ready; waiter will now block
    // Brief yield to let the waiter thread acquire the lock and block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(10));

    events.set(0b0010);

    let result = handle.join().unwrap();
    assert_ne!(result & 0b0010, 0);
    Ok(())
}

// ---------------------------------------------------------------------------
// Finite wait with successful wake-up
// ---------------------------------------------------------------------------

/// Thread A blocks on `wait(mask, finite_ticks)`.  Thread B sets a
/// matching bit before the timeout expires, and Thread A wakes up
/// successfully (exercises the `wait_timeout` wake-up path).
fn event_group_finite_wait_wakes_before_timeout() -> Result<()> {
    use std::sync::Arc;

    let events = Arc::new(EventGroup::new()?);
    let waiter_events = Arc::clone(&events);

    let handle = std::thread::spawn(move || waiter_events.wait(0b0100, 200 as TickType));

    // Give the waiter time to block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(20));
    events.set(0b0100);

    let result = handle.join().unwrap();
    assert_ne!(result & 0b0100, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation paths
// ---------------------------------------------------------------------------

/// `set_from_isr()`, `get_from_isr()`, and `clear_from_isr()` must be
/// non-blocking and follow correct bit-manipulation semantics.
fn event_group_isr_paths_are_non_blocking() -> Result<()> {
    let events = EventGroup::new()?;

    // Set a bit from ISR.
    assert!(events.set_from_isr(0b0001).is_ok());
    // Read it back.
    assert_ne!(events.get_from_isr() & 0b0001, 0);

    // Clear it from ISR.
    assert!(events.clear_from_isr(0b0001).is_ok());
    // Should be gone.
    assert_eq!(events.get_from_isr() & 0b0001, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// Reserved bit masking
// ---------------------------------------------------------------------------

/// Reserved bits (above `MAX_MASK`) must be silently ignored by `set()`,
/// `get()`, and `wait()`.
fn event_group_reserved_bits_are_masked() -> Result<()> {
    let events = EventGroup::new()?;

    let reserved_bits = !EventGroup::MAX_MASK;

    // Setting reserved bits must be a no-op for those bits.
    let result = events.set(reserved_bits);
    assert_eq!(result & reserved_bits, 0);

    // get() must not expose reserved bits.
    assert_eq!(events.get() & reserved_bits, 0);

    Ok(())
}

// ---------------------------------------------------------------------------
// Clearing reserved bits is a no-op
// ---------------------------------------------------------------------------

/// Clearing reserved bits must not affect usable bits.
fn event_group_clear_reserved_bits_is_noop() -> Result<()> {
    let events = EventGroup::new()?;

    // Set some usable bits.
    events.set(0b1111);

    // Clear reserved bits — usable bits must stay.
    let reserved_bits = !EventGroup::MAX_MASK;
    let result = events.clear(reserved_bits);

    // The usable bits (0b1111) must still be set.
    assert_eq!(result & 0b1111, 0b1111);

    Ok(())
}
}

#[test]
fn event_group_tests_smoke() {
    linux_legacy_event_group_tests::run_all_tests().unwrap();
}

mod linux_legacy_mutex_tests {
//! Linux-specific mutex tests.
//!
//! These tests exercise behaviours that are specific to the Linux backend
//! (std::sync::Mutex poisoning, std::thread-based contention, ISR host
//! simulation) and are therefore **not** part of the cross-backend common
//! test suite.


use alloc::sync::Arc;
use std::thread;

use osal_rs::os::*;
use osal_rs::utils::Result;
use osal_rs::{log_debug, log_info};

const TAG: &str = "LinuxMutexTests";

pub fn test_mutex_multi_thread_contention() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_multi_thread_contention");

    let mutex = Arc::new(Mutex::new(0u32));
    const THREADS: usize = 8;
    const ITERS: u32 = 10_000;

    let handles: Vec<_> = (0..THREADS)
        .map(|_| {
            let m = Arc::clone(&mutex);
            thread::spawn(move || {
                for _ in 0..ITERS {
                    let mut guard = m.lock().unwrap();
                    *guard += 1;
                }
            })
        })
        .collect();

    for h in handles {
        h.join().unwrap();
    }

    let final_val = *mutex.lock().unwrap();
    assert_eq!(
        final_val,
        THREADS as u32 * ITERS,
        "multi-thread contention: expected {}, got {}",
        THREADS as u32 * ITERS,
        final_val
    );

    log_info!(TAG, "test_mutex_multi_thread_contention PASSED");
    Ok(())
}

pub fn test_mutex_poison_recovery() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_poison_recovery");

    let mutex = Arc::new(Mutex::new(0u32));

    // Panic while holding the lock — poisons the inner StdMutex
    let m = Arc::clone(&mutex);
    let handle = thread::spawn(move || {
        let _guard = m.lock().unwrap();
        panic!("intentional panic to poison the mutex");
    });
    let _ = handle.join(); // ignore poison panic

    // After recovery the mutex must still be usable
    let guard = mutex.lock();
    assert!(
        guard.is_ok(),
        "mutex must be lockable after poison recovery"
    );
    assert_eq!(*guard.unwrap(), 0, "guarded data must be intact");

    log_info!(TAG, "test_mutex_poison_recovery PASSED");
    Ok(())
}

pub fn test_mutex_isr_path() -> Result<()> {
    log_info!(TAG, "Starting test_mutex_isr_path");

    let mutex = Mutex::new(99u32);

    // 1. Immediate success when free
    {
        let guard = mutex.lock_from_isr();
        assert!(guard.is_ok(), "ISR lock must succeed when mutex is free");
        assert_eq!(*guard.unwrap(), 99);
    }

    // 2. Immediate failure when occupied
    {
        let _guard = mutex.lock()?;
        let result = mutex.lock_from_isr();
        assert!(result.is_err(), "ISR lock must fail when mutex is held");
    }

    // 3. Normal lock() succeeds after ISR guard drop
    {
        let guard = mutex.lock();
        assert!(guard.is_ok(), "normal lock must succeed after guard drop");
    }

    // 4. lock_from_isr_explicit is callable
    {
        let guard = mutex.lock_from_isr_explicit();
        assert!(guard.is_ok(), "ISR explicit lock must succeed");
    }

    // 5. Poisoned data lock recovery (ISR path)
    let poisoned = Arc::new(Mutex::new(0u32));
    let p = Arc::clone(&poisoned);
    let panic_handle = thread::spawn(move || {
        let _g = p.lock().unwrap();
        panic!("poison for ISR test");
    });
    let _ = panic_handle.join();

    {
        let guard = poisoned.lock_from_isr();
        assert!(guard.is_ok(), "ISR lock must recover from poison");
        assert_eq!(*guard.unwrap(), 0);
    }

    log_info!(TAG, "test_mutex_isr_path PASSED");
    Ok(())
}

pub fn test_raw_mutex_handles_are_unique() -> Result<()> {
    log_info!(TAG, "test_raw_mutex_handles_are_unique");
    let m1 = RawMutex::new()?;
    let m2 = RawMutex::new()?;
    assert_ne!(*m1, *m2, "RawMutex handles must differ");
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn run_all_tests() -> Result<()> {
    log_info!(
        TAG,
        "========== Running Linux-Specific Mutex Tests =========="
    );
    test_mutex_multi_thread_contention()?;
    test_mutex_poison_recovery()?;
    test_mutex_isr_path()?;
    test_raw_mutex_handles_are_unique()?;
    log_info!(
        TAG,
        "========== All Linux-Specific Mutex Tests PASSED =========="
    );
    Ok(())
}
}

#[test]
fn mutex_tests_smoke() {
    linux_legacy_mutex_tests::run_all_tests().unwrap();
}

mod linux_legacy_queue_tests {
//! Linux-specific queue tests.
//!
//! These tests verify the fixed-length message contract, dual-Condvar
//! liveness, Max-delay semantics, close lifecycle, poison recovery,
//! and typed QueueStreamed round-trip correctness.  They are **not**
//! part of the cross-backend common test suite.


use alloc::collections::BTreeSet;
use alloc::sync::Arc;
use core::time::Duration;
use std::sync::Mutex as StdMutex;

use osal_rs::log_info;
use osal_rs::os::types::TickType;
use osal_rs::os::*;
use osal_rs::utils::{Error, Result};

#[cfg(not(feature = "serde"))]
use osal_rs::os::{Deserialize as OsalDeserialize, Serialize as OsalSerialize};

#[cfg(feature = "serde")]
use osal_rs_serde::{Deserialize as OsalDeserialize, Serialize as OsalSerialize};

use osal_rs::os::BytesHasLen;

const TAG: &str = "LinuxQueueTests";

// ===========================================================================
// Test type for non-serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TestMessage {
    bytes: [u8; 6],
}

#[cfg(not(feature = "serde"))]
impl TestMessage {
    fn new(id: u32, value: i16) -> Self {
        let mut bytes = [0u8; 6];
        bytes[..4].copy_from_slice(&id.to_le_bytes());
        bytes[4..].copy_from_slice(&value.to_le_bytes());
        Self { bytes }
    }
    fn id(&self) -> u32 {
        u32::from_le_bytes([self.bytes[0], self.bytes[1], self.bytes[2], self.bytes[3]])
    }
    fn value(&self) -> i16 {
        i16::from_le_bytes([self.bytes[4], self.bytes[5]])
    }
}

#[cfg(not(feature = "serde"))]
impl BytesHasLen for TestMessage {
    fn len(&self) -> usize {
        self.bytes.len()
    }
}

#[cfg(not(feature = "serde"))]
impl OsalSerialize for TestMessage {
    fn to_bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for TestMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 6] = bytes.try_into().map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self { bytes: arr })
    }
}

// ===========================================================================
// Test type for serde QueueStreamed round-trip tests
// ===========================================================================

#[cfg(feature = "serde")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, OsalSerialize, OsalDeserialize)]
struct SerdeTestMessage {
    id: u32,
    value: i16,
}

#[cfg(feature = "serde")]
impl BytesHasLen for SerdeTestMessage {
    fn len(&self) -> usize {
        6
    }
}

// ===========================================================================
// Broken test type
// ===========================================================================

#[cfg(not(feature = "serde"))]
#[derive(Debug, Clone, Copy)]
struct BrokenMessage([u8; 8]);

#[cfg(not(feature = "serde"))]
impl BytesHasLen for BrokenMessage {
    fn len(&self) -> usize {
        6
    }
}

#[cfg(not(feature = "serde"))]
impl OsalSerialize for BrokenMessage {
    fn to_bytes(&self) -> &[u8] {
        &self.0
    }
}

#[cfg(not(feature = "serde"))]
impl OsalDeserialize for BrokenMessage {
    fn from_bytes(bytes: &[u8]) -> Result<Self> {
        let arr: [u8; 8] = bytes.try_into().map_err(|_| Error::InvalidMessageSize)?;
        Ok(Self(arr))
    }
}

// ===========================================================================
// Phase 2: Multi-producer/consumer — sentinel-based, uniqueness-verified
// ===========================================================================

pub fn test_queue_multi_producer_consumer() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_multi_producer_consumer");

    const PRODUCERS: usize = 4;
    const CONSUMERS: usize = 4;
    const ITERS: u32 = 250;
    const STOP: u64 = u64::MAX; // sentinel

    let queue = Arc::new(Queue::new(4, 8)?);
    let received = Arc::new(StdMutex::new(BTreeSet::new()));

    // Producers
    let mut producers = vec![];
    for pid in 0..PRODUCERS {
        let q = Arc::clone(&queue);
        producers.push(thread::spawn(move || {
            for seq in 0..ITERS {
                let msg: u64 = ((pid as u64) << 32) | (seq as u64);
                q.post(&msg.to_le_bytes(), TickType::MAX).unwrap();
            }
        }));
    }

    // Consumers — spawned first so they drain the queue concurrently
    let mut consumers = vec![];
    for _ in 0..CONSUMERS {
        let q = Arc::clone(&queue);
        let r = Arc::clone(&received);
        consumers.push(thread::spawn(move || {
            let mut buf = [0u8; 8];
            loop {
                match q.fetch(&mut buf, TickType::MAX) {
                    Ok(()) => {
                        let val = u64::from_le_bytes(buf);
                        if val == STOP {
                            break;
                        }
                        r.lock().unwrap().insert(val);
                    }
                    Err(Error::QueueClosed) => break,
                    Err(_) => break,
                }
            }
        }));
    }

    for p in producers {
        p.join().unwrap();
    }

    // Send sentinels after producers finish, consumers are already draining
    for _ in 0..CONSUMERS {
        queue.post(&STOP.to_le_bytes(), TickType::MAX)?;
    }

    for c in consumers {
        c.join().unwrap();
    }
    queue.close();

    let received = received.lock().unwrap();
    let total = received.len();

    // Verify total
    let expected_total = PRODUCERS * ITERS as usize;
    assert_eq!(
        total, expected_total,
        "expected {} messages, got {}",
        expected_total, total
    );

    // Verify per-producer each sequence is present (uniqueness)
    for pid in 0..PRODUCERS {
        for seq in 0..ITERS {
            let msg: u64 = ((pid as u64) << 32) | (seq as u64);
            assert!(
                received.contains(&msg),
                "missing message (pid={}, seq={})",
                pid,
                seq
            );
        }
    }

    log_info!(TAG, "test_queue_multi_producer_consumer PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Indefinite wait wake-up tests
// ===========================================================================

pub fn test_queue_max_fetch_woken_by_post() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_max_fetch_woken_by_post");
    let queue = Arc::new(Queue::new(1, 4)?);

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4];
        q.fetch(&mut buf, TickType::MAX)
    });

    thread::sleep(Duration::from_millis(20));
    queue.post(&[1, 2, 3, 4], 0)?;

    let result = handle.join().unwrap();
    assert_eq!(result, Ok(()));
    log_info!(TAG, "test_queue_max_fetch_woken_by_post PASSED");
    Ok(())
}

pub fn test_queue_max_post_woken_by_fetch() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_max_post_woken_by_fetch");
    let queue = Arc::new(Queue::new(1, 4)?);
    queue.post(&[9, 8, 7, 6], 0)?;

    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || q.post(&[1, 2, 3, 4], TickType::MAX));

    thread::sleep(Duration::from_millis(20));
    let mut buf = [0u8; 4];
    queue.fetch(&mut buf, 0)?; // drain
    assert_eq!(handle.join().unwrap(), Ok(()));
    log_info!(TAG, "test_queue_max_post_woken_by_fetch PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Finite timeout
// ===========================================================================

pub fn test_queue_fetch_timeout() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_timeout");
    let queue = Queue::new(1, 4)?;
    let mut buf = [0u8; 4];
    let start = std::time::Instant::now();
    assert_eq!(queue.fetch(&mut buf, 50), Err(Error::Timeout));
    assert!(start.elapsed() >= Duration::from_millis(40)); // ~50ms
    log_info!(TAG, "test_queue_fetch_timeout PASSED");
    Ok(())
}

pub fn test_queue_post_timeout() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_timeout");
    let queue = Queue::new(1, 4)?;
    queue.post(&[1, 2, 3, 4], 0)?;
    let start = std::time::Instant::now();
    assert_eq!(queue.post(&[2, 3, 4, 5], 50), Err(Error::Timeout));
    assert!(start.elapsed() >= Duration::from_millis(40));
    log_info!(TAG, "test_queue_post_timeout PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Close lifecycle
// ===========================================================================

pub fn test_queue_close_wakes_blocked_consumer() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_consumer");
    let queue = Arc::new(Queue::new(1, 4)?);
    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || {
        let mut buf = [0u8; 4];
        q.fetch(&mut buf, TickType::MAX)
    });
    thread::sleep(Duration::from_millis(10));
    queue.close();
    assert_eq!(handle.join().unwrap(), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_wakes_blocked_consumer PASSED");
    Ok(())
}

pub fn test_queue_close_wakes_blocked_producer() -> Result<()> {
    use std::thread;
    log_info!(TAG, "Starting test_queue_close_wakes_blocked_producer");
    let queue = Arc::new(Queue::new(1, 4)?);
    queue.post(&[1u8; 4], 0)?;
    let q = Arc::clone(&queue);
    let handle = thread::spawn(move || q.post(&[2u8; 4], TickType::MAX));
    thread::sleep(Duration::from_millis(10));
    queue.close();
    assert_eq!(handle.join().unwrap(), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_wakes_blocked_producer PASSED");
    Ok(())
}

pub fn test_queue_all_ops_fail_after_close() -> Result<()> {
    log_info!(TAG, "Starting test_queue_all_ops_fail_after_close");
    let queue = Queue::new(2, 4)?;
    queue.post(&[1u8; 4], 0)?;
    queue.close();
    assert_eq!(queue.post(&[2u8; 4], 0), Err(Error::QueueClosed));
    assert_eq!(queue.fetch(&mut [0u8; 4], 0), Err(Error::QueueClosed));
    assert_eq!(queue.post_from_isr(&[3u8; 4]), Err(Error::QueueClosed));
    assert_eq!(queue.fetch_from_isr(&mut [0u8; 4]), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_all_ops_fail_after_close PASSED");
    Ok(())
}

pub fn test_queue_close_idempotent() -> Result<()> {
    log_info!(TAG, "Starting test_queue_close_idempotent");
    let queue = Queue::new(1, 4)?;
    queue.close();
    queue.close();
    queue.close();
    assert_eq!(queue.post(&[1u8; 4], 0), Err(Error::QueueClosed));
    log_info!(TAG, "test_queue_close_idempotent PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_closed() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_closed");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 6)?;
    queue.close();
    let msg = TestMessage::new(1, 2);
    assert_eq!(queue.post(&msg, 0), Err(Error::QueueClosed));
    assert_eq!(
        queue.fetch(&mut TestMessage::new(0, 0), 0),
        Err(Error::QueueClosed)
    );
    log_info!(TAG, "test_queue_streamed_closed PASSED");
    Ok(())
}

// ===========================================================================
// Phase 2: Handle
// ===========================================================================

pub fn test_queue_unique_handles() -> Result<()> {
    log_info!(TAG, "Starting test_queue_unique_handles");
    let q1 = Queue::new(1, 4)?;
    let q2 = Queue::new(1, 4)?;
    assert_ne!(*q1, *q2, "different queues must have different handles");
    log_info!(TAG, "test_queue_unique_handles PASSED");
    Ok(())
}

// ===========================================================================
// Raw Queue contract tests (Phase 1)
// ===========================================================================

pub fn test_queue_exact_message_size() -> Result<()> {
    log_info!(TAG, "Starting test_queue_exact_message_size");
    let queue = Queue::new(2, 4)?;
    queue.post(&[1, 2, 3, 4], 0)?;
    let mut received = [0u8; 4];
    queue.fetch(&mut received, 0)?;
    assert_eq!(received, [1, 2, 3, 4]);
    log_info!(TAG, "test_queue_exact_message_size PASSED");
    Ok(())
}

pub fn test_queue_post_too_short_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_short_rejected");
    let queue = Queue::new(2, 4)?;
    assert_eq!(queue.post(&[1, 2, 3], 0), Err(Error::InvalidMessageSize));
    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));
    log_info!(TAG, "test_queue_post_too_short_rejected PASSED");
    Ok(())
}

pub fn test_queue_post_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_post_too_long_rejected");
    let queue = Queue::new(2, 4)?;
    assert_eq!(
        queue.post(&[1, 2, 3, 4, 5], 0),
        Err(Error::InvalidMessageSize)
    );
    let mut received = [0u8; 4];
    assert_eq!(queue.fetch(&mut received, 0), Err(Error::Timeout));
    log_info!(TAG, "test_queue_post_too_long_rejected PASSED");
    Ok(())
}

pub fn test_queue_fetch_buffer_too_short_does_not_consume() -> Result<()> {
    log_info!(
        TAG,
        "Starting test_queue_fetch_buffer_too_short_does_not_consume"
    );
    let queue = Queue::new(2, 4)?;
    queue.post(&[10, 20, 30, 40], 0)?;
    assert_eq!(
        queue.fetch(&mut [0u8; 3], 0),
        Err(Error::InvalidMessageSize)
    );
    let mut correct_buf = [0u8; 4];
    queue.fetch(&mut correct_buf, 0)?;
    assert_eq!(correct_buf, [10, 20, 30, 40]);
    log_info!(
        TAG,
        "test_queue_fetch_buffer_too_short_does_not_consume PASSED"
    );
    Ok(())
}

pub fn test_queue_fetch_buffer_too_long_rejected() -> Result<()> {
    log_info!(TAG, "Starting test_queue_fetch_buffer_too_long_rejected");
    let queue = Queue::new(2, 4)?;
    queue.post(&[1, 2, 3, 4], 0)?;
    assert_eq!(
        queue.fetch(&mut [0u8; 5], 0),
        Err(Error::InvalidMessageSize)
    );
    let mut correct_buf = [0u8; 4];
    queue.fetch(&mut correct_buf, 0)?;
    assert_eq!(correct_buf, [1, 2, 3, 4]);
    log_info!(TAG, "test_queue_fetch_buffer_too_long_rejected PASSED");
    Ok(())
}

pub fn test_queue_isr_post_too_short() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_post_too_short");
    let queue = Queue::new(2, 4)?;
    assert_eq!(queue.post_from_isr(&[1, 2]), Err(Error::InvalidMessageSize));
    let mut buf = [0u8; 4];
    assert_eq!(queue.fetch_from_isr(&mut buf), Err(Error::Timeout));
    log_info!(TAG, "test_queue_isr_post_too_short PASSED");
    Ok(())
}

pub fn test_queue_isr_post_too_long() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_post_too_long");
    let queue = Queue::new(2, 4)?;
    assert_eq!(
        queue.post_from_isr(&[1, 2, 3, 4, 5, 6]),
        Err(Error::InvalidMessageSize)
    );
    let mut buf = [0u8; 4];
    assert_eq!(queue.fetch_from_isr(&mut buf), Err(Error::Timeout));
    log_info!(TAG, "test_queue_isr_post_too_long PASSED");
    Ok(())
}

pub fn test_queue_isr_fetch_buffer_too_short() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_fetch_buffer_too_short");
    let queue = Queue::new(2, 4)?;
    queue.post_from_isr(&[7, 8, 9, 10])?;
    assert_eq!(
        queue.fetch_from_isr(&mut [0u8; 2]),
        Err(Error::InvalidMessageSize)
    );
    let mut correct_buf = [0u8; 4];
    queue.fetch_from_isr(&mut correct_buf)?;
    assert_eq!(correct_buf, [7, 8, 9, 10]);
    log_info!(TAG, "test_queue_isr_fetch_buffer_too_short PASSED");
    Ok(())
}

pub fn test_queue_isr_fetch_buffer_too_long() -> Result<()> {
    log_info!(TAG, "Starting test_queue_isr_fetch_buffer_too_long");
    let queue = Queue::new(2, 4)?;
    queue.post_from_isr(&[3, 4, 5, 6])?;
    assert_eq!(
        queue.fetch_from_isr(&mut [0u8; 8]),
        Err(Error::InvalidMessageSize)
    );
    let mut correct_buf = [0u8; 4];
    queue.fetch_from_isr(&mut correct_buf)?;
    assert_eq!(correct_buf, [3, 4, 5, 6]);
    log_info!(TAG, "test_queue_isr_fetch_buffer_too_long PASSED");
    Ok(())
}

pub fn test_queue_propagates_underlying_error() -> Result<()> {
    log_info!(TAG, "Starting test_queue_propagates_underlying_error");
    let queue = Queue::new(2, 4)?;
    assert_eq!(queue.fetch(&mut [0u8; 4], 0), Err(Error::Timeout));
    assert_eq!(
        queue.fetch(&mut [0u8; 8], 0),
        Err(Error::InvalidMessageSize)
    );
    log_info!(TAG, "test_queue_propagates_underlying_error PASSED");
    Ok(())
}

// ===========================================================================
// Non-serde QueueStreamed
// ===========================================================================

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_round_trip");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(4, 6)?;
    let msg = TestMessage::new(7, -12);
    queue.post(&msg, 0)?;
    let mut received = TestMessage::new(0, 0);
    queue.fetch(&mut received, 0)?;
    assert_eq!(received, msg);
    assert_eq!(received.id(), 7);
    assert_eq!(received.value(), -12);
    log_info!(TAG, "test_queue_streamed_round_trip PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_fifo() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_fifo");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(4, 6)?;
    let messages: Vec<_> = (0..4).map(|i| TestMessage::new(i, -(i as i16))).collect();
    for msg in &messages {
        queue.post(msg, 0)?;
    }
    for expected in &messages {
        let mut received = TestMessage::new(0, 0);
        queue.fetch(&mut received, 0)?;
        assert_eq!(received, *expected, "FIFO order mismatch");
    }
    log_info!(TAG, "test_queue_streamed_fifo PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_wrong_message_size() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_wrong_message_size");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 8)?;
    assert_eq!(
        queue.post(&TestMessage::new(1, 2), 0),
        Err(Error::InvalidMessageSize)
    );
    log_info!(TAG, "test_queue_streamed_wrong_message_size PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_isr_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_isr_round_trip");
    let queue: QueueStreamed<TestMessage> = QueueStreamed::new(2, 6)?;
    let msg = TestMessage::new(42, 99);
    queue.post_from_isr(&msg)?;
    let mut received = TestMessage::new(0, 0);
    queue.fetch_from_isr(&mut received)?;
    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_isr_round_trip PASSED");
    Ok(())
}

#[cfg(not(feature = "serde"))]
pub fn test_queue_streamed_broken_len_consistency() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_broken_len_consistency");
    let queue: QueueStreamed<BrokenMessage> = QueueStreamed::new(2, 8)?;
    let msg = BrokenMessage([1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(queue.post(&msg, 0), Err(Error::InvalidMessageSize));
    assert_eq!(queue.post_from_isr(&msg), Err(Error::InvalidMessageSize));
    log_info!(TAG, "test_queue_streamed_broken_len_consistency PASSED");
    Ok(())
}

// ===========================================================================
// Serde QueueStreamed
// ===========================================================================

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_round_trip");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(4, 6)?;
    let msg = SerdeTestMessage { id: 99, value: -1 };
    queue.post(&msg, 0)?;
    let mut received = SerdeTestMessage { id: 0, value: 0 };
    queue.fetch(&mut received, 0)?;
    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_serde_round_trip PASSED");
    Ok(())
}

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_fifo() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_fifo");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(4, 6)?;
    let messages: Vec<_> = (0..4)
        .map(|i| SerdeTestMessage {
            id: 10 + i,
            value: (i as i16 * 10),
        })
        .collect();
    for msg in &messages {
        queue.post(msg, 0)?;
    }
    for expected in &messages {
        let mut received = SerdeTestMessage { id: 0, value: 0 };
        queue.fetch(&mut received, 0)?;
        assert_eq!(received, *expected, "serde FIFO order mismatch");
    }
    log_info!(TAG, "test_queue_streamed_serde_fifo PASSED");
    Ok(())
}

#[cfg(feature = "serde")]
pub fn test_queue_streamed_serde_isr_round_trip() -> Result<()> {
    log_info!(TAG, "Starting test_queue_streamed_serde_isr_round_trip");
    let queue: QueueStreamed<SerdeTestMessage> = QueueStreamed::new(2, 6)?;
    let msg = SerdeTestMessage { id: 42, value: -9 };
    queue.post_from_isr(&msg)?;
    let mut received = SerdeTestMessage { id: 0, value: 0 };
    queue.fetch_from_isr(&mut received)?;
    assert_eq!(received, msg);
    log_info!(TAG, "test_queue_streamed_serde_isr_round_trip PASSED");
    Ok(())
}

// ===========================================================================
// Run all tests
// ===========================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(
        TAG,
        "========== Running Linux-Specific Queue Tests =========="
    );

    // Phase 1: Raw Queue contract
    test_queue_exact_message_size()?;
    test_queue_post_too_short_rejected()?;
    test_queue_post_too_long_rejected()?;
    test_queue_fetch_buffer_too_short_does_not_consume()?;
    test_queue_fetch_buffer_too_long_rejected()?;
    test_queue_isr_post_too_short()?;
    test_queue_isr_post_too_long()?;
    test_queue_isr_fetch_buffer_too_short()?;
    test_queue_isr_fetch_buffer_too_long()?;
    test_queue_propagates_underlying_error()?;

    // Phase 2: Close lifecycle
    test_queue_close_wakes_blocked_consumer()?;
    test_queue_close_wakes_blocked_producer()?;
    test_queue_all_ops_fail_after_close()?;
    test_queue_close_idempotent()?;

    // Phase 2: Handle
    test_queue_unique_handles()?;

    // Phase 2: MAX delay
    test_queue_max_fetch_woken_by_post()?;
    test_queue_max_post_woken_by_fetch()?;

    // Phase 2: Finite timeout
    test_queue_fetch_timeout()?;
    test_queue_post_timeout()?;

    // Phase 2: Multi-producer/consumer liveness
    test_queue_multi_producer_consumer()?;

    // QueueStreamed close
    #[cfg(not(feature = "serde"))]
    {
        test_queue_streamed_closed()?;
    }

    // Non-serde QueueStreamed
    #[cfg(not(feature = "serde"))]
    {
        test_queue_streamed_round_trip()?;
        test_queue_streamed_fifo()?;
        test_queue_streamed_wrong_message_size()?;
        test_queue_streamed_isr_round_trip()?;
        test_queue_streamed_broken_len_consistency()?;
    }

    // Serde QueueStreamed
    #[cfg(feature = "serde")]
    {
        test_queue_streamed_serde_round_trip()?;
        test_queue_streamed_serde_fifo()?;
        test_queue_streamed_serde_isr_round_trip()?;
    }

    log_info!(
        TAG,
        "========== All Linux-Specific Queue Tests PASSED =========="
    );
    Ok(())
}
}

#[test]
fn queue_tests_smoke() {
    linux_legacy_queue_tests::run_all_tests().unwrap();
}

mod linux_legacy_semaphore_tests {
//! Linux-specific semaphore tests.
//!
//! These tests supplement the common semaphore test suite with tests
//! that exercise Linux-backend-specific behaviours: unique handles,
//! non-blocking ISR paths, infinite wait, finite timeout, and
//! signal-wake semantics.

use core::time::Duration;

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};

/// Entry-point called from `mod.rs` to run all Linux-specific
/// semaphore tests.
pub fn run_all_tests() -> Result<()> {
    semaphore_handles_are_unique()?;
    semaphore_wait_zero_is_non_blocking()?;
    semaphore_signal_wakes_waiter()?;
    semaphore_signal_fails_when_full()?;
    semaphore_wait_times_out()?;
    semaphore_isr_paths_are_non_blocking()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Handle uniqueness
// ---------------------------------------------------------------------------

/// Two new semaphore objects must have distinct handles.
fn semaphore_handles_are_unique() -> Result<()> {
    let s1 = Semaphore::new(1, 1)?;
    let s2 = Semaphore::new(1, 1)?;

    assert_ne!(*s1, *s2);
    Ok(())
}

// ---------------------------------------------------------------------------
// Non-blocking wait(0)
// ---------------------------------------------------------------------------

/// `wait(Duration::ZERO)` must return `False` immediately when the
/// count is 0.
fn semaphore_wait_zero_is_non_blocking() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;

    assert_eq!(sem.wait(Duration::ZERO), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal wakes a blocked waiter
// ---------------------------------------------------------------------------

/// Thread A blocks on infinite wait.  Thread B calls `signal()` and
/// Thread A wakes up successfully.
///
/// Passing `u32::MAX` works because `ToTick` is implemented for
/// `TickType` (= `u32`) as an identity conversion, and the semaphore
/// treats `UBaseType::MAX` (= `u32::MAX`) as an infinite wait.
fn semaphore_signal_wakes_waiter() -> Result<()> {
    use std::sync::Arc;

    let sem = Arc::new(Semaphore::new(1, 0)?);
    let sem_waiter = Arc::clone(&sem);

    let handle = std::thread::spawn(move || sem_waiter.wait(u32::MAX));

    // Give the waiter time to block on the condvar.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(sem.signal(), OsalRsBool::True);

    assert_eq!(handle.join().unwrap(), OsalRsBool::True);
    Ok(())
}

// ---------------------------------------------------------------------------
// Signal when full
// ---------------------------------------------------------------------------

/// `signal()` must return `False` when the count is already at
/// `max_count`.
fn semaphore_signal_fails_when_full() -> Result<()> {
    let sem = Semaphore::new(1, 1)?;

    assert_eq!(sem.signal(), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// Finite timeout
// ---------------------------------------------------------------------------

/// `wait(finite_duration)` returns `False` after the timeout expires
/// with no signal.
fn semaphore_wait_times_out() -> Result<()> {
    let sem = Semaphore::new(1, 0)?;

    assert_eq!(sem.wait(Duration::from_millis(10)), OsalRsBool::False);
    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation paths
// ---------------------------------------------------------------------------

/// `wait_from_isr()` and `signal_from_isr()` are non-blocking and
/// follow the same counting semantics as the blocking variants.
fn semaphore_isr_paths_are_non_blocking() -> Result<()> {
    // Binary semaphore with initial count = 1.
    let sem = Semaphore::new(1, 1)?;

    // First ISR take must succeed (count 1 → 0).
    assert_eq!(sem.wait_from_isr(), OsalRsBool::True);
    // Second ISR take must fail (count is 0).
    assert_eq!(sem.wait_from_isr(), OsalRsBool::False);

    // ISR give must succeed (count 0 → 1).
    assert_eq!(sem.signal_from_isr(), OsalRsBool::True);
    // Next ISR give must fail (count is already at max).
    assert_eq!(sem.signal_from_isr(), OsalRsBool::False);

    Ok(())
}
}

#[test]
fn semaphore_tests_smoke() {
    linux_legacy_semaphore_tests::run_all_tests().unwrap();
}

mod linux_legacy_system_tests {
//! Linux-specific system tests.
//!
//! These tests supplement the common system test suite with tests
//! that exercise Linux-backend-specific behaviours: critical-section
//! mutual exclusion, reentrancy, API alias sharing, and ISR-path
//! lock reuse.

use osal_rs::os::System;
use osal_rs::utils::Result;

/// Entry-point called from `mod.rs` to run all Linux-specific
/// system tests.
pub fn run_all_tests() -> Result<()> {
    critical_section_is_mutually_exclusive()?;
    critical_section_is_reentrant_on_same_thread()?;
    critical_section_aliases_share_same_lock()?;
    critical_section_from_isr_uses_same_lock()?;
    critical_section_blocks_other_threads_until_exit()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Mutual exclusion across threads
// ---------------------------------------------------------------------------

/// Multiple threads entering the critical section must never overlap.
fn critical_section_is_mutually_exclusive() -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let inside = Arc::new(AtomicBool::new(false));
    let overlap_detected = Arc::new(AtomicBool::new(false));

    let mut handles = Vec::new();

    for _ in 0..4 {
        let inside = Arc::clone(&inside);
        let overlap_detected = Arc::clone(&overlap_detected);

        handles.push(std::thread::spawn(move || {
            for _ in 0..50 {
                System::enter_critical();

                if inside.swap(true, Ordering::SeqCst) {
                    overlap_detected.store(true, Ordering::SeqCst);
                }

                std::thread::sleep(std::time::Duration::from_millis(1));

                inside.store(false, Ordering::SeqCst);
                System::exit_critical();
            }
        }));
    }

    for handle in handles {
        handle.join().unwrap();
    }

    assert!(!overlap_detected.load(Ordering::SeqCst));
    Ok(())
}

// ---------------------------------------------------------------------------
// Same-thread nesting does not deadlock
// ---------------------------------------------------------------------------

/// The same thread can nest `enter_critical()` calls without deadlocking.
fn critical_section_is_reentrant_on_same_thread() -> Result<()> {
    System::enter_critical();
    System::enter_critical();
    System::enter_critical();

    System::exit_critical();
    System::exit_critical();
    System::exit_critical();
    Ok(())
}

// ---------------------------------------------------------------------------
// API aliases share the same lock
// ---------------------------------------------------------------------------

/// `critical_section_enter/exit` and `enter_critical/exit_critical`
/// share the same nesting counter.
fn critical_section_aliases_share_same_lock() -> Result<()> {
    System::critical_section_enter();
    System::enter_critical();

    System::exit_critical();
    System::critical_section_exit();
    Ok(())
}

// ---------------------------------------------------------------------------
// ISR simulation path shares the same lock
// ---------------------------------------------------------------------------

/// `enter_critical_from_isr` / `exit_critical_from_isr` reuse the same
/// recursive lock as task-level calls.
fn critical_section_from_isr_uses_same_lock() -> Result<()> {
    let saved = System::enter_critical_from_isr();

    System::enter_critical();
    System::exit_critical();

    System::exit_critical_from_isr(saved);
    Ok(())
}

// ---------------------------------------------------------------------------
// Blocking: held critical section prevents other threads from entering
// ---------------------------------------------------------------------------

/// When one thread holds the critical lock, another thread blocks on
/// `enter_critical()` until the first thread releases it.
fn critical_section_blocks_other_threads_until_exit() -> Result<()> {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let entered = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));

    System::enter_critical();

    let entered_clone = Arc::clone(&entered);
    let completed_clone = Arc::clone(&completed);

    let handle = std::thread::spawn(move || {
        entered_clone.store(true, Ordering::SeqCst);

        System::enter_critical();
        completed_clone.store(true, Ordering::SeqCst);
        System::exit_critical();
    });

    // Wait until the spawned thread has attempted to enter.
    while !entered.load(Ordering::SeqCst) {
        std::thread::yield_now();
    }

    // Give it time to block (it cannot enter while we hold the lock).
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert!(!completed.load(Ordering::SeqCst));

    System::exit_critical();

    handle.join().unwrap();
    assert!(completed.load(Ordering::SeqCst));
    Ok(())
}
}

#[test]
fn system_tests_smoke() {
    linux_legacy_system_tests::run_all_tests().unwrap();
}

mod linux_legacy_thread_tests {
//! Linux-specific thread tests (registry, handles, state machine,
//! notifications, spawn lifecycle).


use alloc::sync::Arc;
use core::time::Duration;

use osal_rs::log_info;
use osal_rs::os::types::TickType;
use osal_rs::os::*;
use osal_rs::utils::Result;

const TAG: &str = "LinuxThreadTests";

// ---------------------------------------------------------------------------
// Handles & Registry
// ---------------------------------------------------------------------------

pub fn test_thread_handles_unique() -> Result<()> {
    log_info!(TAG, "test_thread_handles_unique");
    let t1 = Thread::new("h1", 1024, 1);
    let t2 = Thread::new("h2", 1024, 1);
    assert_ne!(*t1, *t2, "handles must differ");
    t1.delete();
    t2.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_handle_matches_metadata() -> Result<()> {
    log_info!(TAG, "test_thread_handle_matches_metadata");
    let mut t = Thread::new("meta", 2048, 3);
    let m = t.get_metadata();
    assert_eq!(*t, m.thread);
    let spawned = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    let m2 = spawned.get_metadata();
    assert_eq!(*spawned, m2.thread);
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_metadata_from_handle_real() -> Result<()> {
    log_info!(TAG, "test_thread_get_metadata_from_handle_real");
    let t = Thread::new("real", 1024, 5);
    let m = Thread::get_metadata_from_handle(*t);
    assert_eq!(m.name.as_str(), "real");
    assert_eq!(m.priority, 5);
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_metadata_invalid_handle() -> Result<()> {
    log_info!(TAG, "test_thread_get_metadata_invalid_handle");
    let m = Thread::get_metadata_from_handle(0xDEAD as osal_rs::os::types::ThreadHandle);
    assert_eq!(m.state, ThreadState::Invalid);
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Spawn lifecycle
// ---------------------------------------------------------------------------

pub fn test_thread_spawn_twice_rejected() -> Result<()> {
    log_info!(TAG, "test_thread_spawn_twice_rejected");
    let mut t = Thread::new("twice", 1024, 1);
    t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    let r2 = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))));
    assert!(r2.is_err());
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_fast_exit_not_ready() -> Result<()> {
    log_info!(TAG, "test_thread_fast_exit_not_ready");
    let mut t = Thread::new("fast", 1024, 1);
    let s = t.spawn(None, |_, p| Ok(p.unwrap_or_else(|| Arc::new(()))))?;
    s.join(core::ptr::null_mut())?;
    let m = s.get_metadata();
    assert_eq!(m.state, ThreadState::Deleted);
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_join_after_panic_sets_deleted() -> Result<()> {
    log_info!(TAG, "test_thread_join_after_panic_sets_deleted");
    let mut t = Thread::new("panic", 1024, 1);
    let s = t.spawn(None, |_, _p| {
        panic!("intentional");
    })?;
    let r = s.join(core::ptr::null_mut());
    assert!(r.is_err());
    // After join, state should be Deleted
    let m = s.get_metadata();
    assert_eq!(m.state, ThreadState::Deleted);
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

pub fn test_thread_notify_max_delay() -> Result<()> {
    log_info!(TAG, "test_thread_notify_max_delay");
    let mut t = Thread::new("max", 1024, 1);
    let spawned = t.spawn(None, |thread, _p| {
        let v = thread.wait_notification(0, 0xFFFF_FFFF, TickType::MAX)?;
        assert_eq!(v, 0xABCD);
        Ok(Arc::new(()))
    })?;
    std::thread::sleep(Duration::from_millis(20));
    spawned.notify(ThreadNotification::SetValueWithOverwrite(0xABCD))?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_notify_timeout() -> Result<()> {
    log_info!(TAG, "test_thread_notify_timeout");
    let mut t = Thread::new("to", 1024, 1);
    let spawned = t.spawn(None, |thread, _p| {
        let r = thread.wait_notification(0, 0xFFFF_FFFF, 30);
        assert!(r.is_err());
        Ok(Arc::new(()))
    })?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_notify_from_isr_hpw() -> Result<()> {
    log_info!(TAG, "test_thread_notify_from_isr_hpw");
    let t = Thread::new("hpw", 1024, 1);
    let mut hpw: i32 = 0;
    t.notify_from_isr(ThreadNotification::SetBits(1), &mut hpw)?;
    assert_eq!(hpw, 0, "hpw should be 0 when no waiter is blocked");
    t.delete();
    log_info!(TAG, "PASSED");
    Ok(())
}

pub fn test_thread_get_current_waits_for_notification() -> Result<()> {
    log_info!(TAG, "test_thread_get_current_waits_for_notification");
    let mut t = Thread::new("gc", 1024, 1);
    let spawned = t.spawn(None, |_thread, _p| {
        // Use get_current(), not the callback parameter
        let current = Thread::get_current();
        let v = current.wait_notification(0, 0xFFFF_FFFF, TickType::MAX)?;
        assert_eq!(v, 0xDEAD);
        Ok(Arc::new(()))
    })?;
    std::thread::sleep(Duration::from_millis(20));
    spawned.notify(ThreadNotification::SetValueWithOverwrite(0xDEAD))?;
    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Cooperative cancellation
// ---------------------------------------------------------------------------

/// After `delete()` is called on a running thread, `is_delete_requested()`
/// returns `true`.
pub fn test_thread_delete_sets_cancellation_flag() -> Result<()> {
    log_info!(TAG, "test_thread_delete_sets_cancellation_flag");
    let mut t = Thread::new("cancel", 1024, 1);
    let spawned = t.spawn_simple(|| {
        while !Thread::get_current().is_delete_requested() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    spawned.delete();
    assert!(spawned.is_delete_requested());

    spawned.join(core::ptr::null_mut())?;
    log_info!(TAG, "PASSED");
    Ok(())
}

/// Thread polls `current_cancellation_requested()`, exits naturally after
/// `delete()`, and `join()` succeeds.
pub fn test_thread_cooperative_cancellation_exits() -> Result<()> {
    log_info!(TAG, "test_thread_cooperative_cancellation_exits");
    use std::sync::{
        Arc as StdArc,
        atomic::{AtomicBool, Ordering},
    };

    let exited = StdArc::new(AtomicBool::new(false));
    let exited_worker = StdArc::clone(&exited);

    let mut t = Thread::new("coop", 1024, 1);
    let spawned = t.spawn_simple(move || {
        while !Thread::current_cancellation_requested() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        exited_worker.store(true, Ordering::SeqCst);
    })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    spawned.delete();
    spawned.join(core::ptr::null_mut())?;

    assert!(exited.load(Ordering::SeqCst));
    log_info!(TAG, "PASSED");
    Ok(())
}

/// When a thread is blocked in `wait_notification(TickType::MAX)`,
/// `delete()` wakes it up and causes it to return an error.
pub fn test_thread_delete_wakes_wait_notification() -> Result<()> {
    log_info!(TAG, "test_thread_delete_wakes_wait_notification");
    use std::sync::{
        Arc as StdArc,
        atomic::{AtomicBool, Ordering},
    };

    let returned = StdArc::new(AtomicBool::new(false));
    let returned_worker = StdArc::clone(&returned);

    let mut t = Thread::new("wait_cancel", 1024, 1);
    let spawned = t.spawn_simple(move || {
        let current = Thread::get_current();
        let result = current.wait_notification(0, 0, TickType::MAX);
        assert!(result.is_err());
        assert!(current.is_delete_requested());
        returned_worker.store(true, Ordering::SeqCst);
    })?;

    std::thread::sleep(std::time::Duration::from_millis(20));

    spawned.delete();
    spawned.join(core::ptr::null_mut())?;

    assert!(returned.load(Ordering::SeqCst));
    log_info!(TAG, "PASSED");
    Ok(())
}

/// Deleting an unstarted thread marks it as Deleted/Invalid in the registry.
pub fn test_thread_delete_before_spawn_marks_deleted() -> Result<()> {
    log_info!(TAG, "test_thread_delete_before_spawn_marks_deleted");
    let t = Thread::new("not_started", 1024, 1);
    let handle = *t;

    t.delete();

    let meta = Thread::get_metadata_from_handle(handle);
    assert!(matches!(
        meta.state,
        ThreadState::Invalid | ThreadState::Deleted
    ));
    log_info!(TAG, "PASSED");
    Ok(())
}

/// After a thread completes and is joined, the registry no longer returns
/// its metadata.
pub fn test_thread_join_unregisters_completed_thread() -> Result<()> {
    log_info!(TAG, "test_thread_join_unregisters_completed_thread");
    let mut t = Thread::new("join_unreg", 1024, 1);
    let spawned = t.spawn_simple(|| {})?;
    let handle = *spawned;

    spawned.join(core::ptr::null_mut())?;

    let meta = Thread::get_metadata_from_handle(handle);
    assert_eq!(meta.state, ThreadState::Invalid);
    log_info!(TAG, "PASSED");
    Ok(())
}

// ---------------------------------------------------------------------------
// Run all
// ---------------------------------------------------------------------------

pub fn run_all_tests() -> Result<()> {
    log_info!(
        TAG,
        "========== Running Linux-Specific Thread Tests =========="
    );
    test_thread_handles_unique()?;
    test_thread_handle_matches_metadata()?;
    test_thread_get_metadata_from_handle_real()?;
    test_thread_get_metadata_invalid_handle()?;
    test_thread_spawn_twice_rejected()?;
    test_thread_fast_exit_not_ready()?;
    test_thread_join_after_panic_sets_deleted()?;
    test_thread_notify_max_delay()?;
    test_thread_notify_timeout()?;
    test_thread_notify_from_isr_hpw()?;
    test_thread_get_current_waits_for_notification()?;
    test_thread_delete_sets_cancellation_flag()?;
    test_thread_cooperative_cancellation_exits()?;
    test_thread_delete_wakes_wait_notification()?;
    test_thread_delete_before_spawn_marks_deleted()?;
    test_thread_join_unregisters_completed_thread()?;
    log_info!(
        TAG,
        "========== All Linux-Specific Thread Tests PASSED =========="
    );
    Ok(())
}
}

#[test]
fn thread_tests_smoke() {
    linux_legacy_thread_tests::run_all_tests().unwrap();
}

mod linux_legacy_timer_tests {
//! Linux-specific timer tests.
//!
//! These tests verify the worker lifecycle, state machine, generation
//! mechanism, callback parameter write-back, panic/error recovery,
//! clone/drop lifecycle, and handle uniqueness of the Linux Timer
//! backend.  They are **not** part of the cross-backend common test suite.


use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Barrier, mpsc};
use std::time::{Duration, Instant};

use osal_rs::log_info;
use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};

const TAG: &str = "LinuxTimerTests";

fn ms(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

fn ret(
    p: Option<Arc<dyn core::any::Any + Send + Sync>>,
) -> Result<Arc<dyn core::any::Any + Send + Sync>> {
    Ok(p.unwrap_or_else(|| Arc::new(())))
}

// 1
pub fn test_timer_one_shot_exact() -> Result<()> {
    log_info!(TAG, "test_timer_one_shot_exact");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("os", 30, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_one_shot_exact PASSED");
    Ok(())
}

// 2
pub fn test_timer_periodic_auto_reload() -> Result<()> {
    log_info!(TAG, "test_timer_periodic_auto_reload");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("per", 20, true, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(80));
    timer.stop(0);
    let after_stop = counter.load(Ordering::SeqCst);
    assert!(after_stop >= 3, "expected >= 3, got {}", after_stop);
    std::thread::sleep(ms(80));
    assert_eq!(counter.load(Ordering::SeqCst), after_stop);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_periodic_auto_reload PASSED");
    Ok(())
}

// 3
pub fn test_timer_stop_before_expiry() -> Result<()> {
    log_info!(TAG, "test_timer_stop_before_expiry");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("sbe", 100, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(30));
    timer.stop(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_stop_before_expiry PASSED");
    Ok(())
}

// 4
pub fn test_timer_restart_after_stop() -> Result<()> {
    log_info!(TAG, "test_timer_restart_after_stop");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ras", 30, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(10));
    timer.stop(0);
    timer.start(0);
    std::thread::sleep(ms(60));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_restart_after_stop PASSED");
    Ok(())
}

// 5
pub fn test_timer_repeated_start() -> Result<()> {
    log_info!(TAG, "test_timer_repeated_start");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("rs", 50, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    for _ in 0..20 {
        timer.start(0);
    }
    std::thread::sleep(ms(120));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_repeated_start PASSED");
    Ok(())
}

// 6
pub fn test_timer_reset_deadline() -> Result<()> {
    log_info!(TAG, "test_timer_reset_deadline");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("rd", 100, false, None, move |_t, p| {
        let _ = tx.send(Instant::now());
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(70));
    timer.reset(0);
    let t0 = Instant::now();
    rx.recv_timeout(ms(200))
        .expect("timer did not fire after reset");
    assert!(
        t0.elapsed() >= ms(80),
        "reset deadline too short: {:?}",
        t0.elapsed()
    );
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_reset_deadline PASSED");
    Ok(())
}

// 7
pub fn test_timer_change_period_shorten() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_shorten");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("cps", 200, false, None, move |_t, p| {
        let _ = tx.send(Instant::now());
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(30));
    let t0 = Instant::now();
    timer.change_period(50, 0);
    rx.recv_timeout(ms(150))
        .expect("timer did not fire after shorten");
    assert!(t0.elapsed() < ms(100));
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_change_period_shorten PASSED");
    Ok(())
}

// 8
pub fn test_timer_change_period_extend() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_extend");
    let (tx, rx) = mpsc::channel();
    let timer = Timer::new("cpe", 50, false, None, move |_t, p| {
        let _ = tx.send(Instant::now());
        ret(p)
    })?;
    let t0 = Instant::now();
    timer.start(0);
    std::thread::sleep(ms(20));
    timer.change_period(150, 0);
    rx.recv_timeout(ms(300))
        .expect("timer did not fire after extend");
    assert!(t0.elapsed() >= ms(140));
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_change_period_extend PASSED");
    Ok(())
}

// 9 - FIXED: change_period on Stopped must NOT arm the timer
pub fn test_timer_change_period_from_stopped() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_from_stopped");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpfs", 200, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;

    // change_period on a stopped timer must NOT arm it
    assert_eq!(timer.change_period(30, 0), OsalRsBool::True);
    std::thread::sleep(ms(100));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "change_period on stopped timer must NOT start it"
    );

    // start() must fire using the stored period
    timer.start(0);
    std::thread::sleep(ms(80));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "start after change_period must fire"
    );

    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_change_period_from_stopped PASSED");
    Ok(())
}

// 10
pub fn test_timer_delete_before_expiry() -> Result<()> {
    log_info!(TAG, "test_timer_delete_before_expiry");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("dbe", 100, false, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(20));
    let mut timer = timer;
    timer.delete(0);
    std::thread::sleep(ms(150));
    assert_eq!(counter.load(Ordering::SeqCst), 0);
    log_info!(TAG, "test_timer_delete_before_expiry PASSED");
    Ok(())
}

// 11
pub fn test_timer_commands_fail_after_delete() -> Result<()> {
    log_info!(TAG, "test_timer_commands_fail_after_delete");
    let mut timer = Timer::new("cfa", 30, false, None, |_t, p| ret(p))?;
    timer.delete(0);
    assert_eq!(timer.start(0), OsalRsBool::False);
    assert_eq!(timer.stop(0), OsalRsBool::False);
    assert_eq!(timer.reset(0), OsalRsBool::False);
    assert_eq!(timer.change_period(50, 0), OsalRsBool::False);
    assert_eq!(timer.delete(0), OsalRsBool::False);
    log_info!(TAG, "test_timer_commands_fail_after_delete PASSED");
    Ok(())
}

// 12
pub fn test_timer_drop_stops_worker() -> Result<()> {
    log_info!(TAG, "test_timer_drop_stops_worker");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("dsw", 30, true, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    timer.start(0);
    std::thread::sleep(ms(40));
    assert!(counter.load(Ordering::SeqCst) >= 1);
    drop(timer);
    let before = counter.load(Ordering::SeqCst);
    std::thread::sleep(ms(120));
    assert_eq!(counter.load(Ordering::SeqCst), before);
    log_info!(TAG, "test_timer_drop_stops_worker PASSED");
    Ok(())
}

// 13 - FIXED: now verifies param write-back via AtomicU32 tracking
pub fn test_timer_callback_param_update() -> Result<()> {
    log_info!(TAG, "test_timer_callback_param_update");
    let max_val = Arc::new(AtomicU32::new(0));
    let mv = Arc::clone(&max_val);

    let timer = Timer::new(
        "cpu",
        20,
        true,
        Some(Arc::new(0u32)),
        move |t: Box<dyn TimerFn>, p| {
            let val = p
                .and_then(|x| x.downcast_ref::<u32>().copied())
                .unwrap_or(0);
            mv.store(val, Ordering::SeqCst);
            if val >= 3 {
                t.stop(0);
            }
            Ok(Arc::new(val + 1))
        },
    )?;

    timer.start(0);
    std::thread::sleep(ms(200));
    let observed = max_val.load(Ordering::SeqCst);
    assert!(
        observed >= 2,
        "param not propagated: max observed {}",
        observed
    );
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_callback_param_update PASSED");
    Ok(())
}

// 14 - FIXED: must fire exactly once when stop is called in callback
pub fn test_timer_stop_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_stop_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);

    let timer = Timer::new("sic", 20, true, None, move |t: Box<dyn TimerFn>, p| {
        c.fetch_add(1, Ordering::SeqCst);
        t.stop(0);
        let _ = tx.send(());
        ret(p)
    })?;

    timer.start(0);
    // Wait for callback to fire and complete stop inside
    rx.recv_timeout(ms(200)).expect("callback did not fire");
    // Ample time — must NOT fire again
    std::thread::sleep(ms(120));
    assert_eq!(
        counter.load(Ordering::SeqCst),
        1,
        "expected exactly 1 fire, got {}",
        counter.load(Ordering::SeqCst)
    );
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_stop_inside_callback PASSED");
    Ok(())
}

// 15
pub fn test_timer_reset_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_reset_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ric", 30, true, None, move |t: Box<dyn TimerFn>, p| {
        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 {
            t.reset(0);
            let _ = tx.send(());
        }
        if n == 2 {
            let _ = tx.send(());
        }
        ret(p)
    })?;
    timer.start(0);
    rx.recv_timeout(ms(150)).expect("first fire");
    rx.recv_timeout(ms(150)).expect("second fire after reset");
    assert!(counter.load(Ordering::SeqCst) >= 2);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_reset_inside_callback PASSED");
    Ok(())
}

// 16
pub fn test_timer_change_period_inside_callback() -> Result<()> {
    log_info!(TAG, "test_timer_change_period_inside_callback");
    let (tx, rx) = mpsc::channel();
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpic", 30, true, None, move |t: Box<dyn TimerFn>, p| {
        let n = c.fetch_add(1, Ordering::SeqCst) + 1;
        if n == 1 {
            t.change_period(50, 0);
            let _ = tx.send(());
        }
        if n == 2 {
            let _ = tx.send(());
            t.stop(0);
        }
        ret(p)
    })?;
    let t0 = Instant::now();
    timer.start(0);
    rx.recv_timeout(ms(80)).expect("first fire");
    rx.recv_timeout(ms(150)).expect("second fire after change");
    assert!(t0.elapsed() >= ms(60));
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_change_period_inside_callback PASSED");
    Ok(())
}

// 17
pub fn test_timer_callback_err_stops() -> Result<()> {
    log_info!(TAG, "test_timer_callback_err_stops");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("ces", 20, true, None, move |_t, _p| {
        c.fetch_add(1, Ordering::SeqCst);
        Err(osal_rs::utils::Error::Unhandled("test err"))
    })?;
    timer.start(0);
    std::thread::sleep(ms(80));
    assert!(
        counter.load(Ordering::SeqCst) <= 2,
        "should stop after error"
    );
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_callback_err_stops PASSED");
    Ok(())
}

// 18
pub fn test_timer_callback_panic_caught() -> Result<()> {
    log_info!(TAG, "test_timer_callback_panic_caught");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cpc", 30, false, None, move |_t, _p| {
        c.fetch_add(1, Ordering::SeqCst);
        panic!("intentional");
    })?;
    timer.start(0);
    std::thread::sleep(ms(100));
    assert_eq!(counter.load(Ordering::SeqCst), 1);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_callback_panic_caught PASSED");
    Ok(())
}

// 19
pub fn test_timer_period_zero_rejected() -> Result<()> {
    log_info!(TAG, "test_timer_period_zero_rejected");
    assert!(Timer::new("pz", 0, false, None, |_t, p| ret(p)).is_err());
    log_info!(TAG, "test_timer_period_zero_rejected PASSED");
    Ok(())
}

// 20
pub fn test_timer_unique_handles() -> Result<()> {
    log_info!(TAG, "test_timer_unique_handles");
    let mut t1 = Timer::new("h1", 100, false, None, |_t, p| ret(p))?;
    let mut t2 = Timer::new("h2", 100, false, None, |_t, p| ret(p))?;
    assert_ne!(*t1, *t2);
    t1.delete(0);
    t2.delete(0);
    log_info!(TAG, "test_timer_unique_handles PASSED");
    Ok(())
}

// 21
pub fn test_timer_clone_lifecycle() -> Result<()> {
    log_info!(TAG, "test_timer_clone_lifecycle");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Timer::new("cl", 30, true, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?;
    let clone = timer.clone();
    clone.start(0);
    std::thread::sleep(ms(40));
    assert!(counter.load(Ordering::SeqCst) >= 1);
    drop(clone);
    std::thread::sleep(ms(50));
    assert!(counter.load(Ordering::SeqCst) >= 2);
    timer.stop(0);
    let mut timer = timer;
    timer.delete(0);
    log_info!(TAG, "test_timer_clone_lifecycle PASSED");
    Ok(())
}

// 22 - NEW: multi-thread concurrent command stress test
pub fn test_timer_concurrent_commands() -> Result<()> {
    log_info!(TAG, "test_timer_concurrent_commands");
    let counter = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&counter);
    let timer = Arc::new(Timer::new("cc", 50, true, None, move |_t, p| {
        c.fetch_add(1, Ordering::SeqCst);
        ret(p)
    })?);

    const THREADS: usize = 4;
    let barrier = Arc::new(Barrier::new(THREADS));
    let mut handles = vec![];

    for i in 0..THREADS {
        let t = Arc::clone(&timer);
        let b = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            b.wait();
            for _ in 0..50 {
                match i % 4 {
                    0 => {
                        t.start(0);
                    }
                    1 => {
                        t.stop(0);
                    }
                    2 => {
                        t.reset(0);
                    }
                    3 => {
                        t.change_period(30 + (i as u32 % 5) * 10, 0);
                    }
                    _ => {}
                }
                std::thread::yield_now();
            }
        }));
    }

    for h in handles {
        h.join().unwrap();
    }

    let count = counter.load(Ordering::SeqCst);
    assert!(count < 500, "excessive callbacks: {}", count);

    let mut timer = Arc::try_unwrap(timer).unwrap_or_else(|a| (*a).clone());
    timer.delete(0);
    log_info!(TAG, "test_timer_concurrent_commands PASSED");
    Ok(())
}

// ============================================================================
// Run all
// ============================================================================

pub fn run_all_tests() -> Result<()> {
    log_info!(
        TAG,
        "========== Running Linux-Specific Timer Tests =========="
    );
    test_timer_one_shot_exact()?;
    test_timer_periodic_auto_reload()?;
    test_timer_stop_before_expiry()?;
    test_timer_restart_after_stop()?;
    test_timer_repeated_start()?;
    test_timer_reset_deadline()?;
    test_timer_change_period_shorten()?;
    test_timer_change_period_extend()?;
    test_timer_change_period_from_stopped()?;
    test_timer_delete_before_expiry()?;
    test_timer_commands_fail_after_delete()?;
    test_timer_drop_stops_worker()?;
    test_timer_callback_param_update()?;
    test_timer_stop_inside_callback()?;
    test_timer_reset_inside_callback()?;
    test_timer_change_period_inside_callback()?;
    test_timer_callback_err_stops()?;
    test_timer_callback_panic_caught()?;
    test_timer_period_zero_rejected()?;
    test_timer_unique_handles()?;
    test_timer_clone_lifecycle()?;
    test_timer_concurrent_commands()?;
    log_info!(
        TAG,
        "========== All Linux-Specific Timer Tests PASSED =========="
    );
    Ok(())
}
}

#[test]
fn timer_tests_smoke() {
    linux_legacy_timer_tests::run_all_tests().unwrap();
}
