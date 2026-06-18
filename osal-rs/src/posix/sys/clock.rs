//! CLOCK_MONOTONIC helpers.

use libc::{clock_gettime, timespec, CLOCK_MONOTONIC};

pub fn now() -> timespec {
    let mut ts: timespec = timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe { clock_gettime(CLOCK_MONOTONIC, &mut ts) };
    ts
}

pub fn now_ns() -> u64 {
    let ts = now();
    (ts.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(ts.tv_nsec as u64)
}

pub fn ns_to_timespec(ns: u64) -> timespec {
    timespec {
        tv_sec: (ns / 1_000_000_000) as libc::time_t,
        tv_nsec: (ns % 1_000_000_000) as libc::c_long,
    }
}

pub fn ms_to_ns(ms: u64) -> u64 {
    ms.saturating_mul(1_000_000)
}

pub fn deadline_from_ms(ms: u64) -> timespec {
    ns_to_timespec(now_ns().saturating_add(ms_to_ns(ms)))
}
