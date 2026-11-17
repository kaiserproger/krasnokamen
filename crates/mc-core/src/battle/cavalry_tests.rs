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
        state.entities.row(h).entity_life.mount.riders,
        [state.entities.row(r).uuid]
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
    let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
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
        .riders = vec![rider_uuid];
    assert!(crate::mounts::persistence::validate_replay(&corrupt).is_err());
    let horse_uuid = restored.entities.row(slot(&restored, horse)).uuid;
    corrupt
        .entities
        .get_mut(&rider.0)
        .unwrap()
        .state
        .entity_life
        .mount
        .riders = vec![horse_uuid];
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
            .riders
            .is_empty()
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
    // Every soldier stands on his leader's line: live centroids = origins.
    for r in &mut battle.formation.regiments {
        r.live_count = r.len;
        r.live_sum = r.spec.origin.map(|x| x * r.len as f64);
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

/// On relief a regiment's virtual leader marches on while its files lag
/// behind slopes and trees: the swing goes round where the enemy's men stand
/// (their live centroid), not round the leader's empty line ahead of them.
#[test]
fn the_swing_goes_round_the_enemy_men_not_their_leader_line() {
    use super::super::formation::{Order, RegimentSpec};
    let mut battle = BattleState::default();
    let regiment = |id, faction, x: f64, z: f64, facing: f64, frontage| RegimentSpec {
        id,
        faction,
        origin: [x, 64.0, z],
        facing: [0.0, facing],
        files: 10,
        frontage,
        order: Order::Advance,
    };
    let ids = |first: i32, n: i32| (first..first + n).map(EntityId).collect::<Vec<_>>();
    for (spec, first) in [
        (regiment(0, 1, 0.0, 0.0, 1.0, 1.0), 1),
        (regiment(1, 2, 0.0, 4.0, -1.0, 1.0), 101),
        (regiment(2, 1, -8.0, -30.0, 1.0, 2.0), 201),
    ] {
        battle
            .formation
            .add_regiment(spec, &ids(first, 10))
            .unwrap();
    }
    for r in &mut battle.formation.regiments {
        r.live_count = r.len;
        r.live_sum = r.spec.origin.map(|x| x * r.len as f64);
    }
    // The enemy's leader has marched 12 blocks past its men.
    let enemy = &mut battle.formation.regiments[1];
    let men = [6.0, 70.0, 16.0];
    enemy.live_sum = men.map(|x| x * enemy.len as f64);
    battle.columns.ranged = vec![false; 30];
    battle.formation.regiments[0].enemy = Some([0.0, 64.0, 4.0]);
    battle.formation.regiments[0].gap = 4.0;
    let mut plans = Vec::new();
    plan_sweeps(&battle, &mut plans);
    let Some(Sweep::Flank(plan)) = plans[2] else {
        panic!("riders swing round once the foot closes: {:?}", plans[2]);
    };
    assert_eq!(
        plan.origin, men,
        "the swing is laid round the leader's empty line"
    );
}

/// Riders ride down a broken foot regiment's fugitives until they reach the
/// edge of the field; then the chase ends and the riders swing round the
/// enemy foot still fighting. Chasers stood on the enemy's rear edge for 100 s
/// in the G1 rehearsal.
#[test]
fn a_chase_ends_when_the_fugitives_reach_the_edge() {
    use super::super::formation::{Order, RegimentSpec};
    let mut battle = BattleState::default();
    battle
        .formation
        .set_bounds([-100.0, -100.0], [100.0, 100.0])
        .unwrap();
    let regiment = |id, faction, x: f64, z: f64, facing: f64, frontage| RegimentSpec {
        id,
        faction,
        origin: [x, 64.0, z],
        facing: [0.0, facing],
        files: 10,
        frontage,
        order: Order::Advance,
    };
    let ids = |first: i32, n: i32| (first..first + n).map(EntityId).collect::<Vec<_>>();
    for (spec, first) in [
        (regiment(0, 1, 0.0, 0.0, 1.0, 1.0), 1),
        (regiment(1, 2, 0.0, 4.0, -1.0, 1.0), 101),
        (regiment(2, 1, -8.0, -30.0, 1.0, 2.0), 201),
        (regiment(3, 2, 30.0, 40.0, -1.0, 1.0), 301),
    ] {
        battle
            .formation
            .add_regiment(spec, &ids(first, 10))
            .unwrap();
    }
    for r in &mut battle.formation.regiments {
        r.live_count = r.len;
        r.live_sum = r.spec.origin.map(|x| x * r.len as f64);
    }
    battle.columns.ranged = vec![false; 40];
    battle.formation.regiments[0].enemy = Some([0.0, 64.0, 4.0]);
    battle.formation.regiments[0].gap = 4.0;
    battle.formation.regiments[3].morale.broken = true;
    let mut plans = Vec::new();
    plan_sweeps(&battle, &mut plans);
    assert_eq!(plans[2], Some(Sweep::Chase([30.0, 64.0, 40.0])));
    // The fugitives have fled to the far edge.
    let fled = &mut battle.formation.regiments[3];
    fled.spec.origin = [30.0, 64.0, 97.0];
    fled.live_sum = fled.spec.origin.map(|x| x * fled.len as f64);
    plan_sweeps(&battle, &mut plans);
    let Some(Sweep::Flank(plan)) = plans[2] else {
        panic!(
            "the riders stay on the fugitives at the edge: {:?}",
            plans[2]
        );
    };
    assert_eq!(plan.origin, [0.0, 64.0, 4.0]);
    // A broken mounted regiment keeps its plan for its rallied riders.
    battle.formation.regiments[2].morale.broken = true;
    plan_sweeps(&battle, &mut plans);
    assert_eq!(plans[2], Some(Sweep::Flank(plan)));
}

/// Eight riders (4 files x 2 ranks) swing round an enemy foot block: their
/// own foot is 20 blocks from the enemy's front, so it is closing, and the
/// flank point (-1, 28) lies past a U of stone walls open toward the riders
/// (back wall z = 8, x -15..1), so relief deflection holds every rider in
/// that pocket. Returns the horse positions after `ticks` real ticks.
fn sweep_into_pocket(ticks: usize) -> Vec<[f64; 3]> {
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
        let p = slot_position(&cavalry, i);
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
    let horses = sweep_into_pocket(600);
    let (span, closest) = spread(&horses);
    assert!(
        span >= 5.0 && closest >= 0.3,
        "riders piled up: x span {span:.2}, closest pair {closest:.2}: {horses:?}"
    );
}

/// Open ground, no obstacle within 15 blocks: riders chase a broken foot
/// regiment whose two fugitives are boxed in 36 blocks apart, so its live
/// centroid, the chase point, is empty grass with no enemy in reach (the
/// integration take's stacks of 80 and 64 horses on open ground). Riders
/// that share the point stand on one block; riders that ride the chase from
/// their own slots stand as a block with the regiment's frontage.
#[test]
fn chasing_riders_hold_their_frontage_on_open_ground() {
    use super::super::formation::{Order, RegimentSpec, slot_position};
    let (mut state, _, pos) = fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (px, py, pz) = (pos[0] as i32, pos[1] as i32, pos[2] as i32);
    for x in px - 24..=px + 24 {
        for z in pz - 14..=pz + 30 {
            state.world.set_block(x, py - 1, z, stone);
            for dy in 0..6 {
                state.world.set_block(x, py + dy, z, air);
            }
        }
    }
    // A one-block cell, walls three high, for each fugitive.
    for cx in [px - 18, px + 18] {
        for dx in -1..=1 {
            for dz in -1..=1 {
                if (dx, dz) != (0, 0) {
                    for dy in 0..3 {
                        state.world.set_block(cx + dx, py + dy, pz + 20 + dz, stone);
                    }
                }
            }
        }
    }
    state.world.fold_overlay();
    let at = |dx: f64, dz: f64| [pos[0] + dx, pos[1], pos[2] + dz];
    let fugitives: Vec<_> = [at(-18.0, 20.0), at(18.0, 20.0)]
        .into_iter()
        .map(|p| {
            let id = spawn(&mut state, "husk", p);
            state.entities.set_faction(slot(&state, id), 2);
            id
        })
        .collect();
    let battle = &mut state.hostile.battle;
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 1,
                faction: 2,
                origin: at(0.0, 20.0),
                facing: [0.0, -1.0],
                files: 2,
                frontage: 1.0,
                order: Order::Hold,
            },
            &fugitives,
        )
        .unwrap();
    battle.formation.regiments[0].morale.broken = true;
    let cavalry = RegimentSpec {
        id: 2,
        faction: 1,
        origin: at(0.0, -6.0),
        facing: [0.0, 1.0],
        files: 4,
        frontage: 2.0,
        order: Order::Hold,
    };
    let mut riders = Vec::new();
    for i in 0..8 {
        let p = slot_position(&cavalry, i);
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
    for _ in 0..300 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let plan = state.hostile.battle.combat.sweeps.get(1).copied().flatten();
    assert!(
        matches!(plan, Some(Sweep::Chase(point)) if (point[0] - pos[0]).abs() < 1.0),
        "riders must chase the fugitives' empty centroid: {plan:?}"
    );
    let horses: Vec<[f64; 3]> = riders
        .iter()
        .map(|&(rider, horse)| {
            assert_pair(&state, rider, horse);
            state.entities.pos(slot(&state, horse))
        })
        .collect();
    // They arrived: the block stands round the chase point, no enemy in reach.
    assert!(
        horses
            .iter()
            .all(|p| mth::length2(p[0] - pos[0], p[2] - pos[2] - 20.0) < 8.0),
        "riders did not reach the chase point: {horses:?}"
    );
    let (span, closest) = spread(&horses);
    assert!(
        span >= 5.0 && closest >= 0.5,
        "riders piled up on open ground: x span {span:.2}, closest pair {closest:.2}: {horses:?}"
    );
}

/// One battle tick's steering and cavalry passes on a local fixture (open
/// stone floor, no bounds) for [`sweep_pass`].
struct SweepPass {
    plan: Option<Sweep>,
    /// The rider's drive after steering (his foot drive) and after cavalry.
    foot: [f64; 3],
    rider: [f64; 3],
    /// The horse's drive after cavalry.
    horse: [f64; 3],
    /// What the mounted pass makes of the plan for a seated, steady, untargeted
    /// rider alone on open ground: the heading from his leader point at 4.6
    /// blocks per second.
    ridden: [f64; 3],
    rider_pos: [f64; 3],
    /// The rider's identity and whether the cavalry pass recorded him as an
    /// at-risk exposure (an unseated member of a planned regiment that passed
    /// every other `sweep_drives` guard).
    rider_id: EntityId,
    exposed: bool,
    /// The `Battle exposure` lines the recorded episode produced (episode, then
    /// summary), captured after the fixture's halt.
    exposure: Vec<String>,
}

/// A mounted regiment of one rider at its slot (x -6, z -8) and enemy foot
/// whose leader line is about 40 blocks away (x 0, z 32): broken (a Chase) or,
/// with `flank`, unbroken and closing with the riders' own foot (a Flank).
/// With `dismounted` the rider is first mounted and then taken off his horse;
/// `leave_vehicle` sets him 1.5 blocks beside it, so his own destination, his
/// slot, is not where he stands.
fn sweep_pass(flank: bool, dismounted: bool) -> SweepPass {
    sweep_pass_ticks(flank, dismounted, false)
}

/// The same pass with the exposure tracker disabled, for the equality control.
fn sweep_pass_unrecorded(flank: bool, dismounted: bool, closing: bool) -> SweepPass {
    sweep_pass_with(flank, dismounted, closing, false)
}

/// With `closing`, a second battle tick: the body moves 3 blocks (the movement
/// its exposed tick causes) and a target ends the episode, which is what makes
/// the episode's closing sample observable.
fn sweep_pass_ticks(flank: bool, dismounted: bool, closing: bool) -> SweepPass {
    sweep_pass_with(flank, dismounted, closing, true)
}

fn sweep_pass_with(flank: bool, dismounted: bool, closing: bool, record: bool) -> SweepPass {
    use super::super::formation::{Order, RegimentSpec, slot_position};
    let (mut state, _, pos) = fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (px, py, pz) = (pos[0] as i32, pos[1] as i32, pos[2] as i32);
    for x in px - 12..=px + 12 {
        for z in pz - 14..=pz + 32 {
            state.world.set_block(x, py - 1, z, stone);
            for dy in 0..6 {
                state.world.set_block(x, py + dy, z, air);
            }
        }
    }
    state.world.fold_overlay();
    let at = |dx: f64, dz: f64| [pos[0] + dx, pos[1], pos[2] + dz];
    let mut battle = BattleState::default();
    for (id, faction, origin, facing) in [
        (1, 2, at(0.0, 32.0), [0.0, -1.0]),
        (2, 1, at(10.0, 20.0), [0.0, 1.0]),
    ] {
        let soldier = spawn(&mut state, "husk", origin);
        state.entities.set_faction(slot(&state, soldier), faction);
        let spec = RegimentSpec {
            id,
            faction,
            origin,
            facing,
            files: 1,
            frontage: 1.0,
            order: Order::Hold,
        };
        battle.formation.add_regiment(spec, &[soldier]).unwrap();
    }
    let cavalry = RegimentSpec {
        id: 3,
        faction: 1,
        origin: at(-6.0, -8.0),
        facing: [0.0, 1.0],
        files: 1,
        frontage: 2.0,
        order: Order::Hold,
    };
    let (origin, place) = (cavalry.origin, slot_position(&cavalry, 0));
    let horse = spawn(&mut state, "horse", place);
    let rider = spawn(&mut state, "husk", place);
    state.entities.set_faction(slot(&state, rider), 1);
    mount(&mut state, rider, horse).unwrap();
    battle.formation.add_regiment(cavalry, &[rider]).unwrap();
    if dismounted {
        crate::mounts::dismount(&mut state, rider);
    }
    let seat = state
        .entities
        .row(slot(&state, rider))
        .entity_life
        .mount
        .vehicle;
    assert_eq!(seat.is_some(), !dismounted);
    prepare(&state, &mut battle, 0);
    if flank {
        // The riders' own foot closes with the enemy foot.
        battle.formation.regiments[1].enemy = Some(at(0.0, 32.0));
        battle.formation.regiments[1].gap = 4.0;
    } else {
        battle.formation.regiments[0].morale.broken = true;
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    let rider_bi = index(&state, &battle, rider);
    let horse_bi = index(&state, &battle, horse);
    let m = &battle.formation.members[rider_bi];
    assert!(m.alive && !m.wounded && !m.routing);
    assert!(battle.columns.active[rider_bi] && battle.columns.targets[rider_bi].is_none());
    // The battle step of a tick is what feeds the tracker its condition tick.
    battle.exposure.set_enabled(record);
    battle.exposure.begin(100);
    super::super::steering::step(&frame, &mut battle, &mut super::super::Output::default());
    let foot = battle.columns.drive[rider_bi];
    let mut combat = CombatRuntime::default();
    combat.prepare(&battle.columns.ids);
    step(&frame, &mut battle, &mut combat);
    let plan = combat.sweeps[2];
    let rider_pos = battle.columns.positions[rider_bi];
    let ridden = plan.map_or([0.0; 3], |plan| {
        let heading = plan.heading([
            rider_pos[0] - (place[0] - origin[0]),
            rider_pos[1],
            rider_pos[2] - (place[2] - origin[2]),
        ]);
        let length = mth::length2(heading[0], heading[1]);
        [heading[0] / length * 4.6, 0.0, heading[1] / length * 4.6]
    });
    let exposed = battle.exposure.is_exposed(rider);
    if closing {
        // The exposed tick's own movement, then a guard that ends the episode:
        // the wire would carry this step under the next published tick.
        let rider_slot = slot(&state, rider);
        let start = state.entities.pos(rider_slot);
        let moved = [start[0] + 3.0, start[1], start[2]];
        state.entities.set_pos(rider_slot, moved);
        prepare(&state, &mut battle, 1);
        let rider_bi = index(&state, &battle, rider);
        battle.columns.targets[rider_bi] = Some(super::super::targeting::EnemyTarget {
            id: EntityId(-2),
            pos: [moved[0] + 2.0, moved[1], moved[2]],
            distance: 2.0,
        });
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick: 1,
        };
        battle.exposure.begin(101);
        super::super::steering::step(&frame, &mut battle, &mut super::super::Output::default());
        step(&frame, &mut battle, &mut combat);
    }
    // a real boundary sample, as pause/reset/shutdown take it
    let sample = super::super::exposure::Sample {
        entities: &state.entities,
        formation: &battle.formation,
        columns: &battle.columns,
    };
    battle.exposure.halt(Some(&sample));
    let exposure = battle.exposure.lines().to_vec();
    SweepPass {
        plan,
        foot,
        rider: battle.columns.drive[rider_bi],
        horse: battle.columns.drive[horse_bi],
        ridden,
        rider_pos,
        rider_id: rider,
        exposed,
        exposure,
    }
}

/// A rider whose horse is gone stays in his mounted regiment. The sweep
/// heading is a displacement in blocks that only the mounted pass turns into a
/// horse speed; as the dismounted rider's own drive (blocks per second) it
/// skated him a twentieth of the way to the plan point every tick, 45 to 107
/// blocks per second in the field-1k battle takes. He keeps exactly the foot
/// drive steering gave him (an upper bound alone would pass a zeroed drive);
/// on the same fixture a seated rider still rides the plan.
fn assert_walker_keeps_his_foot_drive(walk: &SweepPass, ride: &SweepPass) {
    assert!(
        mth::length2(walk.foot[0], walk.foot[2]) > 0.5,
        "the walker's slot is 1.5 blocks off: steering must give him a foot drive: {:?}",
        walk.foot
    );
    assert_eq!(
        walk.rider.map(f64::to_bits),
        walk.foot.map(f64::to_bits),
        "the cavalry pass must keep the walker's foot drive: {:?} -> {:?}",
        walk.foot,
        walk.rider
    );
    assert!(mth::length2(walk.rider[0], walk.rider[2]) <= 5.6);
    assert_eq!(
        ride.rider, [0.0; 3],
        "a seated rider's drive goes to his horse"
    );
    assert!(
        (0..3).all(|k| (ride.horse[k] - ride.ridden[k]).abs() < 1.0e-9),
        "the horse must ride the plan at the steady 4.6 blocks/s: {:?} vs {:?}",
        ride.horse,
        ride.ridden
    );
}

#[test]
fn a_dismounted_rider_keeps_his_foot_drive_under_a_chase() {
    let (walk, ride) = (sweep_pass(false, true), sweep_pass(false, false));
    for pass in [&walk, &ride] {
        assert!(
            matches!(pass.plan, Some(Sweep::Chase(point))
                if (mth::length2(point[0] - pass.rider_pos[0], point[2] - pass.rider_pos[2])
                    - 40.0)
                    .abs()
                    < 1.0),
            "a chase 40 blocks away: {:?} from {:?}",
            pass.plan,
            pass.rider_pos
        );
    }
    assert_walker_keeps_his_foot_drive(&walk, &ride);
}

#[test]
fn a_dismounted_rider_keeps_his_foot_drive_under_a_flank() {
    let (walk, ride) = (sweep_pass(true, true), sweep_pass(true, false));
    for pass in [&walk, &ride] {
        assert!(
            matches!(pass.plan, Some(Sweep::Flank(_))),
            "{:?}",
            pass.plan
        );
    }
    assert_walker_keeps_his_foot_drive(&walk, &ride);
}

/// The campaign's D1 bound needs server-truth exposure, not a wire proxy: the
/// dismounted rider of a chase or a flank is recorded (he is the member the
/// pre-fix code would have driven), the seated rider who takes the plan is not.
/// The recorded line names the body, the plan and the episode's duration.
#[test]
fn a_dismounted_sweep_member_is_recorded_exposure_and_a_seated_one_is_not() {
    for flank in [false, true] {
        let walk = sweep_pass(flank, true);
        let ride = sweep_pass(flank, false);
        assert!(
            walk.exposed,
            "a dismounted rider of a planned regiment (flank={flank}) is the pre-fix skate case"
        );
        assert!(
            !ride.exposed,
            "a seated rider takes the plan, so he is not exposed (flank={flank})"
        );
        let episode = walk
            .exposure
            .first()
            .unwrap_or_else(|| panic!("an exposure episode is logged at halt (flank={flank})"));
        assert!(
            episode.starts_with(&format!("Battle exposure: id={} ", walk.rider_id.0)),
            "the line names the exposed body: {episode}"
        );
        let plan = if flank {
            " plan=flank "
        } else {
            " plan=chase "
        };
        assert!(episode.contains(plan), "the line names the plan: {episode}");
        assert!(
            episode.contains(" tick=100 ticks=1 motion=101-101 "),
            "the condition tick and the tick the motion is published under: {episode}"
        );
        assert!(
            episode.ends_with("outcome=stopped"),
            "halt closes it: {episode}"
        );
        assert!(
            walk.exposure
                .iter()
                .any(|line| line
                    .starts_with("Battle exposure summary: start=100 episodes=1 bodies=1")),
            "the halt summary counts the one at-risk body: {:?}",
            walk.exposure
        );
    }
}

/// The guard set is evaluated before the tick's motion, so the episode's
/// closing sample is what carries the movement of its last exposed tick. One
/// exposed tick, a 3-block move, then a target: the line must show that 3
/// blocks, not zero, and close as `targeted`.
#[test]
fn an_episode_carries_the_movement_of_its_last_exposed_tick() {
    let pass = sweep_pass_ticks(false, true, true);
    assert!(pass.exposed, "the first tick is an exposure");
    let episode = pass
        .exposure
        .iter()
        .find(|line| line.starts_with("Battle exposure: "))
        .unwrap_or_else(|| panic!("the closed episode is logged: {:?}", pass.exposure));
    assert!(episode.ends_with("outcome=targeted"), "{episode}");
    assert!(
        episode.contains(" tick=100 ticks=1 motion=101-101 "),
        "{episode}"
    );
    let field = |name: &str| -> f64 {
        let start = episode
            .find(name)
            .unwrap_or_else(|| panic!("{name} in {episode}"))
            + name.len();
        let rest = &episode[start..];
        let end = rest.find(' ').unwrap_or(rest.len());
        rest[..end]
            .parse()
            .unwrap_or_else(|_| panic!("{name} parses in {episode}"))
    };
    assert!(
        field("travel=") > 2.0,
        "the last exposed tick's movement is in the episode: {episode}"
    );
    assert!(
        field("peak=") > 2.0,
        "the server's own excess flag sees the pre-fix skate: {episode}"
    );
}

/// A run boundary closes an episode with the entity store's current position, so
/// the last exposed tick's movement is in the line — not the pre-motion columns
/// snapshot (Sol P1-a).
#[test]
fn a_boundary_close_reports_the_movement_of_the_last_exposed_tick() {
    use super::super::formation::{Order, RegimentSpec, slot_position};
    let (mut state, _, pos) = fixture();
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (px, py, pz) = (pos[0] as i32, pos[1] as i32, pos[2] as i32);
    for x in px - 12..=px + 12 {
        for z in pz - 14..=pz + 32 {
            state.world.set_block(x, py - 1, z, stone);
            for dy in 0..6 {
                state.world.set_block(x, py + dy, z, air);
            }
        }
    }
    state.world.fold_overlay();
    let at = |dx: f64, dz: f64| [pos[0] + dx, pos[1], pos[2] + dz];
    let mut battle = BattleState::default();
    for (regiment, faction, origin, facing) in [
        (1, 2, at(0.0, 32.0), [0.0, -1.0]),
        (2, 1, at(10.0, 20.0), [0.0, 1.0]),
    ] {
        let soldier = spawn(&mut state, "husk", origin);
        state.entities.set_faction(slot(&state, soldier), faction);
        battle
            .formation
            .add_regiment(
                RegimentSpec {
                    id: regiment,
                    faction,
                    origin,
                    facing,
                    files: 1,
                    frontage: 1.0,
                    order: Order::Hold,
                },
                &[soldier],
            )
            .unwrap();
    }
    let cavalry = RegimentSpec {
        id: 3,
        faction: 1,
        origin: at(-6.0, -8.0),
        facing: [0.0, 1.0],
        files: 1,
        frontage: 2.0,
        order: Order::Hold,
    };
    let place = super::super::formation::slot_position(&cavalry, 0);
    let horse = spawn(&mut state, "horse", place);
    let rider = spawn(&mut state, "husk", place);
    state.entities.set_faction(slot(&state, rider), 1);
    mount(&mut state, rider, horse).unwrap();
    battle.formation.add_regiment(cavalry, &[rider]).unwrap();
    crate::mounts::dismount(&mut state, rider);
    // a broken enemy foot regiment: the regiment rides a chase
    battle.formation.regiments[0].morale.broken = true;
    state.hostile.battle = battle;
    state.hostile.battle.running = true;
    state.tick = 100;
    crate::battle::step(&mut state);
    let rider_slot = slot(&state, rider);
    let exposed = state.entities.pos(rider_slot);
    // the movement of the exposed tick, then the battle stops between ticks
    let moved = [exposed[0] + 3.0, exposed[1], exposed[2]];
    state.entities.set_pos(rider_slot, moved);
    state.hostile.battle.running = false;
    state.tick += 1;
    crate::battle::step(&mut state);
    let lines: Vec<String> = state.hostile.battle.exposure.lines().to_vec();
    let episode = lines
        .iter()
        .find(|line| line.starts_with(&format!("Battle exposure: id={} ", rider.0)))
        .unwrap_or_else(|| panic!("the boundary close is logged: {lines:?}"));
    assert!(episode.ends_with("outcome=stopped"), "{episode}");
    let travel: f64 = episode
        .split(" travel=")
        .nth(1)
        .and_then(|rest| rest.split(' ').next())
        .and_then(|value| value.parse().ok())
        .unwrap_or_else(|| panic!("travel in {episode}"));
    assert!(
        travel > 2.0,
        "the boundary sample carries the last exposed tick's 3-block move: {episode}"
    );
}

/// An identity the entity store does not hold cannot be sampled at a boundary:
/// the close is `unsettled`, never another body's position.
#[test]
fn a_boundary_close_without_the_identity_is_unsettled() {
    let mut state = fixture().0;
    let mut battle = BattleState::default();
    battle.exposure.begin(10);
    battle.exposure.exposed(
        EntityId(4242),
        0,
        [0.0, 0.0, 0.0],
        super::super::exposure::Plan::Chase,
    );
    state.hostile.battle = battle;
    state.hostile.battle.running = false;
    crate::battle::step(&mut state);
    let lines: Vec<String> = state.hostile.battle.exposure.lines().to_vec();
    assert!(
        lines.iter().any(|line| line.contains("outcome=unsettled")),
        "no trustworthy final sample at the boundary: {lines:?}"
    );
}

/// Recording moves nothing: the same fixture, run with the tracker enabled and
/// disabled, produces identical plans, drives and positions. This is the
/// telemetry-mutation control's counterpart — the mutation must change the log
/// and nothing else.
#[test]
fn exposure_recording_changes_no_drive_plan_or_position() {
    for flank in [false, true] {
        for closing in [false, true] {
            let recorded = sweep_pass_ticks(flank, true, closing);
            let silent = sweep_pass_unrecorded(flank, true, closing);
            // This control covers the simulation only, so it stays green when the
            // observation itself is removed (the telemetry-mutation control): it never
            // asserts that an episode was recorded.
            assert!(
                silent
                    .exposure
                    .iter()
                    .all(|line| !line.starts_with("Battle exposure: ")),
                "the disabled tracker writes no episode: {:?}",
                silent.exposure
            );
            assert_eq!(
                recorded.plan, silent.plan,
                "the plan is unchanged (flank={flank})"
            );
            assert_eq!(
                recorded.foot.map(f64::to_bits),
                silent.foot.map(f64::to_bits),
                "the foot drive is unchanged (flank={flank}, closing={closing})"
            );
            assert_eq!(
                recorded.rider.map(f64::to_bits),
                silent.rider.map(f64::to_bits),
                "the rider's drive is unchanged (flank={flank}, closing={closing})"
            );
            assert_eq!(
                recorded.horse.map(f64::to_bits),
                silent.horse.map(f64::to_bits),
                "the horse's drive is unchanged (flank={flank}, closing={closing})"
            );
            assert_eq!(
                recorded.rider_pos.map(f64::to_bits),
                silent.rider_pos.map(f64::to_bits),
                "the body's position is unchanged (flank={flank}, closing={closing})"
            );
        }
    }
}

/// A horse dies under his rider while his regiment chases a broken foot
/// regiment about 40 blocks ahead. From the next battle tick on, the rider walks
/// back to his slot at foot speed; he never skates on toward the chase point.
#[test]
fn a_rider_whose_horse_dies_walks_from_the_next_battle_tick() {
    use super::super::formation::{Order, RegimentSpec, slot_position};
    const FOOT_CAP: f64 = 5.6 * mc_physics::TICK_SECONDS;
    let (mut state, player, pos) = fixture();
    // The joined player stands aside, out of the riders' way.
    let player_slot = slot(&state, player);
    state
        .entities
        .set_pos(player_slot, [pos[0] + 8.0, pos[1], pos[2]]);
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (px, py, pz) = (pos[0] as i32, pos[1] as i32, pos[2] as i32);
    for x in px - 12..=px + 12 {
        for z in pz - 14..=pz + 32 {
            state.world.set_block(x, py - 1, z, stone);
            for dy in 0..6 {
                state.world.set_block(x, py + dy, z, air);
            }
        }
    }
    // The one fugitive stands boxed in, out of every reach.
    for dx in -1..=1 {
        for dz in -1..=1 {
            if (dx, dz) != (0, 0) {
                for dy in 0..3 {
                    state.world.set_block(px + dx, py + dy, pz + 30 + dz, stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    let at = |dx: f64, dz: f64| [pos[0] + dx, pos[1], pos[2] + dz];
    let fugitive = spawn(&mut state, "husk", at(0.0, 30.0));
    state.entities.set_faction(slot(&state, fugitive), 2);
    let enemy = RegimentSpec {
        id: 1,
        faction: 2,
        origin: at(0.0, 30.0),
        facing: [0.0, -1.0],
        files: 1,
        frontage: 1.0,
        order: Order::Hold,
    };
    let battle = &mut state.hostile.battle;
    battle.formation.add_regiment(enemy, &[fugitive]).unwrap();
    battle.formation.regiments[0].morale.broken = true;
    let cavalry = RegimentSpec {
        id: 2,
        faction: 1,
        origin: at(-6.0, -10.0),
        facing: [0.0, 1.0],
        files: 1,
        frontage: 2.0,
        order: Order::Hold,
    };
    let place = slot_position(&cavalry, 0);
    let horse = spawn(&mut state, "horse", place);
    let rider = spawn(&mut state, "husk", place);
    state.entities.set_faction(slot(&state, rider), 1);
    mount(&mut state, rider, horse).unwrap();
    state
        .hostile
        .battle
        .formation
        .add_regiment(cavalry, &[rider])
        .unwrap();
    state.hostile.battle.running = true;
    let chasing = |state: &TickState| {
        let plan = state.hostile.battle.combat.sweeps.get(1).copied().flatten();
        assert!(
            matches!(plan, Some(Sweep::Chase(point))
                if mth::length2(point[0] - pos[0], point[2] - pos[2] - 30.0) < 2.0),
            "the regiment must ride a chase: {plan:?}"
        );
    };
    for _ in 0..10 {
        crate::tick::run_tick(&mut state, &[]);
        chasing(&state);
        assert_pair(&state, rider, horse);
    }
    let horse_slot = slot(&state, horse);
    assert!(state.entities.pos(horse_slot)[2] > place[2] + 1.5);
    state.merge_damage.push((
        horse_slot,
        crate::tick::DamagePiece {
            amount: 1000.0,
            kind: crate::survival::DamageKind::GenericKill,
            source: None,
            physical: None,
            custom_armor: false,
        },
    ));
    state.commit_merge_effects();
    assert!(!state.entities.body(horse_slot).alive);
    let gap = |state: &TickState| {
        let p = state.entities.pos(slot(state, rider));
        mth::length2(p[0] - place[0], p[2] - place[2])
    };
    let start = gap(&state);
    for tick in 0..30 {
        let before = state.entities.pos(slot(&state, rider));
        crate::tick::run_tick(&mut state, &[]);
        chasing(&state);
        let rider_slot = slot(&state, rider);
        let row = state.entities.row(rider_slot);
        assert_eq!(
            row.entity_life.mount.vehicle, None,
            "tick {tick}: still seated"
        );
        assert!(state.hostile.battle.columns.target(rider_slot).is_none());
        let after = state.entities.pos(rider_slot);
        let step = mth::length2(after[0] - before[0], after[2] - before[2]);
        assert!(
            step <= FOOT_CAP + 1.0e-9,
            "tick {tick} after the horse died: the rider moved {step:.3} blocks, more than \
             the foot cap {FOOT_CAP}: {before:?} -> {after:?}"
        );
    }
    assert!(
        gap(&state) < start - 1.0,
        "the rider must walk back toward his slot: {start:.2} -> {:.2}",
        gap(&state)
    );
}

/// Riders that chose the same foe close on their own places round it, so a
/// pile of riders attacking one man gets drives that fan out. Riding at the
/// foe itself gave every rider of a pile the same drive: in the G1 rehearsal
/// 76 riders stood on one block over two foot soldiers.
#[test]
fn riders_attacking_one_foe_fan_out_round_it() {
    use super::super::formation::{Order, RegimentSpec};
    let (mut state, _, pos) = fixture();
    let target_pos = [pos[0] + 5.0, pos[1], pos[2]];
    let target = spawn(&mut state, "husk", target_pos);
    state.entities.set_faction(slot(&state, target), 2);
    let mut pairs = Vec::new();
    for _ in 0..8 {
        let horse = spawn(&mut state, "horse", pos);
        let rider = spawn(&mut state, "husk", pos);
        state.entities.set_faction(slot(&state, rider), 1);
        mount(&mut state, rider, horse).unwrap();
        pairs.push((rider, horse));
    }
    let mut battle = BattleState::default();
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 2,
                faction: 2,
                origin: target_pos,
                facing: [-1.0, 0.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[target],
        )
        .unwrap();
    prepare(&state, &mut battle, 0);
    // A wavering foe: the riders commit instead of refusing.
    battle.formation.regiments[0].morale.value = 20.0;
    battle.formation.members[0].wavering = true;
    for &(rider, _) in &pairs {
        let bi = index(&state, &battle, rider);
        battle.columns.targets[bi] = Some(super::super::EnemyTarget {
            id: target,
            pos: target_pos,
            distance: 5.0,
        });
    }
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    let mut combat = CombatRuntime::default();
    combat.prepare(&battle.columns.ids);
    step(&frame, &mut battle, &mut combat);
    let angles: Vec<f64> = pairs
        .iter()
        .map(|&(_, horse)| {
            let d = battle.columns.drive[index(&state, &battle, horse)];
            assert!(d[0] > 0.0, "every rider closes on the foe: {d:?}");
            mc_physics::mth::atan2(d[2], d[0]).to_degrees()
        })
        .collect();
    let fan = angles.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - angles.iter().copied().fold(f64::INFINITY, f64::min);
    assert!(
        fan >= 20.0,
        "the pile rides at the foe as one body: drive angles {angles:?}"
    );
}

/// Twenty-four riders two blocks apart ride at one wavering foe. Their places
/// round him (four rings 1.2..3.6 blocks out) lie closer together than a
/// horse is wide, and the push of the nearest horses (at most 3 blocks per
/// second) never held off the 6.4 blocks per second ride at the place: they
/// stood half a block apart, inside each other, as in the G2 take's horse
/// heaps. Kernel steps, horse positions integrated from the drives.
#[test]
fn riders_closing_on_one_foe_stay_a_horse_width_apart() {
    use super::super::formation::{Order, RegimentSpec};
    let (mut state, _, pos) = fixture();
    let target_pos = [pos[0] + 8.0, pos[1], pos[2]];
    let target = spawn(&mut state, "husk", target_pos);
    state.entities.set_faction(slot(&state, target), 2);
    let mut pairs = Vec::new();
    for i in 0..24 {
        let p = [pos[0], pos[1], pos[2] + 2.0 * (f64::from(i) - 11.5)];
        let horse = spawn(&mut state, "horse", p);
        let rider = spawn(&mut state, "husk", p);
        state.entities.set_faction(slot(&state, rider), 1);
        mount(&mut state, rider, horse).unwrap();
        pairs.push((rider, horse));
    }
    let mut battle = BattleState::default();
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 2,
                faction: 2,
                origin: target_pos,
                facing: [-1.0, 0.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[target],
        )
        .unwrap();
    let riders: Vec<_> = pairs.iter().map(|&(rider, _)| rider).collect();
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 1,
                faction: 1,
                origin: pos,
                facing: [1.0, 0.0],
                files: 24,
                frontage: 2.0,
                order: Order::Hold,
            },
            &riders,
        )
        .unwrap();
    let mut combat = CombatRuntime::default();
    for tick in 0..200 {
        prepare(&state, &mut battle, tick);
        // A wavering foe: the riders commit instead of refusing.
        battle.formation.regiments[0].morale.value = 20.0;
        battle.formation.members[0].wavering = true;
        battle
            .formation
            .neighbours
            .rebuild(&battle.formation.members);
        for &rider in &riders {
            let bi = index(&state, &battle, rider);
            battle.columns.targets[bi] = Some(super::super::EnemyTarget {
                id: target,
                pos: target_pos,
                distance: 5.0,
            });
        }
        combat.prepare(&battle.columns.ids);
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick,
        };
        step(&frame, &mut battle, &mut combat);
        let t = mc_physics::TICK_SECONDS;
        for &(rider, horse) in &pairs {
            let d = battle.columns.drive[index(&state, &battle, horse)];
            let (h, r) = (slot(&state, horse), slot(&state, rider));
            let (p, seat) = (
                state.entities.pos(h),
                state.entities.pos(r)[1] - state.entities.pos(h)[1],
            );
            let next = [p[0] + d[0] * t, p[1], p[2] + d[2] * t];
            state.entities.set_pos(h, next);
            // The rider sits on the moved horse: its x and z are the horse's.
            state
                .entities
                .set_pos(r, [next[0], next[1] + seat, next[2]]);
        }
    }
    let horses: Vec<[f64; 3]> = pairs
        .iter()
        .map(|&(_, horse)| state.entities.pos(slot(&state, horse)))
        .collect();
    let near = horses
        .iter()
        .filter(|p| mth::length2(p[0] - target_pos[0], p[2] - target_pos[2]) < 5.0)
        .count();
    assert!(near >= 8, "the riders did not close on the foe: {horses:?}");
    let (_, closest) = spread(&horses);
    assert!(
        closest >= 1.3,
        "horses inside each other: closest pair {closest:.2}: {horses:?}"
    );
}
