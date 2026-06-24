//! Linux legacy backend — minimal smoke tests.
//!
//! Each test exercises exactly one OSAL primitive along the
//! create / use / destroy path.  These tests only verify that the
//! legacy Linux backend can instantiate and minimally operate each
//! primitive.  Full API contract tests live in `crate::api`.
//!
//! The old heavy Linux-specific tests (ISR paths, poison recovery,
//! timer precision, thread cancellation, QueueStreamed, etc.) were
//! moved to `crate::port::linux_legacy_extended_tests`.

use alloc::sync::Arc;
use core::time::Duration;

use osal_rs::os::*;
use osal_rs::utils::{OsalRsBool, Result};

#[test]
fn smoke_thread_create_run_join() {
    let done = Arc::new(Mutex::new(false));
    let d = done.clone();
    let mut t = Thread::new("smoke_th", 4096, 1);
    t.spawn_simple(move || {
        *d.lock().unwrap() = true;
    })
    .unwrap();
    let _ = t.join(core::ptr::null_mut());
    assert!(*done.lock().unwrap());
}

#[test]
fn smoke_semaphore_create_signal_wait() {
    let sem = Semaphore::new(1, 0).unwrap();
    assert_eq!(sem.signal(), OsalRsBool::True);
    assert_eq!(sem.wait(Duration::from_millis(100)), OsalRsBool::True);
}

#[test]
fn smoke_mutex_create_lock_unlock() {
    let m = Mutex::new(0u32);
    {
        let mut g = m.lock().unwrap();
        *g = 42;
    }
    assert_eq!(*m.lock().unwrap(), 42);
}

#[test]
fn smoke_queue_create_send_receive() {
    let q = Queue::new(4, 4).unwrap();
    let data = [1u8, 2, 3, 4];
    let mut buf = [0u8; 4];
    q.post(&data, 0).unwrap();
    q.fetch(&mut buf, 0).unwrap();
    assert_eq!(buf, data);
}

#[test]
fn smoke_event_group_create_set_wait() {
    let eg = EventGroup::new().unwrap();
    eg.set(0b0001);
    let bits = eg.wait(0b0001, 0);
    assert_ne!(bits & 0b0001, 0);
}

#[test]
fn smoke_timer_one_shot_fires() {
    let fired = Arc::new(Mutex::new(false));
    let f = fired.clone();
    let dummy: TimerParam = Arc::new(0u32);
    let mut t = Timer::new("smoke_tmr", 1, false, Some(dummy.clone()), move |_, p| {
        *f.lock().unwrap() = true;
        Ok(p.unwrap_or(dummy.clone()))
    })
    .unwrap();
    let _ = t.start(100);
    for _ in 0..100 {
        System::delay(1);
        if *fired.lock().unwrap() {
            break;
        }
    }
    let _ = t.stop(0);
    assert!(*fired.lock().unwrap());
}

#[test]
fn smoke_system_sleep_and_time() {
    let t0 = System::get_current_time_us();
    System::delay(5);
    let t1 = System::get_current_time_us();
    assert!(t1 >= t0);
}
