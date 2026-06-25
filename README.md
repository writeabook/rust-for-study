# OSAL-RS

Operating System Abstraction Layer for Rust - A cross-platform compatibility layer for embedded and real-time systems development.

[![Crates.io](https://img.shields.io/crates/v/osal-rs.svg)](https://crates.io/crates/osal-rs)
[![Documentation](https://docs.rs/osal-rs/badge.svg)](https://docs.rs/osal-rs)
[![License: LGPL-2.1](https://img.shields.io/badge/License-LGPL%202.1-blue.svg)](LICENSE)

## Overview

OSAL-RS provides a unified API for developing multi-platform embedded applications in Rust. It abstracts operating system-specific functionality, allowing you to write portable code that can run on different platforms with minimal changes.

### Workspace Components

- **osal-rs**: Main Operating System Abstraction Layer with FreeRTOS and POSIX backends
- **osal-rs-build**: Build configuration tools and helpers
- **osal-rs-porting**: C FFI bridge layer for FreeRTOS integration
- **osal-rs-tests**: Comprehensive test suite for all components
- **osal-rs-serde**: ✨ Extensible serialization/deserialization framework with derive macros

## Current Implementation Status

- ✅ **FreeRTOS**: Fully implemented and tested
- ✅ **Serialization**: Complete osal-rs-serde implementation with derive macros
- ✅ **POSIX**: Native pthread + libc backend (mutex, semaphore, queue, event-group, thread, timer, system) — Linux host via `generic_linux` BSP
- 🚧 **Other RTOSes**: Under consideration

## Features

### Core OSAL Features
- **Thread Management**: Create, manage, and synchronize threads with priorities
- **Synchronization Primitives**: Mutexes (recursive & non-recursive), binary & counting semaphores, event groups
- **Message Queues**: Type-safe inter-thread communication with blocking/non-blocking operations
- **Software Timers**: Periodic and one-shot timers with callbacks
- **Memory Allocation**: Custom allocator integration for heap management
- **Time Management**: Duration handling and tick-based timing
- **System Control**: Scheduler control, task notifications, and system information
- **No-std Support**: Fully compatible with bare-metal embedded systems

### 🆕 osal-rs-serde Features

A complete serialization framework designed specifically for embedded systems:

- **No-std Compatible**: Works in bare-metal environments without standard library
- **Zero-Copy**: Direct buffer operations with no intermediate allocations
- **Derive Macros**: Automatic `#[derive(Serialize, Deserialize)]` implementation
- **Rich Type Support**: Primitives, arrays, tuples, Option<T>, Vec<T>, nested structs
- **Extensible Architecture**: Create custom serializers for any format (JSON, MessagePack, CBOR, etc.)
- **Memory Efficient**: Little-endian binary format with predictable sizes
- **Compile-Time Guarantees**: Type-safe serialization with static checks
- **Standalone**: Can be used independently in any Rust project

#### osal-rs-serde Quick Example

```rust
use osal_rs_serde::{Serialize, Deserialize, to_bytes, from_bytes};

#[derive(Serialize, Deserialize, Debug, PartialEq)]
struct SensorData {
    temperature: i16,
    humidity: u8,
    pressure: u32,
    status: Option<u8>,
}

let data = SensorData { 
    temperature: 25, 
    humidity: 60, 
    pressure: 1013,
    status: Some(0xFF),
};

// Serialize to stack buffer
let mut buffer = [0u8; 32];
let len = to_bytes(&data, &mut buffer).unwrap();

// Deserialize from buffer
let restored: SensorData = from_bytes(&buffer[..len]).unwrap();
assert_eq!(data, restored);
```

#### Integration with OSAL Queues

For typed inter-task communication, prefer `QueueStreamed<T>`
(automatic serialisation — see `typed_message_queue_demo.rs`):

```rust
use osal_rs::os::QueueStreamed;
use osal_rs::traits::BytesHasLen;
use osal_rs_serde::{Deserialize, Serialize};

const CMD_SIZE: usize = 12; // u32 + 4 × u16

#[derive(Serialize, Deserialize)]
struct Command { id: u32, params: [u16; 4] }

impl BytesHasLen for Command {
    fn len(&self) -> usize { CMD_SIZE }
}

let queue: QueueStreamed<Command> = QueueStreamed::new(10, CMD_SIZE as _).unwrap();
queue.post(&Command { id: 42, params: [1, 2, 3, 4] }, 100).unwrap();

let mut cmd = Command { id: 0, params: [0; 4] };
queue.fetch(&mut cmd, 100).unwrap();
```

Raw `Queue` with manual `to_bytes` / `from_bytes` is also available
when you need full control over byte buffers.

For comprehensive documentation, examples, and advanced features, see:
- [osal-rs-serde README](osal-rs-serde/README.md) - Complete feature documentation
- [osal-rs-serde/derive README](osal-rs-serde/derive/README.md) - Derive macro guide
- `osal-rs-serde/examples/` - Working code examples

## Prerequisites

Before using OSAL-RS in your project, ensure that:

1. **FreeRTOS is properly configured** in your project
2. **FreeRTOS is linked** to your final binary
3. **C porting layer files** from `osal-rs-porting` must be compiled and linked to your project
4. **CMake build system** is set up for your embedded project
5. **Rust toolchain** with appropriate target support is installed

### Configuration

OSAL-RS requires proper FreeRTOS configuration. Ensure your `FreeRTOSConfig.h` includes:

```c
#define configUSE_MUTEXES                1
#define configUSE_RECURSIVE_MUTEXES      1
#define configUSE_COUNTING_SEMAPHORES    1
#define configUSE_TIMERS                 1
#define configUSE_QUEUE_SETS             1
#define configSUPPORT_DYNAMIC_ALLOCATION 1
```

## CMake Integration

OSAL-RS is designed to be integrated into CMake-based projects. Here are several integration examples:

**Important**: Always ensure that the C porting layer files from `osal-rs-porting/freertos/` are compiled and linked to your project, as they provide the necessary FFI bridge between Rust and FreeRTOS.

### Basic CMake Integration

Add OSAL-RS to your existing CMake project:

```cmake
cmake_minimum_required(VERSION 3.20)
project(my_embedded_project C CXX)

# Configure FreeRTOS (assuming it's already in your project)
add_subdirectory(freertos)

# Add OSAL-RS porting layer
add_library(osal_rs_porting STATIC
    osal-rs-porting/freertos/src/osal_rs_freertos.c
)

target_include_directories(osal_rs_porting PUBLIC
    osal-rs-porting/freertos/inc
    ${FREERTOS_INCLUDE_DIRS}
)

target_link_libraries(osal_rs_porting PUBLIC
    freertos
)

# Configure Rust library
set(RUST_TARGET "thumbv7em-none-eabihf")  # Adjust for your target
set(OSAL_RS_LIB "${CMAKE_CURRENT_SOURCE_DIR}/osal-rs/target/${RUST_TARGET}/release/libosal_rs.a")

# Custom command to build Rust library
add_custom_command(
    OUTPUT ${OSAL_RS_LIB}
    COMMAND cargo build --release --target ${RUST_TARGET} --features freertos
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/osal-rs
    COMMENT "Building OSAL-RS library"
)

add_custom_target(osal_rs_build DEPENDS ${OSAL_RS_LIB})

# Create imported library for OSAL-RS
add_library(osal_rs STATIC IMPORTED GLOBAL)
set_target_properties(osal_rs PROPERTIES
    IMPORTED_LOCATION ${OSAL_RS_LIB}
)
add_dependencies(osal_rs osal_rs_build)

# Your main application
add_executable(my_app
    src/main.c
)

target_link_libraries(my_app PRIVATE
    osal_rs
    osal_rs_porting
    freertos
)
```

### Advanced CMake Integration with Multiple Configurations

```cmake
# Function to build OSAL-RS for different configurations
function(add_osal_rs_library TARGET_NAME RUST_TARGET CARGO_PROFILE)
    set(PROFILE_DIR ${CARGO_PROFILE})
    if(CARGO_PROFILE STREQUAL "release")
        set(CARGO_FLAGS "--release")
    else()
        set(CARGO_FLAGS "")
    endif()

    set(LIB_PATH "${CMAKE_CURRENT_SOURCE_DIR}/osal-rs/target/${RUST_TARGET}/${PROFILE_DIR}/libosal_rs.a")

    add_custom_command(
        OUTPUT ${LIB_PATH}
        COMMAND cargo build ${CARGO_FLAGS} --target ${RUST_TARGET} --features freertos
        WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/osal-rs
        COMMENT "Building OSAL-RS (${CARGO_PROFILE}) for ${RUST_TARGET}"
    )

    add_custom_target(${TARGET_NAME}_build DEPENDS ${LIB_PATH})

    add_library(${TARGET_NAME} STATIC IMPORTED GLOBAL)
    set_target_properties(${TARGET_NAME} PROPERTIES
        IMPORTED_LOCATION ${LIB_PATH}
    )
    add_dependencies(${TARGET_NAME} ${TARGET_NAME}_build)
endfunction()

# Use it in your project
add_osal_rs_library(osal_rs "thumbv7em-none-eabihf" "release")
```

### Integration with Corrosion (Recommended)

For a more seamless Rust-CMake integration, use [Corrosion](https://github.com/corrosion-rs/corrosion):

```cmake
cmake_minimum_required(VERSION 3.20)
project(my_embedded_project C CXX)

# Include Corrosion
include(FetchContent)
FetchContent_Declare(
    Corrosion
    GIT_REPOSITORY https://github.com/corrosion-rs/corrosion.git
    GIT_TAG v0.4
)
FetchContent_MakeAvailable(Corrosion)

# Import OSAL-RS crate
corrosion_import_crate(
    MANIFEST_PATH osal-rs/Cargo.toml
    FEATURES freertos
)

# Configure FreeRTOS
add_subdirectory(freertos)

# OSAL-RS porting layer
add_library(osal_rs_porting STATIC
    osal-rs-porting/freertos/src/osal_rs_freertos.c
)

target_include_directories(osal_rs_porting PUBLIC
    osal-rs-porting/freertos/inc
    ${FREERTOS_INCLUDE_DIRS}
)

target_link_libraries(osal_rs_porting PUBLIC
    freertos
)

# Your application
add_executable(my_app src/main.c)

target_link_libraries(my_app PRIVATE
    osal-rs
    osal_rs_porting
    freertos
)
```

### Cross-Compilation Setup

Example CMake toolchain file for ARM Cortex-M:

```cmake
# toolchain-arm-none-eabi.cmake
set(CMAKE_SYSTEM_NAME Generic)
set(CMAKE_SYSTEM_PROCESSOR arm)

set(CMAKE_C_COMPILER arm-none-eabi-gcc)
set(CMAKE_CXX_COMPILER arm-none-eabi-g++)
set(CMAKE_ASM_COMPILER arm-none-eabi-gcc)

set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)

# Rust target
set(RUST_TARGET "thumbv7em-none-eabihf")
```

Use it with:

```bash
cmake -DCMAKE_TOOLCHAIN_FILE=toolchain-arm-none-eabi.cmake -B build
cmake --build build
```

### Custom FreeRTOS Configuration Path

By default, OSAL-RS looks for `FreeRTOSConfig.h` at `<workspace_root>/inc/FreeRTOSConfig.h`. You can override this path using the `FREERTOS_CONFIG_PATH` environment variable.

#### Setting via CMake

```cmake
# Set custom path to FreeRTOSConfig.h
set(FREERTOS_CONFIG_PATH "${CMAKE_SOURCE_DIR}/inc/hhg-config/pico/FreeRTOSConfig.h")

# Pass to Cargo build via environment variable
add_custom_command(
    OUTPUT ${OSAL_RS_LIB}
    COMMAND ${CMAKE_COMMAND} -E env FREERTOS_CONFIG_PATH=${FREERTOS_CONFIG_PATH}
            cargo build --release --target ${RUST_TARGET} --features freertos
    WORKING_DIRECTORY ${CMAKE_CURRENT_SOURCE_DIR}/osal-rs
    COMMENT "Building OSAL-RS library"
)
```

#### Setting via Environment Variable

```bash
# Set environment variable before building
export FREERTOS_CONFIG_PATH="/path/to/your/FreeRTOSConfig.h"
cargo build --release --target thumbv7em-none-eabihf --features freertos
```

#### Using with Corrosion

```cmake
# Set environment variable for Corrosion
set(FREERTOS_CONFIG_PATH "${CMAKE_SOURCE_DIR}/inc/custom/FreeRTOSConfig.h")

corrosion_import_crate(
    MANIFEST_PATH osal-rs/Cargo.toml
    FEATURES freertos
)

# Set environment for the build
corrosion_set_env_vars(osal-rs
    FREERTOS_CONFIG_PATH=${FREERTOS_CONFIG_PATH}
)
```

**Note**: The build system will automatically regenerate Rust type bindings from the specified `FreeRTOSConfig.h` during the build process.


## Usage Example

```rust
use osal_rs::os::*;
use core::time::Duration;
use std::sync::Arc;

fn main() -> osal_rs::utils::Result<()> {
    // Create a mutex
    let counter = Arc::new(Mutex::new(0u32));
    let c = counter.clone();

    // Create and spawn a thread
    let mut thread = Thread::new("worker", 4096, 2);
    thread.spawn_simple(move || {
        *c.lock().unwrap() += 1;
    })?;

    // Create a queue for inter-task messages
    let queue = Queue::new(16, 4)?;
    queue.post(&[1u8, 2, 3, 4], 100)?;

    let mut buf = [0u8; 4];
    queue.fetch(&mut buf, 100)?;

    // Join and clean up
    thread.join(core::ptr::null_mut())?;
    Ok(())
}
```

See `osal-rs/examples/` for full multi-task pipeline demos.

## Building

### For FreeRTOS targets:

```bash
# Install Rust target (example for ARM Cortex-M4F)
rustup target add thumbv7em-none-eabihf

# Build with FreeRTOS support
cargo build --release --target thumbv7em-none-eabihf --features freertos
```

### For native development/testing:

```bash
# Build with POSIX support for native development/testing
cargo build --no-default-features --features "posix std"
```

## Cargo Features

OSAL-RS provides several Cargo features to customize the build configuration for different platforms and use cases:

### Available Features

| Feature | Default | Description |
|---------|---------|-------------|
| `freertos` | ✅ | Enable FreeRTOS backend implementation. This is the default and fully implemented feature for embedded RTOS development. |
| `posix` | ❌ | Enable POSIX pthread/libc backend. Does **not** require `std`; host examples and tests usually enable `std` for the binary runtime. |
| `std` | ❌ | Enable standard library support. Automatically enables `disable_panic`. Use this for native development and testing environments. |
| `disable_panic` | ❌ | Disable custom panic handler. Enabled automatically when `std` feature is active. Useful when you want to use the default panic behavior. |
| `serde` | ❌ | Enable serialization/deserialization support via `osal-rs-serde`. Includes derive macros for automatic implementation. |

### Feature Combinations

#### FreeRTOS Embedded Development (Default)
```bash
cargo build --target thumbv7em-none-eabihf --features freertos
```

#### FreeRTOS with Serialization Support
```bash
cargo build --target thumbv7em-none-eabihf --features freertos,serde
```

#### Native Development with Standard Library
```bash
cargo build --no-default-features --features "posix std"
```

#### Native Development with Serialization
```bash
cargo build --no-default-features --features "posix std serde"
```

### Using Features in Cargo.toml

To use OSAL-RS in your project with specific features:

```toml
[dependencies]
osal-rs = { version = "0.5", default-features = false, features = ["freertos"] }

# Or with serialization support
osal-rs = { version = "0.5", default-features = false, features = ["freertos", "serde"] }
```

## Testing Strategy

The test suite follows a NASA OSAL-style four-layer structure:

### 1. Contract Tests (`osal-rs-tests/src/common/`)

Shared behavioural tests that every backend must pass.  These verify the
trait contracts (mutex lock/unlock, semaphore wait/signal, queue timeout,
event-group set/wait/clear, thread spawn/join/notify, timer lifecycle,
system delay/tick).  Contract tests are **backend-agnostic** — they never
reference `pthread`, `std::thread`, or FreeRTOS C APIs.

```bash
# Run contract tests against the POSIX backend
cargo test -p osal-rs-tests --no-default-features --features posix

# With serde support
cargo test -p osal-rs-tests --no-default-features --features "posix serde"
```

### 2. Backend Runners (`osal-rs-tests/src/{posix,freertos}/`)

Thin `#[test]` entry points that execute the shared contract suite
(`crate::common`) against a specific backend.  The POSIX runner also
accepts a small number of backend-specific edge tests (e.g. pthread
stack-size clamping, `CLOCK_MONOTONIC` timeout precision).

### 3. Backend-Specific Edge Tests

Implementation-detail tests that are **not** portable across backends.
Examples: `PTHREAD_STACK_MIN` clamping in the POSIX sys thread layer,
`_from_isr` try-lock fast-path behaviour on the host.  These live in
backend-specific modules and are clearly named (e.g.
`posix/sys_thread_tests.rs`).

### 4. Portable Integration Demo (`osal-rs/examples/`)

A single application structured to run against POSIX host and FreeRTOS
backends with minimal backend-specific runner code.  Validates the core
OSAL promise: same portable core, different OS.

```bash
# Run the portable demo on the POSIX backend (Linux host)
cargo run --example portable_osal_integration_demo --no-default-features --features "posix std"
```

### 5. Typed Message Queue Demo (`osal-rs/examples/`)

Demonstrates structured inter-task communication using `QueueStreamed<T>`
and `osal-rs-serde`.  A timer periodically notifies the monitor task,
producers send typed `SensorPacket` messages, and consumers receive them
without manual byte packing.

Compared with `portable_osal_integration_demo.rs`, this example keeps the
same multi-task pipeline (2 producers, 3 consumers, supervisor lifecycle,
head-start phase, mid-demo period change, graceful shutdown) but replaces
the raw `[u8; 16]` byte queue with `QueueStreamed<SensorPacket>`.

```text
  Producers (×2) ──post──>  QueueStreamed<SensorPacket> ──fetch──>  Consumers (×3)
                                                                        │
                                                                        ▼
                                                                 Shared Stats (Mutex)

  Timer ──notify──> Monitor ──reads──> Stats
  Supervisor controls START / STOP via EventGroup
```

```bash
# POSIX backend (Linux host)
cargo run --example typed_message_queue_demo --no-default-features --features "posix std serde"
```

## Project Structure

```
osal-rs/
├── osal-rs/              # Main library crate
│   ├── src/
│   │   ├── freertos/      # FreeRTOS backend
│   │   ├── posix/         # POSIX backend (pthread + libc)
│   │   │   ├── sys/       # pthread / clock / condvar wrappers
│   │   │   └── bsp/       # BSP selection (generic_linux)
│   │   └── lib.rs
│   └── examples/
├── osal-rs-tests/        # Contract and backend test suite
├── osal-rs-serde/        # no_std serialization framework
│   └── derive/            # Derive macros (Serialize, Deserialize)
├── osal-rs-build/        # Build utilities
├── osal-rs-porting/      # FreeRTOS C FFI bridge
├── doc/                  # Design notes and contract docs
└── README.md
```

## Design Notes

- [OSAL Contract](doc/osal-contract.md) — portable behavior contract for all backends
- [OSAL 行为契约](doc/osal-contract-zh.md) (Chinese)
- [FreeRTOS ↔ POSIX Host Backend Alignment Gaps](doc/freertos-posix-alignment-gaps.md)
- [FreeRTOS ↔ POSIX 主机后端行为差异说明](doc/freertos-posix-alignment-gaps-zh.md) (Chinese)

## License

This project is licensed under the LGPL-2.1-or-later License - see the LICENSE file for details.

## Contributing

Contributions are welcome! Please feel free to submit pull requests or open issues for bugs and feature requests.

## Author

Antonio Salsi - [passy.linux@zresa.it](mailto:passy.linux@zresa.it)

## Links

- [Repository](https://github.com/writeabook/rust-for-study)
- [Documentation](https://docs.rs/osal-rs)
- [Crates.io](https://crates.io/crates/osal-rs)
