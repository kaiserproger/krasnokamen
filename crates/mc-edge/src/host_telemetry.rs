//! Host telemetry for load graphs: the server cgroup's memory and CPU time,
//! hypervisor steal and process RSS, sampled once per 20-tick window on the
//! core tick stream and keyed by the tick that triggered the sample, so they
//! join the dashboard's per-tick timing ring exactly (`/telemetry.jsonl`).
//!
//! Linux sources; a value that cannot be read is `None`, never zero:
//! - cgroup v2 `memory.current` (bytes) and `cpu.stat` `usage_usec`
//!   (cumulative microseconds), both including descendants, of the cgroup
//!   `/proc/self/cgroup` names on its `0::<path>` line, under `/sys/fs/cgroup`
//!   (<https://docs.kernel.org/admin-guide/cgroup-v2.html>);
//! - the aggregate `cpu` line of `/proc/stat` in `USER_HZ` ticks: `steal`
//!   (eighth field) and `user` through `steal` summed. `guest` and
//!   `guest_nice` are left out of the sum because the kernel already counts
//!   them in `user` and `nice`
//!   (<https://man7.org/linux/man-pages/man5/proc_stat.5.html>);
//! - `VmRSS` of `/proc/self/status` (process resident memory).
//!
//! Counters are cumulative: graph CPU as the difference between samples.

use std::path::{Path, PathBuf};

use serde::Serialize;

/// Core ticks per host sample (one per second at 20 TPS).
pub(crate) const SAMPLE_TICKS: u64 = 20;

/// One host sample.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct HostSample {
    pub sampled_unix_ns: u64,
    pub cgroup_memory_current_bytes: Option<u64>,
    pub cgroup_cpu_usage_usec: Option<u64>,
    pub cpu_total_ticks: Option<u64>,
    pub cpu_steal_ticks: Option<u64>,
    pub process_rss_bytes: Option<u64>,
}

/// Samples the first tick it sees in each [`SAMPLE_TICKS`] window.
#[derive(Debug)]
pub(crate) struct HostSampler {
    cgroup: Option<PathBuf>,
    last_window: Option<u64>,
}

impl HostSampler {
    /// Resolve this process's cgroup v2 directory once.
    pub(crate) fn new() -> Self {
        let cgroup = std::fs::read_to_string("/proc/self/cgroup")
            .ok()
            .and_then(|text| cgroup_v2_dir(Path::new("/sys/fs/cgroup"), &text));
        Self {
            cgroup,
            last_window: None,
        }
    }

    /// A sample when `tick` opens a window not sampled yet.
    pub(crate) fn on_tick(&mut self, tick: u64) -> Option<HostSample> {
        let window = tick / SAMPLE_TICKS;
        if self.last_window == Some(window) {
            return None;
        }
        self.last_window = Some(window);
        Some(self.sample())
    }

    fn sample(&self) -> HostSample {
        let read = |path: &Path| std::fs::read_to_string(path).ok();
        let cgroup_file = |name: &str| self.cgroup.as_ref().and_then(|dir| read(&dir.join(name)));
        let cpu = read(Path::new("/proc/stat")).and_then(|text| proc_stat_cpu(&text));
        HostSample {
            sampled_unix_ns: crate::dashboard::unix_ns(),
            cgroup_memory_current_bytes: cgroup_file("memory.current")
                .and_then(|text| text.trim().parse().ok()),
            cgroup_cpu_usage_usec: cgroup_file("cpu.stat").and_then(|text| cpu_stat_usage(&text)),
            cpu_total_ticks: cpu.map(|(total, _)| total),
            cpu_steal_ticks: cpu.map(|(_, steal)| steal),
            process_rss_bytes: resident_memory_bytes(),
        }
    }
}

/// The cgroup v2 directory named by `/proc/self/cgroup` (`0::<path>`).
fn cgroup_v2_dir(root: &Path, proc_self_cgroup: &str) -> Option<PathBuf> {
    let path = proc_self_cgroup
        .lines()
        .find_map(|line| line.strip_prefix("0::"))?;
    Some(root.join(path.trim_start_matches('/')))
}

/// `usage_usec` of a cgroup v2 `cpu.stat`.
fn cpu_stat_usage(text: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix("usage_usec "))?
        .trim()
        .parse()
        .ok()
}

/// `(user..steal summed, steal)` of the aggregate `/proc/stat` `cpu` line.
fn proc_stat_cpu(text: &str) -> Option<(u64, u64)> {
    let fields: Vec<u64> = text
        .lines()
        .find_map(|line| line.strip_prefix("cpu "))?
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let busy_and_idle = fields.get(..8)?;
    Some((busy_and_idle.iter().sum(), busy_and_idle[7]))
}

/// This process's resident memory (`VmRSS`).
pub(crate) fn resident_memory_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kib.checked_mul(1024)
}

#[cfg(test)]
#[path = "host_telemetry_tests.rs"]
mod host_telemetry_tests;
