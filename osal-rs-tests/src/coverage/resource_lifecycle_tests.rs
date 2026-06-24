//! Resource lifecycle and handle identity tests.
//!
//! Tests that verify resource handles are unique, repeated
//! create/drop cycles are safe, and basic resource cleanup works.

use osal_rs::os::*;
use osal_rs::utils::Result;

pub fn run_all_tests() -> Result<()> {
    resource_handles_are_unique()?;
    resource_repeated_create_drop()?;
    Ok(())
}

fn resource_handles_are_unique() -> Result<()> {
    // Two objects of the same type must have distinct handles.
    let m1 = Mutex::new(0u32);
    let m2 = Mutex::new(1u32);
    assert_ne!(*m1, *m2);

    let s1 = Semaphore::new(1, 1)?;
    let s2 = Semaphore::new(1, 1)?;
    assert_ne!(*s1, *s2);

    let e1 = EventGroup::new()?;
    let e2 = EventGroup::new()?;
    assert_ne!(*e1, *e2);

    let q1 = Queue::new(2, 4)?;
    let q2 = Queue::new(2, 4)?;
    assert_ne!(*q1, *q2);

    let r1 = RawMutex::new()?;
    let r2 = RawMutex::new()?;
    assert_ne!(*r1, *r2);

    Ok(())
}

fn resource_repeated_create_drop() -> Result<()> {
    // Repeated create-drop cycles must not leak or panic.
    for _ in 0..10 {
        let _s = Semaphore::new(1, 1)?;
        let _m = Mutex::new(0u32);
        let _e = EventGroup::new()?;
        let _q = Queue::new(2, 4)?;
    }
    Ok(())
}
