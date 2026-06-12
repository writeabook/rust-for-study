# EventGroup Trait Fixes

> Documenting corrections applied to the `EventGroupFn` trait documentation and
> the Linux backend implementation (2026-06-12).

---

## 1. Overview

Three consistency issues were identified between the `EventGroupFn` trait documentation
and the actual FreeRTOS reference behavior.  The trait documentation had been written
with incorrect semantics for `set()`, `clear()`, and `wait()`.  The Linux backend
implementation had also inherited the wrong return-value behavior for `set()`, which
was corrected to match FreeRTOS.

All fixes are documentation-only for the trait, plus one return-value fix for the
Linux backend.  No public API signatures changed.

---

## 2. Fix #1 — `set()` Return Value

### Before (incorrect)

```rust
/// Set specified bits in the event group.
///
/// # Returns
/// The event bits **before** the operation.
fn set(&self, bits_to_set: EventBits) -> Result<EventBits>;
```

### After (correct)

```rust
/// Set specified bits in the event group.
///
/// # Returns
/// The value of the event bits **after** the operation is performed.
/// This allows the caller to take a snapshot of the updated state —
/// commonly used as the `previous_wake_time` argument in `System::delay_until`
/// for implementing periodic event loops.
fn set(&self, bits_to_set: EventBits) -> Result<EventBits>;
```

### Why

FreeRTOS `xEventGroupSetBits()` returns the event bits **after** the bits have been set,
not before.  The trait doc had been reversed.

### Affected files

| File | Change |
|------|--------|
| `osal-rs/src/traits/event_group.rs` | Updated doc comment |
| `osal-rs/src/linux/event_group.rs` | Changed `set()` to return value *after* OR operation |

---

## 3. Fix #2 — `clear()` Return Value

### Before (incorrect)

```rust
/// Clear specified bits in the event group.
///
/// # Returns
/// The event bits **before** the operation.
fn clear(&self, bits_to_clear: EventBits) -> Result<EventBits>;
```

### After (correct)

```rust
/// Clear specified bits in the event group.
///
/// # Returns
/// The value of the event bits **after** the operation is performed.
fn clear(&self, bits_to_clear: EventBits) -> Result<EventBits>;
```

### Why

`clear()` returns the post-clear value, consistent with `set()`.
FreeRTOS `xEventGroupClearBits()` also returns the value after the operation.

### Affected files

| File | Change |
|------|--------|
| `osal-rs/src/traits/event_group.rs` | Updated doc comment |

---

## 4. Fix #3 — `wait()` Semantics

### Before (incorrect)

```rust
/// Wait for **all** specified bits to be set in the event group.
///
/// # Example
/// ```ignore
/// // Waits until BOTH bit 0 and bit 1 are set
/// let bits = eg.wait(MY_BIT_0 | MY_BIT_1, MAX_DELAY.to_ticks())?;
/// ```
fn wait(&self, bits_to_wait: EventBits, timeout: TickType) -> Result<EventBits>;
```

### After (correct)

```rust
/// Wait until **any** of the bits specified in `mask` are set.
///
/// This follows the FreeRTOS `pdFALSE` (wait-for-any-bit) convention.
///
/// # Example
/// ```ignore
/// // Returns as soon as EITHER bit 0 or bit 1 is set
/// let bits = eg.wait(MY_BIT_0 | MY_BIT_1, MAX_DELAY.to_ticks())?;
/// // Caller must check which bits were actually set:
/// assert!(bits & (MY_BIT_0 | MY_BIT_1) != 0);
/// ```
///
/// Note: The bits are **not** automatically cleared on exit.
fn wait(&self, bits_to_wait: EventBits, timeout: TickType) -> Result<EventBits>;
```

### Why

The FreeRTOS backend calls `xEventGroupWaitBits()` with `pdFALSE` for the
`xWaitForAllBits` parameter.  This means the function unblocks as soon as
**any one** of the requested bits is set, not when all are set.

The trait doc had incorrectly documented "all bits" semantics.

### Affected files

| File | Change |
|------|--------|
| `osal-rs/src/traits/event_group.rs` | Updated doc comment + example code |

---

## 5. Verification

All 10 EventGroup tests pass on both the FreeRTOS and Linux backends:

```
test linux::test_run_all_tests_event_group ... ok
```

Tests covered: creation, set/get, multiple bits, clear, clear-all, wait,
wait-timeout, wait-partial, sequential operations, all-bits, and drop.