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
fn battle_resume_first_tick_reset_survives_wal_only_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path()).crash();
    hydrate_battle(&mut drill.state);
    let checkpoint_tick = drill.manifest.tick;
    let (reset, _ack) = crate::command_console::ConsoleCommand::new("battle reset".into());
    drill.run(&[PlayerInput::ConsoleCommand(reset)]).unwrap();
    assert!(drill.state.hostile.battle.formation.members.is_empty());
    assert!(!drill.state.hostile.battle.running);
    let cleared = image(&drill.state);
    assert!(drill.state.tick > checkpoint_tick);
    assert_eq!(read_manifest(&drill.config).unwrap().tick, checkpoint_tick);

    drill = drill.crash();
    assert_eq!(
        image(&drill.state),
        cleared,
        "the first command after recovery must durably clear the old battle"
    );
    drill = drill.crash();
    assert_eq!(
        image(&drill.state),
        cleared,
        "replaying the reset twice must not resurrect battle history"
    );
}

/// D36 3.4a: the facing state is runtime-only, so the contract is the same
/// result from the same start state. Checkpoint plus WAL replay in the middle
/// of the fight restores yaw and pitch as journaled (the head is not persisted
/// and comes back on the yaw), the projection starts again from them with every
/// head inside its cone, and the resumed soldiers land new strikes.
#[test]
fn battle_resume_rebuilds_facing_from_the_journaled_pose() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let rotations = |state: &TickState| -> BTreeMap<i32, [f32; 3]> {
        state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .filter_map(|member| {
                let slot = state.entities.slot_of(member.id)?;
                Some((member.id.0, state.entities.rot(slot)))
            })
            .collect()
    };
    for _ in 0..5 {
        drill.run(&[]).unwrap();
    }
    let before = rotations(&drill.state);
    drill = drill.crash();
    let after = rotations(&drill.state);
    assert_eq!(before.len(), after.len());
    for (id, [yaw, pitch, head]) in &after {
        let [old_yaw, old_pitch, _] = before[id];
        assert_eq!(
            [yaw.to_bits(), pitch.to_bits()],
            [old_yaw.to_bits(), old_pitch.to_bits()],
            "{id}: yaw and pitch are journaled"
        );
        assert_eq!(
            head.to_bits(),
            yaw.to_bits(),
            "{id}: the head restarts on the yaw"
        );
    }
    // The cone below is around the drawn body, which is the yaw under A.
    drill.state.hostile.battle.facing.policy = crate::battle::facing::Policy::Body;
    hydrate_battle(&mut drill.state);
    for _ in 0..40 {
        drill.run(&[]).unwrap();
        for (id, [yaw, _, head]) in rotations(&drill.state) {
            let off = (head - yaw + 180.0).rem_euclid(360.0) - 180.0;
            assert!(
                off.abs() <= 75.0 + 1.0e-3,
                "{id}: head {head} off its yaw {yaw}"
            );
        }
    }
    continues(&mut drill);
}

/// D36 section 5: a fighter whose charge bonus lasts keeps the previous
/// whole-body look, so the bonus knockback of its blow (along its stored yaw,
/// tick/damage.rs) pushes the victim along the bearing it strikes at, as before
/// D36; also after a crash recovered from checkpoint and WAL (the charge is
/// journaled, the facing state rebuilt). Each measured windup starts with the
/// striker's yaw and head turned 60 degrees off its victim and the facing state
/// dropped, as after a restore: the split law would rebuild that body where it
/// stands and hold it past the 10-tick windup, and the push would follow it.
/// This melee lands about one blow in forty (cinematic removal odds), so at
/// the impact tick the blow the strike builds (striker, position, stored yaw,
/// charge bonus; battle/combat.rs `strike`) is merged natively instead of
/// waiting for a landed roll.
#[test]
fn a_charged_blow_pushes_its_victim_along_the_strike_bearing_across_wal_recovery() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let charged = charge_side(&mut drill);
    let before = charged_push(&mut drill, charged);
    assert!(
        before < 15.0,
        "push {before} degrees off the strike bearing"
    );
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let charged = charge_side(&mut drill);
    drill.run(&[]).unwrap();
    let mut drill = drill.crash();
    hydrate_battle(&mut drill.state);
    let after = charged_push(&mut drill, charged);
    assert!(
        after < 15.0,
        "after recovery: push {after} degrees off the strike bearing"
    );
}

/// Side 1's soldiers charged from the next battle tick, which is returned.
fn charge_side(drill: &mut Drill) -> u64 {
    let battle = &mut drill.state.hostile.battle;
    let tick = battle.elapsed_ticks;
    let side: Vec<_> = battle
        .formation
        .members
        .iter()
        .filter(|member| battle.formation.regiments[member.regiment].spec.faction == 1)
        .map(|member| member.id)
        .collect();
    for id in side {
        battle.combat.charge_from(id, tick);
    }
    tick
}

/// The angle in degrees between a victim's push and the bearing from its
/// striker: the first windup of a standing charged soldier of side 1 on his
/// column target, turned 60 degrees off at its start, struck at its impact
/// tick with the blow `strike` builds.
fn charged_push(drill: &mut Drill, charged: u64) -> f32 {
    let bearing = |from: [f64; 3], to: [f64; 3]| {
        -(mc_physics::mth::atan2(to[0] - from[0], to[2] - from[2]) as f32)
            * mc_physics::mth::RAD_TO_DEG
    };
    let wrap = |angle: f32| (angle + 180.0).rem_euclid(360.0) - 180.0;
    let windup = u64::from(crate::battle::combat::Weapon::Sword.windup());
    while drill.state.hostile.battle.elapsed_ticks + windup + 2
        < charged + crate::battle::cavalry::CHARGE_DECAY_TICKS
    {
        drill.run(&[]).unwrap();
        let state = &drill.state;
        let battle = &state.hostile.battle;
        let now = battle.elapsed_ticks - 1;
        let Some((actor, victim, start)) = battle.formation.members.iter().find_map(|member| {
            let (target, start) = battle.combat.windup_of(member.id)?;
            let slot = state.entities.slot_of(member.id)?;
            let bi = battle.columns.index(slot)?;
            (start == now
                && battle.formation.regiments[member.regiment].spec.faction == 1
                && battle.columns.drive[bi] == [0.0; 3]
                && battle.columns.targets[bi].map(|column| column.id) == Some(target))
            .then_some((member.id, target, start))
        }) else {
            continue;
        };
        let slot = state.entities.slot_of(actor).unwrap();
        let Some(victim_slot) = state.entities.slot_of(victim) else {
            continue;
        };
        let toward = bearing(state.entities.pos(slot), state.entities.pos(victim_slot));
        let [_, pitch, _] = state.entities.rot(slot);
        drill
            .state
            .entities
            .set_facing(slot, [toward + 60.0, pitch, toward + 60.0]);
        drill.state.hostile.battle.facing = Default::default();
        // Up to the impact tick: its battle step strikes with the yaw stored now.
        let impact = start + windup;
        while drill.state.hostile.battle.elapsed_ticks < impact
            && drill.state.hostile.battle.combat.windup_of(actor) == Some((victim, start))
        {
            drill.run(&[]).unwrap();
        }
        let state = &mut drill.state;
        if state.hostile.battle.combat.windup_of(actor) != Some((victim, start))
            || !state.entities.body(victim_slot).alive
        {
            continue;
        }
        let position = state.entities.pos(slot);
        let struck_at = bearing(position, state.entities.pos(victim_slot));
        let hurt = state.entities.body(victim_slot).hurt_at;
        state.entities.set_vel(victim_slot, [0.0; 3]);
        state.merge_damage.push((
            victim_slot,
            crate::tick::DamagePiece {
                amount: 1.0,
                kind: crate::survival::DamageKind::Melee,
                source: Some(crate::intent::DamageSource::Melee {
                    attacker: actor,
                    position,
                    yaw: state.entities.rot(slot)[0],
                    bonus_knockback: f64::from(crate::battle::cavalry::impact_bonus(
                        impact - charged,
                    )),
                    sweep: false,
                    fire_ticks: 0,
                    bane_ticks: 0,
                }),
                physical: None,
                custom_armor: false,
            },
        ));
        state.commit_merge_effects();
        if state.entities.body(victim_slot).hurt_at == hurt {
            // Inside its hurt window: no knockback to read.
            continue;
        }
        let [x, _, z] = state.entities.vel(victim_slot);
        assert!(x * x + z * z > 1.0e-6, "the blow pushes");
        let pushed = -(mc_physics::mth::atan2(x, z) as f32) * mc_physics::mth::RAD_TO_DEG;
        return wrap(pushed - struck_at).abs();
    }
    panic!("no standing charged windup within the charge bonus");
}

/// Serialized bytes of a tick's battle records (full image or patch), by the
/// WAL's own JSON, so the measurement does not depend on the record variant.
fn battle_record_bytes(entries: &[JournalEntry]) -> usize {
    entries
        .iter()
        .map(|entry| match entry {
            JournalEntry::Dimension { entries, .. } => battle_record_bytes(entries),
            entry => {
                let json = serde_json::to_string(entry).unwrap();
                if json.starts_with("{\"Battle") {
                    json.len()
                } else {
                    0
                }
            }
        })
        .sum()
}

fn first_battle_record(entries: &[JournalEntry]) -> Option<String> {
    entries.iter().find_map(|entry| match entry {
        JournalEntry::Dimension { entries, .. } => first_battle_record(entries),
        entry => {
            Some(serde_json::to_string(entry).unwrap()).filter(|json| json.starts_with("{\"Battle"))
        }
    })
}

/// WAL bloat regression (CoreWalBloat, 2026-11-17): a running battle once
/// journaled its whole image every tick. Through the production journal and
/// WAL writer, only the first battle record after a checkpoint capture or a
/// recovery is the full image; later ticks journal a small fraction of it,
/// and crash recovery reproduces the live image over patches on both a
/// checkpoint and a recovered baseline.
#[test]
fn battle_wal_journals_changed_parts_not_the_whole_image_each_tick() {
    const TICKS: usize = 40;
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let full = persistence::encode(&drill.state).unwrap().len();
    for round in ["checkpoint", "recovered"] {
        let mut bytes = Vec::new();
        for step in 0..TICKS {
            let batch = drill.capture_tick(&[]);
            let record = first_battle_record(&batch.entries).expect("a running battle changes");
            assert_eq!(
                record.starts_with("{\"Battle\":"),
                step == 0,
                "{round}: only the first battle record of the tail is the full image (tick {})",
                batch.tick
            );
            bytes.push(battle_record_bytes(&batch.entries));
            drill.append(&batch).unwrap();
        }
        let steady = bytes[1..].iter().sum::<usize>() / (TICKS - 1);
        eprintln!(
            "{round}: full image {full} B, first record {} B, steady {steady} B/tick",
            bytes[0]
        );
        assert!(
            steady * 4 <= full,
            "{round}: a steady tick must journal a small fraction of the {full} B image, not {steady} B"
        );
        let expected = image(&drill.state);
        let expected_pose = pose(&drill.state);
        drill = drill.crash();
        assert_eq!(
            image(&drill.state),
            expected,
            "{round}: recovery folds the patches"
        );
        assert_eq!(
            pose(&drill.state),
            expected_pose,
            "{round}: poses share the tick"
        );
        hydrate_battle(&mut drill.state);
    }
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

/// Kill the real tick/WAL owner at the disputed capture boundary, before any
/// following battle pass can drain transient observations or staging decisions.
fn capture_boundary_sigkill(test: &str, scenario: fn(&mut Drill)) {
    use std::io::{BufRead, BufReader};
    use std::os::unix::process::ExitStatusExt;
    const CHILD: &str = "KRASNOKAMEN_BATTLE_CAPTURE_DRILL_PATH";
    const READY: &str = "BATTLE_CAPTURE_WAL_DURABLE ";
    if let Some(path) = std::env::var_os(CHILD) {
        let path = std::path::PathBuf::from(path);
        let mut drill = prepared(&path);
        scenario(&mut drill);
        let expected = serde_json::json!({
            "image": image(&drill.state),
            "pose": pose(&drill.state),
        });
        let mut file = std::fs::File::create(path.join("capture-expected.json")).unwrap();
        file.write_all(&serde_json::to_vec(&expected).unwrap())
            .unwrap();
        file.sync_all().unwrap();
        let receipt = serde_json::json!({"case": test, "tick": drill.state.tick});
        println!("{READY}{receipt}");
        std::io::stdout().flush().unwrap();
        let mut release = String::new();
        std::io::stdin().read_line(&mut release).unwrap();
        panic!("parent must SIGKILL before another tick or graceful checkpoint");
    }
    let directory = tempfile::tempdir().unwrap();
    let exact =
        format!("core::native_checkpoint_crash_tests::persistence_drills::battle_resume::{test}");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &exact, "--nocapture"])
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
            eprintln!("capture child: {line}");
        }
    });
    let receipt = ready_rx.recv_timeout(std::time::Duration::from_secs(60));
    let _ = child.kill();
    let stopped = child.wait().unwrap();
    observer.join().unwrap();
    let receipt = receipt.expect("child reached disputed durable capture before SIGKILL");
    assert_eq!(
        stopped.signal(),
        Some(9),
        "actual SIGKILL, not graceful exit"
    );
    eprintln!("capture-boundary SIGKILL: {receipt}");
    let expected: serde_json::Value = serde_json::from_slice(
        &std::fs::read(directory.path().join("capture-expected.json")).unwrap(),
    )
    .unwrap();
    let mut config = CoreConfig::new(directory.path().to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    let mut recovered = Drill::open(config);
    assert_eq!(recovered.state.tick, receipt["tick"].as_u64().unwrap());
    assert_eq!(image(&recovered.state), expected["image"]);
    assert_eq!(
        serde_json::to_value(pose(&recovered.state)).unwrap(),
        expected["pose"]
    );
    if recovered.state.hostile.battle.running {
        continues(&mut recovered);
    }
    recovered.checkpoint();
    let expected = image(&recovered.state);
    recovered = recovered.crash();
    assert_eq!(
        image(&recovered.state),
        expected,
        "subsequent checkpoint/restart"
    );
}

fn queue_ground_death(drill: &mut Drill, victim: EntityId) {
    let slot = drill.state.entities.slot_of(victim).unwrap();
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
}

/// Inspect the post-merge/pre-capture owner, then use the production capture.
fn append_capture_tick(drill: &mut Drill, output: &crate::boundary::TickOutput) {
    let batch = Batch {
        tick: output.tick,
        entries: journal_entries_for_tick(&mut drill.state, output).unwrap(),
    };
    drill.append(&batch).unwrap();
}

#[test]
fn battle_resume_capture_boundary_discards_unheld_player_blow() {
    capture_boundary_sigkill(
        "battle_resume_capture_boundary_discards_unheld_player_blow",
        |drill| {
            let connection = ConnectionId(77);
            drill
                .run(&[PlayerInput::Join {
                    connection,
                    uuid: uuid::Uuid::new_v4(),
                    name: "Visitor".into(),
                    permission_level: 0,
                }])
                .unwrap();
            let player = drill.state.connections.entity_of(connection).unwrap();
            let slot = drill.state.entities.slot_of(player).unwrap();
            let victim = drill.state.hostile.battle.formation.members[0].id;
            let target = drill.state.entities.slot_of(victim).unwrap();
            let at = drill.state.entities.pos(target);
            drill
                .state
                .entities
                .set_pos(slot, [at[0], at[1], at[2] - 1.5]);
            drill.state.entities.set_ground(slot, true);
            let health = drill.state.entities.row(target).health;
            let (output, _, _) = crate::tick::run_tick(
                &mut drill.state,
                &[PlayerInput::Attack {
                    connection,
                    target: victim,
                }],
            );
            assert!(
                drill.state.entities.row(target).health < health,
                "actual player blow"
            );
            let pending = image(&drill.state)["battle"]["retaliation"]["pending"].clone();
            assert!(
                pending
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|pair| pair[1] == player.0),
                "ordinary post-merge owner contains the unheld player's observation"
            );
            append_capture_tick(drill, &output);
        },
    );
}

#[test]
fn battle_resume_capture_boundary_death_before_settle() {
    capture_boundary_sigkill(
        "battle_resume_capture_boundary_death_before_settle",
        |drill| {
            drill.state.hostile.battle.running = false;
            let victim = drill.state.hostile.battle.formation.members[0].id;
            drill.state.hostile.battle.ground_pending.push(victim);
            queue_ground_death(drill, victim);
            drill.state.commit_merge_effects();
            let (output, _, _) = crate::tick::run_tick(&mut drill.state, &[]);
            assert!(!drill.state.hostile.battle.ground_pending.contains(&victim));
            append_capture_tick(drill, &output);
        },
    );
}

#[test]
fn battle_resume_capture_boundary_death_after_refusal() {
    capture_boundary_sigkill(
        "battle_resume_capture_boundary_death_after_refusal",
        |drill| {
            drill.state.hostile.battle.running = false;
            let victim = drill.state.hostile.battle.formation.members[0].id;
            let slot = drill.state.entities.slot_of(victim).unwrap();
            drill.state.entities.set_pos(slot, [8.5, 200.0, 8.5]);
            let water = crate::crafter::tests::block(&drill.state, "water", &[("level", "0")]);
            for x in -2_i32..=18 {
                for z in -2_i32..=18 {
                    let chunk = drill
                        .state
                        .world
                        .chunk(mc_world::ChunkPos {
                            x: x.div_euclid(16),
                            z: z.div_euclid(16),
                        })
                        .unwrap();
                    let geometry = chunk.geometry();
                    // Native heightmaps retain generated terrain heights; cover
                    // the actual scan band, not the fixture's high stone slab.
                    let top = geometry.min_y()
                        + chunk.heightmaps["MOTION_BLOCKING"]
                            .get(x.rem_euclid(16) as u8, z.rem_euclid(16) as u8)
                            as i32
                        + 16;
                    let wet_y = top.min(geometry.max_y()) - 1;
                    drill.state.world.set_block(x, wet_y, z, water);
                }
            }
            // Ground's column scan reads the resident chunk, not the write overlay.
            drill.run(&[]).unwrap();
            drill.state.hostile.battle.ground_pending.push(victim);
            drill.run(&[]).unwrap();
            assert!(
                drill.state.hostile.battle.ground_refused.contains(&victim),
                "real wet terrain must produce Placement::Nowhere"
            );
            drill.checkpoint();
            queue_ground_death(drill, victim);
            let (output, _, _) = crate::tick::run_tick(&mut drill.state, &[]);
            assert!(!drill.state.entities.body(slot).alive);
            assert!(drill.state.hostile.battle.ground_refused.contains(&victim));
            append_capture_tick(drill, &output);
        },
    );
}

#[test]
fn battle_resume_capture_boundary_death_after_waiting_settle() {
    capture_boundary_sigkill(
        "battle_resume_capture_boundary_death_after_waiting_settle",
        |drill| {
            drill.state.hostile.battle.running = false;
            let victim = drill.state.hostile.battle.formation.members[0].id;
            let slot = drill.state.entities.slot_of(victim).unwrap();
            drill.state.entities.set_pos(slot, [800.0, 200.0, 800.0]);
            drill.state.hostile.battle.ground_pending.push(victim);
            queue_ground_death(drill, victim);
            let (output, _, _) = crate::tick::run_tick(&mut drill.state, &[]);
            assert!(!drill.state.entities.body(slot).alive);
            assert!(
                drill.state.hostile.battle.ground_pending.contains(&victim),
                "settle waited on the unavailable chunk before this tick's death merge"
            );
            append_capture_tick(drill, &output);
        },
    );
}

/// The start tick of a fighter's windup on `target` in the battle image.
fn windup_on(state: &TickState, target: EntityId) -> Option<u64> {
    image(state)["battle"]["combat"]["fighters"]
        .as_array()
        .unwrap()
        .iter()
        .find_map(|fighter| {
            let windup = &fighter["action"]["Windup"];
            (windup["target"] == target.0).then(|| windup["start"].as_u64().unwrap())
        })
}

/// A11 P0 (BATTLE-RECOVERY-UNKNOWN-WINDUP-TARGET): a cavalry horse extra died
/// while a soldier wound up on it. The next battle pass dropped the horse from
/// the execution columns and its dying row never gets a checkpoint record, yet
/// the soldier's Windup kept naming it: a clean checkpoint inside the windup
/// window made every later start refuse `unknown windup target`.
#[test]
fn battle_resume_clean_checkpoint_after_a_windup_target_horse_dies_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    // Red's file-0 front-rank soldier stands on red's -x flank.
    let red = drill.state.hostile.battle.formation.members[0].id;
    let flank = |state: &TickState| {
        let at = state.entities.pos(state.entities.slot_of(red)?);
        Some([at[0] - 1.5, at[1], at[2]])
    };
    let horse = drill.state.spawn_mob(
        drill
            .state
            .registry
            .entity_type_id("minecraft:horse")
            .unwrap(),
        flank(&drill.state).expect("red file 0 alive"),
        20.0,
    );
    let slot = drill.state.entities.slot_of(horse).unwrap();
    // Blue's side outside blue's roster: an execution-column extra, as a
    // cavalry horse that lost its rider.
    drill.state.entities.set_faction(slot, 2);
    drill.state.entities.set_ground(slot, true);
    let mut start = None;
    for _ in 0..2000 {
        if let Some(at) = flank(&drill.state) {
            drill.state.entities.set_pos(slot, at);
            drill.state.entities.set_vel(slot, [0.0; 3]);
        }
        drill.run(&[]).unwrap();
        start = windup_on(&drill.state, horse);
        if start.is_some() {
            break;
        }
    }
    let start = start.expect("a red soldier winds up on the horse extra");
    let battle = &drill.state.hostile.battle;
    assert!(battle.columns.ids.contains(&horse));
    let impact = start + u64::from(crate::battle::combat::Weapon::Sword.windup());
    assert!(
        impact > battle.elapsed_ticks + 1,
        "the death and the capture fall inside the windup window"
    );
    queue_ground_death(&mut drill, horse);
    drill.state.commit_merge_effects();
    assert!(!drill.state.entities.body(slot).alive);
    for _ in 0..2 {
        drill.run(&[]).unwrap();
    }
    assert!(
        !drill.state.hostile.battle.columns.ids.contains(&horse),
        "the dead horse left the execution columns"
    );
    checkpoint_reopens(drill);
}

/// A clean checkpoint, offline recovery, and the reopened battle fights on.
fn checkpoint_reopens(mut drill: Drill) {
    drill.checkpoint();
    let config = drill.config.clone();
    drop(drill);
    let offline = crate::native_checkpoint::recover_offline(&config).unwrap();
    assert!(
        offline.dimensions[&crate::dimensions::Dimension::Overworld]
            .battle
            .is_some()
    );
    drop(offline);
    let mut recovered = Drill::open(config);
    continues(&mut recovered);
}

/// A soldier of `side` outside his hurt window, so a light blow takes health
/// and the damage merge reports it to the battle (`retaliation::hurt`).
fn fresh_soldier(state: &TickState, side: u16) -> crate::entities::EntitySlot {
    let formation = &state.hostile.battle.formation;
    formation
        .members
        .iter()
        .filter(|member| formation.regiments[member.regiment].spec.faction == side)
        .find_map(|member| {
            let slot = state.entities.slot_of(member.id)?;
            let hurt = state.entities.body(slot).hurt_at;
            hurt.is_none_or(|at| {
                state.tick.saturating_sub(at) >= crate::entities::HURT_INVULNERABLE_TICKS
            })
            .then_some(slot)
        })
        .expect("a soldier outside his hurt window")
}

fn melee_blow(attacker: EntityId) -> crate::tick::DamagePiece {
    crate::tick::DamagePiece {
        amount: 1.0,
        kind: crate::survival::DamageKind::Melee,
        source: Some(crate::intent::DamageSource::Melee {
            attacker,
            position: [0.0; 3],
            yaw: 0.0,
            bonus_knockback: 0.0,
            sweep: false,
            fire_ticks: 0,
            bane_ticks: 0,
        }),
        physical: None,
        custom_armor: false,
    }
}

/// The merged blows awaiting the next battle pass name `attacker`.
fn pending_blow_by(state: &TickState, attacker: EntityId) -> bool {
    image(state)["battle"]["retaliation"]["pending"]
        .as_array()
        .unwrap()
        .iter()
        .any(|pair| pair[1] == attacker.0)
}

/// BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER: a blow merged after the
/// pass in which its attacker, a dead horse extra, left the execution columns
/// waits in `retaliation.pending` for the next pass. A clean checkpoint in
/// that tick named an identity the image did not carry (the dying row gets no
/// record), and every start refused `unknown retaliation identity`.
#[test]
fn battle_resume_clean_checkpoint_with_a_pending_blow_by_a_departed_horse_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let red = drill.state.entities.pos(fresh_soldier(&drill.state, 1));
    let horse = drill.state.spawn_mob(
        drill
            .state
            .registry
            .entity_type_id("minecraft:horse")
            .unwrap(),
        [red[0] - 1.5, red[1], red[2]],
        20.0,
    );
    let slot = drill.state.entities.slot_of(horse).unwrap();
    drill.state.entities.set_faction(slot, 2);
    drill.state.entities.set_ground(slot, true);
    drill.run(&[]).unwrap();
    assert!(
        drill.state.hostile.battle.columns.ids.contains(&horse),
        "the horse is a blue execution-column extra"
    );
    queue_ground_death(&mut drill, horse);
    drill.state.commit_merge_effects();
    drill.run(&[]).unwrap();
    assert!(
        !drill.state.hostile.battle.columns.ids.contains(&horse),
        "the dead horse left the execution columns"
    );
    // A blow the horse dealt before it died, merged after its departure.
    let victim = fresh_soldier(&drill.state, 1);
    drill.state.merge_damage.push((victim, melee_blow(horse)));
    drill.state.commit_merge_effects();
    assert!(pending_blow_by(&drill.state, horse));
    checkpoint_reopens(drill);
}

/// BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER: a foreign mob hurts a
/// soldier and dies in the same damage merge; its pending blow named a dying
/// row without a checkpoint record.
#[test]
fn battle_resume_clean_checkpoint_after_a_foreign_attacker_dies_in_its_blow_merge_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let victim = fresh_soldier(&drill.state, 1);
    let at = drill.state.entities.pos(victim);
    let zombie = drill.state.spawn_mob(
        drill
            .state
            .registry
            .entity_type_id("minecraft:zombie")
            .unwrap(),
        [at[0] + 1.0, at[1], at[2]],
        20.0,
    );
    drill.state.merge_damage.push((victim, melee_blow(zombie)));
    queue_ground_death(&mut drill, zombie);
    drill.state.commit_merge_effects();
    let slot = drill.state.entities.slot_of_any(zombie).unwrap();
    assert!(!drill.state.entities.body(slot).alive);
    assert!(pending_blow_by(&drill.state, zombie));
    checkpoint_reopens(drill);
}

/// BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER: a foreign creeper's blast
/// hurts soldiers. The creeper's row is removed before its blast is merged,
/// so every pending blow it leaves names an identity no record carries.
#[test]
fn battle_resume_clean_checkpoint_after_a_creeper_blast_on_soldiers_reopens() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    // Keep the slab the soldiers stand on.
    drill
        .state
        .gamerules
        .insert("mob_griefing".into(), "false".into());
    let red = drill.state.entities.pos(fresh_soldier(&drill.state, 1));
    let creeper = drill.state.spawn_mob(
        drill
            .state
            .registry
            .entity_type_id("minecraft:creeper")
            .unwrap(),
        [red[0], red[1], red[2] - 3.0],
        20.0,
    );
    let slot = drill.state.entities.slot_of(creeper).unwrap();
    drill.state.entities.set_ground(slot, true);
    let row = drill.state.entities.row_mut(slot);
    row.persistent = true;
    row.controls.ignited = true;
    let mut blast = None;
    for _ in 0..200 {
        let (output, _, _) = crate::tick::run_tick(&mut drill.state, &[]);
        if drill.state.entities.slot_of_any(creeper).is_none() {
            blast = Some(output);
            break;
        }
        append_capture_tick(&mut drill, &output);
    }
    let output = blast.expect("the ignited creeper exploded");
    assert!(
        pending_blow_by(&drill.state, creeper),
        "the blast hurt a soldier"
    );
    append_capture_tick(&mut drill, &output);
    checkpoint_reopens(drill);
}

fn battle_command(drill: &mut Drill, command: crate::battle::staging::BattleCommand) {
    crate::battle::staging::execute(
        &mut drill.state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 200.0, 8.5],
        &command,
    )
    .unwrap_or_else(|error| panic!("{command:?}: {error}"));
}

/// BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER (review B1): a held
/// player's blow waits in `retaliation.pending` while the battle is paused,
/// and he leaves: his row goes, the image keeps his last identity. WAL and
/// checkpoint capture keep the blow, so after recovery his reconnect rebinds
/// it to his new wire ID and the resumed battle answers him.
#[test]
fn battle_resume_pending_blow_by_a_held_player_who_left_rebinds_after_recovery() {
    use crate::battle::staging::BattleCommand;
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let uuid = uuid::Uuid::new_v4();
    let join = |connection| PlayerInput::Join {
        connection,
        uuid,
        name: "PovRed".into(),
        permission_level: 0,
    };
    battle_command(&mut drill, BattleCommand::Pause);
    drill.run(&[join(ConnectionId(77))]).unwrap();
    battle_command(
        &mut drill,
        BattleCommand::Live {
            name: "PovRed".into(),
            side: 1,
        },
    );
    drill.run(&[]).unwrap();
    battle_command(&mut drill, BattleCommand::Start);
    let holder = drill.state.connections.entity_of(ConnectionId(77)).unwrap();
    let blue = {
        let slot = fresh_soldier(&drill.state, 2);
        drill.state.entities.body(slot).id
    };
    let face_blue = |state: &mut TickState, player: EntityId| {
        let at = state
            .entities
            .pos(state.entities.slot_of(blue).expect("blue soldier alive"));
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.set_pos(slot, [at[0], at[1], at[2] - 1.5]);
        state.entities.set_ground(slot, true);
    };
    let health = |state: &TickState| {
        let slot = state.entities.slot_of(blue).unwrap();
        state.entities.row(slot).health
    };
    face_blue(&mut drill.state, holder);
    let before = health(&drill.state);
    let (output, _, _) = crate::tick::run_tick(
        &mut drill.state,
        &[PlayerInput::Attack {
            connection: ConnectionId(77),
            target: blue,
        }],
    );
    assert!(health(&drill.state) < before, "actual held player blow");
    assert!(pending_blow_by(&drill.state, holder));
    append_capture_tick(&mut drill, &output);
    // Paused before the next battle pass weighs the blow; then he leaves.
    battle_command(&mut drill, BattleCommand::Pause);
    drill
        .run(&[PlayerInput::Leave {
            connection: ConnectionId(77),
        }])
        .unwrap();
    assert!(drill.state.entities.slot_of_any(holder).is_none());
    assert!(
        pending_blow_by(&drill.state, holder),
        "WAL capture keeps the offline holder's blow"
    );
    let mut drill = drill.crash();
    assert!(pending_blow_by(&drill.state, holder), "WAL-only recovery");
    drill.checkpoint();
    let config = drill.config.clone();
    drop(drill);
    let offline = crate::native_checkpoint::recover_offline(&config).unwrap();
    assert!(
        offline.dimensions[&crate::dimensions::Dimension::Overworld]
            .battle
            .is_some()
    );
    drop(offline);
    let mut recovered = Drill::open(config);
    hydrate_battle(&mut recovered.state);
    assert!(
        pending_blow_by(&recovered.state, holder),
        "checkpoint capture and recovery keep it"
    );
    recovered.run(&[join(ConnectionId(78))]).unwrap();
    let mut rejoined = None;
    for _ in 0..40 {
        rejoined = recovered
            .state
            .connections
            .entity_of(ConnectionId(78))
            .filter(|&id| pending_blow_by(&recovered.state, id));
        if rejoined.is_some() {
            break;
        }
        recovered.run(&[]).unwrap();
    }
    let rejoined = rejoined.expect("his reconnect rebinds the pending blow");
    assert_ne!(rejoined, holder);
    assert!(!pending_blow_by(&recovered.state, holder));
    face_blue(&mut recovered.state, rejoined);
    battle_command(&mut recovered, BattleCommand::Start);
    recovered.run(&[]).unwrap();
    assert_eq!(
        image(&recovered.state)["battle"]["retaliation"]["grudges"][blue.0.to_string()]["attacker"],
        rejoined.0,
        "the resumed battle answers the reconnected holder's blow"
    );
}

/// The compact entity records must survive the same kill point as the battle
/// history: a published batch is durable, so the reopened world has the shield
/// pose, the health and the in-flight projectile of the last published tick.
/// A checkpoint capture in between reseeds the base image.
fn shield(state: &mut TickState, id: EntityId, raised: bool) {
    let slot = state.entities.slot_of(id).unwrap();
    assert!(crate::hostile::set_extra(
        state,
        slot,
        crate::battle::LIVING_FLAGS,
        crate::boundary::EntityDataValue::Byte(0),
        crate::boundary::EntityDataValue::Byte(if raised { 3 } else { 0 }),
    ));
    state.appearance_changed.push(id);
}

fn shield_pose(state: &TickState, id: EntityId) -> Option<i8> {
    let slot = state.entities.slot_of(id)?;
    state
        .entities
        .row(slot)
        .appearance
        .extra
        .iter()
        .find(|(index, _)| *index == crate::battle::LIVING_FLAGS)
        .map(|(_, value)| match value {
            crate::boundary::EntityDataValue::Byte(flags) => *flags,
            other => panic!("living flags is a byte, got {other:?}"),
        })
}

fn compact_ids(entries: &[JournalEntry], variant: fn(&JournalEntry) -> Option<i32>) -> Vec<i32> {
    entries.iter().filter_map(variant).collect()
}

#[test]
fn compact_entity_records_survive_a_crash_after_the_published_batch() {
    let directory = tempfile::tempdir().unwrap();
    // `prepared` ends with a checkpoint, so the first change must be a full image.
    let mut drill = prepared(directory.path());
    let member = drill.state.hostile.battle.formation.members[0].id;
    // The battle itself raises shields, so toggle from the live pose.
    let raised = shield_pose(&drill.state, member).unwrap_or(0) != 0;
    shield(&mut drill.state, member, !raised);
    let first = drill.capture_tick(&[]);
    drill.append(&first).unwrap();
    assert!(
        first
            .entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::Entity { id, .. } if *id == member.0)),
        "the first change after a capture writes the whole image"
    );
    // Later changes are compact.
    shield(&mut drill.state, member, raised);
    let compact = drill.capture_tick(&[]);
    let appearances = compact_ids(&compact.entries, |entry| match entry {
        JournalEntry::EntityAppearance { id, .. } => Some(*id),
        _ => None,
    });
    assert!(
        appearances.contains(&member.0),
        "the toggled body takes a compact record: {appearances:?}"
    );
    assert!(
        !compact
            .entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::Entity { id, .. } if *id == member.0))
    );
    drill.append(&compact).unwrap();
    let durable_tick = drill.state.tick;
    let expected = shield_pose(&drill.state, member);
    let expected_health = drill
        .state
        .entities
        .row(drill.state.entities.slot_of(member).unwrap())
        .health;
    // The next tick stays unpublished, exactly like a kill during the tick.
    let unpublished = drill.capture_tick(&[]);
    assert_eq!(unpublished.tick, durable_tick + 1);
    let mut recovered = drill.crash();
    assert_eq!(recovered.state.tick, durable_tick);
    assert_eq!(
        shield_pose(&recovered.state, member),
        expected,
        "the compact record's metadata byte survives the kill"
    );
    assert_eq!(
        recovered
            .state
            .entities
            .row(recovered.state.entities.slot_of(member).unwrap())
            .health,
        expected_health,
        "the member's health survives the kill"
    );
    assert_eq!(
        recovered.state.hostile.battle.formation.members[0].id, member,
        "the battle history still names the body the compact record updated"
    );
    // A checkpoint capture drops the bases: the next change is a full image
    // again, and that image is what a later recovery folds.
    recovered.checkpoint();
    let raised = shield_pose(&recovered.state, member).unwrap_or(0) != 0;
    shield(&mut recovered.state, member, !raised);
    let reseeded = recovered.capture_tick(&[]);
    recovered.append(&reseeded).unwrap();
    assert!(
        reseeded
            .entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::Entity { id, .. } if *id == member.0)),
        "a checkpoint capture reseeds the base image"
    );
    let expected = shield_pose(&recovered.state, member);
    let durable = recovered.state.tick;
    recovered = recovered.crash();
    assert_eq!(recovered.state.tick, durable);
    assert_eq!(shield_pose(&recovered.state, member), expected);
}

/// Offline recovery is a shared entry point (export, conversion, census): it must
/// refuse a compact record whose full image is not in the tail — both when the
/// body exists only in the checkpoint and when it exists nowhere.
#[test]
fn offline_recovery_refuses_a_compact_record_without_its_tail_image() {
    use crate::native_checkpoint::recover_offline;
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let member = drill.state.hostile.battle.formation.members[0].id;
    let tick = drill.state.tick + 1;
    let compact = |id: i32| JournalEntry::EntityAppearance {
        id,
        tick,
        age: 0,
        health: 20.0,
        alive: true,
        active_effects: Default::default(),
        appearance: Default::default(),
    };
    // `prepared` checkpointed this body, so it exists in `Replay.entities` with no
    // tail image: existence must not count as a base.
    drill
        .append(&Batch {
            tick,
            entries: vec![compact(member.0), compact(2_000_000)],
        })
        .unwrap();
    // The drill holds the native store open; offline recovery opens its own.
    let config = drill.config.clone();
    drop(drill);
    let error = match recover_offline(&config) {
        Ok(_) => panic!("no tail image must refuse"),
        Err(error) => error,
    };
    assert!(
        format!("{error}").contains("without an earlier full image"),
        "{error}"
    );
}

/// Control for the refusal above: the same body with its image in the tail
/// recovers offline, and the compact record folds.
#[test]
fn offline_recovery_accepts_a_compact_record_with_its_tail_image() {
    use crate::native_checkpoint::recover_offline;
    let directory = tempfile::tempdir().unwrap();
    let mut drill = prepared(directory.path());
    let member = drill.state.hostile.battle.formation.members[0].id;
    let raised = shield_pose(&drill.state, member).unwrap_or(0) != 0;
    shield(&mut drill.state, member, !raised);
    let image = drill.capture_tick(&[]);
    assert!(
        image
            .entries
            .iter()
            .any(|entry| matches!(entry, JournalEntry::Entity { id, .. } if *id == member.0))
    );
    drill.append(&image).unwrap();
    shield(&mut drill.state, member, raised);
    let compact = drill.capture_tick(&[]);
    assert!(compact.entries.iter().any(
        |entry| matches!(entry, JournalEntry::EntityAppearance { id, .. } if *id == member.0)
    ));
    drill.append(&compact).unwrap();
    let config = drill.config.clone();
    drop(drill);
    let recovered = match recover_offline(&config) {
        Ok(recovered) => recovered,
        Err(error) => panic!("an image-then-delta tail must recover: {error}"),
    };
    let record =
        &recovered.dimensions[&crate::dimensions::Dimension::Overworld].entities[&member.0];
    assert!(record.alive);
}
