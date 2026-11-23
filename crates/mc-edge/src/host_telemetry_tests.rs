use super::*;

#[test]
fn cgroup_v2_line_names_the_directory_under_the_root() {
    let text = "0::/system.slice/solaris.service\n";
    assert_eq!(
        cgroup_v2_dir(Path::new("/sys/fs/cgroup"), text),
        Some(PathBuf::from("/sys/fs/cgroup/system.slice/solaris.service"))
    );
    assert_eq!(
        cgroup_v2_dir(Path::new("/sys/fs/cgroup"), "0::/\n"),
        Some(PathBuf::from("/sys/fs/cgroup"))
    );
    // cgroup v1 only: no unified hierarchy line, so no cgroup values.
    assert_eq!(
        cgroup_v2_dir(Path::new("/sys/fs/cgroup"), "4:memory:/user.slice\n"),
        None
    );
}

#[test]
fn cpu_stat_reads_usage_usec() {
    let text = "usage_usec 123456789\nuser_usec 100\nsystem_usec 23\nnr_periods 0\n";
    assert_eq!(cpu_stat_usage(text), Some(123_456_789));
    assert_eq!(cpu_stat_usage("user_usec 1\n"), None);
}

/// CPUQuota saturation: the throttle counters separate expensive code from too little CPU
/// (PERF-RECEIPT-OMITS-CPU-SATURATION). Cumulative, so None when the kernel does not report them.
#[test]
fn cpu_stat_reads_the_throttle_counters() {
    let text = "usage_usec 9\nnr_periods 900\nnr_throttled 17\nthrottled_usec 4242\n";
    assert_eq!(cpu_stat_field(text, "nr_throttled"), Some(17));
    assert_eq!(cpu_stat_field(text, "throttled_usec"), Some(4242));
    assert_eq!(cpu_stat_field("nr_periods 900\n", "nr_throttled"), None);
    // `nr_throttled_extra` must not answer for `nr_throttled`.
    assert_eq!(cpu_stat_field("nr_throttled_extra 5\n", "nr_throttled"), None);
}

/// PSI memory saturation beside memory.current (PERF-RECEIPT-OMITS-MEMORY-PRESSURE).
#[test]
fn memory_pressure_parses_some_and_full() {
    let text = "some avg10=0.45 avg60=0.10 avg300=0.02 total=987654\n\
                full avg10=1.20 avg60=0.30 avg300=0.05 total=12345\n";
    let pressure = parse_pressure(text).expect("a pressure file with both kinds");
    assert_eq!(pressure.some_avg10_centi, Some(45));
    assert_eq!(pressure.some_total_usec, Some(987_654));
    assert_eq!(pressure.full_avg10_centi, Some(120));
    assert_eq!(pressure.full_total_usec, Some(12_345));
    assert_eq!(parse_pressure(""), None, "an empty file is unavailable, never zero");
    // A truncated kernel without `full` still reports what it has.
    let some_only = parse_pressure("some avg10=0.00 avg60=0.00 avg300=0.00 total=0\n")
        .expect("some is present");
    assert_eq!(some_only.full_total_usec, None);
}

/// `steal` is the eighth field; the total stops there because guest time
/// is already inside `user`/`nice`; per-CPU lines are ignored.
#[test]
fn proc_stat_sums_user_through_steal() {
    let text = "cpu  10 1 5 100 2 0 3 7 40 0\ncpu0 5 0 2 50 1 0 1 4 20 0\nintr 1 2 3\n";
    assert_eq!(proc_stat_cpu(text), Some((128, 7)));
    assert_eq!(proc_stat_cpu("cpu  1 2 3\n"), None, "pre-steal kernels");
    assert_eq!(proc_stat_cpu("cpu0 1 2 3 4 5 6 7 8\n"), None);
}

#[test]
fn sampler_takes_the_first_tick_of_each_window_and_skips_gaps() {
    let mut sampler = HostSampler {
        cgroup: None,
        last_window: None,
    };
    let sampled: Vec<u64> = [3, 4, 19, 20, 21, 65, 66, 80]
        .into_iter()
        .filter(|&tick| sampler.on_tick(tick).is_some())
        .collect();
    assert_eq!(sampled, [3, 20, 65, 80]);
}

/// Unreadable cgroup files are unavailable, never zero.
#[test]
fn missing_cgroup_reads_none() {
    let sampler = HostSampler {
        cgroup: Some(PathBuf::from("/nonexistent/krasnokamen-test-cgroup")),
        last_window: None,
    };
    let sample = sampler.sample();
    assert_eq!(sample.cgroup_memory_current_bytes, None);
    assert_eq!(sample.cgroup_cpu_usage_usec, None);
    assert_eq!(sample.cgroup_cpu_nr_throttled, None);
    assert_eq!(sample.cgroup_cpu_throttled_usec, None);
    assert_eq!(sample.cgroup_memory_pressure, None);
    assert!(sample.sampled_unix_ns > 0);
}

/// A temporary cgroup directory named by the test: the production `sample()` reads exactly these
/// files, so the registered refs fail at their assertion when the sample wiring is reversed — the
/// parse helpers above cannot be refs, they would stop compiling instead of going red.
fn fixture_cgroup(name: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "krasnokamen-host-telemetry-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("fixture cgroup directory");
    for (file, text) in files {
        std::fs::write(dir.join(file), text).expect("fixture cgroup file");
    }
    dir
}

/// PERF-RECEIPT-OMITS-CPU-SATURATION on the server: the sample carries the throttle counters.
#[test]
fn sampler_reads_the_throttle_counters_from_the_cgroup_fixture() {
    let dir = fixture_cgroup(
        "cpu-stat",
        &[(
            "cpu.stat",
            "usage_usec 123456789\nuser_usec 100\nsystem_usec 23\nnr_periods 900\n\
             nr_throttled 17\nthrottled_usec 4242\n",
        )],
    );
    let sampler = HostSampler {
        cgroup: Some(dir.clone()),
        last_window: None,
    };
    let sample = sampler.sample();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(sample.cgroup_cpu_usage_usec, Some(123_456_789));
    assert_eq!(sample.cgroup_cpu_nr_throttled, Some(17));
    assert_eq!(sample.cgroup_cpu_throttled_usec, Some(4242));
}

/// PERF-RECEIPT-OMITS-MEMORY-PRESSURE on the server: the sample carries the cgroup PSI.
#[test]
fn sampler_reads_memory_pressure_from_the_cgroup_fixture() {
    let dir = fixture_cgroup(
        "memory-pressure",
        &[(
            "memory.pressure",
            "some avg10=0.45 avg60=0.10 avg300=0.02 total=987654\n\
             full avg10=1.20 avg60=0.30 avg300=0.05 total=12345\n",
        )],
    );
    let sampler = HostSampler {
        cgroup: Some(dir.clone()),
        last_window: None,
    };
    let sample = sampler.sample();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(
        sample.cgroup_memory_pressure,
        Some(MemoryPressure {
            some_avg10_centi: Some(45),
            some_total_usec: Some(987_654),
            full_avg10_centi: Some(120),
            full_total_usec: Some(12_345),
        })
    );
}

#[cfg(target_os = "linux")]
#[test]
fn live_linux_sample_reads_proc() {
    let sample = HostSampler::new().sample();
    assert!(sample.process_rss_bytes.is_some_and(|rss| rss > 0));
    let (total, steal) = (
        sample.cpu_total_ticks.unwrap(),
        sample.cpu_steal_ticks.unwrap(),
    );
    assert!(total > 0 && steal <= total);
}
