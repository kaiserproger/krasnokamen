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
    assert!(sample.sampled_unix_ns > 0);
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
