use core::time::Duration;

use crate::linux::config::TICK_PERIOD_MS;
use crate::linux::types::TickType;
use crate::traits::{FromTick, ToTick};

impl ToTick for Duration {
    fn to_ticks(&self) -> TickType {
        let millis = self.as_millis() as TickType;
        let period = TICK_PERIOD_MS as TickType;

        if period == 0 {
            TickType::MAX
        } else {
            millis / period
        }
    }
}

impl FromTick for Duration {
    fn ticks(&mut self, tick: TickType) {
        *self = Duration::from_millis(tick.saturating_mul(TICK_PERIOD_MS as TickType) as u64);
    }
}