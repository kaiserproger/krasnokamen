//! Opt-in tick diagnostics. Disabled collectors hold no clock or allocation.
//! `lap` partitions elapsed work; `record` accepts an already measured span
//! without moving that boundary (for summaries and nested breakdowns).

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

    /// Records a known span without changing the next lap's starting point.
    pub(crate) fn record(&mut self, name: &'static str, time: Duration) {
        if self.last.is_some() {
            self.rows.push((name, time));
        }
    }

    /// `  <label> detail ms: a=0.12 b=3.40 ...`; nothing when disabled.
    pub(crate) fn print(&self, label: &str) {
        self.print_line(format_args!("  {label} detail ms:"));
    }

    /// Retains the summary prefixes consumed by the coreperf log reader.
    pub(crate) fn print_summary(&self, label: &str, context: std::fmt::Arguments<'_>) {
        self.print_line(format_args!("{label} ms: {context}"));
    }

    fn line(&self, prefix: std::fmt::Arguments<'_>) -> Option<String> {
        self.last?;
        let mut line = prefix.to_string();
        for (name, time) in &self.rows {
            line.push_str(&format!(" {name}={:.2}", time.as_secs_f64() * 1000.0));
        }
        Some(line)
    }

    fn print_line(&self, prefix: std::fmt::Arguments<'_>) {
        if let Some(line) = self.line(prefix) {
            eprintln!("{line}");
        }
    }
}

#[cfg(test)]
#[path = "phase_laps_tests.rs"]
mod tests;
