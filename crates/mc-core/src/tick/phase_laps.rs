//! Per-callee lap timers inside one tick phase, printed only under
//! `MC_CORE_PHASE_LOG`. Disabled laps hold no clock and no allocation: `lap`
//! is one `Option` test. Each lap covers the code since the previous lap, so a
//! phase's laps sum to its total (statements between calls land in the next lap).

use std::time::{Duration, Instant};

pub(crate) struct Laps {
    last: Option<Instant>,
    rows: Vec<(&'static str, Duration)>,
}

impl Laps {
    /// Starts at `start` when `enabled`; otherwise every call is a no-op.
    pub(crate) fn new(enabled: bool, start: Instant) -> Self {
        Self {
            last: enabled.then_some(start),
            rows: Vec::new(),
        }
    }

    /// Attributes the time since the previous lap (or the start) to `name`.
    pub(crate) fn lap(&mut self, name: &'static str) {
        if let Some(last) = self.last {
            let now = Instant::now();
            self.rows.push((name, now.duration_since(last)));
            self.last = Some(now);
        }
    }

    /// `  <label> detail ms: a=0.12 b=3.40 ...`; nothing when disabled.
    pub(crate) fn print(&self, label: &str) {
        if self.last.is_none() {
            return;
        }
        let mut line = format!("  {label} detail ms:");
        for (name, time) in &self.rows {
            line.push_str(&format!(" {name}={:.2}", time.as_secs_f64() * 1000.0));
        }
        eprintln!("{line}");
    }
}
