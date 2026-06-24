# osal-rs-tests

This crate contains the verification suite for `osal-rs`.

The primary purpose of this crate is to verify the **public OSAL API contract**.
Tests are organized by OSAL capability, not by operating system backend.

## Test Layers

### `api/`

Public OSAL API contract tests. These tests define behavior that every supported
backend must satisfy. They are backend-agnostic — they only use `osal_rs::os::*`
and never reference backend-internal types.

API contract tests must cover not only sequential API calls, but also core OSAL
concurrency semantics such as blocking, wakeup, timeout, cross-thread signaling,
and race-sensitive behavior. Tests in this layer use the OSAL Thread API for
concurrency, not `std::thread::spawn`, so they remain portable to FreeRTOS.

### `unit/`

Internal utility and pure Rust logic tests. Small helpers, type conversions,
error handling — white-box tests for library internals.

### `coverage/`

Boundary, error-path, and defensive tests. Invalid parameters, timeout edge
cases, zero/max duration, empty names, queue full/empty, resource lifecycle,
handle identity, and API surface completeness checks. Covers what `api/`
success-path tests do not.

### `port/`

Minimal port bring-up and smoke tests. These verify that a specific backend
can be built and minimally exercised — create/use/destroy for each primitive.

**`port/` must not become a second API test suite.** It should only contain
minimal bring-up, smoke tests, and documented port limitations. Full API
contract tests live in `api/` and are exercised by every backend runner.

### `common/`

Shared test helpers only. Timeout constants, assertion macros, test scaffolding.
This directory must not contain `#[test]` functions or backend-specific logic.

## Backend Policy

| Backend | Status | Notes |
|---|---|---|
| `posix` | Primary host backend | Supports Linux, macOS, and other POSIX-like systems. Default. |
| `freertos` | Primary embedded RTOS backend | Requires target runner / hardware / QEMU for execution. |
| `linux` | Transitional / legacy | Pure Rust reference implementation. The legacy `linux` backend is transitional and should not be expanded. Linux-specific tests should not grow unless they verify a legacy implementation detail that cannot be expressed through the portable OSAL API. May be removed after POSIX fully covers host functionality. |

## Running Tests

```bash
# POSIX (default)
cargo test -p osal-rs-tests

# Linux legacy
cargo test -p osal-rs-tests --no-default-features --features linux

# FreeRTOS (check only; execution requires embedded target)
cargo check -p osal-rs-tests --no-default-features --features freertos
```

## Rule of Thumb

- If a test describes what **every OSAL backend should do**, put it in `api/`.
- If a test describes **internal helper behavior**, put it in `unit/`.
- If a test covers **invalid inputs, edge cases, or error paths**, put it in `coverage/`.
- If a test only checks that a **specific port can be built or minimally initialized**, put it in `port/`.
- Do **not** duplicate API contract tests under backend-specific directories.

## Directory Structure

```
src/
  api/         OSAL API contract tests (sequential + concurrency)
  unit/        Internal utility tests
  coverage/    Boundary, error-path, lifecycle, and timeout tests
  port/        Backend smoke / bring-up tests (minimal only)
  common/      Shared test helpers
  linux/       Linux legacy runner (individual #[test] per API test)
  posix/       POSIX runner (individual #[test] per API test)
  freertos/    FreeRTOS runner (manual run_all_tests entry point)
```

## Host Runner Granularity

POSIX and Linux host runners expose each API test function as an individual
`#[test]`. This ensures `cargo test` output names the exact test case on
failure rather than an aggregate `run_all_tests` runner.

FreeRTOS retains `run_all_tests()` as the manual aggregation entry point for
use from embedded firmware tasks.
