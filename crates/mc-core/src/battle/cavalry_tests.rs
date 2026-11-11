use super::super::combat::tests::{fixture, index, prepare};
use super::*;
use crate::boundary::ChangeSet;
use crate::mounts::tests::{slot, spawn};

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
    let rider_slot = slot(&state, rider);
    state.entities.set_faction(rider_slot, 1);
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
    let boat = spawn(&mut restored, "oak_boat", pos);
    let boat_row = restored.entities.row(slot(&restored, boat));
    let vehicle = boat_row
        .entity_life
        .vehicle
        .clone()
        .expect("native boat state");
    let boat_type = restored
        .registry
        .entity_type_id("minecraft:oak_boat")
        .unwrap();
    let item_type = restored.registry.entity_type_id("minecraft:item").unwrap();
    let stone = restored
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    for case in ["boat", "item", "dead", "zero_health", "negative_health"] {
        let mut corrupt = Replay {
            entities: replay.entities.clone(),
            ..Default::default()
        };
        let passenger = corrupt.entities.get_mut(&rider.0).unwrap();
        match case {
            "boat" => {
                passenger.type_id = boat_type;
                passenger.state.entity_life.vehicle = Some(vehicle.clone());
            }
            "item" => {
                passenger.type_id = item_type;
                passenger.item = Some(mc_data::ItemStack::new(stone, 1));
            }
            "dead" => passenger.alive = false,
            "zero_health" => passenger.health = 0.0,
            "negative_health" => passenger.health = -1.0,
            _ => unreachable!(),
        }
        assert!(
            crate::mounts::persistence::validate_replay(&corrupt).is_err(),
            "{case} rider must fail recovery"
        );
    }
}

#[test]
fn mounted_kernel_veers_away_from_steady_front_and_commits_to_wavering_front() {
    let (mut state, _, pos) = fixture();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    let target_pos = [pos[0] + 5.0, pos[1], pos[2]];
    let target = spawn(&mut state, "husk", target_pos);
    for (id, faction) in [(rider, 1), (target, 2)] {
        let s = slot(&state, id);
        state.entities.set_faction(s, faction);
    }
    mount(&mut state, rider, horse).unwrap();
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
                frontage: 1.0,
                order: super::super::formation::Order::Hold,
            },
            &[target],
        )
        .unwrap();
    prepare(&state, &mut battle, 0);
    let rider_bi = index(&state, &battle, rider);
    let horse_bi = index(&state, &battle, horse);
    battle.columns.targets[rider_bi] = Some(super::super::EnemyTarget {
        id: target,
        pos: target_pos,
        distance: 5.0,
    });
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    let mut combat = CombatRuntime::default();
    combat.prepare(&battle.columns.ids);
    step(&frame, &mut battle, &mut combat);
    assert_eq!(combat.work.charge_checks, 1);
    assert!(
        battle.columns.drive[horse_bi][0] < 0.0,
        "refusal must not move through the line"
    );
    assert_ne!(
        battle.columns.drive[horse_bi][2], 0.0,
        "horse must run out to a flank"
    );
    assert_eq!(combat.fighters[rider_bi].charge.bonus(0), 0.0);
    battle.formation.regiments[0].morale.value = 20.0;
    battle.formation.members[0].wavering = true;
    let mut combat = CombatRuntime::default();
    combat.prepare(&battle.columns.ids);
    step(&frame, &mut battle, &mut combat);
    assert_eq!(combat.work.charge_checks, 1);
    assert!(battle.columns.drive[horse_bi][0] > 0.0);
    assert_eq!(combat.fighters[rider_bi].charge.bonus(0), 2.0);
    assert_eq!(combat.fighters[rider_bi].charge.bonus(260), 0.0);
}

#[test]
fn mounted_speed_and_charge_remain_inside_loaded_bounds() {
    let (mut state, _, pos) = fixture();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    let rider_slot = slot(&state, rider);
    state.entities.set_faction(rider_slot, 1);
    mount(&mut state, rider, horse).unwrap();
    let target = spawn(&mut state, "husk", [pos[0] + 5.0, pos[1], pos[2]]);
    let target_slot = slot(&state, target);
    state.entities.set_faction(target_slot, 2);
    let mut battle = BattleState::default();
    battle
        .formation
        .set_bounds([pos[0] - 1.0, pos[2] - 1.0], [pos[0] + 0.01, pos[2] + 1.0])
        .unwrap();
    prepare(&state, &mut battle, 0);
    let rider_bi = index(&state, &battle, rider);
    let horse_bi = index(&state, &battle, horse);
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    for targeted in [false, true] {
        if targeted {
            battle.columns.targets[rider_bi] = Some(super::super::EnemyTarget {
                id: target,
                pos: [pos[0] + 5.0, pos[1], pos[2]],
                distance: 5.0,
            });
        }
        battle.columns.drive[rider_bi] = [0.02, 0.0, 0.0];
        let mut combat = CombatRuntime::default();
        combat.prepare(&battle.columns.ids);
        step(&frame, &mut battle, &mut combat);
        assert!(battle.columns.drive[horse_bi][0] > 0.0);
        assert!(
            pos[0] + battle.columns.drive[horse_bi][0] * mc_physics::TICK_SECONDS <= pos[0] + 0.01
        );
    }
}

#[test]
fn mounted_battle_horse_moves_toward_enemy_during_real_ticks() {
    use super::super::formation::{Order, RegimentSpec};
    let (mut state, player, pos) = fixture();
    let player_slot = slot(&state, player);
    state
        .entities
        .set_pos(player_slot, [pos[0], pos[1], pos[2] + 6.0]);
    let stone = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
        .default
        .0;
    for x in pos[0] as i32 - 2..=pos[0] as i32 + 32 {
        for z in pos[2] as i32 - 2..=pos[2] as i32 + 2 {
            state.world.set_block(x, pos[1] as i32 - 1, z, stone);
            for dy in 0..4 {
                state
                    .world
                    .set_block(x, pos[1] as i32 + dy, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    let horse = spawn(&mut state, "horse", pos);
    let rider = spawn(&mut state, "husk", pos);
    assert!(
        mount(&mut state, rider, horse).is_err(),
        "faction zero is not a battle rider"
    );
    assert!(
        state
            .entities
            .row(slot(&state, horse))
            .entity_life
            .mount
            .rider
            .is_none()
    );
    let rider_slot = slot(&state, rider);
    state.entities.set_faction(rider_slot, 1);
    mount(&mut state, rider, horse).unwrap();
    assert_eq!(state.entities.body(slot(&state, horse)).faction, 1);
    let target_pos = [pos[0] + 30.0, pos[1], pos[2]];
    let target = spawn(&mut state, "husk", target_pos);
    let target_slot = slot(&state, target);
    state.entities.set_faction(target_slot, 2);
    for (id, faction, origin, facing, order, soldier) in [
        (1, 1, pos, [1.0, 0.0], Order::Advance, rider),
        (2, 2, target_pos, [-1.0, 0.0], Order::Hold, target),
    ] {
        state
            .hostile
            .battle
            .formation
            .add_regiment(
                RegimentSpec {
                    id,
                    faction,
                    origin,
                    facing,
                    files: 1,
                    frontage: 1.0,
                    order,
                },
                &[soldier],
            )
            .unwrap();
    }
    state.hostile.battle.running = true;
    // The formation anchor advances 0.066 blocks/tick; the horse must follow
    // it rather than run ahead at its free charge speed.
    for tick in 0..40 {
        crate::tick::run_tick(&mut state, &[]);
        assert_pair(&state, rider, horse);
        if tick == 19 {
            assert!(state.entities.pos(slot(&state, horse))[0] > pos[0] + 0.5);
        }
    }
    let after = state.entities.pos(slot(&state, horse));
    assert!(
        after[0] > pos[0] + 2.0,
        "ridden horse must physically advance, before={pos:?}, after={after:?}"
    );
    assert!((target_pos[0] - after[0]).abs() < target_pos[0] - pos[0]);
}

#[test]
fn flank_heading_swings_wide_then_behind_then_into_the_rear() {
    // Enemy front centred at the origin facing +z, 46 files, 11 ranks.
    let flank = Flank {
        origin: [0.0, 64.0, 0.0],
        facing: [0.0, 1.0],
        side: [-1.0, 0.0],
        half_width: 22.5,
        depth: 20.0,
    };
    let step = |pos: [f64; 3]| {
        let h = flank.heading(pos);
        let len = (h[0] * h[0] + h[1] * h[1]).sqrt();
        [h[0] / len, h[1] / len]
    };
    // In front of the line: mostly outward along the chosen side.
    let out = step([-10.0, 64.0, 40.0]);
    assert!(out[0] < -0.5, "{out:?}");
    // Past the end file: round behind the last rank.
    let around = step([-40.0, 64.0, 10.0]);
    assert!(around[1] < -0.5, "{around:?}");
    // Behind the last rank: turn in towards the rear half, not the front.
    let rear = step([-35.0, 64.0, -30.0]);
    assert!(rear[0] > 0.5 && rear[1] > 0.0, "{rear:?}");
}

#[test]
fn riders_swing_round_once_their_foot_closes_and_ride_down_a_rout() {
    use super::super::formation::{Order, RegimentSpec};
    let mut battle = BattleState::default();
    let foot = |id, faction, z, facing: f64| RegimentSpec {
        id,
        faction,
        origin: [0.0, 64.0, z],
        facing: [0.0, facing],
        files: 10,
        frontage: 1.0,
        order: Order::Advance,
    };
    let ids = |first: i32, n: i32| (first..first + n).map(EntityId).collect::<Vec<_>>();
    battle
        .formation
        .add_regiment(foot(0, 1, 0.0, 1.0), &ids(1, 30))
        .unwrap();
    battle
        .formation
        .add_regiment(foot(1, 2, 4.0, -1.0), &ids(101, 30))
        .unwrap();
    let mut horse = foot(2, 1, -30.0, 1.0);
    horse.origin[0] = -8.0;
    horse.frontage = 2.0;
    battle.formation.add_regiment(horse, &ids(201, 10)).unwrap();
    for r in &mut battle.formation.regiments {
        r.live_count = r.len;
    }
    battle.columns.ranged = vec![false; 70];
    let mut plans = Vec::new();
    plan_sweeps(&battle, &mut plans);
    assert!(plans[2].is_none(), "riders left before the foot closed");
    battle.formation.regiments[0].enemy = Some([0.0, 64.0, 4.0]);
    battle.formation.regiments[0].gap = 4.0;
    plan_sweeps(&battle, &mut plans);
    let Some(Sweep::Flank(plan)) = plans[2] else {
        panic!("riders swing round once the foot closes: {:?}", plans[2]);
    };
    assert_eq!(plan.origin, [0.0, 64.0, 4.0]);
    // The riders sit on the enemy's -x side: they keep to that flank.
    assert!(plan.side[0] < 0.0, "{plan:?}");
    assert!(plans[0].is_none() && plans[1].is_none());
    // Once that foot breaks, the riders ride down its fugitives instead.
    battle.formation.regiments[1].morale.broken = true;
    battle.formation.regiments[1].spec.origin = [3.0, 64.0, 40.0];
    plan_sweeps(&battle, &mut plans);
    assert_eq!(plans[2], Some(Sweep::Chase([3.0, 64.0, 40.0])));
}

/// Eight riders (4 files x 2 ranks) swing round an enemy foot block: their
/// own foot is 20 blocks from the enemy's front, so it is closing, and the
/// flank point (-1, 28) lies past a U of stone walls open toward the riders
/// (back wall z = 8, x -15..1), so relief deflection holds every rider in
/// that pocket. Riders start in their slots, or all on one point inside the
/// pocket (`piled`). Returns the horse positions after `ticks` real ticks.
fn sweep_into_pocket(piled: bool, ticks: usize) -> Vec<[f64; 3]> {
    use super::super::formation::{Order, RegimentSpec, slot_position};
    let (mut state, _, pos) = fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (px, py, pz) = (pos[0] as i32, pos[1] as i32, pos[2] as i32);
    for x in px - 20..=px + 12 {
        for z in pz - 14..=pz + 32 {
            state.world.set_block(x, py - 1, z, stone);
            for dy in 0..6 {
                state.world.set_block(x, py + dy, z, air);
            }
        }
    }
    for dy in 0..4 {
        for x in px - 15..=px + 1 {
            state.world.set_block(x, py + dy, pz + 8, stone);
        }
        for z in pz + 2..=pz + 8 {
            state.world.set_block(px - 15, py + dy, z, stone);
            state.world.set_block(px + 1, py + dy, z, stone);
        }
    }
    state.world.fold_overlay();
    let at = |dx: f64, dz: f64| [pos[0] + dx, pos[1], pos[2] + dz];
    let foot = |id, faction, origin, facing| RegimentSpec {
        id,
        faction,
        origin,
        facing,
        files: 1,
        frontage: 1.0,
        order: Order::Hold,
    };
    for (id, faction, origin, facing) in [
        (1, 1, at(6.0, 0.0), [0.0, 1.0]),
        (2, 2, at(6.0, 20.0), [0.0, -1.0]),
    ] {
        let soldier = spawn(&mut state, "husk", origin);
        state.entities.set_faction(slot(&state, soldier), faction);
        state
            .hostile
            .battle
            .formation
            .add_regiment(foot(id, faction, origin, facing), &[soldier])
            .unwrap();
    }
    let cavalry = RegimentSpec {
        id: 3,
        faction: 1,
        origin: at(-7.0, -6.0),
        facing: [0.0, 1.0],
        files: 4,
        frontage: 2.0,
        order: Order::Hold,
    };
    let mut riders = Vec::new();
    for i in 0..8 {
        let p = if piled {
            at(-7.0, 5.0)
        } else {
            slot_position(&cavalry, i)
        };
        let horse = spawn(&mut state, "horse", p);
        let rider = spawn(&mut state, "husk", p);
        state.entities.set_faction(slot(&state, rider), 1);
        mount(&mut state, rider, horse).unwrap();
        riders.push((rider, horse));
    }
    let ids: Vec<_> = riders.iter().map(|&(rider, _)| rider).collect();
    state
        .hostile
        .battle
        .formation
        .add_regiment(cavalry, &ids)
        .unwrap();
    state.hostile.battle.running = true;
    for _ in 0..ticks {
        crate::tick::run_tick(&mut state, &[]);
    }
    let plan = state.hostile.battle.combat.sweeps.get(2).copied().flatten();
    assert!(
        matches!(plan, Some(Sweep::Flank(_))),
        "riders must be swinging round the enemy's end: {plan:?}"
    );
    let horses: Vec<[f64; 3]> = riders
        .iter()
        .map(|&(rider, horse)| {
            assert_pair(&state, rider, horse);
            state.entities.pos(slot(&state, horse))
        })
        .collect();
    // The terrain held them: all inside the pocket, short of the point.
    assert!(
        horses.iter().all(|p| p[2] < pos[2] + 8.0),
        "riders left the pocket: {horses:?}"
    );
    horses
}

/// (x span, closest pair) of horse positions.
fn spread(horses: &[[f64; 3]]) -> (f64, f64) {
    let span = horses
        .iter()
        .map(|p| p[0])
        .fold(f64::NEG_INFINITY, f64::max)
        - horses.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let closest = horses
        .iter()
        .enumerate()
        .flat_map(|(i, a)| {
            horses[i + 1..]
                .iter()
                .map(move |b| mth::length2(a[0] - b[0], a[2] - b[2]))
        })
        .fold(f64::INFINITY, f64::min);
    (span, closest)
}

/// Riders that share one flank point converge on it while the pocket holds
/// them (the G1 rehearsal's 457-rider stack); riders that ride the plan from
/// their own slots keep the block's frontage. Pressed against the back wall
/// the two ranks close up, but no two riders share a spot.
#[test]
fn sweeping_riders_keep_their_spacing_in_a_concave_pocket() {
    let horses = sweep_into_pocket(false, 600);
    let (span, closest) = spread(&horses);
    assert!(
        span >= 5.0 && closest >= 0.3,
        "riders piled up: x span {span:.2}, closest pair {closest:.2}: {horses:?}"
    );
}

/// A pile (riders on one point, as the rehearsal stack after t 80) spreads
/// to the block's frontage again; with one shared point every rider of the
/// pile got the same drive and the pile rode on as one body.
#[test]
fn riders_piled_in_a_pocket_spread_again() {
    let horses = sweep_into_pocket(true, 300);
    let (span, closest) = spread(&horses);
    assert!(
        span >= 4.0 && closest >= 0.3,
        "the pile stayed together: x span {span:.2}, closest pair {closest:.2}: {horses:?}"
    );
}
