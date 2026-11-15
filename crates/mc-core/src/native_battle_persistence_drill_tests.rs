//! Battle policy and native poses must share the actual tick-WAL/checkpoint fence.
use super::*;
use crate::battle::{
    formation::{Order, RegimentSpec},
    persistence,
};
use std::io::Write;

fn image(state: &TickState) -> serde_json::Value {
    serde_json::from_str(&persistence::encode(state).unwrap()).unwrap()
}

fn pose(state: &TickState) -> BTreeMap<i32, ([f64; 3], [f64; 3], f32)> {
    state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .filter_map(|member| {
            let slot = state.entities.slot_of(member.id)?;
            Some((
                member.id.0,
                (
                    state.entities.pos(slot),
                    state.entities.vel(slot),
                    state.entities.row(slot).health,
                ),
            ))
        })
        .collect()
}

fn hydrate_battle(state: &mut TickState) {
    let chunks = (-1..=1)
        .flat_map(|x| (-1..=1).map(move |z| (crate::boundary::ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&chunks);
}

fn prepared(path: &std::path::Path) -> Drill {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_test_writer()
        .try_init();
    let mut config = CoreConfig::new(path.to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let mut drill = Drill::open(config);
    let state = &mut drill.state;
    hydrate_battle(state);
    let stone = crate::crafter::tests::block(state, "stone", &[]);
    for x in 0..16 {
        for z in 0..16 {
            state.world.set_block(x, 199, z, stone);
        }
    }
    state.tickets.insert(
        "krasnokamen:battle".into(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 1),
    );
    state.refresh_active_chunks(state.tick);
    let sword = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_sword").unwrap())
        .unwrap();
    for side in 1..=2 {
        let spec = RegimentSpec {
            id: u32::from(side),
            faction: side,
            origin: [8.0, 200.0, 6.0 + f64::from(side - 1) * 1.2],
            facing: [0.0, if side == 1 { 1.0 } else { -1.0 }],
            files: 4,
            frontage: 1.0,
            order: Order::Advance,
        };
        let mut members = Vec::new();
        for index in 0..8 {
            let pos = crate::battle::formation::slot_position(&spec, index);
            let id = state.spawn_mob(
                state.registry.entity_type_id("minecraft:husk").unwrap(),
                pos,
                20.0,
            );
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_faction(slot, side);
            state.entities.set_ground(slot, true);
            let row = state.entities.row_mut(slot);
            row.persistent = true;
            Arc::make_mut(&mut row.appearance).equipment[0] =
                Some(mc_data::ItemStack::new(sword, 1));
            members.push(id);
        }
        state
            .hostile
            .battle
            .formation
            .add_regiment(spec, &members)
            .unwrap();
    }
    state
        .hostile
        .battle
        .formation
        .set_bounds([0.0, 0.0], [16.0, 16.0])
        .unwrap();
    state.hostile.battle.running = true;
    // Wait on a simulation event, not wall time. This also proves the fixture
    // was fighting BEFORE the crash rather than merely restoring a stage.
    let mut strikes = 0;
    for _ in 0..2000 {
        drill.run(&[]).unwrap();
        strikes += drill.state.hostile.battle.combat.work.strikes;
        if strikes > 0 {
            break;
        }
    }
    assert!(
        strikes > 0,
        "battle must reach an ordinary native strike before capture"
    );
    drill.checkpoint();
    drill
}

fn continues(drill: &mut Drill) {
    // Production's chunk loader precedes ticks; this tick-only drill must load
    // the restored battle ticket too, rather than let soldiers fall in a void.
    hydrate_battle(&mut drill.state);
    let elapsed = drill.state.hostile.battle.elapsed_ticks;
    let health = |state: &TickState| {
        state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .filter_map(|member| {
                state
                    .entities
                    .slot_of(member.id)
                    .map(|slot| state.entities.row(slot).health)
            })
            .sum::<f32>()
    };
    let before = health(&drill.state);
    let mut strikes = 0;
    let (mut wounds, mut kills, mut parries) = (0, 0, 0);
    // Cinematic melee intentionally removes a man on only ~0.8% of blows.
    // Wait on actual native damage, allowing this tiny frontage enough swings.
    for _ in 0..10_000 {
        drill.run(&[]).unwrap();
        strikes += drill.state.hostile.battle.combat.work.strikes;
        wounds += drill.state.hostile.battle.combat.work.wounded;
        kills += drill.state.hostile.battle.combat.work.killed;
        parries += drill.state.hostile.battle.combat.work.parries;
        if strikes > 0 && health(&drill.state) < before {
            break;
        }
    }
    assert!(drill.state.hostile.battle.running);
    assert!(drill.state.hostile.battle.elapsed_ticks > elapsed);
    assert!(
        strikes > 0,
        "recovered soldiers must land another native strike"
    );
    assert!(
        health(&drill.state) < before,
        "a resumed native blow must damage a soldier: health {before} -> {}, strikes {strikes}, wounds {wounds}, kills {kills}, parries {parries}",
        health(&drill.state)
    );
}

#[test]
fn battle_resume_wal_published_tick_excludes_inflight_decisions_and_motion() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    for _ in 0..5 {
        drill.run(&[]).unwrap();
    }
    let durable_tick = drill.state.tick;
    let expected = image(&drill.state);
    let expected_pose = pose(&drill.state);
    let unpublished = drill.capture_tick(&[]);
    assert_eq!(unpublished.tick, durable_tick + 1);
    assert_ne!(image(&drill.state), expected);
    drill = drill.crash();
    assert_eq!(drill.state.tick, durable_tick);
    assert_eq!(
        image(&drill.state),
        expected,
        "formation/order/targets/actions/morale share the published tick"
    );
    assert_eq!(
        pose(&drill.state),
        expected_pose,
        "decision history cannot be paired with older intra-chunk native poses"
    );
    continues(&mut drill);
    drill.checkpoint();
    let expected = image(&drill.state);
    drill = drill.crash();
    assert_eq!(
        image(&drill.state),
        expected,
        "fresh checkpoint survives normal WAL pruning"
    );
}

#[test]
fn battle_resume_abrupt_checkpoint_commit_recovers_whole_history() {
    const CHILD: &str = "KRASNOKAMEN_BATTLE_CHECKPOINT_DRILL_PATH";
    const TEST: &str = "core::native_checkpoint_crash_tests::persistence_drills::battle_resume::battle_resume_abrupt_checkpoint_commit_recovers_whole_history";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = std::path::PathBuf::from(path);
        let stop = std::env::var("KRASNOKAMEN_BATTLE_CHECKPOINT_DRILL_STAGE").unwrap();
        let mut config = CoreConfig::new(path.clone(), 42);
        config.view_distance = 0;
        config.simulation_distance = 0;
        let mut drill = Drill::open(config);
        hydrate_battle(&mut drill.state);
        for _ in 0..5 {
            drill.run(&[]).unwrap();
        }
        std::fs::write(
            path.join("battle-expected.json"),
            serde_json::to_vec(&image(&drill.state)).unwrap(),
        )
        .unwrap();
        let (job, _ack, _reply) = drill.capture_checkpoint();
        CHECKPOINT_STAGE_OBSERVERS.lock().push((
            path.clone(),
            Box::new(move |stage| {
                if stage == stop {
                    let mut file =
                        std::fs::File::create(path.join("battle-checkpoint-stage")).unwrap();
                    writeln!(file, "{stage}").unwrap();
                    file.sync_all().unwrap();
                    std::process::exit(39);
                }
            }),
        ));
        drill.dispatch_checkpoint(job, None, None);
        panic!("requested mid-checkpoint stop was not reached");
    }
    for stage in ["before_native_commit", "native_committed"] {
        let directory = tempfile::tempdir().unwrap();
        let drill = prepared(directory.path());
        let config = drill.config.clone();
        drop(drill);
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env(CHILD, directory.path())
            .env("KRASNOKAMEN_BATTLE_CHECKPOINT_DRILL_STAGE", stage)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(39), "actual abrupt stop at {stage}");
        assert_eq!(
            std::fs::read_to_string(directory.path().join("battle-checkpoint-stage"))
                .unwrap()
                .trim(),
            stage
        );
        let expected: serde_json::Value = serde_json::from_slice(
            &std::fs::read(directory.path().join("battle-expected.json")).unwrap(),
        )
        .unwrap();
        // Read-only offline tooling deliberately refuses a dirty redb header.
        // Normal writable admission repairs it without advancing gameplay/WAL.
        drop(fixture_store(&config).unwrap());
        let offline = crate::native_checkpoint::recover_offline(&config).unwrap();
        assert!(
            offline.dimensions[&crate::dimensions::Dimension::Overworld]
                .battle
                .is_some()
        );
        drop(offline);
        let mut recovered = Drill::open(config);
        assert_eq!(
            image(&recovered.state),
            expected,
            "{stage}: complete battle recovered online after offline replay"
        );
        continues(&mut recovered);
    }
}

#[test]
fn battle_resume_encoding_failure_halts_before_wal_or_publication() {
    use crate::battle::persistence::{ENCODING_FAULT, EncodingFault};
    use std::time::Duration;
    struct Disarm;
    impl Drop for Disarm {
        fn drop(&mut self) {
            *ENCODING_FAULT.lock() = None;
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let drill = prepared(directory.path());
    let mut config = drill.config.clone();
    config.checkpoint_every_ticks = 100_000;
    let checkpoint_tick = drill.manifest.tick;
    drop(drill);
    let core = crate::core::Core::open(config.clone()).unwrap();
    let store = fixture_store(&config).unwrap();
    let mut ticks = core.subscribe();
    assert!(core.submit(PlayerInput::Join {
        connection: ConnectionId(9100),
        uuid: uuid::Uuid::new_v4(),
        name: "EncodeFault".into(),
        permission_level: 0,
    }));
    for _ in 0..5 {
        ticks.blocking_recv().unwrap();
    }
    let (closed_tx, closed_rx) = mpsc::channel();
    let listener = std::thread::spawn(move || {
        let mut published = Vec::new();
        loop {
            match ticks.blocking_recv() {
                Ok(output) => published.push(output.tick),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
        let _ = closed_tx.send(published);
    });
    let (observed, failure) = mpsc::channel();
    let _disarm = Disarm;
    *ENCODING_FAULT.lock() = Some(EncodingFault {
        store: Arc::downgrade(&store),
        observed,
    });
    let failed_tick = failure.recv_timeout(Duration::from_secs(60));
    let published = closed_rx.recv_timeout(Duration::from_secs(60));
    let disconnected = core
        .try_drain_outbox()
        .into_iter()
        .any(|(connection, changes)| {
            connection == ConnectionId(9100)
                && changes.iter().any(|change| {
                    matches!(
                        change,
                        crate::boundary::PlayerChange::Disconnect { reason }
                            if reason.contains("journal capture failed")
                    )
                })
        });
    core.shutdown();
    listener.join().unwrap();
    let failed_tick = failed_tick.expect("the selected world's image encoding failed");
    let published = published.expect("encoding failure closes tick publication");
    assert!(
        published.iter().all(|&tick| tick < failed_tick),
        "failed tick {failed_tick} reached publication: {published:?}"
    );
    assert!(
        disconnected,
        "authority halt reliably disconnects the live player"
    );
    assert!(
        read_complete_batches_after(&config.world_dir.join("journal"), failed_tick - 1)
            .unwrap()
            .is_empty(),
        "failed image tick cannot reach the durable WAL"
    );
    assert_eq!(
        read_manifest(&config).unwrap().tick,
        checkpoint_tick,
        "halt must not checkpoint the unjournaled state"
    );
}

#[test]
fn battle_resume_sigkill_during_dying_body_keeps_tombstone_authority() {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    const CHILD: &str = "KRASNOKAMEN_BATTLE_DYING_DRILL_PATH";
    const TEST: &str = "core::native_checkpoint_crash_tests::persistence_drills::battle_resume::battle_resume_sigkill_during_dying_body_keeps_tombstone_authority";
    const READY: &str = "BATTLE_DYING_WAL_DURABLE ";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = std::path::PathBuf::from(path);
        let mut drill = prepared(&path);
        let victim = drill.state.hostile.battle.formation.members[0].id;
        let slot = drill.state.entities.slot_of(victim).unwrap();
        drill.state.entities.set_vel(slot, [2.0, 0.2, 0.0]);
        drill.state.merge_damage.push((
            slot,
            crate::tick::DamagePiece {
                amount: 1000.0,
                kind: crate::survival::DamageKind::GenericKill,
                source: None,
                physical: None,
                custom_armor: false,
            },
        ));
        drill.state.commit_merge_effects();
        drill.run(&[]).unwrap();
        let death_position = drill.state.entities.pos(slot);
        assert!(!drill.state.entities.body(slot).alive);
        assert!(drill.state.entities.slot_of_any(victim).is_some());
        // A corpse can still receive its final native pose write after its
        // durable death image. Exercise that same-chunk write explicitly.
        drill.state.entities.set_pos(
            slot,
            [
                death_position[0] + 0.125,
                death_position[1],
                death_position[2],
            ],
        );
        let corpse_tick = drill.capture_tick(&[]);
        assert!(
            drill.state.entities.slot_of_any(victim).is_some(),
            "the physical corpse remains during its normal 20-tick animation"
        );
        assert_ne!(
            drill.state.entities.pos(slot),
            death_position,
            "ordinary corpse motion must exercise the post-tombstone pose window"
        );
        drill.append(&corpse_tick).unwrap();
        let mut expected = std::fs::File::create(path.join("dying-expected.json")).unwrap();
        expected
            .write_all(
                serde_json::to_string(&image(&drill.state))
                    .unwrap()
                    .as_bytes(),
            )
            .unwrap();
        expected.sync_all().unwrap();
        let receipt = serde_json::json!({"tick": drill.state.tick, "victim": victim.0});
        println!("{READY}{receipt}");
        std::io::stdout().flush().unwrap();
        let mut release = String::new();
        std::io::stdin().read_line(&mut release).unwrap();
        panic!("parent must SIGKILL the process before shutdown/checkpoint");
    }
    let directory = tempfile::tempdir().unwrap();
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, directory.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (ready_tx, ready_rx) = mpsc::channel();
    let observer = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let line = line.unwrap();
            if let Some(receipt) = line.strip_prefix(READY) {
                let _ = ready_tx.send(serde_json::from_str::<serde_json::Value>(receipt).unwrap());
                return;
            }
        }
    });
    let receipt = ready_rx.recv_timeout(std::time::Duration::from_secs(60));
    let _ = child.kill();
    let stopped = child.wait().unwrap();
    observer.join().unwrap();
    let receipt = receipt.expect("child reached durable death/corpse tick before SIGKILL");
    assert_eq!(
        stopped.signal(),
        Some(9),
        "actual SIGKILL, not graceful exit"
    );
    eprintln!("death-window SIGKILL: {receipt}");
    let expected: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("dying-expected.json")).unwrap(),
    )
    .unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let mut recovered = Drill::open(config);
    assert_eq!(recovered.state.tick, receipt["tick"].as_u64().unwrap());
    assert_eq!(image(&recovered.state), expected);
    let victim = EntityId(receipt["victim"].as_i64().unwrap() as i32);
    assert!(recovered.state.entities.slot_of_any(victim).is_none());
    assert!(
        recovered
            .state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .any(|member| { member.id == victim && !member.alive })
    );
    continues(&mut recovered);
}
