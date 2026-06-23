# osal-rs Linux Backend Test Architecture Design

## 1. Overall Principles

Testing for osal-rs shouldn't just verify whether a single backend can run. Instead, it should verify:

**Whether the same set of OSAL abstraction APIs exhibits consistent behavior across different backends.**

Therefore, the test system should be divided into two parts:

```
Common behavior tests (common tests)
    ↕
Different backend test entry points (backend runners)
```

In other words, the actual test logic should be written as common code as much as possible, reusable across FreeRTOS, Linux, and potentially RT-Thread, Zephyr, POSIX in the future.

---

## 2. Differences Between FreeRTOS and Linux Testing Approaches

### FreeRTOS Backend

FreeRTOS is an embedded RTOS backend and cannot directly rely on standard `cargo test`.

The reasons are:

- FreeRTOS backend requires a real RTOS scheduler
- Needs to link against the FreeRTOS Kernel
- Requires C bridge / porting layers
- Many behaviors must run within tasks

Therefore, FreeRTOS testing is suited to this pattern:

```
Embedded project startup
    ↓
Start FreeRTOS scheduler
    ↓
Create test task
    ↓
Call osal_rs_tests::freertos::run_all_tests() in the test task
```

Thus, FreeRTOS test code doesn't need `#[test]`, but instead manually writes:

```rust
pub fn run_all_tests() -> Result<()> {
    queue_tests::run_all_tests()?;
    mutex_tests::run_all_tests()?;
    semaphore_tests::run_all_tests()?;
    thread_tests::run_all_tests()?;
    timer_tests::run_all_tests()?;
    Ok(())
}
```

Its testing approach is: **Manual runner within firmware**

### Linux Backend

Linux is different.

Linux can run directly on the host machine, so it should use Rust's native testing system:

```
cargo test
```

Linux backend should use:

```rust
#[test]
fn test_xxx() {
    ...
}
```

Because Linux backend doesn't require flashing, doesn't need a board, doesn't need to manually enter RTOS tasks, and doesn't need to bypass Rust's test harness.

So Linux testing approach is: **Automatic runner via `cargo test`**

---

## 3. Final Recommended Test Architecture

The structure I recommend most is:

```
osal-rs-tests/
  src/
    lib.rs

    common/
      mod.rs
      duration_tests.rs
      system_tests.rs
      mutex_tests.rs
      semaphore_tests.rs
      queue_tests.rs
      thread_tests.rs
      timer_tests.rs
      event_group_tests.rs
      api_surface.rs

    freertos/
      mod.rs

    linux/
      mod.rs
```

The core idea is:

- Write the actual test logic in `common/`
- Write FreeRTOS manual entry points in `freertos/`
- Write `#[test]` wrapper entry points in `linux/`

---

## 4. Common Test Layer

The common layer doesn't care about specific backends.

It only cares whether OSAL API behavior is correct.

For example:

```rust
// osal-rs-tests/src/common/queue_tests.rs

use osal_rs::os::*;
use osal_rs::utils::Result;

pub fn test_queue_post_fetch() -> Result<()> {
    let queue = Queue::new(4, 4)?;

    let input = [1u8, 2, 3, 4];
    queue.post(&input, 100)?;

    let mut output = [0u8; 4];
    queue.fetch(&mut output, 100)?;

    assert_eq!(input, output);

    Ok(())
}

pub fn test_queue_fifo_order() -> Result<()> {
    let queue = Queue::new(2, 4)?;

    let a = [1u8, 0, 0, 0];
    let b = [2u8, 0, 0, 0];

    queue.post(&a, 100)?;
    queue.post(&b, 100)?;

    let mut out1 = [0u8; 4];
    let mut out2 = [0u8; 4];

    queue.fetch(&mut out1, 100)?;
    queue.fetch(&mut out2, 100)?;

    assert_eq!(out1, a);
    assert_eq!(out2, b);

    Ok(())
}

pub fn run_all_tests() -> Result<()> {
    test_queue_post_fetch()?;
    test_queue_fifo_order()?;
    Ok(())
}
```

There are no `#[test]` attributes here.

Because these functions are just common test logic, they don't directly determine how to run.

---

## 5. FreeRTOS Test Entry Point

FreeRTOS backend continues to use manual runner.

```rust
// osal-rs-tests/src/freertos/mod.rs

use osal_rs::utils::Result;

pub fn run_all_tests() -> Result<()> {
    crate::common::duration_tests::run_all_tests()?;
    crate::common::system_tests::run_all_tests()?;
    crate::common::mutex_tests::run_all_tests()?;
    crate::common::semaphore_tests::run_all_tests()?;
    crate::common::queue_tests::run_all_tests()?;
    crate::common::thread_tests::run_all_tests()?;
    crate::common::timer_tests::run_all_tests()?;

    Ok(())
}
```

Then call it in some task within the FreeRTOS project:

```rust
fn test_task() {
    match osal_rs_tests::freertos::run_all_tests() {
        Ok(_) => {
            // tests passed
        }
        Err(e) => {
            // tests failed
        }
    }
}
```

This part still doesn't need `#[test]`.

---

## 6. Linux Test Entry Point

Linux backend uses `#[test]` to wrap common test functions.

```rust
// osal-rs-tests/src/linux/mod.rs

#[test]
fn queue_post_fetch() {
    crate::common::queue_tests::test_queue_post_fetch().unwrap();
}

#[test]
fn queue_fifo_order() {
    crate::common::queue_tests::test_queue_fifo_order().unwrap();
}

#[test]
fn mutex_lock_unlock() {
    crate::common::mutex_tests::test_mutex_lock_unlock().unwrap();
}

#[test]
fn semaphore_wait_signal() {
    crate::common::semaphore_tests::test_semaphore_wait_signal().unwrap();
}

#[test]
fn thread_spawn_basic() {
    crate::common::thread_tests::test_thread_spawn_basic().unwrap();
}

#[test]
fn timer_one_shot() {
    crate::common::timer_tests::test_timer_one_shot().unwrap();
}
```

This way, under Linux you can directly run:

```bash
cargo test -p osal-rs-tests --no-default-features --features linux,std
```

---

## 7. Semantics That Must Be Consistent

These are the common OSAL semantics and must be strictly consistent across backends:

- Queue FIFO ordering
- Mutex mutual exclusion
- Semaphore wait/signal
- Thread spawn
- Timer callbacks
- System delay
- Duration/tick conversion
- EventGroup bit wait
- Queue close lifecycle (operations return `Error::QueueClosed` after `close()`)

---

## 8. Semantics That Are "Best-Effort" Under Linux

Linux is not an RTOS. We cannot force Linux to be strictly equivalent to FreeRTOS for these features. Tests should avoid overly strict assertions for:

- Thread priority
- Thread stack size
- Real-time scheduling
- Task suspend
- Task resume
- Task delete
- Precise tick counts
- ISR-safe APIs
- Thread cooperative cancellation (`delete` sets a flag; does not force-terminate)
- Thread join semantics (`join()` is a Linux extension, not in FreeRTOS trait)
- Mutex<T> non-recursive behavior (same-thread double lock returns `Error::MutexLockFailed`)