use serde::{Deserialize, Serialize};

use crate::ProtocolError;

/// Daily / clock time window when the GPU may run Horde work.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimeWindow {
    /// Inclusive start hour in local wall clock (0–23).
    pub start_hour: u8,
    /// Exclusive end hour in local wall clock (1–24). Window may wrap midnight.
    pub end_hour: u8,
    /// Soft budget: max GPU-minutes per calendar day (None = unlimited within window).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_gpu_minutes_per_day: Option<u32>,
}

impl Default for TimeWindow {
    fn default() -> Self {
        Self {
            start_hour: 0,
            end_hour: 24,
            max_gpu_minutes_per_day: None,
        }
    }
}

impl TimeWindow {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.start_hour > 23 {
            return Err(ProtocolError::LimitRejected(
                "start_hour must be 0–23".into(),
            ));
        }
        if self.end_hour == 0 || self.end_hour > 24 {
            return Err(ProtocolError::LimitRejected(
                "end_hour must be 1–24".into(),
            ));
        }
        Ok(())
    }

    /// Whether `hour` (0–23) falls inside the window.
    pub fn contains_hour(&self, hour: u8) -> bool {
        if self.start_hour < self.end_hour {
            hour >= self.start_hour && hour < self.end_hour
        } else if self.start_hour > self.end_hour {
            // wraps midnight, e.g. 22–7
            hour >= self.start_hour || hour < self.end_hour
        } else {
            // start == end → treat as always open only if 0–24 style; equal hours = empty
            false
        }
    }
}

/// Per-physical-server hard caps (always apply, including local mode).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerLimits {
    /// Maximum GPU utilization percentage allowed for Horde (1–100).
    pub gpu_percent: u8,
    pub time_window: TimeWindow,
    /// Disk budget for model cache in MiB.
    pub disk_budget_mb: u64,
    /// When false, server only accepts local-collective jobs (not network).
    pub accept_network_jobs: bool,
    pub enabled: bool,
}

impl Default for ServerLimits {
    fn default() -> Self {
        Self {
            gpu_percent: 100,
            time_window: TimeWindow::default(),
            disk_budget_mb: 50_000,
            accept_network_jobs: false,
            enabled: true,
        }
    }
}

impl ServerLimits {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.gpu_percent == 0 || self.gpu_percent > 100 {
            return Err(ProtocolError::LimitRejected(
                "gpu_percent must be 1–100".into(),
            ));
        }
        self.time_window.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_wraps_midnight() {
        let w = TimeWindow {
            start_hour: 22,
            end_hour: 7,
            max_gpu_minutes_per_day: None,
        };
        assert!(w.contains_hour(23));
        assert!(w.contains_hour(3));
        assert!(!w.contains_hour(12));
    }
}
