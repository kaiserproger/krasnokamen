use super::*;
use crate::boundary::ChangeSet;
use crate::mounts::tests::{fixture, slot, spawn};

#[test]
fn steady_front_refuses_but_wavering_flank_and_pursuit_take_impact() {
    assert_eq!(charge_decision(Contact::SteadyFront), Decision::Refuse);
    for contact in [Contact::Wavering, Contact::Flank, Contact::Pursuit] {
        assert_eq!(charge_decision(contact), Decision::Impact);
    }
    assert_eq!(impact_bonus(0), 2.0);
    assert_eq!(impact_bonus(130), 1.0);
    assert_eq!(impact_bonus(259), 2.0 / 260.0);
    assert_eq!(impact_bonus(260), 0.0);
    assert_eq!(impact_bonus(1000), 0.0);
}

fn assert_pair(state: &TickState, rider: EntityId, horse: EntityId) {
    let h = slot(state, horse);
    let r = slot(state, rider);
    assert_eq!(
        state.entities.row(h).entity_life.mount.rider,
        Some(state.entities.row(r).uuid)
    );
    assert_eq!(state.entities.row(r).entity_life.mount.vehicle, Some(horse));
    assert_eq!(
        state.entities.row(h).appearance.passengers.as_slice(),
        [rider]
    );
    assert!(state.entities.pos(r)[1] > state.entities.pos(h)[1]);
}

#[test]
fn mob_rider_survives_native_checkpoint_restart_and_rejects_duplicate_or_nested_links() {
    use crate::journal::{JournalWriter, Replay, read_complete_batches_after, replay_batches};
    let (mut state, _, pos) = fixture();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    mount(&mut state, rider, horse).unwrap();
    crate::mounts::tick(&mut state, &mut ChangeSet::default());
    assert_pair(&state, rider, horse);
    let (mut output, _, _) = crate::tick::run_tick(&mut state, &[]);
    std::sync::Arc::get_mut(&mut output.changes)
        .unwrap()
        .entities_spawned
        .extend([horse, rider]);
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    let root = tempfile::tempdir().unwrap();
    let mut writer = JournalWriter::open(root.path(), 1).unwrap();
    writer.append(1, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let batches = read_complete_batches_after(&root.path().join("journal"), 0).unwrap();
    let replay = replay_batches(Replay::default(), &batches);
    crate::mounts::persistence::validate_replay(&replay).unwrap();
    let (mut restarted, _, _) = fixture();
    for (id, record) in &replay.entities {
        crate::core::restore_entity(&mut restarted, *id, record).unwrap();
    }
    crate::mounts::tick(&mut restarted, &mut ChangeSet::default());
    assert_pair(&restarted, rider, horse);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&restarted, restarted.tick);
    let native = crate::native_checkpoint::encode_entities(
        crate::dimensions::Dimension::Overworld,
        &chunks,
        &restarted.registry.items,
        restarted.tick,
    )
    .unwrap();
    let config = crate::CoreConfig::new(root.path().join("checkpoint"), 42);
    crate::native_checkpoint::tests::write_manifest(&config, &Default::default()).unwrap();
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    let base = store.read_checkpoint().unwrap();
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: restarted.tick,
                previous_tick: base.tick,
                chunks: &[],
                entities: &native,
                players: &[],
                globals: &base.globals,
            },
            &restarted.registry.blocks,
            &restarted.registry.items,
        )
        .unwrap();
    drop(store);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    crate::mounts::persistence::validate_replay(replay).unwrap();
    let (mut restored, _, _) = fixture();
    for (id, record) in &replay.entities {
        crate::core::restore_entity(&mut restored, *id, record).unwrap();
    }
    crate::mounts::tick(&mut restored, &mut ChangeSet::default());
    assert_pair(&restored, rider, horse);
    let mut corrupt = Replay {
        entities: replay.entities.clone(),
        ..Default::default()
    };
    let rider_uuid = restored.entities.row(slot(&restored, rider)).uuid;
    corrupt
        .entities
        .get_mut(&rider.0)
        .unwrap()
        .state
        .entity_life
        .mount
        .rider = Some(rider_uuid);
    assert!(crate::mounts::persistence::validate_replay(&corrupt).is_err());
    let horse_uuid = restored.entities.row(slot(&restored, horse)).uuid;
    corrupt
        .entities
        .get_mut(&rider.0)
        .unwrap()
        .state
        .entity_life
        .mount
        .rider = Some(horse_uuid);
    assert!(crate::mounts::persistence::validate_replay(&corrupt).is_err());
}

#[test]
fn mounted_kernel_veers_away_from_steady_front_and_commits_to_wavering_front() {
    let (mut state, _, pos) = fixture();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    let target_pos = [pos[0] + 5.0, pos[1], pos[2]];
    let target = spawn(&mut state, "husk", target_pos);
    mount(&mut state, rider, horse).unwrap();
    for (id, faction) in [(horse, 1), (rider, 1), (target, 2)] {
        let s = slot(&state, id);
        state.entities.set_faction(s, faction);
    }
    let mut battle = BattleState::default();
    battle
        .formation
        .add_regiment(
            super::super::formation::RegimentSpec {
                id: 2,
                faction: 2,
                origin: target_pos,
                facing: [-1.0, 0.0],
                files: 1,
                order: super::super::formation::Order::Hold,
            },
            &[target],
        )
        .unwrap();
    battle.targets.insert(
        rider,
        super::super::EnemyTarget {
            id: target,
            pos: target_pos,
            distance: 5.0,
        },
    );
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    let mut combat = CombatRuntime::default();
    step(&frame, &mut battle, &mut combat);
    assert_eq!(combat.work.charge_checks, 1);
    assert!(
        battle.steering[&horse][0] < 0.0,
        "refusal must not move through the line"
    );
    assert_ne!(
        battle.steering[&horse][2], 0.0,
        "horse must run out to a flank"
    );
    assert_eq!(combat.cavalry.bonus(rider, 0), 0.0);
    battle.formation.regiments[0].morale.value = 20.0;
    battle.formation.members[0].wavering = true;
    let mut combat = CombatRuntime::default();
    step(&frame, &mut battle, &mut combat);
    assert_eq!(combat.work.charge_checks, 1);
    assert!(battle.steering[&horse][0] > 0.0);
    assert_eq!(combat.cavalry.bonus(rider, 0), 2.0);
    assert_eq!(combat.cavalry.bonus(rider, 260), 0.0);
}

#[test]
fn mounted_speed_and_charge_remain_inside_loaded_bounds() {
    let (mut state, _, pos) = fixture();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    mount(&mut state, rider, horse).unwrap();
    let rider_slot = slot(&state, rider);
    state.entities.set_faction(rider_slot, 1);
    let target = spawn(&mut state, "husk", [pos[0] + 5.0, pos[1], pos[2]]);
    let target_slot = slot(&state, target);
    state.entities.set_faction(target_slot, 2);
    let mut battle = BattleState::default();
    battle
        .formation
        .set_bounds([pos[0] - 1.0, pos[2] - 1.0], [pos[0] + 0.01, pos[2] + 1.0])
        .unwrap();
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    for targeted in [false, true] {
        if targeted {
            battle.targets.insert(
                rider,
                super::super::EnemyTarget {
                    id: target,
                    pos: [pos[0] + 5.0, pos[1], pos[2]],
                    distance: 5.0,
                },
            );
        }
        battle.steering.insert(rider, [0.02, 0.0, 0.0]);
        let mut combat = CombatRuntime::default();
        step(&frame, &mut battle, &mut combat);
        assert!(battle.steering[&horse][0] > 0.0);
        assert!(pos[0] + battle.steering[&horse][0] * mc_physics::TICK_SECONDS <= pos[0] + 0.01);
    }
}
