//! Background threads take `SCHED_IDLE`; the tick thread keeps its policy.
//!
//! These read the policy back from the kernel, so they hold in the agent lane
//! where the whole process inherits `nice -n 19` (niceness cannot separate the
//! tick from its workers there) as well as on a normally niceness-ed server.

use crate::thread_sched::current_policy;

/// `SCHED_IDLE` as the kernel reports it.
#[cfg(target_os = "linux")]
use crate::thread_sched::IDLE_POLICY;

/// The policy of the thread named `name` in this process, read from
/// `/proc/self/task/<tid>/stat` after matching its `comm`, waiting for the
/// thread to apply its policy (the `chrt` helper it spawns takes a few
/// milliseconds). Yields instead of sleeping; bounded so a broken thread fails
/// the assertion instead of hanging the suite.
#[cfg(target_os = "linux")]
fn policy_of_named_thread(name: &str) -> Option<i32> {
    let mut seen = None;
    for _ in 0..200_000 {
        if let Some(policy) = read_policy_of_named_thread(name) {
            if policy == IDLE_POLICY {
                return Some(policy);
            }
            seen = Some(policy);
        }
        std::thread::yield_now();
    }
    seen
}

#[cfg(target_os = "linux")]
fn read_policy_of_named_thread(name: &str) -> Option<i32> {
    // `comm` holds at most 15 characters (TASK_COMM_LEN 16 including NUL).
    let expected: String = name.chars().take(15).collect();
    let tasks = std::fs::read_dir("/proc/self/task").ok()?;
    for entry in tasks.flatten() {
        let comm = std::fs::read_to_string(entry.path().join("comm")).ok()?;
        if comm.trim() != expected {
            continue;
        }
        return crate::thread_sched::policy_of(
            std::fs::read_to_string(entry.path().join("stat")).ok()?,
        );
    }
    None
}

#[test]
#[cfg(target_os = "linux")]
fn an_idle_marked_thread_reads_back_as_sched_idle() {
    let before = current_policy().expect("linux reports a scheduling policy");
    assert_ne!(
        before, IDLE_POLICY,
        "the test process must not start on the idle policy"
    );
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("idle-probe".into())
        .spawn(move || {
            crate::thread_sched::mark_current_thread_idle();
            let _ = tx.send(current_policy());
        })
        .expect("spawn probe thread");
    assert_eq!(
        rx.recv().expect("probe reports its policy"),
        Some(IDLE_POLICY),
        "a marked background thread runs at SCHED_IDLE"
    );
    assert_eq!(
        current_policy(),
        Some(before),
        "marking a worker never changes the calling (tick) thread's policy"
    );
}

#[test]
#[cfg(target_os = "linux")]
fn the_input_log_writer_runs_at_idle_policy() {
    use crate::input_log::InputLog;
    let dir = std::env::temp_dir().join(format!(
        "mc-core-thread-sched-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    let log = InputLog::open(&dir, 0, Vec::new()).expect("open input log");
    assert_eq!(
        policy_of_named_thread("mc-core-input-log"),
        Some(IDLE_POLICY),
        "the spawned log writer took the idle policy"
    );
    assert_ne!(
        current_policy(),
        Some(IDLE_POLICY),
        "the caller keeps its own policy"
    );
    drop(log);
    let _ = std::fs::remove_dir_all(dir);
}
