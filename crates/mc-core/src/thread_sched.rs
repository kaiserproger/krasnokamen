//! Priority separation for background threads (Linux).
//!
//! The tick thread's 50 ms deadline competes with background work in the same
//! process: chunk generation for a moving player, a datapack reload, a
//! `/locate` scan, the input log. Niceness cannot express that separation in
//! every lane: raising the tick needs `CAP_SYS_NICE`, and a server started
//! through the agent runner already inherits `nice -n 19`, where no offset is
//! left but down. `SCHED_IDLE` is the lever an unprivileged process has there:
//! such a thread runs only when no other thread on its CPU wants it, so the
//! tick wins whenever both are runnable.
//!
//! Marked threads are pure background workers. Nothing the tick waits for may
//! take this policy — the core's rayon pool runs the tick's own phases, and
//! the journal and native checkpoint writers order persistence.

/// `SCHED_IDLE` from the kernel's `sched.h`: a thread that runs only on
/// otherwise-idle CPU.
#[cfg(all(test, target_os = "linux"))]
pub(crate) const IDLE_POLICY: i32 = 5;

/// Put the calling thread on `SCHED_IDLE`; a failure is logged once and never
/// aborts startup (the thread simply keeps the inherited policy).
///
/// The workspace forbids `unsafe`, so the `sched_setscheduler` call goes
/// through util-linux's `chrt`: an unprivileged process may lower the policy of
/// its own threads, and `chrt -i -p 0 <tid>` performs exactly that syscall on
/// the thread whose kernel task id we pass.
#[cfg(target_os = "linux")]
pub(crate) fn mark_current_thread_idle() {
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};

    if !*BACKGROUND_IDLE_ENABLED {
        return;
    }
    static LOGGED: AtomicBool = AtomicBool::new(false);
    let failure = match current_thread_id() {
        Some(tid) => match Command::new("chrt")
            .args(["-i", "-p", "0", &tid])
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .output()
        {
            Ok(output) if output.status.success() => return,
            Ok(output) => format!(
                "chrt {tid} exited {}: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            Err(error) => format!("chrt unavailable: {error}"),
        },
        None => "no thread id from /proc/thread-self".to_owned(),
    };
    if !LOGGED.swap(true, Ordering::Relaxed) {
        tracing::warn!(
            %failure,
            "SCHED_IDLE refused; background thread keeps its inherited policy"
        );
    }
}

/// No-op where the kernel has no `SCHED_IDLE`.
#[cfg(not(target_os = "linux"))]
pub(crate) fn mark_current_thread_idle() {}

/// `KRASNOKAMEN_BACKGROUND_IDLE=0` (or `off`/`false`) keeps background threads
/// on the inherited policy. Read once; the A/B receipt `G6-AB.md` alternates
/// the two arms on one binary.
#[cfg(target_os = "linux")]
static BACKGROUND_IDLE_ENABLED: std::sync::LazyLock<bool> = std::sync::LazyLock::new(|| {
    !matches!(
        std::env::var("KRASNOKAMEN_BACKGROUND_IDLE").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
});

/// The calling thread's kernel task id: `/proc/thread-self` links to
/// `<pid>/task/<tid>`.
#[cfg(target_os = "linux")]
fn current_thread_id() -> Option<String> {
    let target = std::fs::read_link("/proc/thread-self").ok()?;
    target
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
}

/// The calling thread's scheduling policy, for the regression test that reads
/// the policy back (`/proc/thread-self/stat` field 41).
#[cfg(test)]
pub(crate) fn current_policy() -> Option<i32> {
    #[cfg(target_os = "linux")]
    {
        policy_of(std::fs::read_to_string("/proc/thread-self/stat").ok()?)
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Field 41 of a `/proc/.../stat` line: the scheduling policy. Fields after the
/// parenthesized command name start at field 3, so the policy is index 38.
#[cfg(all(test, target_os = "linux"))]
pub(crate) fn policy_of(stat: String) -> Option<i32> {
    let tail = stat.rsplit_once(')')?.1;
    tail.split_whitespace().nth(38)?.parse().ok()
}
