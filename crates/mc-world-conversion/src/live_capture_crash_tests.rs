//! Linux crash-tail boundary. inotify wakes the publication observer; after the
//! child exits, the native WAL decoder verifies its stable contiguous history.
use super::*;
use std::process::Stdio;

const CHILD: &str = "recovery_roundtrip_tests::live_capture::crash::crash_child";
const CRASH_EXIT: i32 = 73;

async fn published_through(root: &Path, target: u64) {
    use rustix::fs::inotify;
    let journal = root.join("journal");
    let fd = inotify::init(inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC).unwrap();
    inotify::add_watch(&fd, &journal, inotify::WatchFlags::MOVED_TO).unwrap();
    let fd = tokio::io::unix::AsyncFd::new(fd).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(120), async {
        loop {
            // Directory enumeration is not a snapshot while Core publishes new
            // files. Wait for a candidate covering the action, then verify the
            // complete history in the parent after the writer has exited.
            let published = std::fs::read_dir(&journal).unwrap().any(|entry| {
                let entry = entry.unwrap();
                let name = entry.file_name();
                let name = name.to_string_lossy();
                name.strip_prefix("batch-")
                    .and_then(|name| name.strip_suffix(".bin"))
                    .and_then(|range| range.split_once('-'))
                    .and_then(|(_, hi)| hi.parse::<u64>().ok())
                    .is_some_and(|hi| hi >= target)
            });
            if published {
                return;
            }
            // Install the watch before checking files, so a publication between
            // the check and readiness wait cannot be missed. Names and event
            // counts are only wake conditions, never final state evidence.
            let mut ready = fd.readable().await.unwrap();
            let mut events = [0u8; 4096];
            match ready.try_io(|fd| {
                rustix::io::read(fd.get_ref(), &mut events).map_err(std::io::Error::from)
            }) {
                Ok(result) => {
                    result.expect("read WAL publication notification");
                }
                Err(_) => continue,
            }
        }
    })
    .await
    .expect("WAL publication through the observed domain-action tick");
}

#[tokio::test]
#[ignore = "invoked only by the Linux crash-tail parent tests"]
async fn crash_child() {
    let root = std::path::PathBuf::from(
        std::env::var_os("KRASNOKAMEN_CAPTURE_CHILD_ROOT").expect("child root"),
    );
    let domain: Domain = serde_json::from_str(
        &std::env::var("KRASNOKAMEN_CAPTURE_CHILD_DOMAIN").expect("child domain"),
    )
    .unwrap();
    let (_live, evidence) = scenario(&root, domain).await;
    published_through(&root, evidence.through_tick).await;
    let mut marker =
        std::fs::File::create(root.parent().unwrap().join("crash-evidence.json")).unwrap();
    serde_json::to_writer(&mut marker, &evidence).unwrap();
    marker.sync_all().unwrap();
    // Intentionally bypass Core/Running destructors: no shutdown checkpoint,
    // no test-side JournalWriter append, and no offline replacement of state.
    std::process::exit(CRASH_EXIT);
}

async fn crash_roundtrip(domain: Domain) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("source");
    let log_path = temporary.path().join("child.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--ignored",
            "--exact",
            CHILD,
            "--nocapture",
            "--test-threads=1",
        ])
        .env("KRASNOKAMEN_CAPTURE_CHILD_ROOT", &root)
        .env(
            "KRASNOKAMEN_CAPTURE_CHILD_DOMAIN",
            serde_json::to_string(&domain).unwrap(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let status = match tokio::time::timeout(std::time::Duration::from_secs(600), child.wait()).await
    {
        Ok(status) => status.unwrap(),
        Err(_) => {
            child
                .kill()
                .await
                .expect("kill and reap overdue capture child");
            panic!(
                "capture child deadline: {}",
                std::fs::read_to_string(&log_path).unwrap()
            );
        }
    };
    assert_eq!(
        status.code(),
        Some(CRASH_EXIT),
        "child must reach the observed tail-publication boundary: {}",
        std::fs::read_to_string(&log_path).unwrap()
    );
    let evidence: Evidence = serde_json::from_reader(
        std::fs::File::open(temporary.path().join("crash-evidence.json")).unwrap(),
    )
    .unwrap();
    let mut records = 0;
    let through = mc_core::journal::visit_complete_batches_after(
        &root.join("journal"),
        evidence.checkpoint_tick,
        |(_, _, entries)| {
            records += entries.len();
            Ok(())
        },
    )
    .expect("stable crash WAL must decode contiguously with valid payloads");
    assert!(through >= evidence.through_tick && records > 0);
    // redb deliberately refuses read-only admission after an abrupt exit.
    // Open/close its real native owner to repair database bookkeeping only:
    // no Core restart, checkpoint, WAL rewrite or test-side state replacement.
    let store = crate::test_support::fixture_store(&config(&root)).unwrap();
    assert_eq!(
        crate::test_support::read_manifest(&store).unwrap().tick,
        evidence.checkpoint_tick
    );
    drop(store);
    assert_export(&root, &evidence, true);
}

#[tokio::test]
async fn wandering_trader_checkpoint_and_wal_tail_survive_anvil_roundtrip() {
    crash_roundtrip(Domain::Trader).await;
}
#[tokio::test]
async fn lazy_warning_decay_exports_the_replayed_clock_without_native_anchors() {
    crash_roundtrip(Domain::Warning).await;
}
#[tokio::test]
async fn villager_wal_claim_exports_native_profession_and_unique_job_memory() {
    crash_roundtrip(Domain::Claim).await;
}
#[tokio::test]
async fn inhabited_time_checkpoint_tail_and_anvil_export_share_wal_tick() {
    crash_roundtrip(Domain::Inhabited).await;
}
