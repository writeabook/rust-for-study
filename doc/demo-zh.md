# osal-rs Demo 使用指南

本文说明如何使用已经适配好的 osal-rs 仓库，在 POSIX 环境和 QEMU FreeRTOS 环境中运行示例 demo。

当前文档主要覆盖两类运行方式：

- 在 Linux/POSIX 环境下直接运行 demo；
- 在 QEMU FreeRTOS 环境下，将 demo 编译进 FreeRTOS 固件并运行。

本文默认使用的 QEMU FreeRTOS 目标为：

```
CORTEX_MPS2_QEMU_IAR_GCC
```

对应 QEMU 参数为：

```
-machine mps2-an385
-cpu cortex-m3
```

对应 Rust target 为：

```
thumbv7m-none-eabi
```

如果只需要在 POSIX 环境下运行 demo，可以直接跳转到第 17 节。

---

## 1. Demo 运行方式概览

FreeRTOS 不能直接运行 Rust 源码，也不能在 FreeRTOS 中直接执行：

```
cargo run --example xxx
```

在 FreeRTOS/QEMU 环境中运行 Rust demo 的正确方式是：

```
osal-rs Rust demo
        ↓
cargo build --target thumbv7m-none-eabi
        ↓
生成 libosal_rs.a
        ↓
FreeRTOS QEMU 工程链接 libosal_rs.a
        ↓
同时编译 osal-rs-porting/freertos/src/osal_rs_freertos.c
        ↓
main.c 调用 rust_demo_entry()
        ↓
Rust demo 在 FreeRTOS scheduler 中运行
```

本仓库已经完成了 QEMU FreeRTOS 所需的 osal-rs 内部适配。使用者不需要再修改 osal-rs 内部源码，只需要在 FreeRTOS QEMU 工程中完成以下外部集成工作：

1. 修改 FreeRTOSConfig.h
2. 修改 main.c
3. 修改 build/gcc/Makefile
4. 修改 build/gcc/mps2_m3.ld
5. 编译并运行 QEMU demo

---

## 2. 安装基础工具

本文使用的系统环境为：

```
Ubuntu 20.04.6 LTS
```

安装基础工具：

```bash
sudo apt update
sudo apt install -y \
  git \
  make \
  build-essential \
  curl \
  gcc-arm-none-eabi \
  qemu-system-arm
```

如果需要使用与本文一致的 Ubuntu 20.04 软件包版本，也可以指定版本安装：

```bash
sudo apt update
sudo apt install -y \
  gcc-arm-none-eabi=15:9-2019-q4-0ubuntu1 \
  qemu-system-arm=1:4.2-3ubuntu6.30
```

检查工具是否安装成功：

```bash
arm-none-eabi-gcc --version
qemu-system-arm --version
make --version
git --version
```

---

## 3. 安装 Rust 工具链

安装 Cortex-M3 对应的 Rust target：

```bash
rustup target add thumbv7m-none-eabi
```

确认 target 安装成功：

```bash
rustup target list --installed | grep thumbv7m
```

应看到：

```
thumbv7m-none-eabi
```

---

## 4. 下载 FreeRTOS

本文示例将 FreeRTOS 放在：

```
~/freertos
```

执行：

```bash
mkdir -p ~/freertos
cd ~/freertos
git clone --recursive https://github.com/FreeRTOS/FreeRTOS.git
```

如果已经下载过 FreeRTOS，但子模块不完整，可以执行：

```bash
cd ~/freertos/FreeRTOS
git submodule update --init --recursive
```

---

## 5. 定位 QEMU FreeRTOS demo

进入 FreeRTOS 根目录：

```bash
cd ~/freertos/FreeRTOS
```

查找 QEMU demo：

```bash
find ./FreeRTOS/Demo -maxdepth 2 -type d | grep -i qemu
```

常见结果为：

```
./FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC
```

后续本文默认 FreeRTOS QEMU demo 路径为：

```
~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC
```

FreeRTOSConfig.h 路径为：

```
~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h
```

检查路径是否存在：

```bash
ls ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC
ls ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h
```

---

## 6. 验证原始 FreeRTOS QEMU demo

在接入 osal-rs 前，建议先确认原始 FreeRTOS QEMU demo 可以正常编译和运行。

进入 gcc 构建目录：

```bash
cd ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc
make clean
make
```

如果编译成功，检查输出文件：

```bash
ls output
```

应看到类似：

```
RTOSDemo.out
RTOSDemo.map
```

运行 QEMU：

```bash
qemu-system-arm \
  -machine mps2-an385 \
  -cpu cortex-m3 \
  -kernel output/RTOSDemo.out \
  -monitor none \
  -nographic \
  -serial stdio
```

---

## 7. 准备 osal-rs 仓库

本文默认已经获取适配好的 osal-rs 仓库，并放在：

```
~/osal-rs
```

其中 osal-rs crate 路径为：

```
~/osal-rs/osal-rs
```

FreeRTOS porting layer 路径为：

```
~/osal-rs/osal-rs-porting/freertos
```

检查关键文件是否存在：

```bash
ls ~/osal-rs/osal-rs/Cargo.toml
ls ~/osal-rs/osal-rs-porting/freertos/src/osal_rs_freertos.c
ls ~/osal-rs/osal-rs-porting/freertos/inc
```

如果这些文件不存在，请确认仓库路径是否正确。

---

## 8. 选择要运行的 Rust demo

当前仓库支持通过 `rust_demo_entry()` 从 C 侧进入 Rust demo。

通常通过仓库内的以下文件选择实际运行的 demo：

```
osal-rs/src/freertos_demo_export.rs
```

仓库已经默认配置好 demo 入口，一般只需要修改 `#[path = "..."]` 这一行即可切换 demo。

如果运行普通综合 demo，使用：

```rust
#[path = "../examples/portable_osal_integration_demo.rs"]
mod portable_demo;
```

如果运行 typed message queue demo，使用：

```rust
#[path = "../examples/typed_message_queue_demo.rs"]
mod typed_demo;
```

**示例：运行 portable_osal_integration_demo 时**，`freertos_demo_export.rs` 可以写成：

```rust
use core::ffi::c_int;

#[path = "../examples/portable_osal_integration_demo.rs"]
mod portable_demo;

#[unsafe(no_mangle)]
pub extern "C" fn rust_demo_entry() -> c_int {
    match portable_demo::freertos_demo_entry() {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
```

**示例：运行 typed_message_queue_demo 时**，`freertos_demo_export.rs` 可以写成：

```rust
use core::ffi::c_int;

#[path = "../examples/typed_message_queue_demo.rs"]
mod typed_demo;

#[unsafe(no_mangle)]
pub extern "C" fn rust_demo_entry() -> c_int {
    match typed_demo::freertos_demo_entry() {
        Ok(_) => 0,
        Err(_) => -1,
    }
}
```

---

## 9. 单独编译 osal-rs 静态库

进入 osal-rs crate 目录：

```bash
cd ~/osal-rs/osal-rs
```

如果运行普通 demo，执行：

```bash
FREERTOS_CONFIG_PATH=~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h \
cargo build --release \
  --target thumbv7m-none-eabi \
  --no-default-features \
  --features "freertos"
```

如果运行 typed message queue demo，执行：

```bash
FREERTOS_CONFIG_PATH=~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h \
cargo build --release \
  --target thumbv7m-none-eabi \
  --no-default-features \
  --features "freertos serde"
```

**注意**：`typed_message_queue_demo` 必须启用 `serde` feature。

---

## 10. 修改 FreeRTOSConfig.h

打开：

```bash
nano ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h
```

确认以下配置已启用：

```c
#define configUSE_MUTEXES                1
#define configUSE_RECURSIVE_MUTEXES      1
#define configUSE_COUNTING_SEMAPHORES    1
#define configUSE_TIMERS                 1
#define configUSE_QUEUE_SETS             1
#define configSUPPORT_DYNAMIC_ALLOCATION 1
```

一般情况下，QEMU demo 中前几项已经存在，可能只需要新增：

```c
#define configSUPPORT_DYNAMIC_ALLOCATION 1
```

如果后续 demo 创建任务、队列、信号量或定时器失败，可以适当增大以下配置：

```c
#define configTOTAL_HEAP_SIZE
#define configTIMER_QUEUE_LENGTH
#define configTIMER_TASK_STACK_DEPTH
#define configMAX_PRIORITIES
```

---

## 11. 修改 main.c

进入 QEMU demo 目录：

```bash
cd ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC
cp main.c main.c.bak
nano main.c
```

找到原来的 demo 声明：

```c
extern void main_blinky( void );
extern void main_full( void );
```

在下面添加 Rust 入口声明：

```c
extern int rust_demo_entry( void );
```

修改后类似：

```c
extern void main_blinky( void );
extern void main_full( void );
extern int rust_demo_entry( void );
```

然后在 `main()` 中保留原有硬件初始化，尤其是 UART 初始化：

```c
prvUARTInit();
```

找到原来选择运行 `main_blinky()` 或 `main_full()` 的代码，通常类似：

```c
#if ( mainCREATE_SIMPLE_BLINKY_DEMO_ONLY == 1 )
{
    main_blinky();
}
#else
{
    main_full();
}
#endif
```

将其替换为：

```c
int ret = rust_demo_entry();

if( ret != 0 )
{
    printf( "\r\n\r\nrust_demo_entry failed: %d\r\n", ret );
    portDISABLE_INTERRUPTS();

    for( ; ; )
    {
    }
}

for( ; ; )
{
}
```

---

## 12. 修改 Makefile

进入 gcc 构建目录：

```bash
cd ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc
cp Makefile Makefile.bak
nano Makefile
```

### 12.1 添加 osal-rs 路径变量

建议放在 Makefile 顶部附近：

```makefile
.DEFAULT_GOAL := all

OSAL_RS_ROOT := /home/user123/osal-rs
OSAL_RS_CRATE := $(OSAL_RS_ROOT)/osal-rs
OSAL_RS_TARGET := thumbv7m-none-eabi
OSAL_RS_LIB := $(OSAL_RS_ROOT)/target/$(OSAL_RS_TARGET)/release/libosal_rs.a
FREERTOS_CONFIG_PATH := /home/user123/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/FreeRTOSConfig.h
```

如果你的用户名或目录不同，请改成自己的实际路径。

**注意**：Makefile 中建议使用完整路径，例如：

```
/home/user123/osal-rs
```

### 12.2 编译 osal-rs FreeRTOS porting C 文件

在 Makefile 中加入：

```makefile
VPATH += $(OSAL_RS_ROOT)/osal-rs-porting/freertos/src
INCLUDE_DIRS += -I$(OSAL_RS_ROOT)/osal-rs-porting/freertos/inc
SOURCE_FILES += osal_rs_freertos.c
```

建议放在 Makefile 中定义 `VPATH`、`INCLUDE_DIRS`、`SOURCE_FILES` 的区域附近。

这一步必须有。只链接 Rust 静态库 `libosal_rs.a` 不够，还必须编译：

```
osal-rs-porting/freertos/src/osal_rs_freertos.c
```

它是 Rust OSAL 到 FreeRTOS C API 的桥接层。

### 12.3 让 make 先构建 Rust 静态库

找到：

```makefile
all: $(IMAGE)
```

改成：

```makefile
all: rust_osal $(IMAGE)
```

这样执行 `make` 时，会先构建 `libosal_rs.a`。

### 12.4 添加 Rust 构建目标

如果运行普通 demo，添加：

```makefile
.PHONY: rust_osal

rust_osal:
	cd $(OSAL_RS_CRATE) && \
	FREERTOS_CONFIG_PATH=$(FREERTOS_CONFIG_PATH) \
	cargo build --release --target $(OSAL_RS_TARGET) --no-default-features --features "freertos"
```

如果运行 typed demo，添加：

```makefile
.PHONY: rust_osal

rust_osal:
	cd $(OSAL_RS_CRATE) && \
	FREERTOS_CONFIG_PATH=$(FREERTOS_CONFIG_PATH) \
	cargo build --release --target $(OSAL_RS_TARGET) --no-default-features --features "freertos serde"
```

**注意**：typed demo 必须启用 `serde` feature。

### 12.5 最终链接时加入 libosal_rs.a

找到最终链接规则，通常类似：

```makefile
$(IMAGE): ./mps2_m3.ld $(OBJS_OUTPUT) Makefile
	@echo ""
	@echo ""
	@echo "--- Final linking ---"
	@echo ""
	$(LD) $(CFLAGS) $(LDFLAGS) $(OBJS_OUTPUT) -o $(IMAGE)
	$(SIZE) $(IMAGE)
```

改成：

```makefile
$(IMAGE): ./mps2_m3.ld $(OBJS_OUTPUT) $(OSAL_RS_LIB) Makefile
	@echo ""
	@echo ""
	@echo "--- Final linking ---"
	@echo ""
	$(LD) $(CFLAGS) $(LDFLAGS) $(OBJS_OUTPUT) $(OSAL_RS_LIB) -o $(IMAGE)
	$(SIZE) $(IMAGE)
```

关键是链接命令中必须包含：

```
$(OSAL_RS_LIB)
```

否则可能报：

```
undefined reference to `rust_demo_entry'
```

---

## 13. 修改 mps2_m3.ld

Rust 静态库链接到 ARM bare-metal 工程后，可能需要 `.ARM.exidx` 相关段和符号。否则可能出现：

```
undefined reference to `__exidx_start'
undefined reference to `__exidx_end'
```

进入 gcc 目录：

```bash
cd ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc
cp mps2_m3.ld mps2_m3.ld.bak
nano mps2_m3.ld
```

在 `.text` 段之后、`.data` 段之前加入：

```ld
.ARM.extab :
{
    *(.ARM.extab* .gnu.linkonce.armextab.*)
} > FLASH

.ARM.exidx :
{
    PROVIDE_HIDDEN (__exidx_start = .);
    KEEP(*(.ARM.exidx* .gnu.linkonce.armexidx.*))
    PROVIDE_HIDDEN (__exidx_end = .);
} > FLASH
```

最终结构类似：

```ld
.text :
{
    ...
} > FLASH

.ARM.extab :
{
    *(.ARM.extab* .gnu.linkonce.armextab.*)
} > FLASH

.ARM.exidx :
{
    PROVIDE_HIDDEN (__exidx_start = .);
    KEEP(*(.ARM.exidx* .gnu.linkonce.armexidx.*))
    PROVIDE_HIDDEN (__exidx_end = .);
} > FLASH

.data :
{
    ...
} > RAM AT> FLASH
```

---

## 14. 编译 FreeRTOS QEMU 工程

进入 gcc 目录：

```bash
cd ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc
make clean
make
```

如果成功，应看到类似：

```
cargo build ...
arm-none-eabi-gcc ...
--- Final linking ---
arm-none-eabi-size ./output/RTOSDemo.out
```

检查输出：

```bash
ls output
```

应看到：

```
RTOSDemo.out
RTOSDemo.map
```

---

## 15. 检查 rust_demo_entry 是否链接成功

如果链接失败，先检查 Rust 静态库中是否存在 `rust_demo_entry` 符号：

```bash
arm-none-eabi-nm ~/osal-rs/target/thumbv7m-none-eabi/release/libosal_rs.a | grep rust_demo_entry
```

正常应看到类似：

```
00000000 T rust_demo_entry
```

也可以检查最终固件中是否包含 Rust demo 相关字符串：

```bash
arm-none-eabi-strings output/RTOSDemo.out | grep -i rust
```

---

## 16. 运行 QEMU

执行：

```bash
qemu-system-arm \
  -machine mps2-an385 \
  -cpu cortex-m3 \
  -kernel output/RTOSDemo.out \
  -monitor none \
  -nographic \
  -serial stdio
```

如果运行 `portable_osal_integration_demo`，正常应看到类似：

```
[main] run portable demo on freertos backend
[init] Portable OSAL Integration Demo
[init] queue capacity=128 message_size=16
[init] producer-0 spawned
[init] producer-1 spawned
[init] consumer-0 spawned
[init] consumer-1 spawned
[init] consumer-2 spawned
[init] monitor spawned
[init] heartbeat timer period=1000ms
[init] supervisor spawned
...
```

如果运行 `typed_message_queue_demo`，正常会看到 typed message queue 相关日志。

退出 QEMU：

```
Ctrl + A
```

然后按 `X`。

---

## 17. POSIX 下运行 demo

如果只想在 Linux/POSIX 下运行普通 demo：

```bash
cd ~/osal-rs/osal-rs

cargo run -p osal-rs \
  --example portable_osal_integration_demo \
  --no-default-features \
  --features "posix std"
```

运行 typed demo：

```bash
cd ~/osal-rs/osal-rs

cargo run -p osal-rs \
  --example typed_message_queue_demo \
  --no-default-features \
  --features "posix std serde"
```

**注意**：typed demo 必须启用 `serde` feature。

---

## 18. 常见问题

### 18.1 undefined reference to rust_demo_entry

如果链接时报：

```
undefined reference to `rust_demo_entry'
```

检查三点：

```bash
arm-none-eabi-nm ~/osal-rs/target/thumbv7m-none-eabi/release/libosal_rs.a | grep rust_demo_entry

grep -n "rust_demo_entry" \
  ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/main.c

grep -n "OSAL_RS_LIB\|rust_osal" \
  ~/freertos/FreeRTOS/FreeRTOS/Demo/CORTEX_MPS2_QEMU_IAR_GCC/build/gcc/Makefile
```

通常原因是：

1. `freertos_demo_export.rs` 没有被 `freertos` feature 编译进去
2. Makefile 链接的 `libosal_rs.a` 路径不对
3. 最终链接命令里没有加入 `$(OSAL_RS_LIB)`

### 18.2 \_\_exidx_start / \_\_exidx_end 未定义

如果链接时报：

```
undefined reference to `__exidx_start'
undefined reference to `__exidx_end'
```

说明 `mps2_m3.ld` 没有加入 `.ARM.exidx`。按第 13 节修改 linker script。

### 18.3 手动 cargo build 通过，但 make 失败

如果手动执行 `cargo build` 可以通过，但在 gcc 目录执行 `make` 仍然失败，说明 Makefile 里的 cargo 参数和手动命令不一致。

尤其是运行 typed demo 时，Makefile 中也必须包含：

```
--features "freertos serde"
```

不能只有：

```
--features "freertos"
```

### 18.4 typed demo 报 Serialize / Deserialize 错误

如果出现类似错误：

```
unresolved import `osal_rs_serde`
SensorPacket: Serialize is not satisfied
SensorPacket: Deserialize is not satisfied
```

说明没有启用 `serde` feature。

FreeRTOS/QEMU 下应使用：

```
--features "freertos serde"
```

POSIX 下应使用：

```
--features "posix std serde"
```

### 18.5 QEMU 没有日志输出

确认启动命令包含：

```
-serial stdio
```

同时确认 FreeRTOS 工程编译并链接了：

```
osal-rs-porting/freertos/src/osal_rs_freertos.c
```

本仓库的 porting layer 已经适配 QEMU MPS2 UART，日志应通过 `-serial stdio` 输出。

---

## 19. 总结

在 QEMU FreeRTOS 上运行 osal-rs demo 的关键步骤是：

1. 安装 `qemu-system-arm`、`arm-none-eabi-gcc`、Rust 和 `thumbv7m-none-eabi` target。
2. 先确认原始 FreeRTOS QEMU demo 能正常编译运行。
3. 选择要运行的 osal-rs demo。
4. 将 osal-rs 编译成 `libosal_rs.a`。
5. FreeRTOS 工程编译 `osal_rs_freertos.c`。
6. `main.c` 调用 `rust_demo_entry()`。
7. Makefile 链接 `libosal_rs.a`。
8. `mps2_m3.ld` 补 `.ARM.exidx`。
9. typed demo 额外启用 `serde` feature。
10. 使用 `qemu-system-arm -machine mps2-an385 -cpu cortex-m3` 运行 `RTOSDemo.out`。
