use super::*;
use std::io::Read;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Set in the child process this module's gate spawns.
const CHILD_ENV: &str = "CORESERVECLOSE_PARKED_CHILD";
/// The child's completion marker: printed only after the shared shutdown
/// returned with the parked task still inside its synchronous section.
const CHILD_DONE: &str = "parked-task child finished";

/// The scenario, run in a child process: a task parked in a synchronous section
/// (the fanout pool's `install`, a held lock) must not keep the process alive
/// once the halt error returns. In the parent this would hang the test binary
/// itself when the shutdown is broken, so the parent bounds the child and
/// reports the failure (see `a_parked_task_does_not_keep_the_shutdown_alive`).
#[test]
fn parked_task_child() {
    if std::env::var_os(CHILD_ENV).is_none() {
        return;
    }
    let parked = Arc::new((Mutex::new(false), Condvar::new()));
    let worker = Arc::clone(&parked);
    let result = run_on_edge_runtime(async move {
        let (entered_tx, entered) = tokio::sync::oneshot::channel();
        tokio::task::spawn_blocking(move || {
            let _ = entered_tx.send(());
            // A synchronous section: no abort and no timer reaches it.
            let (lock, wake) = &*worker;
            let mut released = lock.lock().unwrap();
            while !*released {
                released = wake.wait(released).unwrap();
            }
        });
        entered
            .await
            .expect("the task entered its synchronous section");
        Err::<(), _>(anyhow::anyhow!("core halted: wedged pump"))
    });
    assert!(result.is_err(), "the halt error is returned");
    // Release the parked task: the shutdown must not have waited for it.
    let (lock, wake) = &*parked;
    *lock.lock().unwrap() = true;
    wake.notify_all();
    println!("{CHILD_DONE}");
}

/// A halt error returns while a task is parked in a section no abort reaches;
/// a shutdown that joined it would keep the process (here: the child) alive.
#[test]
fn a_parked_task_does_not_keep_the_shutdown_alive() {
    let exe = std::env::current_exe().expect("test binary path");
    let mut child = std::process::Command::new(exe)
        .args([
            "--exact",
            "edge_runtime::tests::parked_task_child",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn the parked-task child");
    let mut stdout = child.stdout.take().expect("child stdout");
    let (done_tx, done) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stdout.read_to_string(&mut text);
        let _ = done_tx.send(text);
    });
    match done.recv_timeout(Duration::from_secs(60)) {
        Ok(text) => {
            let status = child.wait().expect("reap the parked-task child");
            assert!(status.success(), "the parked-task child failed: {status}");
            assert!(
                text.contains(CHILD_DONE),
                "the child did not reach the shutdown: {text}"
            );
        }
        Err(_) => {
            // The shutdown joined the parked task: kill and reap it, then fail.
            let _ = child.kill();
            let _ = child.wait();
            panic!("the edge runtime shutdown waited for a parked task");
        }
    }
    let _ = reader.join();
}
