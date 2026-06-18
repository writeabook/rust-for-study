/***************************************************************************
 *
 * osal-rs
 * Copyright (C) 2026 Antonio Salsi <passy.linux@zresa.it>
 *
 * This library is free software; you can redistribute it and/or
 * modify it under the terms of the GNU Lesser General Public
 * License as published by the Free Software Foundation; either
 * version 2.1 of the License, or (at your option) any later version.
 *
 * This library is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the GNU
 * Lesser General Public License for more details.
 *
 * You should have received a copy of the GNU Lesser General Public
 * License along with this library; if not, see <https://www.gnu.org/licenses/>.
 *
 ***************************************************************************/

//! Minimal POSIX pthread FFI bindings.
//!
//! Provides the `pthread_mutex_*` family needed by the POSIX backend mutex
//! module without pulling in the `libc` crate.

use core::alloc::Layout;
use core::ptr;

// ---------------------------------------------------------------------------
// Opaque pthread_mutex_t — sized for the largest common platform (64 bytes)
// ---------------------------------------------------------------------------

/// Opaque `pthread_mutex_t` storage.
///
/// Sized to 64 bytes, which covers all common platforms:
/// - Linux x86_64:   40 bytes
/// - Linux aarch64:  48 bytes
/// - macOS x86_64:   64 bytes
/// - macOS aarch64:  64 bytes
#[repr(C, align(8))]
pub(crate) struct PthreadMutex {
    _opaque: [u8; 64],
}

// ---------------------------------------------------------------------------
// Attribute constants
// ---------------------------------------------------------------------------

/// `PTHREAD_MUTEX_RECURSIVE` — allows the same thread to lock the mutex
/// multiple times without deadlocking.
pub(crate) const PTHREAD_MUTEX_RECURSIVE: i32 = 1;   // Linux; also works on macOS via NP

/// `PTHREAD_MUTEX_ERRORCHECK` — returns `EDEADLK` if the same thread
/// attempts to lock a mutex it already holds.
pub(crate) const PTHREAD_MUTEX_ERRORCHECK: i32 = 2;

// ---------------------------------------------------------------------------
// FFI declarations
// ---------------------------------------------------------------------------

unsafe extern "C" {
    /// `int pthread_mutex_init(pthread_mutex_t *m, const pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutex_init(m: *mut PthreadMutex, attr: *const ()) -> i32;

    /// `int pthread_mutex_destroy(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_destroy(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_lock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_lock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_trylock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_trylock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutex_unlock(pthread_mutex_t *m);`
    pub(crate) fn pthread_mutex_unlock(m: *mut PthreadMutex) -> i32;

    /// `int pthread_mutexattr_init(pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutexattr_init(attr: *mut PthreadMutexAttr) -> i32;

    /// `int pthread_mutexattr_destroy(pthread_mutexattr_t *attr);`
    pub(crate) fn pthread_mutexattr_destroy(attr: *mut PthreadMutexAttr) -> i32;

    /// `int pthread_mutexattr_settype(pthread_mutexattr_t *attr, int kind);`
    pub(crate) fn pthread_mutexattr_settype(attr: *mut PthreadMutexAttr, kind: i32) -> i32;
}

// ---------------------------------------------------------------------------
// Opaque pthread_mutexattr_t — 8 bytes on most platforms
// ---------------------------------------------------------------------------

#[repr(C, align(8))]
pub(crate) struct PthreadMutexAttr {
    _opaque: [u8; 8],
}

// ---------------------------------------------------------------------------
// Heap allocation + init helpers
// ---------------------------------------------------------------------------

/// Allocates and initialises a `PthreadMutex` with the given type attribute.
///
/// Returns a pointer to a heap-allocated, initialised mutex, or `None` if
/// allocation or initialisation fails.
pub(crate) fn create_mutex(mutex_type: i32) -> Option<*mut PthreadMutex> {
    let layout = Layout::new::<PthreadMutex>();
    // SAFETY: layout has non-zero size; allocation may fail
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut PthreadMutex };
    if ptr.is_null() {
        return None;
    }

    let mut attr_ptr: *mut PthreadMutexAttr = ptr::null_mut();

    let mut attr: PthreadMutexAttr = PthreadMutexAttr { _opaque: [0u8; 8] };
    let attr_init_ret = unsafe { pthread_mutexattr_init(&mut attr) };
    if attr_init_ret == 0 {
        let settype_ret = unsafe { pthread_mutexattr_settype(&mut attr, mutex_type) };
        if settype_ret == 0 {
            attr_ptr = &mut attr;
        }
    }

    let init_ret = unsafe { pthread_mutex_init(ptr, attr_ptr as *const ()) };

    if attr_ptr != ptr::null_mut() {
        unsafe { pthread_mutexattr_destroy(&mut attr) };
    }

    if init_ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Destroys and deallocates a `PthreadMutex`.
pub(crate) unsafe fn destroy_mutex(ptr: *mut PthreadMutex) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        pthread_mutex_destroy(ptr);
        alloc::alloc::dealloc(ptr as *mut u8, Layout::new::<PthreadMutex>());
    }
}

// ===========================================================================
// POSIX semaphore (sem_t) bindings
// ===========================================================================

/// Opaque `sem_t` storage.
///
/// Sized to 48 bytes to cover all common platforms
/// (Linux x86_64/aarch64: 32 bytes, macOS: varies).
#[repr(C, align(8))]
pub(crate) struct SemT {
    _opaque: [u8; 48],
}

/// `struct timespec` for `sem_timedwait`.
#[repr(C)]
pub(crate) struct Timespec {
    pub tv_sec: i64,
    pub tv_nsec: i64,
}

unsafe extern "C" {
    /// `int sem_init(sem_t *sem, int pshared, unsigned int value);`
    pub(crate) fn sem_init(sem: *mut SemT, pshared: i32, value: u32) -> i32;

    /// `int sem_destroy(sem_t *sem);`
    pub(crate) fn sem_destroy(sem: *mut SemT) -> i32;

    /// `int sem_wait(sem_t *sem);`
    pub(crate) fn sem_wait(sem: *mut SemT) -> i32;

    /// `int sem_trywait(sem_t *sem);`
    pub(crate) fn sem_trywait(sem: *mut SemT) -> i32;

    /// `int sem_timedwait(sem_t *sem, const struct timespec *abs_timeout);`
    pub(crate) fn sem_timedwait(sem: *mut SemT, abs_timeout: *const Timespec) -> i32;

    /// `int sem_post(sem_t *sem);`
    pub(crate) fn sem_post(sem: *mut SemT) -> i32;

    /// `int sem_getvalue(sem_t *sem, int *sval);`
    pub(crate) fn sem_getvalue(sem: *mut SemT, sval: *mut i32) -> i32;
}

/// Allocates and initialises a `sem_t` with the given initial value.
///
/// Uses `sem_init` (process-local, `pshared = 0`).
pub(crate) fn create_sem(initial_value: u32) -> Option<*mut SemT> {
    let layout = Layout::new::<SemT>();
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut SemT };
    if ptr.is_null() {
        return None;
    }
    let ret = unsafe { sem_init(ptr, 0, initial_value) };
    if ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Destroys and deallocates a `sem_t`.
pub(crate) unsafe fn destroy_sem(ptr: *mut SemT) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        sem_destroy(ptr);
        alloc::alloc::dealloc(ptr as *mut u8, Layout::new::<SemT>());
    }
}

// ===========================================================================
// pthread condition variable bindings
// ===========================================================================

/// Opaque `pthread_cond_t` storage (48 bytes covers all common platforms).
#[repr(C, align(8))]
pub(crate) struct PthreadCond {
    _opaque: [u8; 48],
}

/// `CLOCK_REALTIME` — used by `pthread_cond_timedwait`.
pub(crate) const CLOCK_REALTIME: i32 = 0;

unsafe extern "C" {
    /// `int pthread_cond_init(pthread_cond_t *cond, const pthread_condattr_t *attr);`
    pub(crate) fn pthread_cond_init(c: *mut PthreadCond, attr: *const PthreadCondAttr) -> i32;

    /// `int pthread_cond_destroy(pthread_cond_t *cond);`
    pub(crate) fn pthread_cond_destroy(c: *mut PthreadCond) -> i32;

    /// `int pthread_cond_wait(pthread_cond_t *cond, pthread_mutex_t *mutex);`
    pub(crate) fn pthread_cond_wait(c: *mut PthreadCond, m: *mut PthreadMutex) -> i32;

    /// `int pthread_cond_timedwait(pthread_cond_t *cond, pthread_mutex_t *mutex, const struct timespec *abstime);`
    pub(crate) fn pthread_cond_timedwait(c: *mut PthreadCond, m: *mut PthreadMutex, t: *const Timespec) -> i32;

    /// `int pthread_cond_signal(pthread_cond_t *cond);`
    pub(crate) fn pthread_cond_signal(c: *mut PthreadCond) -> i32;

    /// `int pthread_cond_broadcast(pthread_cond_t *cond);`
    pub(crate) fn pthread_cond_broadcast(c: *mut PthreadCond) -> i32;

    /// `int clock_gettime(clockid_t clk_id, struct timespec *tp);`
    pub(crate) fn clock_gettime(clk_id: i32, tp: *mut Timespec) -> i32;

    /// `int pthread_condattr_init(pthread_condattr_t *attr);`
    pub(crate) fn pthread_condattr_init(attr: *mut PthreadCondAttr) -> i32;

    /// `int pthread_condattr_destroy(pthread_condattr_t *attr);`
    pub(crate) fn pthread_condattr_destroy(attr: *mut PthreadCondAttr) -> i32;

    /// `int pthread_condattr_setclock(pthread_condattr_t *attr, clockid_t clock_id);`
    pub(crate) fn pthread_condattr_setclock(attr: *mut PthreadCondAttr, clock_id: i32) -> i32;
}

/// Opaque `pthread_condattr_t` (8 bytes on most platforms).
#[repr(C, align(8))]
pub(crate) struct PthreadCondAttr {
    _opaque: [u8; 8],
}

/// Allocates and initialises a `pthread_cond_t` with `CLOCK_MONOTONIC`.
pub(crate) fn create_cond_monotonic() -> Option<*mut PthreadCond> {
    let layout = Layout::new::<PthreadCond>();
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut PthreadCond };
    if ptr.is_null() {
        return None;
    }
    let mut attr: PthreadCondAttr = PthreadCondAttr { _opaque: [0u8; 8] };
    let ok = unsafe { pthread_condattr_init(&mut attr) } == 0
        && unsafe { pthread_condattr_setclock(&mut attr, CLOCK_MONOTONIC) } == 0;
    let ret = unsafe { pthread_cond_init(ptr, if ok { &attr } else { core::ptr::null() }) };
    if ok {
        unsafe { pthread_condattr_destroy(&mut attr) };
    }
    if ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Allocates and initialises a `pthread_cond_t`.
pub(crate) fn create_cond() -> Option<*mut PthreadCond> {
    let layout = Layout::new::<PthreadCond>();
    let ptr = unsafe { alloc::alloc::alloc(layout) as *mut PthreadCond };
    if ptr.is_null() {
        return None;
    }
    let ret = unsafe { pthread_cond_init(ptr, core::ptr::null()) };
    if ret == 0 {
        Some(ptr)
    } else {
        unsafe { alloc::alloc::dealloc(ptr as *mut u8, layout) };
        None
    }
}

/// Destroys and deallocates a `pthread_cond_t`.
pub(crate) unsafe fn destroy_cond(ptr: *mut PthreadCond) {
    if ptr.is_null() {
        return;
    }
    unsafe {
        pthread_cond_destroy(ptr);
        alloc::alloc::dealloc(ptr as *mut u8, Layout::new::<PthreadCond>());
    }
}

/// Returns the current `CLOCK_REALTIME` as a `Timespec`.
pub(crate) fn realtime_now() -> Timespec {
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_REALTIME, &mut ts) };
    ts
}

/// Returns the current `CLOCK_MONOTONIC` as a `Timespec`.
pub(crate) fn realtime_monotonic() -> Timespec {
    let mut ts = Timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    ts
}

/// Adds `ms` milliseconds to a `Timespec`.
pub(crate) fn timespec_add_ms(ts: &Timespec, ms: u64) -> Timespec {
    let total_ns = ts.tv_nsec + (ms as i64).saturating_mul(1_000_000);
    Timespec {
        tv_sec: ts.tv_sec + total_ns / 1_000_000_000,
        tv_nsec: total_ns % 1_000_000_000,
    }
}

// ===========================================================================
// POSIX per-process timer (timer_create / timer_settime / timer_delete)
// ===========================================================================

use core::ffi::c_void;

/// Opaque `timer_t` — on Linux this is `void*`.
pub(crate) type TimerT = *mut c_void;

/// `union sigval` — used to pass data to the timer callback.
#[repr(C)]
#[derive(Copy, Clone)]
pub(crate) union SigVal {
    pub sival_ptr: *mut c_void,
    pub sival_int: i32,
}

/// `SIGEV_THREAD` — notify via a new thread that calls the given function.
pub(crate) const SIGEV_THREAD: i32 = 2;

/// `CLOCK_MONOTONIC` — monotonic clock, not affected by system time changes.
pub(crate) const CLOCK_MONOTONIC: i32 = 1;

/// Timer callback trampoline signature: `void (*)(union sigval)`.
pub(crate) type SigevNotifyFn = unsafe extern "C" fn(SigVal);

/// `struct sigevent` — opaque storage (glibc: 72 bytes on x86_64).
///
/// We define this as an opaque byte array because glibc's actual layout
/// includes a large `__SIZEOF_PTHREAD_ATTR_T`-sized union padding between
/// `sigev_notify` and the notification-function pointer.
#[repr(C, align(8))]
pub(crate) struct SigEvent {
    _opaque: [u8; 80],
}

/// `struct itimerspec` — timer period specification.
#[repr(C)]
pub(crate) struct ITimerSpec {
    pub it_interval: Timespec,   // reload period (0 = one-shot)
    pub it_value: Timespec,      // initial expiration (0 = disarmed)
}

unsafe extern "C" {
    /// `int timer_create(clockid_t, struct sigevent *, timer_t *);`
    pub(crate) fn timer_create(clockid: i32, evp: *mut SigEvent, timerid: *mut TimerT) -> i32;

    /// `int timer_settime(timer_t, int flags, const struct itimerspec *, struct itimerspec *);`
    pub(crate) fn timer_settime(
        timerid: TimerT,
        flags: i32,
        new_value: *const ITimerSpec,
        old_value: *mut ITimerSpec,
    ) -> i32;

    /// `int timer_delete(timer_t);`
    pub(crate) fn timer_delete(timerid: TimerT) -> i32;
}

/// Creates a `SigEvent` configured for `SIGEV_THREAD` delivery with the
/// given trampoline function and argument pointer.
///
/// Writes fields at the appropriate offsets inside the opaque struct
/// so that the layout matches glibc's `struct sigevent` on all platforms.
pub(crate) fn make_sigevent(func: SigevNotifyFn, arg: *mut c_void) -> SigEvent {
    let mut ev: SigEvent = SigEvent { _opaque: [0u8; 80] };
    unsafe {
        // Offset 0: sigev_value (SigVal = 8 bytes)
        let value_ptr = ev._opaque.as_mut_ptr() as *mut SigVal;
        *value_ptr = SigVal { sival_ptr: arg };
        // Offset 8: sigev_signo (i32)
        let signo_ptr = ev._opaque.as_mut_ptr().add(8) as *mut i32;
        *signo_ptr = 0;
        // Offset 12: sigev_notify (i32)
        let notify_ptr = ev._opaque.as_mut_ptr().add(12) as *mut i32;
        *notify_ptr = SIGEV_THREAD;
        // Offset 16: _sigev_un._sigev_thread._function (function pointer, 8 bytes)
        let func_ptr = ev._opaque.as_mut_ptr().add(16) as *mut Option<SigevNotifyFn>;
        *func_ptr = Some(func);
        // Offset 24: _sigev_un._sigev_thread._attribute (pthread_attr_t*, 8 bytes)
        let attr_ptr = ev._opaque.as_mut_ptr().add(24) as *mut *mut c_void;
        *attr_ptr = core::ptr::null_mut();
    }
    ev
}

/// Builds an `ITimerSpec` from a period in milliseconds.
///
/// `it_value` is set to `period_ms` (the initial expiration).
/// `it_interval` is set to `period_ms` if `auto_reload` is true, otherwise 0.
pub(crate) fn make_itimerspec(period_ms: u64, auto_reload: bool) -> ITimerSpec {
    let sec = (period_ms / 1000) as i64;
    let nsec = ((period_ms % 1000) * 1_000_000) as i64;
    let interval = if auto_reload {
        Timespec { tv_sec: sec, tv_nsec: nsec }
    } else {
        Timespec { tv_sec: 0, tv_nsec: 0 }
    };
    ITimerSpec {
        it_interval: interval,
        it_value: Timespec { tv_sec: sec, tv_nsec: nsec },
    }
}

/// Builds a zero `ITimerSpec` that disarms the timer.
pub(crate) fn zero_itimerspec() -> ITimerSpec {
    ITimerSpec {
        it_interval: Timespec { tv_sec: 0, tv_nsec: 0 },
        it_value: Timespec { tv_sec: 0, tv_nsec: 0 },
    }
}
