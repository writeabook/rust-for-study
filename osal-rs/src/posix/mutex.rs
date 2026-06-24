//! Native POSIX mutex using `libc::pthread_mutex_t`.
//!
//! - **RawMutex**: `PTHREAD_MUTEX_RECURSIVE`.
//! - **Mutex\<T\>**: `PTHREAD_MUTEX_ERRORCHECK` + `UnsafeCell<Box<T>>`.

use core::cell::UnsafeCell;
use core::fmt::{Debug, Display, Formatter};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::ptr;
use core::sync::atomic::{AtomicUsize, Ordering};

use alloc::boxed::Box;
use alloc::sync::Arc;

use libc::PTHREAD_MUTEX_ERRORCHECK;
use libc::PTHREAD_MUTEX_RECURSIVE;

use super::sys::mutex::PosixMutex;
use super::types::MutexHandle;
use crate::traits::{MutexFn, MutexGuardFn, RawMutexFn};
use crate::utils::{Error, OsalRsBool, Result};

// ===========================================================================
// RawMutex — recursive
// ===========================================================================

/// Monotonic counter for `RawMutex` and `Mutex<T>` handles.
///
/// Using a counter instead of `PosixMutex::raw_ptr()` guarantees that two
/// live mutexes never get the same handle, even when the allocator reuses
/// memory or NLL drops a value early.
static NEXT_MUTEX_HANDLE: AtomicUsize = AtomicUsize::new(1);

pub struct RawMutex {
    inner: PosixMutex,
    handle: MutexHandle,
}

unsafe impl Send for RawMutex {}
unsafe impl Sync for RawMutex {}

impl RawMutex {
    pub fn new() -> Result<Self> {
        let inner = PosixMutex::new(PTHREAD_MUTEX_RECURSIVE).ok_or(Error::OutOfMemory)?;
        let handle = NEXT_MUTEX_HANDLE.fetch_add(1, Ordering::Relaxed) as MutexHandle;
        Ok(Self { inner, handle })
    }
}

impl RawMutexFn for RawMutex {
    fn lock(&self) -> OsalRsBool {
        if self.inner.lock() {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }
    fn lock_from_isr(&self) -> OsalRsBool {
        if self.inner.try_lock() {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }
    fn unlock(&self) -> OsalRsBool {
        if self.inner.unlock() {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }
    fn unlock_from_isr(&self) -> OsalRsBool {
        if self.inner.unlock() {
            OsalRsBool::True
        } else {
            OsalRsBool::False
        }
    }
    fn delete(&mut self) {} // Drop handles destroy
}

impl Deref for RawMutex {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Debug for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RawMutex")
            .field("handle", &self.handle)
            .finish()
    }
}
impl Display for RawMutex {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "RawMutex {{ handle: {:?} }}", self.handle)
    }
}

// ===========================================================================
// Mutex<T> — non-recursive
// ===========================================================================

pub struct Mutex<T: ?Sized> {
    inner: PosixMutex,
    data: UnsafeCell<Box<T>>,
    handle: MutexHandle,
}

unsafe impl<T: ?Sized + Send> Send for Mutex<T> {}
unsafe impl<T: ?Sized + Send> Sync for Mutex<T> {}

impl<T: ?Sized> Deref for Mutex<T> {
    type Target = MutexHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl<T> Mutex<T> {
    pub fn new(data: T) -> Self {
        let inner = PosixMutex::new(PTHREAD_MUTEX_ERRORCHECK).expect("Mutex: pthread_mutex_init");
        let handle = NEXT_MUTEX_HANDLE.fetch_add(1, Ordering::Relaxed) as MutexHandle;
        Self {
            inner,
            data: UnsafeCell::new(Box::new(data)),
            handle,
        }
    }
    pub fn new_arc(data: T) -> Arc<Self> {
        Arc::new(Self::new(data))
    }
}

impl<T: ?Sized> Mutex<T> {
    pub fn lock_from_isr_explicit(&self) -> Result<MutexGuardFromIsr<'_, T>> {
        self.lock_from_isr()
    }
}

impl<T: ?Sized> MutexFn<T> for Mutex<T> {
    type Guard<'a>
        = MutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;
    type GuardFromIsr<'a>
        = MutexGuardFromIsr<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn lock(&self) -> Result<Self::Guard<'_>> {
        if self.inner.lock() {
            Ok(MutexGuard {
                mutex: self,
                _phantom: PhantomData,
            })
        } else {
            Err(Error::MutexLockFailed)
        }
    }

    fn lock_from_isr(&self) -> Result<Self::GuardFromIsr<'_>> {
        if self.inner.try_lock() {
            Ok(MutexGuardFromIsr {
                mutex: self,
                _phantom: PhantomData,
            })
        } else {
            Err(Error::MutexLockFailed)
        }
    }

    fn into_inner(self) -> Result<T>
    where
        Self: Sized,
        T: Sized,
    {
        let data_ptr: *mut Box<T> = self.data.get();
        core::mem::forget(self);
        let boxed = unsafe { ptr::read(data_ptr) };
        Ok(*boxed)
    }

    fn get_mut(&mut self) -> &mut T {
        self.data.get_mut().as_mut()
    }
}

impl<T: ?Sized> Debug for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Mutex")
            .field("handle", &self.handle)
            .finish()
    }
}
impl<T: ?Sized> Display for Mutex<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> core::fmt::Result {
        write!(f, "Mutex {{ handle: {:?} }}", self.handle)
    }
}

// ===========================================================================
// MutexGuard / MutexGuardFromIsr
// ===========================================================================

pub struct MutexGuard<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}
pub struct MutexGuardFromIsr<'a, T: ?Sized + 'a> {
    mutex: &'a Mutex<T>,
    _phantom: PhantomData<&'a mut T>,
}

impl<'a, T: ?Sized> Deref for MutexGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }.as_ref()
    }
}
impl<'a, T: ?Sized> DerefMut for MutexGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }.as_mut()
    }
}
impl<'a, T: ?Sized> Drop for MutexGuard<'a, T> {
    fn drop(&mut self) {
        let _ = self.mutex.inner.unlock();
    }
}

impl<'a, T: ?Sized> Deref for MutexGuardFromIsr<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.mutex.data.get() }.as_ref()
    }
}
impl<'a, T: ?Sized> DerefMut for MutexGuardFromIsr<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.mutex.data.get() }.as_mut()
    }
}
impl<'a, T: ?Sized> Drop for MutexGuardFromIsr<'a, T> {
    fn drop(&mut self) {
        let _ = self.mutex.inner.unlock();
    }
}

impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuard<'a, T> {
    fn update(&mut self, t: &T)
    where
        T: Clone,
    {
        **self = t.clone();
    }
}
impl<'a, T: ?Sized> MutexGuardFn<'a, T> for MutexGuardFromIsr<'a, T> {
    fn update(&mut self, t: &T)
    where
        T: Clone,
    {
        **self = t.clone();
    }
}
