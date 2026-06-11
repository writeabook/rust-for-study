# OSAL-RS 抽象层契约文档

> 本文档基于 FreeRTOS 参考实现，定义每个 trait 的方法语义、约束和 POSIX 后端必须满足的行为。
> FreeRTOS 实现 = 参考标准 | POSIX 实现 = 待开发目标

---

## 1. 类型系统 (Types)

### 1.1 平台类型别名

```rust
// FreeRTOS 类型 (编译时自动从 FreeRTOSConfig.h 生成)
pub type TickType = u32;       // 系统滴答计数类型
pub type BaseType = i32;       // 有符号基础类型
pub type UBaseType = u32;      // 无符号基础类型
pub type StackType = u32;      // 栈大小类型
pub type EventBits = u32;      // 事件位类型 (24位可用)
pub type ThreadHandle = *mut c_void; // 线程/任务句柄
pub type MutexHandle = *mut c_void;  // 互斥锁句柄
pub type SemaphoreHandle = *mut c_void; // 信号量句柄
pub type QueueHandle = *mut c_void;    // 队列句柄
pub type TimerHandle = *mut c_void;    // 定时器句柄
pub type EventGroupHandle = *mut c_void; // 事件组句柄
```

**POSIX 要求**: 保持相同类型定义。Handle 可以用 `AtomicUsize` 自增 ID 代替裸指针。

### 1.2 线程状态枚举

```rust
pub enum ThreadState {
    Running = 0,    // 当前正在执行
    Ready = 1,      // 就绪但未执行
    Blocked = 2,    // 等待事件中 (信号量/队列等)
    Suspended = 3,  // 被明确挂起
    Deleted = 4,    // 已被删除
    Invalid,        // 无效状态
}
```

**POSIX 要求**: 状态语义需与 FreeRTOS 一致。

---

## 2. SystemFn Trait — 系统级操作

### 2.1 `start()`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 启动调度器，启用中断，开始执行最高优先级任务。**永不返回。** |
| **调用前要求** | 必须至少创建并 spawn 一个线程 |
| **POSIX 要求** | 空操作（POSIX 无调度器概念），必须不 panic |

### 2.2 `get_state() -> ThreadState`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 返回当前调度器状态 (Running/Suspended/NotStarted) |
| **POSIX 要求** | 永远返回 `ThreadState::Running` |

### 2.3 `suspend_all()` / `resume_all() -> BaseType`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 暂停/恢复调度器（支持嵌套），`resume_all` 返回嵌套层数 |
| **POSIX 要求** | 空操作（暂不支持）；`resume_all` 返回 0 |

### 2.4 `stop()`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 停止调度器（仅 Cortex-M 等架构支持） |
| **POSIX 要求** | 空操作 |

### 2.5 `get_tick_count() -> TickType`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 返回自调度器启动以来的 tick 计数。溢出时回绕。 |
| **POSIX 要求** | 基于 `Instant::now()` 计算毫秒数，saturate 到 `TickType::MAX` |

### 2.6 `get_current_time_us() -> Duration`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 返回系统运行总时间 |
| **POSIX 要求** | 返回自 `start_time` 以来的 `Duration`（基于 `Instant::now()`） |

### 2.7 `get_us_from_tick(duration: &Duration) -> TickType`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 将 Duration 转为微秒 tick 值 |
| **POSIX 要求** | `as_millis()` 并 saturate 到 `TickType::MAX` |

### 2.8 `delay(ticks: TickType)`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 阻塞当前任务 `ticks` 个系统滴答 |
| **POSIX 要求** | `std::thread::sleep(Duration::from_millis(ticks))` — 每 tick = 1ms |

### 2.9 `delay_until(previous_wake_time: &mut TickType, time_increment: TickType)`
| 项 | 契约 |
|----|------|
| **FreeRTOS 行为** | 周期性延时：修正漂移。计算 `next = *prev + increment`，延迟到 next。 |
| **POSIX 要求** | 相同逻辑：`next - now > 0` 才 sleep，然后 `*prev = next` |

### 2.10 `check_timer(timestamp: &Duration, time: &Duration) -> OsalRsBool`
| 项 | 契约 |
|----|------|
| **行为** | 检查自 `timestamp` 以来是否经过 ≥ `time`。非阻塞轮询。 |
| **返回值** | `True` 超时已到，`False` 未到 |

### 2.11 临界区方法
| 方法 | FreeRTOS 行为 | POSIX 要求 |
|------|-------------|------------|
| `critical_section_enter()` | 禁用中断 | 空操作（暂无实现） |
| `critical_section_exit()` | 恢复中断 | 空操作（暂无实现） |
| `enter_critical()` → `UBaseType` | 进入任务级临界区，返回之前状态 | 空操作，返回 0 |
| `exit_critical(UBaseType)` | 恢复临界区状态 | 空操作 |
| `enter_critical_from_isr()` → `UBaseType` | ISR 级临界区 | 返回 0 |
| `exit_critical_from_isr(UBaseType)` | 恢复 ISR 临界区 | 空操作 |

> **重要**: POSIX 后端不使用临界区保护共享资源。线程安全由 `Arc<Mutex<...>>` 保证。

### 2.12 ISR 辅助方法
| 方法 | 行为 |
|------|------|
| `yield_from_isr(BaseType)` | 如果 `≠0`，触发上下文切换 → POSIX: `std::thread::yield_now()` |
| `end_switching_isr(BaseType)` | 同上 |

### 2.13 系统信息
| 方法 | FreeRTOS 行为 | POSIX 要求 |
|------|-------------|------------|
| `count_threads() -> usize` | 实际任务数 | 返回 1（桩） |
| `get_all_thread() -> SystemState` | 所有任务的 `ThreadMetadata` 列表 | 返回含一条 Running 记录的列表 |
| `get_free_heap_size() -> usize` | FreeRTOS 堆剩余字节 | 返回 0 |

---

## 3. MutexFn / RawMutexFn Trait — 互斥锁

### 3.1 RawMutex 契约

| 方法 | 语义 |
|------|------|
| `lock() -> OsalRsBool` | **阻塞**直到获取锁。FreeRTOS: `xSemaphoreTakeRecursive(handle, MAX_DELAY)` |
| `lock_from_isr() -> OsalRsBool` | **非阻塞**尝试获取锁，ISR 上下文。成功则 yield |
| `unlock() -> OsalRsBool` | 解锁（递归锁需匹配 lock 次数） |
| `unlock_from_isr() -> OsalRsBool` | ISR 解锁，成功则 yield |
| `delete(&mut self)` | 销毁互斥锁，释放资源 |

### 3.2 Mutex\<T\> 契约

| 方法 | 语义 |
|------|------|
| `lock() -> Result<MutexGuard<'_, T>>` | 阻塞获取 RAII guard |
| `lock_from_isr() -> Result<MutexGuardFromIsr<'_, T>>` | ISR 非阻塞获取 guard |
| `into_inner(self) -> Result<T>` | 消费 Mutex 取出内部值 |
| `get_mut(&mut self) -> &mut T` | 获取可变引用（需要 `&mut self`，无需加锁） |

### 3.3 MutexGuard 契约

| 要求 | 说明 |
|------|------|
| `Deref<Target=T>` | 不可变访问 |
| `DerefMut` | 可变访问 |
| `Drop` | **必须 unlock**。Guard 析构 = 自动释放锁 |
| `MutexGuardFn::update(&mut self, &T)` | 用 `Clone` 更新内部值 |

### 3.4 POSIX 当前状态

| 组件 | 状态 |
|------|------|
| `RawMutex` | ❌ 完全桩：lock/unlock 永远返回 True，无实际锁定 |
| `Mutex<T>` | ❌ 依赖 RawMutex，无实际互斥保护 |
| `MutexGuard` | ❌ 结构存在但无有效保护 |

**POSIX 需要**: 用 `std::sync::Mutex` + `Arc` 或 `pthread_mutex_t` 实现真实的互斥。

---

## 4. SemaphoreFn Trait — 信号量

### 4.1 构造

| 方法 | 参数 | 行为 |
|------|------|------|
| `new(max_count, initial_count)` | 最大计数值, 初始计数值 | 创建计数信号量 |
| `new_with_count(initial_count)` | 初始计数值 | 创建 `max_count = MAX` 的计数信号量（常用于二进制信号量） |

### 4.2 操作契约

| 方法 | 语义 |
|------|------|
| `wait(impl ToTick) -> OsalRsBool` | **阻塞**等待。count > 0 → 减 1；count = 0 → 阻塞至超时 |
| `wait_from_isr() -> OsalRsBool` | **非阻塞**等待。count > 0 → 减 1 返回 True；= 0 → 返回 False |
| `signal() -> OsalRsBool` | **释放**。count < max → 加 1，唤醒等待任务；= max → 返回 False |
| `signal_from_isr() -> OsalRsBool` | ISR 释放，同上逻辑 |
| `delete(&mut self)` | 销毁信号量 |

### 4.3 超时约定

| timeout 值 | 行为 |
|------------|------|
| `Duration::ZERO` 或 `0` | 不阻塞，立即返回 |
| `Duration::MAX` 或 `TickType::MAX` | 无限等待 |
| 其他值 | 等待最多 timeout 时间 |

### 4.4 POSIX 当前状态

❌ **完全桩代码**：wait/signal 全部返回 True，无任何实际同步。需要用 `std::sync::Condvar` + `Mutex<count>` 实现。

---

## 5. EventGroupFn Trait — 事件组

### 5.1 构造

```rust
EventGroup::new() -> Result<Self>  // 所有 bit 初始为 0
```

### 5.2 操作契约

| 方法 | 语义 |
|------|------|
| `set(bits: EventBits) -> EventBits` | **OR** 操作设置位。返回设置前的值。唤醒等待的任务。 |
| `set_from_isr(bits) -> Result<()>` | ISR 版本 |
| `get() -> EventBits` | 返回当前所有位的值 |
| `get_from_isr() -> EventBits` | ISR 版本 |
| `clear(bits: EventBits) -> EventBits` | **AND NOT** 清除位。返回清除前的值。 |
| `clear_from_isr(bits) -> Result<()>` | ISR 版本 |
| `wait(mask, timeout_ticks) -> EventBits` | **阻塞**等待 mask 中所有位被设置。返回当前位值。 |
| `delete(&mut self)` | 销毁事件组 |

### 5.3 wait 行为细节

```
1. 阻塞直到 (current_bits & mask) == mask
2. 返回 current_bits（不是 mask）
3. timeout = 0 → 不阻塞
4. timeout = MAX → 无限等待
```

### 5.4 `MAX_MASK` 常量

高 8 位被 FreeRTOS 保留，用户可用 `EventBits::MAX >> 8` 位（即 24 位或 56 位）。

### 5.5 POSIX 当前状态

❌ **完全桩代码**：set/get/clear/wait 全部返回 0 或 Ok，无任何实际同步。

---

## 6. QueueFn Trait — 消息队列

### 6.1 构造

```rust
Queue::new(size: UBaseType, message_size: UBaseType) -> Result<Self>
// size > 0 && message_size > 0 否则返回 InvalidQueueSize
```

### 6.2 操作契约

| 方法 | 语义 |
|------|------|
| `fetch(buffer, time) -> Result<()>` | **阻塞**从队首取出消息。空队列 → 阻塞等 timeout。 |
| `fetch_from_isr(buffer) -> Result<()>` | ISR 非阻塞取。空 → Timeout。成功时 yield。 |
| `post(item, time) -> Result<()>` | **阻塞**向队尾发送消息。满队列 → 阻塞等 timeout。 |
| `post_from_isr(item) -> Result<()>` | ISR 非阻塞发。满 → 返回错误。成功时 yield。 |
| `delete(&mut self)` | 销毁队列 |

### 6.3 超时约定（与 Semaphore 一致）

| timeout 值 | 行为 |
|------------|------|
| `0` | 不阻塞，立即返回 |
| `TickType::MAX` | 无限等待 |
| 其他 | 等待最多 timeout ticks |

### 6.4 QueueStreamed\<T\> 契约

类型安全队列，对实现了 `Serialize + BytesHasLen + Deserialize` 的类型提供与 Queue 相同语义的 fetch/post。

### 6.5 POSIX 当前状态

❌ **完全桩代码**：fetch 永远返回 Timeout，post 永远返回 Ok。需要用 `Arc<RwLock<VecDeque<u8>>>` + `Condvar` 实现。

---

## 7. TimerFn Trait — 软件定时器

> **这是 POSIX 后端最大的缺口，需要重点关注。**

### 7.1 构造

```rust
Timer::new(name, timer_period_in_ticks, auto_reload, param, callback) -> Result<Self>
```

| 参数 | 说明 |
|------|------|
| `name` | 调试名称 |
| `timer_period_in_ticks` | 定时周期（tick 数） |
| `auto_reload` | `true` = 周期定时器, `false` = 单次定时器 |
| `param: Option<TimerParam>` | 传递给回调的参数 (`Arc<dyn Any+Send+Sync>`) |
| `callback: Fn(Box<dyn TimerFn>, Option<TimerParam>) -> Result<TimerParam>` | 到期回调 |

### 7.2 操作契约

| 方法 | 语义 |
|------|------|
| `start(ticks_to_wait) -> OsalRsBool` | 启动定时器。如果已在运行，无效果。 |
| `stop(ticks_to_wait) -> OsalRsBool` | 停止定时器。如果已停止，无效果。 |
| `reset(ticks_to_wait) -> OsalRsBool` | 重新开始计时（重启剩余时间为期满值）。 |
| `change_period(new_period_in_ticks, new_period_ticks) -> OsalRsBool` | 改变周期。`new_period_ticks` 是阻塞等命令队列的时间。 |
| `delete(&mut self, ticks_to_wait) -> OsalRsBool` | 停止并销毁定时器。`ticks_to_wait` 是等待定时器守护任务处理的时间。 |
| `get_expiry_time() -> TickType` | 返回下个到期时间的 **绝对 tick 值** |
| `get_name() -> &str` | 返回定时器名称 |

### 7.3 回调执行上下文

| 规则 | 说明 |
|------|------|
| **执行线程** | 在定时器守护任务/调度线程中执行，**非 ISR 上下文** |
| **执行顺序** | 所有回调串行执行（单线程） |
| **时间约束** | 回调必须短小，不能阻塞。长回调会延迟其他定时器。 |
| **禁止操作** | 回调中不要调用 `delay`、`mutex.lock` 等可能阻塞的 API |
| **推荐操作** | 设置事件标志、post 消息到队列、signal 信号量 |

### 7.4 FreeRTOS 内部实现参考

FreeRTOS 使用：
- `xTimerCreate()` 创建定时器 → 注册 C 回调包装函数
- 定时器守护任务 (Timer Service Task) 处理定时器命令队列
- 回调通过 `callback_c_wrapper` 桥接到 Rust 闭包
- `start/stop/reset/change_period` → 向守护任务发送命令

### 7.5 POSIX 需要的实现方案

POSIX 端需要一个**全局 TimerScheduler**：

```
TimerScheduler {
    pending: BinaryHeap<(expiry_time, timer_id)>,  // 最小堆，按到期时间排序
    timers: HashMap<timer_id, TimerState>,          // timer_id → 定时器状态
    condvar: Condvar,                               // 唤醒调度线程
    next_id: AtomicUsize,                           // 单调递增 ID 生成器
}
```

**调度线程核心逻辑**:

```
loop {
    lock();
    let next = pending.peek();          // 最近到期的
    let now = System::get_tick_count();
    
    if next.expiry_time <= now {
        取出所有到期 timer;
        for each expired:
            if auto_reload:
                计算新 expiry = now + period, push 回堆;
            else:
                标记为 stopped;
            在锁外调用 callback(timer, param);
    } else {
        wait_until(next.expiry_time - now);
    }
    unlock();
}
```

### 7.6 POSIX 当前状态

❌ **完全桩代码** —— `start/stop/reset/change_period` 全部立即返回 True，无任何实际逻辑。回调从未被调用。

---

## 8. ThreadFn Trait — 线程/任务管理

### 8.1 构造

```rust
Thread::new(name: &str, stack_depth: StackType, priority: UBaseType) -> Self
// 创建未初始化的线程，必须调用 spawn/spawn_simple 才会实际创建
```

### 8.2 操作契约

| 方法 | 语义 |
|------|------|
| `spawn(param, callback) -> Result<Self>` | 创建并启动线程。回调: `Fn(Box<dyn ThreadFn>, Option<ThreadParam>) -> Result<ThreadParam>` |
| `spawn_simple(callback) -> Result<Self>` | 简化版，回调: `Fn() + Send + Sync + 'static` |
| `delete(&self)` | 删除线程（不等待完成） |
| `suspend(&self)` | 挂起线程 |
| `resume(&self)` | 恢复线程 |
| `join(&self, retval: DoublePtr) -> Result<i32>` | 等待线程结束并获取返回值 |
| `get_metadata(&self) -> ThreadMetadata` | 获取线程元数据 |
| `get_current() -> Self` | 获取当前线程的 Thread 对象 |

### 8.3 线程通知 (Notification) 契约

| 方法 | 语义 |
|------|------|
| `notify(notification) -> Result<()>` | 发送通知（Increment/SetBits/SetValue 等） |
| `notify_from_isr(notification, &mut BaseType) -> Result<()>` | ISR 通知 |
| `wait_notification(bits_clear_entry, bits_clear_exit, timeout) -> Result<u32>` | 阻塞等待通知，返回通知值 |

### 8.4 ThreadNotification 枚举

```rust
pub enum ThreadNotification {
    NoAction,                           // 不更新值
    SetBits(u32),                       // 按位 OR
    Increment,                          // +1
    SetValueWithOverwrite(u32),         // 覆盖
    SetValueWithoutOverwrite(u32),      // 仅当无待处理通知时设置
}
```

### 8.5 POSIX 当前状态

❌ **完全桩代码** — `spawn` 只保存回调但从不执行，`notify`/`wait_notification` 无实际操作。

---

## 9. ToTick / FromTick — 时间转换

### 9.1 ToTick 契约

```rust
pub trait ToTick: Sized + Copy {
    fn to_ticks(&self) -> TickType;
}
```

| 实现者 | 转换逻辑 |
|--------|---------|
| `Duration` | `as_millis() / TICK_PERIOD_MS` |
| `TickType` (u32/u64) | 直接返回自身 |

**POSIX 配置**: `TICK_PERIOD_MS = 1` → 1 tick = 1ms

### 9.2 FromTick 契约

```rust
pub trait FromTick {
    fn ticks(&mut self, tick: TickType);
}
```

| 实现者 | 转换逻辑 |
|--------|---------|
| `Duration` | `*self = Duration::from_millis(tick)` (因 TICK_PERIOD_MS=1) |

---

## 10. 错误类型

```rust
pub enum Error<'a> {
    Timeout,              // 操作超时
    NullPtr,              // 空指针/空句柄
    OutOfMemory,          // 内存不足
    InvalidParameter,     // 无效参数
    InvalidQueueSize,     // 队列大小无效 (size=0 或 message_size=0)
    MutexLockFailed,      // 互斥锁获取失败
    Unhandled(&'a str),   // 未处理错误（带消息）
    ReadError(&'a str),   // 读取错误
}
```

---

## 11. OsalRsBool 布尔类型

```rust
pub enum OsalRsBool {
    True = 1,
    False = 0,
}
```

用于 RTOS 返回值的布尔类型，与 FreeRTOS `pdTRUE`/`pdFALSE` 对应。

---

## 12. MAX_DELAY 常量

```rust
pub const MAX_DELAY: Duration = Duration::MAX;
```

表示"无限等待"。在需要 `TickType` 的地方用 `MAX_DELAY.to_ticks()` 转换。

---

## 13. 线程安全约定

| 实体 | 要求 |
|------|------|
| 所有句柄类型 | `Send + Sync` |
| `ThreadParam` / `TimerParam` | `Arc<dyn Any + Send + Sync>` |
| 回调函数 | `Fn(...) + Send + Sync + 'static` |
| 所有 trait 方法 | 必须线程安全 |

---

## 14. POSIX 后端开发优先级总结

| 优先级 | 模块 | 当前完成度 | 核心工作 |
|--------|------|-----------|---------|
| 🔴 P0 | `ffi.rs` | 不存在 | 创建 C 标准库 FFI 声明 |
| 🔴 P0 | `mutex.rs` (RawMutex) | 0% 桩 | 实现真实互斥（std::sync::Mutex + Arc） |
| 🔴 P0 | `timer.rs` | 0% 桩 | 实现 TimerScheduler + 全部 TimerFn 方法 |
| 🟡 P1 | `semaphore.rs` | 0% 桩 | Condvar + Mutex\<count\> 实现 |
| 🟡 P1 | `queue.rs` | 0% 桩 | VecDeque + Condvar 实现 |
| 🟡 P1 | `event_group.rs` | 0% 桩 | Condvar + EventBits 实现 |
| 🟡 P1 | `thread.rs` | 0% 桩 | std::thread::spawn + 通知机制 |
| 🟢 P2 | `system.rs` 临界区 | 空操作 | 可以保持空操作，用文档说明 |
| 🟢 P2 | `system.rs` 线程统计 | 虚假数据 | 可改进但非必须 |

---

> **本文档基于 `osal-rs` FreeRTOS 参考实现 (2026-06-10)，作为 POSIX 后端开发的权威契约。**
> 每个 trait 方法的 POSIX 实现必须满足本文档定义的行为语义。
