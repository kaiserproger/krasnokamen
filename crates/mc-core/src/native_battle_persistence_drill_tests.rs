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
