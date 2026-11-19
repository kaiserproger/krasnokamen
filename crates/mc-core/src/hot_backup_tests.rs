use crate::boundary::{ChunkPos, PlayerInput, TickOutput};
use crate::command_console::ConsoleCommand;
use crate::config::CoreConfig;
use crate::core::Core;

fn copy_world(
    world: &std::path::Path,
    out: &std::path::Path,
    checkpoint: u64,
    through: u64,
) -> Result<crate::hot_backup::HotBackup, crate::config::CoreError> {
    let mut database = std::fs::File::open(world.join("native/state.redb")).unwrap();
    database.try_lock().unwrap();
    crate::hot_backup::copy_world(
        world,
        out,
        &mut database,
        checkpoint,
        through,
        identity([9; 16]),
    )
}

fn identity(store: [u8; 16]) -> crate::journal::WalIdentity {
    crate::journal::WalIdentity {
        store,
        palette: [0; 8],
    }
}

fn batch(dir: &std::path::Path, lo: u64, hi: u64) {
    std::fs::write(
        dir.join(format!("batch-{lo:020}-{hi:020}.bin")),
        crate::journal::frame_batch(lo, hi, b"", identity([9; 16])).unwrap(),
    )
    .unwrap();
}

fn fake_world() -> tempfile::TempDir {
    let world = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(world.path().join("native")).unwrap();
    std::fs::create_dir_all(world.path().join("journal")).unwrap();
    std::fs::write(world.path().join("native/state.redb"), b"database").unwrap();
    // The marker travels with the store; a store with a checkpoint always has one.
    mc_world::store_marker::write(world.path(), [9; 16], 10).unwrap();
    world
}

#[test]
fn copy_takes_database_and_only_the_wal_between_checkpoint_and_through() {
    let world = fake_world();
    let journal = world.path().join("journal");
    // Pruned/older, covered, covered, and appended after the requested tick.
    batch(&journal, 1, 10);
    batch(&journal, 11, 11);
    batch(&journal, 12, 12);
    batch(&journal, 13, 13);
    std::fs::write(
        journal.join("batch-00000000000000000014-00000000000000000014.bin.tmp"),
        b"x",
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let backup = copy_world(world.path(), out.path(), 10, 12).unwrap();
    assert_eq!((backup.checkpoint_tick, backup.wal_tick), (10, 12));
    let mut names: Vec<_> = backup.files.keys().map(String::as_str).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "checkpoint.marker",
            "journal/batch-00000000000000000011-00000000000000000011.bin",
            "journal/batch-00000000000000000012-00000000000000000012.bin",
            "native/state.redb",
        ]
    );
    assert_eq!(
        std::fs::read(out.path().join("native/state.redb")).unwrap(),
        b"database"
    );
    assert_eq!(
        mc_world::store_marker::read(out.path()).unwrap(),
        mc_world::store_marker::read(world.path()).unwrap(),
        "the marker travels with the store"
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        backup.files["native/state.redb"],
        format!("{:x}", Sha256::digest(b"database"))
    );
}

#[test]
fn copy_reads_the_owned_database_snapshot_not_a_second_path_handle() {
    let world = fake_world();
    let out = tempfile::tempdir().unwrap();
    let bytes = b"lease-holder snapshot";
    let backup = crate::hot_backup::copy_world(
        world.path(),
        out.path(),
        &mut bytes.as_slice(),
        10,
        10,
        identity([9; 16]),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(out.path().join("native/state.redb")).unwrap(),
        bytes
    );
    use sha2::{Digest, Sha256};
    assert_eq!(
        backup.files["native/state.redb"],
        format!("{:x}", Sha256::digest(bytes))
    );
}

/// Books R4-P1: a copy whose store is behind its own marker is a rolled-back
/// store; the backup is refused instead of producing a copy that restore and
/// startup would reject.
#[test]
fn copy_refuses_a_marker_ahead_of_the_committed_checkpoint() {
    let world = fake_world();
    let out = tempfile::tempdir().unwrap();
    let error = crate::hot_backup::copy_world(
        world.path(),
        out.path(),
        &mut b"database".as_slice(),
        9,
        9,
        identity([9; 16]),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("ahead of the committed checkpoint"),
        "{error}"
    );
}

#[test]
fn copy_refuses_a_marker_from_another_store() {
    let world = fake_world();
    let out = tempfile::tempdir().unwrap();
    let error = crate::hot_backup::copy_world(
        world.path(),
        out.path(),
        &mut b"database".as_slice(),
        10,
        10,
        identity([8; 16]),
    )
    .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("would pair two different worlds"),
        "{error}"
    );
}

#[test]
fn copy_refuses_a_wal_hole_and_a_straddling_batch() {
    let world = fake_world();
    let journal = world.path().join("journal");
    batch(&journal, 11, 11);
    batch(&journal, 13, 13);
    let out = tempfile::tempdir().unwrap();
    let error = copy_world(world.path(), out.path(), 10, 13).unwrap_err();
    assert!(error.to_string().contains("expected 12"), "{error}");

    let world = fake_world();
    batch(&world.path().join("journal"), 8, 12);
    let out = tempfile::tempdir().unwrap();
    let error = copy_world(world.path(), out.path(), 10, 12).unwrap_err();
    assert!(error.to_string().contains("expected 11"), "{error}");

    let world = fake_world();
    batch(&world.path().join("journal"), 11, 11);
    let out = tempfile::tempdir().unwrap();
    let error = copy_world(world.path(), out.path(), 10, 12).unwrap_err();
    assert!(error.to_string().contains("journal hole"), "{error}");
}

fn config(dir: &std::path::Path) -> CoreConfig {
    let mut config = CoreConfig::new(dir.to_path_buf(), 42);
    config.view_distance = 1;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    config
}

fn next_output(rx: &mut tokio::sync::broadcast::Receiver<TickOutput>, at_least: u64) -> TickOutput {
    loop {
        match rx.blocking_recv() {
            Ok(output) if output.tick >= at_least => return output,
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => panic!("tick thread died"),
        }
    }
}

fn next_resident(
    rx: &mut tokio::sync::broadcast::Receiver<TickOutput>,
    pos: ChunkPos,
) -> TickOutput {
    loop {
        let output = next_output(rx, 1);
        if output.snapshot.chunks.iter().any(|chunk| chunk.pos == pos) {
            return output;
        }
    }
}

fn block(output: &TickOutput, [x, y, z]: [i32; 3]) -> u32 {
    let pos = ChunkPos {
        x: x.div_euclid(16),
        z: z.div_euclid(16),
    };
    let chunk = output
        .snapshot
        .chunks
        .iter()
        .find(|chunk| chunk.pos == pos)
        .expect("spawn chunk resident");
    chunk
        .chunk
        .get_block(x.rem_euclid(16) as u8, y, z.rem_euclid(16) as u8)
        .expect("block in chunk")
        .0
}

fn set_block(
    core: &Core,
    rx: &mut tokio::sync::broadcast::Receiver<TickOutput>,
    at: [i32; 3],
    name: &str,
) -> TickOutput {
    let (command, completed) = ConsoleCommand::new(format!(
        "setblock {} {} {} minecraft:{name}",
        at[0], at[1], at[2]
    ));
    assert!(core.submit(PlayerInput::ConsoleCommand(command)));
    let tick = completed.blocking_recv().expect("command executed");
    next_output(rx, tick)
}

#[test]
fn backup_of_a_running_core_restores_the_state_at_its_tick() {
    let root = tempfile::tempdir().unwrap();
    let live = root.path().join("live");
    let copy = root.path().join("copy");
    let core = Core::open(config(&live)).unwrap();
    let mut rx = core.subscribe();
    let [sx, sy, sz] = core.world_spawn();
    let (before, after) = ([sx, sy + 8, sz], [sx + 1, sy + 8, sz]);
    // No player is online: ticket the chunk under test and wait for the
    // loader to publish it.
    let chunk = ChunkPos {
        x: sx.div_euclid(16),
        z: sz.div_euclid(16),
    };
    core.add_ticket(crate::core::RemoteTicket {
        owner: "hot-backup-test".into(),
        chunk,
        radius: 0,
        reason: "hot backup test".into(),
        budget_chunks: 1,
    })
    .unwrap();
    let initial = next_resident(&mut rx, chunk);
    let (before_original, after_original) = (block(&initial, before), block(&initial, after));

    let output = set_block(&core, &mut rx, before, "gold_block");
    let before_changed = block(&output, before);
    assert_ne!(before_changed, before_original);

    std::fs::create_dir(&copy).unwrap();
    let backup = core.backup(&copy).unwrap();
    assert!(backup.wal_tick >= backup.checkpoint_tick);
    assert!(backup.files.contains_key("native/state.redb"));

    // Changes after the backup must not appear in it.
    let output = set_block(&core, &mut rx, after, "diamond_block");
    assert_ne!(block(&output, after), after_original);
    core.shutdown();

    let restored = Core::open(config(&copy)).unwrap();
    let mut rx = restored.subscribe();
    let output = next_resident(&mut rx, chunk);
    assert!(output.tick >= backup.wal_tick);
    assert_eq!(block(&output, before), before_changed);
    assert_eq!(block(&output, after), after_original);
    restored.shutdown();
}

/// Books R5 G3: the copy must prove the SOURCE is intact, not only that the copy
/// matches it. A batch whose payload no longer matches its CRC fails the backup
/// and names the file, instead of being copied into a silently unrestorable
/// backup.
#[test]
fn copy_refuses_a_source_batch_that_fails_its_crc() {
    let world = fake_world();
    let journal = world.path().join("journal");
    batch(&journal, 11, 11);
    let damaged = journal.join("batch-00000000000000000012-00000000000000000012.bin");
    let mut frame = crate::journal::frame_batch(12, 12, b"{}\n", identity([9; 16])).unwrap();
    let last = frame.len() - 2;
    frame[last] ^= 0xff; // flip one payload byte; the frame's CRC stays the old one
    std::fs::write(&damaged, frame).unwrap();
    let out = tempfile::tempdir().unwrap();

    let error = copy_world(world.path(), out.path(), 10, 12).unwrap_err();

    assert!(
        error
            .to_string()
            .contains("batch-00000000000000000012-00000000000000000012"),
        "the refusal must name the damaged batch: {error}"
    );
    assert!(
        error.to_string().contains("CRC mismatch"),
        "the refusal must name the failed check: {error}"
    );
    assert!(
        !out.path()
            .join("journal/batch-00000000000000000012-00000000000000000012.bin")
            .exists(),
        "the damaged batch is never published into the backup"
    );
}


/// HOT-BACKUP-STATE-DATABASE-COPY-UNVERIFIED: a backup reports success only
/// after the finished copy restores. A copy whose database was damaged, and a
/// copy whose CRC-clean tail carries a record recovery refuses, each fail the
/// backup instead of being published as verified.
#[test]
fn backup_refuses_a_copy_that_does_not_restore() {
    for mode in [
        crate::hot_backup::test_hooks::DATABASE,
        crate::hot_backup::test_hooks::UNREPLAYABLE_BATCH,
    ] {
        let root = tempfile::tempdir().unwrap();
        let live = root.path().join("live");
        let copy = root.path().join("copy");
        let core = Core::open(config(&live)).unwrap();
        std::fs::create_dir(&copy).unwrap();
        crate::hot_backup::test_hooks::arm(mode);
        let error = core
            .backup(&copy)
            .expect_err("an unrestorable copy is not a backup");
        core.shutdown();
        assert!(
            error.to_string().contains("does not"),
            "the refusal names the failed step: {error}"
        );
    }
}
