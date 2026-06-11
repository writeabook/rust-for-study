# OSAL Contract for `osal-rs` v0.1

## 1. Purpose

This document defines the expected behavior of the Linux backend for `osal-rs`.

The Linux backend should not define a separate behavior model. It should implement the same public OSAL semantics currently represented by the FreeRTOS backend, unless a FreeRTOS-specific feature has no meaningful Linux userspace equivalent.

The goal is:

* Application code should depend only on the `osal-rs` public OSAL API.
* The same application-level code should behave consistently on FreeRTOS and Linux.
* Linux should serve as a development, testing, CI, and simulation backend for the OSAL layer.
* Backend differences must be explicit, documented, and testable.

This contract describes behavior, not implementation.

---

## 2. General Backend Principles

### 2.1 API compatibility

The Linux backend must expose the same public API surface as the existing OSAL backend selected through `osal_rs::os::*`.

The Linux backend must not introduce Linux-only behavior into the common OSAL API unless it is hidden behind a Linux-specific extension feature.

### 2.2 Behavior compatibility

The Linux backend should match the FreeRTOS backend at the OSAL behavior level.

It does not need to copy FreeRTOS internal implementation details.

For example:

* FreeRTOS may use `xTaskGetTickCount()`.
* Linux may use `std::time::Instant`.
* Both must expose a monotonic OSAL tick count.

### 2.3 Unsupported behavior

If a FreeRTOS feature has no direct Linux userspace equivalent, the Linux backend must do one of the following:

* provide a safe approximate behavior;
* return a clear unsupported/error result if the API allows it;
* implement a documented no-op only when a no-op is safe and does not mislead users.

The Linux backend must not silently claim success for behavior that is not actually implemented.

### 2.4 Blocking and timeout conventions

Unless otherwise stated:

* `timeout = 0` means non-blocking / immediate return.
* finite timeout means block for at most the requested OSAL tick duration.
* maximum delay value means wait forever where the API supports it.
* timeout expiration should return the same error or false-like result as the FreeRTOS backend.

Linux userspace scheduling is not real-time by default. Therefore, time-based APIs guarantee that they should not return earlier than the requested delay, but they do not guarantee exact wake-up timing.

---

## 3. Tick and Duration Contract

### 3.1 Tick meaning

`TickType` represents an OSAL logical tick.

The Linux backend must define a stable mapping between one OSAL tick and wall-clock duration. The first version should use the same tick-period model as the existing OSAL configuration.

### 3.2 Monotonic time

`System::get_tick_count()` must return a monotonic tick counter.

It represents elapsed time since backend initialization or process start.

It must not be based on wall-clock time, because wall-clock time can move backward or jump forward.

Recommended Linux implementation:

* use `std::time::Instant`;
* store a process-start `Instant`;
* compute elapsed duration from that instant;
* convert elapsed duration into OSAL ticks.

### 3.3 Duration to tick conversion

`Duration::to_ticks()` must convert a standard duration into OSAL ticks.

The first Linux version should preserve the existing FreeRTOS-style integer conversion behavior unless the whole project later decides to change the shared contract.

Current baseline behavior:

```text
ticks = duration_millis * tick_rate_hz / 1000
```

This is integer division. Therefore, sub-tick durations may become zero ticks.

If this behavior is later improved to round non-zero durations upward, the change must be applied consistently across all backends.

### 3.4 Tick to duration conversion

Converting ticks to `Duration` must use the configured OSAL tick rate or tick period.

The conversion should saturate or fail safely on overflow.

### 3.5 Delay

`System::delay(ticks)` must block the current execution context for at least the requested number of OSAL ticks.

Linux implementation may use `std::thread::sleep`.

`delay(0)` should be documented as no delay. If the implementation chooses to yield instead, that behavior must be documented and tested.

### 3.6 Delay until

`System::delay_until(previous_wake_time, time_increment)` must implement periodic-delay behavior.

Expected behavior:

* calculate the next wake time based on `previous_wake_time + time_increment`;
* sleep until that logical tick time if it is still in the future;
* update `previous_wake_time` to the next wake time;
* if the next wake time has already passed, return without additional blocking but still advance `previous_wake_time`.

This should be compatible with periodic task loops.

### 3.7 Current time duration

`System::get_current_time_us()` should return monotonic uptime as a `Duration`.

Despite the current method name, the semantic meaning should be ��current monotonic uptime��, not wall-clock date/time.

### 3.8 Timer check

`System::check_timer(timestamp, time)` must return true if the elapsed monotonic time since `timestamp` is greater than or equal to `time`.

It must return false otherwise.

Overflow behavior must be safe.

---

## 4. System Contract

### 4.1 Scheduler start

`System::start()` starts the FreeRTOS scheduler on FreeRTOS.

On Linux userspace, there is no equivalent application-level scheduler to start.

Linux backend v0.1 may implement `System::start()` as:

* no-op; or
* a documented blocking runtime entry point if the project later introduces one.

For the first Linux backend, no-op is acceptable, but it must be documented.

### 4.2 Scheduler stop

`System::stop()` stops the FreeRTOS scheduler on FreeRTOS.

On Linux userspace, there is no equivalent global scheduler stop.

Linux backend v0.1 may implement `System::stop()` as a documented no-op.

### 4.3 Suspend and resume all

`System::suspend_all()` and `System::resume_all()` suspend and resume FreeRTOS scheduling.

Linux userspace has no safe process-wide equivalent.

Linux backend v0.1 should not pretend to globally stop all threads.

Acceptable v0.1 behavior:

* no-op with documentation; or
* internal OSAL-runtime lock if the Linux backend later owns all OSAL threads.

If implemented as no-op, tests must not rely on it for mutual exclusion.

### 4.4 Critical section

FreeRTOS critical sections disable interrupts or enter kernel critical regions.

Linux userspace cannot disable interrupts.

Linux backend v0.1 should define critical sections as process-local OSAL critical sections only if it has a global lock.

Otherwise, `critical_section_enter()` and `critical_section_exit()` may be documented no-ops.

They must not be used to protect shared data in Linux backend tests unless implemented with real synchronization.

### 4.5 ISR APIs

Linux userspace has no true ISR context.

Any API ending in `_from_isr` must be mapped carefully.

Recommended Linux v0.1 rule:

* `_from_isr` functions should be non-blocking.
* They may call the same non-blocking logic as normal APIs.
* They must not block.
* They must not falsely emulate hardware interrupt semantics.
* If the operation cannot be safely supported, it should return failure or an unsupported error if available.

### 4.6 Yield from ISR

`System::yield_from_isr()` and `System::end_switching_isr()` are FreeRTOS scheduling hooks.

Linux backend v0.1 may implement them as documented no-ops.

---

## 5. Mutex Contract

### 5.1 Mutex kind

The OSAL mutex contract should follow the current FreeRTOS backend behavior:

The mutex is recursive.

This means the same thread may acquire the same mutex multiple times.

Each successful lock must be matched by one unlock.

The mutex is fully released only when the recursion count reaches zero.

### 5.2 Lock behavior

`Mutex::lock()` must block until the mutex is acquired or until the backend reports an unrecoverable error.

For v0.1, `lock()` should behave as an indefinite wait.

### 5.3 Guard behavior

The lock guard must provide RAII semantics.

When the guard is dropped, exactly one lock level must be released.

If the same thread locked the mutex three times, dropping one guard releases only one recursion level.

### 5.4 Mutual exclusion

The mutex must protect access to the contained value.

If one thread holds the mutex, another thread must not enter the protected critical section until the mutex is released.

### 5.5 Recursive ownership

The backend must track ownership.

Only the owning thread may recursively acquire the mutex without blocking.

A different thread must block or fail according to the API being used.

### 5.6 ISR lock behavior

If `lock_from_isr()` exists in the common API, Linux backend v0.1 should treat it as a non-blocking try-lock operation.

Expected behavior:

* return success if the mutex is immediately available;
* return failure if another thread owns the mutex;
* never block.

If recursive try-lock from the owner is supported, it must increase the recursion count.

### 5.7 Implementation note

Linux v0.1 may implement recursive mutex using Rust standard synchronization primitives:

* internal `std::sync::Mutex<State>`;
* `std::sync::Condvar`;
* owner thread id;
* recursion count.

It does not need to use pthread FFI in v0.1.

---

## 6. Semaphore Contract

### 6.1 Semaphore type

The semaphore is a counting semaphore.

It has:

* a maximum count;
* a current count;
* wait/take operation;
* signal/give operation.

### 6.2 Creation

`Semaphore::new(max_count, initial_count)` must create a semaphore with the specified maximum and initial count.

If `initial_count > max_count`, Linux backend should return an error rather than silently creating an invalid semaphore.

If allocation fails, it should return the same error category used by the FreeRTOS backend for allocation failure.

### 6.3 Wait

`wait(timeout)` attempts to decrement the semaphore count.

Expected behavior:

* if count > 0, decrement count and return true;
* if count == 0 and timeout == 0, return false immediately;
* if count == 0 and timeout is finite, block until signaled or timeout expires;
* if timeout expires, return false;
* if wait-forever is requested, block until signaled.

### 6.4 Signal

`signal()` increments the semaphore count.

Expected behavior:

* if count < max_count, increment count and wake one waiter;
* if count == max_count, return false or failure according to the existing OSAL API.

### 6.5 ISR variants

Linux `_from_isr` semaphore functions must be non-blocking.

`wait_from_isr()` should attempt to take immediately.

`signal_from_isr()` should signal without blocking.

They should not emulate hardware interrupt priority or context switching.

---

## 7. Queue Contract

### 7.1 Queue type

The queue is a bounded FIFO message queue.

It has:

* fixed capacity;
* fixed message type or message size according to the existing API;
* send/post operation;
* receive/fetch operation;
* timeout behavior.

### 7.2 Creation

Creating a queue with invalid capacity or invalid message size must fail.

The Linux backend must not create a queue that cannot actually store messages.

### 7.3 Post/send

Posting to a queue must follow FIFO behavior.

Expected behavior:

* if the queue is not full, push item and return success;
* if the queue is full and timeout == 0, return queue-full or timeout-style failure immediately;
* if the queue is full and timeout is finite, wait until space becomes available or timeout expires;
* if timeout expires, return the same error category used by the FreeRTOS backend for failed queue send;
* if wait-forever is requested, block until space is available.

### 7.4 Fetch/receive

Fetching from a queue must remove the oldest item.

Expected behavior:

* if the queue is not empty, pop oldest item and return success;
* if the queue is empty and timeout == 0, return timeout-style failure immediately;
* if the queue is empty and timeout is finite, wait until item becomes available or timeout expires;
* if wait-forever is requested, block until an item is available.

### 7.5 Wake-up rules

When an item is posted, one waiting receiver should be woken.

When an item is fetched, one waiting sender should be woken.

### 7.6 ISR variants

Linux queue `_from_isr` APIs, if present, must be non-blocking.

They may map to immediate try-send or try-receive behavior.

They must not block.

---

## 8. EventGroup Contract

### 8.1 Event bits

An event group stores event bits.

The common usable mask should follow the FreeRTOS-compatible model where the top bits may be reserved.

Linux backend should preserve the same `MAX_MASK` behavior as the common API.

### 8.2 Set

`set(bits)` sets the specified bits.

It returns the event bits after the operation.

Any waiters whose condition becomes true should be woken.

### 8.3 Get

`get()` returns the current event bits.

It is non-blocking.

### 8.4 Clear

`clear(bits)` clears the specified bits.

It returns the event bits before or after clear according to the current public API behavior. The Linux backend must match the FreeRTOS backend behavior used by the OSAL API tests.

### 8.5 Wait

`wait(mask, timeout_ticks)` must wait until any of the specified bits in `mask` is set.

The default contract follows the current FreeRTOS backend call style:

* wait for any bit, not all bits;
* do not clear bits automatically on exit;
* timeout is expressed in OSAL ticks;
* return the current event bits when the function returns.

The caller determines success by checking:

```text
returned_bits & mask != 0
```

If timeout expires, the returned bits may not contain the requested mask.

### 8.6 ISR variants

Linux event-group `_from_isr` APIs must be non-blocking.

They may reuse the same internal lock briefly, but they must not wait on a condition variable.

---

## 9. Timer Contract

### 9.1 Timer type

The timer is a software timer.

It has:

* name;
* period;
* one-shot or periodic mode if supported by the existing API;
* callback;
* start operation;
* stop operation;
* reset operation;
* change-period operation.

### 9.2 Callback execution

The callback must execute after the timer expires.

Linux backend v0.1 may use a background timer manager thread.

The callback must not run before the configured period elapses.

The callback may run later due to Linux scheduling latency.

### 9.3 Periodic behavior

For periodic timers, the timer should reschedule after each callback.

The period should be measured from the scheduled expiration time where practical, not merely from the callback completion time, unless the implementation documents otherwise.

### 9.4 Stop

Stopping a timer should prevent future callback execution.

If a callback is already running, `stop()` does not need to forcibly interrupt it.

### 9.5 Reset

Resetting a timer should restart its countdown from the time of reset.

### 9.6 Change period

Changing a timer period should update the timer period for subsequent expirations.

If the timer is active, the implementation must document whether the new period takes effect immediately or on the next restart.

### 9.7 Implementation note

Timer should not be the first Linux backend module implemented.

It depends on correct time, thread, mutex, and condition-variable behavior.

---

## 10. Thread Contract

### 10.1 Thread creation

A thread has:

* name;
* stack size;
* priority;
* entry closure/function.

Linux backend v0.1 may use `std::thread::Builder`.

The thread name and stack size should be passed to the Linux/Rust thread builder where possible.

### 10.2 Thread start

If the common API separates construction and start, Linux must preserve that lifecycle.

A created thread should not execute user code until `start()` is called.

If this is difficult with `std::thread`, Linux backend should internally store the closure and spawn only on `start()`.

### 10.3 Join

If join is part of the public API, it should wait until the target thread exits and return its result/status according to the existing API.

### 10.4 Priority

FreeRTOS priority maps directly to RTOS scheduling priority.

Linux userspace priority is not equivalent.

Linux backend v0.1 should preserve the priority field but does not need to enforce real scheduling priority.

It must document that priority is currently informational unless a future Linux real-time feature is enabled.

It must not silently imply deterministic priority scheduling.

### 10.5 Thread state

Linux thread state information may be approximate.

If exact FreeRTOS-like states are not available, Linux backend may expose limited states such as:

* Created;
* Running;
* Finished;
* Invalid/Unknown.

The limitation must be documented.

---

## 11. Error Contract

### 11.1 Error consistency

Linux backend should use the existing `osal-rs` error types.

It should not introduce Linux-specific raw errno values into the public common API.

### 11.2 Allocation failure

If object creation fails because memory or system resources are unavailable, return the same allocation-related error used by the FreeRTOS backend.

### 11.3 Timeout

Timeout failures should use the same timeout-style error or false return behavior as the current FreeRTOS backend.

### 11.4 Unsupported behavior

If the existing error type has no `Unsupported` variant, Linux backend v0.1 may return the closest safe existing error.

A later API improvement may add an explicit unsupported error.

---

## 12. Linux Backend Implementation Strategy v0.1

The first Linux backend should prefer safe Rust standard library primitives.

Recommended mapping:

```text
System time      -> std::time::Instant
Delay            -> std::thread::sleep
Thread           -> std::thread::Builder
Recursive Mutex  -> std::sync::Mutex + std::sync::Condvar + owner ThreadId + recursion count
Semaphore        -> std::sync::Mutex + std::sync::Condvar + counter
Queue            -> std::sync::Mutex + std::sync::Condvar + VecDeque
EventGroup       -> std::sync::Mutex + std::sync::Condvar + bitmask
Timer            -> background TimerManager thread, later phase
```

Linux backend v0.1 does not need direct FFI.

FFI or Linux-native APIs may be added later for:

* pthread recursive mutex;
* realtime scheduling;
* CPU affinity;
* timerfd;
* eventfd;
* epoll;
* futex;
* `/proc` runtime statistics.

---

## 13. Conformance Tests

Each contract rule should have backend-independent tests.

The same test cases should run against FreeRTOS and Linux where possible.

Minimum Linux v0.1 tests:

### Time/System

* tick count is monotonic;
* delay waits at least the requested time;
* delay_until updates previous wake time;
* duration-to-tick conversion matches contract;
* check_timer returns false before expiration and true after expiration.

### Mutex

* basic create/lock/unlock;
* guard drop unlocks;
* protected value mutation works;
* recursive lock by same thread works;
* other thread blocks while mutex is held;
* multi-thread counter test reaches exact expected value;
* from_isr/try-lock behavior is non-blocking.

### Semaphore

* initial count works;
* wait decrements count;
* signal increments count;
* wait times out when count is zero;
* signal wakes a waiting thread;
* signal at max count fails or returns false.

### Queue

* FIFO order;
* send succeeds when not full;
* receive succeeds when not empty;
* send fails or times out when full;
* receive fails or times out when empty;
* blocked sender wakes after receiver consumes;
* blocked receiver wakes after sender posts.

### EventGroup

* set/get works;
* clear works;
* wait returns when any requested bit is set;
* wait does not auto-clear bits;
* wait times out when no requested bit is set.

### Timer

* one-shot timer fires once;
* periodic timer fires repeatedly;
* stop prevents later callbacks;
* reset restarts countdown;
* change period updates timing.

Timer tests may be postponed until the Linux backend has stable thread and synchronization primitives.

---

## 14. Development Order

Recommended Linux backend development order:

```text
1. Time / Duration / System tick
2. Recursive Mutex
3. Semaphore
4. Queue
5. EventGroup
6. Thread lifecycle
7. Timer
8. Linux-specific extensions
```

The first milestone should not include Linux-specific APIs.

The first milestone should prove that the common OSAL behavior can run safely on Linux userspace.
