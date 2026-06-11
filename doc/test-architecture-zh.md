# osal-rs Linux 后端测试架构设计

## 1. 总体原则

osal-rs 的测试不应该只验证某个后端是否能运行，而应该验证：

**同一套 OSAL 抽象 API 在不同后端之间是否表现出一致的行为。**

因此，测试系统应该分为两部分：

```
公共行为测试（common tests）
    ↕
不同后端的测试入口（backend runners）
```

也就是说，实际的测试逻辑应该尽可能写成公共代码，可被 FreeRTOS、Linux，以及未来可能的 RT-Thread、Zephyr、POSIX 等后端复用。

---

## 2. FreeRTOS 与 Linux 测试方式的差异

### FreeRTOS 后端

FreeRTOS 是嵌入式 RTOS 后端，不能直接依赖标准的 `cargo test`。

原因如下：

- FreeRTOS 后端需要真正的 RTOS 调度器
- 需要链接 FreeRTOS Kernel
- 需要 C 桥接 / 移植层
- 许多行为必须在任务中运行

因此，FreeRTOS 测试适合以下模式：

```
嵌入式项目启动
    ↓
启动 FreeRTOS 调度器
    ↓
创建测试任务
    ↓
在测试任务中调用 osal_rs_tests::freertos::run_all_tests()
```

因此，FreeRTOS 测试代码不需要 `#[test]`，而是手动编写：

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

其测试方式是：**固件内手动运行器（Manual runner within firmware）**

### Linux 后端

Linux 则不同。

Linux 可以直接在宿主机上运行，因此应该使用 Rust 原生的测试系统：

```
cargo test
```

Linux 后端应该使用：

```rust
#[test]
fn test_xxx() {
    ...
}
```

因为 Linux 后端不需要烧录、不需要开发板、不需要手动进入 RTOS 任务、不需要绕过 Rust 的测试框架。

所以 Linux 测试方式是：**通过 `cargo test` 自动运行器（Automatic runner via `cargo test`）**

---

## 3. 最终推荐的测试架构

我最推荐的结构是：

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

    freertos/
      mod.rs

    linux/
      mod.rs
```

核心理念是：

- 在 `common/` 中编写实际的测试逻辑
- 在 `freertos/` 中编写 FreeRTOS 手动入口
- 在 `linux/` 中编写 `#[test]` 包装器入口

---

## 4. 公共测试层

公共层不关心具体的后端。

它只关心 OSAL API 的行为是否正确。

例如：

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

这里没有 `#[test]` 注解。

因为这些函数只是公共测试逻辑，它们不直接决定如何运行。

---

## 5. FreeRTOS 测试入口

FreeRTOS 后端继续使用手动运行器。

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

然后在 FreeRTOS 项目的某个任务中调用它：

```rust
fn test_task() {
    match osal_rs_tests::freertos::run_all_tests() {
        Ok(_) => {
            // 测试通过
        }
        Err(e) => {
            // 测试失败
        }
    }
}
```

这部分仍然不需要 `#[test]`。

---

## 6. Linux 测试入口

Linux 后端使用 `#[test]` 来包装公共测试函数。

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

这样，在 Linux 下直接运行：

```bash
cargo test -p osal-rs-tests --no-default-features --features linux,std
```

---

## 7. 必须严格一致的语义

以下是 OSAL 的公共语义，必须在所有后端之间严格一致：

- 队列 FIFO 顺序
- 互斥锁互斥访问
- 信号量 wait/signal
- 线程创建（spawn）
- 定时器回调
- 系统延时（delay）
- Duration/tick 转换
- EventGroup 位等待

---

## 8. Linux 下仅能"尽力而为"的语义

Linux 不是 RTOS。我们不能强求 Linux 后端完美模拟所有 FreeRTOS 行为。测试应避免对以下特性做过严的断言：

- 线程优先级
- 线程栈大小
- 实时调度
- 任务挂起（suspend）
- 任务恢复（resume）
- 任务删除（delete）
- 精确的 tick 计数
- ISR 安全 API