//! System-level operations stub for Linux backend.
//!
//! Placeholder until the full implementation is developed.

use alloc::vec::Vec;
use crate::linux::thread::{ThreadMetadata, ThreadState};

#[derive(Debug, Clone)]
pub struct SystemState {
    pub tasks: Vec<ThreadMetadata>,
    pub total_run_time: u32,
}

impl core::ops::Deref for SystemState {
    type Target = [ThreadMetadata];

    fn deref(&self) -> &Self::Target {
        &self.tasks
    }
}

use crate::linux::types::TickType;
use core::time::Duration;

pub struct System;

impl System {
    #[allow(dead_code)]
    pub fn get_current_time_us() -> Duration {
        Duration::ZERO
    }

    #[allow(dead_code)]
    pub fn get_tick_count() -> TickType {
        0
    }
}
