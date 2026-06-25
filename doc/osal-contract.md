# OSAL Contract for `osal-rs`

## 1. Purpose

This document defines the portable behavior contract for the public
`osal-rs` OSAL API.

The contract applies to all backend implementations exposed through
`osal_rs::os::*`, including:

- **FreeRTOS backend**
- **POSIX backend**

Linux is not modeled as a standalone OSAL backend.  Linux host support
is provided through the POSIX backend and the `posix/bsp/generic_linux`
BSP.

The goal is:

- Application code should depend only on the `osal-rs` public OSAL API.
- The same application-level code should behave consistently on FreeRTOS
  and POSIX.
- POSIX (via `generic_linux`) should serve as a development, testing,
  CI, and simulation platform for the OSAL layer.
- Backend differences must be explicit, documented, and testable.

This contract describes **behavior**, not implementation.

---

## 2. Scope and Backend Model

`freertos` and `posix` are OSAL backend features.

`posix` selects the POSIX OS adaptation layer.  It does not select a
specific operating system.

The current POSIX host BSP is `generic_linux`, which provides platform
constants and type aliases for Linux host validation.

Passing the POSIX test suite validates the POSIX backend on the current
host platform.  It does not imply that every POSIX-like platform or
future BSP is supported.

---

## 3. General Backend Principles

### 3.1 API compatibility

Each backend must expose the same public API surface selected through
`osal_rs::os::*`.

A backend must not introduce backend-specific behavior into the common
OSAL API unless it is hidden behind an explicit extension.

### 3.2 Behavior compatibility

Backends should match the same OSAL-level behavior.  They do not need
to share internal implementation details.

For example:

- FreeRTOS may use `xTaskGetTickCount()`.
- POSIX may use `clock_gettime(CLOCK_MONOTONIC)`.
- Both must expose a monotonic OSAL tick count.

### 3.3 Unsupported behavior

If a feature has no meaningful equivalent on a given backend, that
backend must do one of the following:

- provide a safe approximate behavior;
- return a clear unsupported/error result if the API allows it;
- implement a documented no-op only when a no-op is safe and does not
  mislead users.

A backend must not silently claim success for behavior that is not
actually implemented.

### 3.4 Blocking and timeout conventions

Unless otherwise stated:

- `timeout = 0` means non-blocking / immediate return.
- finite timeout means block for at most the requested OSAL tick duration.
- maximum delay value means wait forever where the API supports it.
- timeout expiration should return the same error or false-like result
  across backends.

POSIX host scheduling is not real-time by default.  Therefore, time-based
APIs guarantee that they should not return earlier than the requested
delay, but they do not guarantee exact wake-up timing.

---

## 4. Tick and Duration Contract

### 4.1 Tick meaning

`TickType` represents an OSAL logical tick.

Each backend must define a stable mapping between OSAL ticks and real
elapsed time.  The POSIX backend obtains its tick period from the active
POSIX BSP (`posix/bsp/generic_linux` — `TICK_PERIOD_MS = 1`, one tick
per millisecond).

### 4.2 Monotonic time

`System::get_tick_count()` must return a monotonic tick counter.

It represents elapsed time since backend initialization or process start.

It must not be based on wall-clock time, because wall-clock time can move
backward or jump forward.

Recommended POSIX implementation:

- use a monotonic clock such as `clock_gettime(CLOCK_MONOTONIC)`;
- store a process-start timestamp via `pthread_once_t` initialisation;
- compute elapsed duration from that timestamp;
- convert elapsed duration into OSAL ticks.

### 4.3 Duration to tick conversion

`Duration::to_ticks()` must convert a standard duration into OSAL ticks.

The current baseline behavior is integer division:

```text
ticks = duration_millis * tick_rate_hz / 1000
```

Sub-tick durations may become zero ticks.

If this behavior is later improved to round non-zero durations upward,
the change must be applied consistently across all backends.

### 4.4 Tick to duration conversion

Converting ticks to `Duration` must use the configured OSAL tick rate
or tick period.  The conversion should saturate or fail safely on
overflow.

### 4.5 Delay

`System::delay(ticks)` must block the current execution context for at
least the requested number of OSAL ticks.

The POSIX backend may use a POSIX sleep primitive such as `nanosleep`
or a condition-variable-based wait.  `delay(0)` must return immediately
without blocking.

### 4.6 Delay until

`System::delay_until(previous_wake_time, time_increment)` must implement
periodic-delay behavior:

- calculate the next wake time based on `previous_wake_time + time_increment`;
- sleep until that logical tick time if it is still in the future;
- update `previous_wake_time` to the next wake time;
- if the next wake time has already passed, return without additional
  blocking but still advance `previous_wake_time`.

### 4.7 Current time duration

`System::get_current_time_us()` should return monotonic uptime as a
`Duration`.  Despite the current method name, the semantic meaning
should be "current monotonic uptime", not wall-clock date/time.

### 4.8 Timer check

`System::check_timer(timestamp, time)` must return true if the elapsed
monotonic time since `timestamp` is greater than or equal to `time`.
It must return false otherwise.  Overflow behavior must be safe.

---

## 5. System Contract

### 5.1 Scheduler start / stop

On POSIX host environments, there is no application-level scheduler
equivalent to the FreeRTOS scheduler.

The POSIX backend may implement `System::start()` and `System::stop()`
as documented no-ops unless a future POSIX runtime manager is introduced.

### 5.2 Suspend and resume all

`System::suspend_all()` and `System::resume_all()` have no safe
process-wide POSIX equivalent.  The POSIX backend may implement them as
documented no-ops.  Tests must not rely on them for mutual exclusion.

### 5.3 Critical section

FreeRTOS critical sections disable interrupts or enter kernel critical
regions.  POSIX host user space cannot disable interrupts.

The POSIX backend implements critical sections as process-local OSAL
critical sections using a recursive `pthread_mutex_t`, with per-thread
nesting depth tracked via `pthread_key_t` TLS.  This provides mutual
exclusion among OSAL callers but does **not** disable OS scheduling or
hardware interrupts.

### 5.4 ISR APIs

POSIX host user space has no true ISR context.

Any API ending in `_from_isr` must be mapped carefully:

- `_from_isr` functions should be non-blocking.
- They may call the same non-blocking logic as normal APIs.
- They must not block.
- They must not falsely emulate hardware interrupt semantics.
- If the operation cannot be safely supported, it should return failure
  or an unsupported error.

### 5.5 Yield from ISR

`System::yield_from_isr()` and `System::end_switching_isr()` are FreeRTOS
scheduling hooks.  The POSIX backend may implement them as documented
no-ops.

---

## 6. Mutex Contract

### 6.1 Mutex kind

The OSAL mutex is recursive.  The same thread may acquire the same mutex
multiple times.

Each successful lock must be matched by one unlock.  The mutex is fully
released only when the recursion count reaches zero.

### 6.2 Lock behavior

`Mutex::lock()` must block until the mutex is acquired or until the
backend reports an unrecoverable error.

### 6.3 Guard behavior

The lock guard must provide RAII semantics.  When the guard is dropped,
exactly one lock level must be released.  If the same thread locked the
mutex three times, dropping one guard releases only one recursion level.

### 6.4 Mutual exclusion

The mutex must protect access to the contained value.  If one thread
holds the mutex, another thread must not enter the protected critical
section until the mutex is released.

### 6.5 Recursive ownership

The backend must track ownership.  Only the owning thread may recursively
acquire the mutex without blocking.  A different thread must block or
fail according to the API being used.

### 6.6 ISR lock behavior

If `lock_from_isr()` exists in the common API, the POSIX backend should
treat it as a non-blocking try-lock operation:

- return success if the mutex is immediately available;
- return failure if another thread owns the mutex;
- never block.

### 6.7 Implementation note

The contract does not require a specific implementation.  The FreeRTOS
backend may use FreeRTOS recursive mutex primitives.  The POSIX backend
may use `pthread_mutex_t` (PTHREAD_MUTEX_RECURSIVE / ERRORCHECK) or its
internal `posix/sys` wrappers.

---

## 7. Semaphore Contract

### 7.1 Semaphore type

The semaphore is a counting semaphore with a maximum count, current
count, wait/take operation, and signal/give operation.

### 7.2 Creation

`Semaphore::new(max_count, initial_count)` must create a semaphore with
the specified maximum and initial count.

If `initial_count > max_count`, creation must fail rather than silently
creating an invalid semaphore.

### 7.3 Wait

`wait(timeout)` attempts to decrement the semaphore count:

- if count > 0, decrement count and return true;
- if count == 0 and timeout == 0, return false immediately;
- if count == 0 and timeout is finite, block until signaled or timeout
  expires;
- if timeout expires, return false;
- if wait-forever is requested, block until signaled.

### 7.4 Signal

`signal()` increments the semaphore count:

- if count < max_count, increment count and wake one waiter;
- if count == max_count, return false.

### 7.5 ISR variants

`_from_isr` semaphore functions must be non-blocking.  On POSIX host
environments, they should behave as non-blocking variants and must not
emulate hardware interrupt priority.

---

## 8. Queue Contract

### 8.1 Queue types

The OSAL queue contract covers both raw message queues and typed streamed
queues:

- `Queue` transports fixed-size raw messages.
- `QueueStreamed<T>` transports typed messages by serializing /
  deserializing `T` through `osal-rs-serde` when the `serde` feature is
  enabled.

Both types share the same behavioral contract:

- bounded FIFO;
- fixed capacity;
- timeout on full / empty;
- post wakes receiver;
- fetch wakes sender.

`QueueStreamed<T>` must preserve the same FIFO and timeout behavior as
the underlying queue.  Serialization failure must be reported as an OSAL
error and must not enqueue a partial message.  Deserialization failure
must be reported as an OSAL error and must not expose partially
initialized user data.

### 8.2 Creation

Creating a queue with invalid capacity or invalid message size must fail.

### 8.3 Post / send

- if the queue is not full, push item and return success;
- if the queue is full and timeout == 0, return queue-full or timeout
  failure immediately;
- if the queue is full and timeout is finite, wait until space becomes
  available or timeout expires;
- if wait-forever is requested, block until space is available.

### 8.4 Fetch / receive

- if the queue is not empty, pop oldest item (FIFO) and return success;
- if the queue is empty and timeout == 0, return timeout failure
  immediately;
- if the queue is empty and timeout is finite, wait until an item
  becomes available or timeout expires;
- if wait-forever is requested, block until an item is available.

### 8.5 Wake-up rules

When an item is posted, one waiting receiver should be woken.  When an
item is fetched, one waiting sender should be woken.

### 8.6 ISR variants

`_from_isr` queue APIs, if present, must be non-blocking.  They may map
to immediate try-send or try-receive behavior.

---

## 9. EventGroup Contract

### 9.1 Event bits

An event group stores event bits.  The common usable mask should follow
the FreeRTOS-compatible model where the top bits may be reserved.

### 9.2 Set

`set(bits)` sets the specified bits and returns the event bits after the
operation.  Any waiters whose condition becomes true should be woken.

### 9.3 Get / Clear

`get()` returns the current event bits (non-blocking).  `clear(bits)`
clears the specified bits.

### 9.4 Wait

`wait(mask, timeout_ticks)` must wait until any of the specified bits in
`mask` is set:

- wait for any bit, not all bits;
- do not auto-clear bits on exit;
- timeout is expressed in OSAL ticks;
- return the current event bits when the function returns;
- caller checks `returned_bits & mask != 0` for success;
- if timeout expires, the returned bits may not contain the requested mask.

### 9.5 ISR variants

`_from_isr` event-group APIs must be non-blocking.  They may reuse the
same internal lock briefly, but they must not wait on a condition variable.

---

## 10. Timer Contract

### 10.1 Timer type

A software timer with name, period, one-shot or periodic mode, callback,
start / stop / reset / change-period operations.

### 10.2 Callback execution

The callback must execute after the timer expires.  A POSIX host backend
may use a background timer worker thread.

The callback must not run before the configured period elapses.  It may
run later due to host scheduling latency.  Real-time precision is
backend-dependent; the contract guarantees ordering and minimum delay,
not exact wake-up latency.

### 10.3 Periodic behavior

For periodic timers, the timer should reschedule after each callback.
The period should be measured from the scheduled expiration time where
practical, not merely from callback completion time.

### 10.4 Stop / Reset / Change period

- `stop()` should prevent future callback execution.  If a callback is
  already running, `stop()` does not need to forcibly interrupt it.
- `reset()` should restart the countdown from the time of reset.
- `change_period()` should update the timer period for subsequent
  expirations.

---

## 11. Thread Contract

### 11.1 Thread creation

A thread has a name, stack size, priority, and entry closure/function.

The POSIX backend may use `pthread_create` / `pthread_join` through
`posix/sys/thread` wrappers.  POSIX host scheduling priority is not
equivalent to FreeRTOS task priority unless real-time scheduling support
is explicitly implemented.

### 11.2 Thread lifecycle

If the common API separates construction and start, the backend must
preserve that lifecycle: a created thread should not execute user code
until `start()` is called.

### 11.3 Join

If join is part of the public API, it should wait until the target thread
exits and return its result/status according to the existing API.

### 11.4 Priority

FreeRTOS priority maps directly to RTOS scheduling priority.  POSIX host
priority is advisory unless real-time scheduling is explicitly configured.

The backend must document that priority is currently informational
unless a real-time scheduling feature is enabled.

### 11.5 Thread state

POSIX host thread state information may be approximate.  The backend may
expose limited states (Created, Running, Finished, Invalid/Unknown) and
must document the limitation.

---

## 12. Error Contract

### 12.1 Error consistency

Backends should use the existing `osal-rs` error types.  Backends must
not expose raw backend-specific error codes such as `errno` or FreeRTOS
status codes through the common public API unless explicitly wrapped.

### 12.2 Allocation failure

If object creation fails because memory or system resources are
unavailable, return the same allocation-related error used by other
backends.

### 12.3 Timeout

Timeout failures should use the same timeout-style error or false return
behavior across backends.

### 12.4 Unsupported behavior

If the existing error type has no `Unsupported` variant, the backend may
return the closest safe existing error.  A later API improvement may add
an explicit unsupported error.

---

## 13. POSIX Host Notes

The current POSIX backend implements all OSAL primitives using POSIX
APIs through `posix/sys` wrappers (pthread mutex, pthread condvar,
`clock_gettime(CLOCK_MONOTONIC)`, `pthread_create`/`pthread_join`).

Global initialisation uses `pthread_once_t`; per-thread state uses
`pthread_key_t` TLS; the default allocator delegates to
`libc::malloc`/`libc::free`.

Platform constants and type aliases are provided by
`posix/bsp/generic_linux` (the current and only BSP target).

---

## 14. Conformance Tests

The same contract tests should run against every backend where practical.

Current host-side validation commands:

```bash
cargo test -p osal-rs-tests --no-default-features --features posix
cargo test -p osal-rs-tests --no-default-features --features "posix serde"
```

FreeRTOS conformance may require target or simulator-specific runners.

### Minimum contract tests

**Time / System:** tick count is monotonic; delay waits at least the
requested time; delay_until updates previous wake time; duration-to-tick
conversion matches contract; check_timer returns correct results.

**Mutex:** basic create/lock/unlock; guard drop unlocks; recursive lock
by same thread works; other thread blocks while mutex is held;
multi-thread counter test reaches expected value; from_isr/try-lock is
non-blocking.

**Semaphore:** initial count works; wait decrements count; signal
increments count; wait times out when count is zero; signal wakes a
waiting thread; signal at max count returns false.

**Queue:** FIFO order; send succeeds when not full; receive succeeds
when not empty; send fails/timeout when full; receive fails/timeout
when empty; blocked sender wakes after receiver consumes; blocked
receiver wakes after sender posts.

**EventGroup:** set/get works; clear works; wait returns when any
requested bit is set; wait does not auto-clear bits; wait times out
when no requested bit is set.

**Timer:** one-shot timer fires once; periodic timer fires repeatedly;
stop prevents later callbacks; reset restarts countdown; change period
updates timing.
