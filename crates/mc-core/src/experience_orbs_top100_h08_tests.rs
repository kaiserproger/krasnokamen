use super::*;
use crate::boundary::{ChangeSet, EntityEvent};
use crate::commands::GameMode;

fn spawn(state: &mut TickState, pos: [f64; 3], value: i32) -> EntityId {
    let mut spawned = Vec::new();
    award(state, pos, value, &mut spawned);
    let id = spawned[0];
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_vel(slot, [0.0; 3]);
    id
}

#[test]
fn pickup_cooldown_preserves_remaining_orb_and_spectators_do_not_collect() {
    let (mut state, player, pos) = crate::hostile_tests::setup(0.0);
    state.tick = 1;
    let first = spawn(&mut state, pos, 3);
    let second = spawn(&mut state, pos, 7);
    let mut changes = ChangeSet::default();
    let mut events = Vec::new();
    state.players.get_mut(&player).unwrap().game_mode = GameMode::Spectator;
    run(&mut state, &mut changes, &mut events);
    assert!(events.is_empty());
    assert!(state.entities.slot_of(first).is_some());
    assert!(state.entities.slot_of(second).is_some());
    state.players.get_mut(&player).unwrap().game_mode = GameMode::Survival;
    run(&mut state, &mut changes, &mut events);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, EntityEvent::Pickup { .. }))
            .count(),
        1
    );
    assert_eq!(state.entities.experience_orbs().count(), 1);
    run(&mut state, &mut changes, &mut events);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, EntityEvent::Pickup { .. }))
            .count(),
        1
    );
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.body_mut(slot).pickup_delay = 0;
    run(&mut state, &mut changes, &mut events);
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, EntityEvent::Pickup { .. }))
            .count(),
        2
    );
    assert_eq!(state.entities.experience_orbs().count(), 0);
}

#[test]
fn expired_orb_removal_does_not_skip_a_swapped_live_orb() {
    let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
    state.tick = 1;
    let expired = spawn(&mut state, pos, 3);
    let live = spawn(&mut state, pos, 7);
    let slot = state.entities.slot_of(expired).unwrap();
    state.entities.row_mut(slot).age = 6000;
    let mut changes = ChangeSet::default();
    let mut sample = crate::boundary::CorePerfTick::default();
    run_counted(&mut state, &mut changes, &mut Vec::new(), &mut sample);
    assert_eq!(sample.xp_orbs_processed, 2);
    assert_eq!(changes.entities_removed, vec![expired]);
    assert!(state.entities.slot_of(expired).is_none());
    assert!(state.entities.slot_of(live).is_some());
}

#[test]
fn equal_distance_pickup_uses_lowest_player_id() {
    use crate::boundary::{ConnectionId, PlayerInput};
    let (mut state, first, pos) = crate::hostile_tests::setup(0.0);
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(2),
            uuid: uuid::Uuid::new_v4(),
            name: "second".to_owned(),
            permission_level: 0,
        }],
    );
    let second = state.connections.entity_of(ConnectionId(2)).unwrap();
    assert!(first < second);
    for player in [first, second] {
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.set_pos(slot, pos);
    }
    state.tick = 1;
    let id = spawn(&mut state, pos, 3);
    let mut events = Vec::new();
    run(&mut state, &mut ChangeSet::default(), &mut events);
    assert!(events.iter().any(|event| matches!(
        event,
        EntityEvent::Pickup { item, collector, .. } if *item == id && *collector == first
    )));
    assert_eq!(state.players[&first].vitals.experience.total, 3);
    assert_eq!(state.players[&second].vitals.experience.total, 0);
}

#[test]
fn merged_same_group_conserves_value_and_keeps_other_values_separate() {
    let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
    let mut ids = Vec::new();
    for i in 0..41 {
        ids.push(spawn(
            &mut state,
            [pos[0] + f64::from(i) * 16.0, pos[1], pos[2]],
            3,
        ));
    }
    assert_eq!(ids[0].0.rem_euclid(40), ids[40].0.rem_euclid(40));
    let moved = state.entities.slot_of(ids[40]).unwrap();
    state.entities.set_pos(moved, pos);
    let different = spawn(&mut state, pos, 7);
    state.tick = 20;
    let mut changes = ChangeSet::default();
    run(&mut state, &mut changes, &mut Vec::new());
    let first = state.entities.slot_of(ids[0]).unwrap();
    assert_eq!(
        state.entities.row(first).appearance.experience_orb,
        Some((3, 2))
    );
    assert!(state.entities.slot_of(ids[40]).is_none());
    let other = state.entities.slot_of(different).unwrap();
    assert_eq!(
        state.entities.row(other).appearance.experience_orb,
        Some((7, 1))
    );
}

/// The lifecycle owner starts after spawn; insert every retained orb through the
/// entity-store owner rather than award's intentional birth-time coalescing.
fn populate(state: &mut TickState, pos: [f64; 3], count: usize) {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:experience_orb").unwrap())
        .unwrap();
    for _ in 0..count {
        let id = state.entities.alloc_id();
        state.entities.spawn(
            id,
            state.entity_uuid(id),
            EntityKind::ExperienceOrb { type_id },
            ColdFields {
                pos,
                vel: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                ground: false,
                health: 5.0,
                breed_cooldown: 0,
                pickup_delay: 0,
                item: None,
                appearance: Arc::new(EntityAppearance {
                    experience_orb: Some((3, 1)),
                    health: 5.0,
                    ..Default::default()
                }),
                movement: crate::blueprint::MovementClass::Item,
                half_width: 0.25,
                height: 0.5,
                speed: 0.0,
                attack_damage: 0.0,
                sight_range: 0.0,
                attack_reach: 0.0,
            },
        );
    }
}

#[test]
fn body_visit_ladder_bounds_all_search_work() {
    use crate::boundary::{ConnectionId, PlayerInput};
    let mut excessive = Vec::new();
    for orbs in [0, 10_000, 100_000] {
        for players in [1, 100, 1000] {
            for tick in [1, 20] {
                let (mut state, first, pos) = crate::hostile_tests::setup(8.0);
                let joins: Vec<_> = (2..=players)
                    .map(|n| PlayerInput::Join {
                        connection: ConnectionId(n),
                        uuid: uuid::Uuid::from_u128(n as u128),
                        name: format!("p{n}"),
                        permission_level: 0,
                    })
                    .collect();
                crate::tick::run_tick(&mut state, &joins);
                assert_eq!(state.players.len(), players as usize);
                // No local players: the control must reject distant bodies without
                // visiting all of them for each orb. The first player is distant too.
                let ids: Vec<_> = state.players.keys().copied().collect();
                assert!(ids.contains(&first));
                for (n, id) in ids.into_iter().enumerate() {
                    let slot = state.entities.slot_of(id).unwrap();
                    state
                        .entities
                        .set_pos(slot, [4096.0 + n as f64 * 32.0, pos[1], 4096.0]);
                }
                populate(&mut state, pos, orbs);
                state.tick = tick;
                let work = OrbWork::default();
                run_inner(
                    &mut state,
                    &mut ChangeSet::default(),
                    &mut Vec::new(),
                    None,
                    &work,
                );
                let visits = work.orbs.get() + work.players.get();
                println!(
                    "H08 O={orbs} P={players} tick={tick} orb={} player={} total={visits}",
                    work.orbs.get(),
                    work.players.get()
                );
                let xp: i64 = state
                    .entities
                    .experience_orbs()
                    .map(|id| {
                        let slot = state.entities.slot_of(id).unwrap();
                        let (value, count) =
                            state.entities.row(slot).appearance.experience_orb.unwrap();
                        i64::from(value) * i64::from(count)
                    })
                    .sum();
                assert_eq!(xp, 3 * orbs as i64);
                if visits > 8 * orbs as u64 + 2 * players {
                    excessive.push((orbs, players, tick, visits));
                }
            }
        }
    }
    assert!(
        excessive.is_empty(),
        "non-local or deleted bodies revisited: {excessive:?}"
    );
}

#[test]
fn moving_colocated_pile_does_not_rescan_unmoved_or_deleted_members() {
    let (mut state, player, pos) = crate::hostile_tests::setup(8.0);
    let player_slot = state.entities.slot_of(player).unwrap();
    state
        .entities
        .set_pos(player_slot, [4096.0, pos[1], 4096.0]);
    populate(&mut state, pos, 10_000);
    let ids: Vec<_> = state.entities.experience_orbs().collect();
    for id in ids {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_vel(slot, [40.0, 0.0, 0.0]);
    }
    state.tick = 20;
    let work = OrbWork::default();
    run_inner(
        &mut state,
        &mut ChangeSet::default(),
        &mut Vec::new(),
        None,
        &work,
    );
    assert_eq!(state.entities.experience_orbs().count(), 40);
    let count: i32 = state
        .entities
        .experience_orbs()
        .map(|id| {
            state
                .entities
                .row(state.entities.slot_of(id).unwrap())
                .appearance
                .experience_orb
                .unwrap()
                .1
        })
        .sum();
    assert_eq!(count, 10_000);
    println!(
        "H08 moving-pile orb={} player={}",
        work.orbs.get(),
        work.players.get()
    );
    assert!(work.orbs.get() + work.players.get() < 100_000);
}

#[test]
fn merge_keeps_original_coarse_cell_admission_and_candidate_order() {
    let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
    populate(&mut state, pos, 121);
    let ids: Vec<_> = state.entities.experience_orbs().collect();
    // Same merge group; two candidates start in different coarse cells.
    let owner = ids[0];
    for (id, dx) in [(ids[40], 4.0), (ids[80], -0.5), (ids[120], 12.0)] {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, [pos[0] + dx, pos[1], pos[2]]);
    }
    // The owner travels to the +4 candidate; the original +12 bucket
    // remains outside the old coarse-cell admission window.
    let slot = state.entities.slot_of(owner).unwrap();
    state.entities.set_vel(slot, [80.0, 0.0, 0.0]);
    state.tick = 20;
    let mut changes = ChangeSet::default();
    run(&mut state, &mut changes, &mut Vec::new());
    assert!(changes.entities_removed.contains(&ids[40]));
    assert!(state.entities.slot_of(ids[120]).is_some());
    let slot = state.entities.slot_of(owner).unwrap();
    assert_eq!(
        state.entities.row(slot).appearance.experience_orb,
        Some((3, 2))
    );
}

#[test]
fn fine_index_preserves_old_cell_fence_and_cell_then_id_order_after_motion() {
    let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
    populate(&mut state, pos, 121);
    let ids: Vec<_> = state.entities.experience_orbs().collect();
    let mut index = MergeCells::default();
    let at = [4.0, pos[1], 4.0];
    for (id, x) in [(ids[40], 5.0), (ids[80], 3.5), (ids[120], 12.0)] {
        let before = [x, pos[1], 4.0];
        index.insert(id, 3, before);
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, at);
        index.moved(id, 3, before, at);
    }
    let mut nearby = Vec::new();
    index.nearby(&state, ids[0], 3, at, &mut nearby, &OrbWork::default());
    assert_eq!(nearby, vec![ids[80], ids[40]]);
}
