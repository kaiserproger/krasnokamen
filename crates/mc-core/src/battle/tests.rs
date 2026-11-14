use super::*;
use rayon::prelude::*;

fn unit(id: i32, faction: u16, x: f64, y: f64, z: f64) -> Unit {
    Unit { id: EntityId(id), faction, pos: [x, y, z] }
}

fn oracle(units: &[Unit], actor: Unit, reach: f64) -> Option<EnemyTarget> {
    units.iter().filter(|other| enemies(actor.faction, other.faction))
        .filter_map(|other| {
            let d2: f64 = (0..3).map(|axis| (actor.pos[axis] - other.pos[axis]).powi(2)).sum();
            (d2 <= reach * reach).then_some((d2, *other))
        })
        .min_by(|(a, lhs), (b, rhs)| a.total_cmp(b).then(lhs.id.cmp(&rhs.id)))
        .map(|(d2, other)| EnemyTarget { id: other.id, pos: other.pos, distance: d2.sqrt() })
}

#[test]
fn same_faction_and_neutral_never_target_each_other() {
    let actor = unit(10, 1, -0.5, 64.0, -0.5);
    let friend = unit(1, 1, -0.4, 64.0, -0.5);
    let neutral = unit(2, 0, -0.5, 64.0, -0.4);
    let enemy = unit(3, 2, -0.5, 64.0, 5.5);
    let mut grid = TargetGrid::default();
    grid.rebuild([actor, friend, neutral, enemy].into_iter());
    let query = grid.nearest_enemy(actor, 6.0);
    assert_eq!(query.target.unwrap().id, enemy.id);
    assert_eq!(query.target.unwrap().distance, 6.0);
    assert_eq!(query.examined, 1);
    assert_eq!(grid.nearest_enemy(actor, 5.999).target, None);
    assert_eq!(grid.nearest_enemy(neutral, 100.0).target, None);
    grid.rebuild([actor, friend, neutral].into_iter());
    assert_eq!(grid.nearest_enemy(actor, 100.0), Query::default());
}

#[test]
fn nearest_enemy_matches_exhaustive_oracle_and_stable_id_ties() {
    let actor = unit(100, 1, -4.0, 64.0, 0.0);
    let mut units = vec![actor, unit(9, 2, -3.0, 64.0, 0.0), unit(2, 3, -5.0, 64.0, 0.0)];
    let mut seed = 0xb177_1e_u64;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for id in 101..601 {
        let faction = (next() % 4) as u16;
        let x = (next() % 200) as f64 * 0.5 - 50.0;
        let y = (next() % 30) as f64 * 0.5 + 60.0;
        let z = (next() % 200) as f64 * 0.5 - 50.0;
        units.push(unit(id, faction, x, y, z));
    }
    let mut grid = TargetGrid::default();
    for reverse in [false, true] {
        if reverse { units.reverse(); }
        grid.rebuild(units.iter().copied());
        assert_eq!(grid.nearest_enemy(actor, 1.0).target.unwrap().id, EntityId(2));
        for actor in &units {
            for reach in [0.0, 1.0, 6.0, 15.0] {
                assert_eq!(grid.nearest_enemy(*actor, reach).target, oracle(&units, *actor, reach));
            }
        }
    }
    // A nearby horizontal point on another floor is outside the 3D radius.
    grid.rebuild([unit(1, 2, -4.0, 71.0, 0.0)].into_iter());
    assert!(grid.nearest_enemy(actor, 6.0).target.is_none());
}

#[test]
fn stable_retarget_slices_visit_every_unit_once_in_any_ten_ticks() {
    for start in [0, 7, 10_000, u64::MAX - 10] {
        for id in -100..20_000 {
            let ticks: Vec<_> = (start..start + 10).filter(|tick| retarget_due(EntityId(id), *tick)).collect();
            assert_eq!(ticks.len(), 1, "id {id}, start {start}");
        }
    }
}

#[test]
fn target_results_are_identical_on_one_two_four_threads() {
    let units: Vec<_> = (0..1024).map(|id| {
        unit(id, (id % 3 + 1) as u16, f64::from(id % 32) - 16.5, 64.0, f64::from(id / 32))
    }).collect();
    let mut grid = TargetGrid::default();
    grid.rebuild(units.iter().copied());
    let expected: Vec<_> = units.iter().map(|unit| grid.nearest_enemy(*unit, 6.0)).collect();
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new().num_threads(threads).build().unwrap();
        let actual: Vec<_> = pool.install(|| units.par_iter().map(|unit| grid.nearest_enemy(*unit, 6.0)).collect());
        assert_eq!(actual, expected, "{threads} workers");
    }
}

fn army_units(gap: f64) -> Vec<Unit> {
    (0..20_000).map(|id| {
        let team = id / 10_000;
        let local = id % 10_000;
        let x = f64::from(local % 100) + if team == 0 { -100.0 } else { gap };
        unit(id + 1, (team + 1) as u16, x, 64.0, f64::from(local / 100))
    }).collect()
}

/// Work, not wall time: deterministic counters survive CPU contention. Run with
/// --nocapture and retain the log as the 20k micro receipt.
#[test]
fn twenty_thousand_units_record_sliced_query_work() {
    for (scene, gap) in [("contact", 0.0), ("approach", 40.0)] {
        let units = army_units(gap);
        let mut grid = TargetGrid::default();
        grid.rebuild(units.iter().copied());
        for reach in [6.0, 15.0] {
            let mut total = 0;
            for tick in 0..RETARGET_TICKS {
                let mut work = Work::default();
                let mut acquired = 0;
                for actor in units.iter().filter(|actor| retarget_due(actor.id, tick)) {
                    work.retargeted += 1;
                    let query = grid.nearest_enemy(*actor, reach);
                    work.examined += query.examined;
                    acquired += u64::from(query.target.is_some());
                }
                assert_eq!(work.retargeted, 2_000);
                assert!(work.examined < 200_000, "local search degenerated: {work:?}");
                if scene == "approach" { assert_eq!(acquired, 0); }
                eprintln!("battle20k scene={scene} reach={reach} tick={tick} retargeted={} examined={} acquired={acquired}", work.retargeted, work.examined);
                total += work.examined;
            }
            let unsliced: u64 = units.iter().map(|actor| grid.nearest_enemy(*actor, reach).examined).sum();
            assert_eq!(total, unsliced, "each decision executes exactly once in ten ticks");
        }
    }
}

fn fixture() -> crate::tick::TickState {
    let registry = std::sync::Arc::new(CoreRegistry::load(None).unwrap());
    let mut world = crate::horde_bench::flat_world(&registry);
    let chunks = [crate::boundary::ChunkPos { x: 0, z: 0 }];
    world.ensure_blocking(&chunks.into_iter().map(|chunk| (chunk, 0)).collect());
    world.set_active_chunks(chunks.into_iter().collect(), 1);
    crate::tick::TickState::new(registry, world, 2, 512)
}

fn spawn(state: &mut crate::tick::TickState, faction: u16, name: &str, pos: [f64; 3]) -> EntityId {
    let type_id = state.registry.entity_type_id(name).unwrap();
    let id = state.spawn_mob(type_id, pos, 20.0);
    state.entities.set_faction(state.entities.slot_of(id).unwrap(), faction);
    id
}

#[test]
fn native_target_update_is_sliced_and_rechecks_dead_factions_and_range() {
    let mut state = fixture();
    let actor = spawn(&mut state, 1, "minecraft:zombie", [4.0, 100.0, 4.0]);
    let friend = spawn(&mut state, 1, "minecraft:zombie", [4.5, 100.0, 4.0]);
    let enemy = spawn(&mut state, 2, "minecraft:zombie", [7.0, 100.0, 4.0]);
    let slot = state.entities.slot_of(actor).unwrap();
    let enemy_slot = state.entities.slot_of(enemy).unwrap();
    let mut battle = BattleState::default();
    let update = |state: &crate::tick::TickState, battle: &mut BattleState, tick| {
        let frame = Frame { entities: &state.entities, registry: &state.registry, world: &state.world, tick };
        targeting::update(&frame, battle);
    };
    let due = i64::from(actor.0).rem_euclid(10) as u64;
    update(&state, &mut battle, due + 1);
    assert!(!battle.targets.contains_key(&actor));
    update(&state, &mut battle, due);
    assert_eq!(battle.targets[&actor].id, enemy);
    assert_ne!(battle.targets[&actor].id, friend);
    state.entities.set_pos(enemy_slot, [8.0, 100.0, 4.0]);
    update(&state, &mut battle, due + 1);
    assert_eq!(battle.targets[&actor].pos, [8.0, 100.0, 4.0]);
    state.entities.set_faction(enemy_slot, 1);
    update(&state, &mut battle, due + 2);
    assert!(!battle.targets.contains_key(&actor));
    state.entities.set_faction(enemy_slot, 2);
    update(&state, &mut battle, due + 3);
    assert!(!battle.targets.contains_key(&actor), "no off-slice reacquisition");
    update(&state, &mut battle, due + 10);
    assert_eq!(battle.targets[&actor].id, enemy);
    state.entities.row_mut(enemy_slot).health = 0.0;
    update(&state, &mut battle, due + 11);
    assert!(!battle.targets.contains_key(&actor));
    state.entities.row_mut(enemy_slot).health = 20.0;
    state.entities.set_pos(enemy_slot, [11.0, 100.0, 4.0]);
    update(&state, &mut battle, due + 20);
    assert!(!battle.targets.contains_key(&actor), "no 35-block follow-range targeting");
    assert!(retained_enemy(&state.entities, slot, enemy, 6.0).is_none());
}

#[test]
fn archers_target_with_attack_range_not_melee_or_follow_range() {
    let mut state = fixture();
    let archer = spawn(&mut state, 1, "minecraft:skeleton", [1.0, 100.0, 1.0]);
    let enemy = spawn(&mut state, 2, "minecraft:zombie", [11.0, 100.0, 1.0]);
    let slot = state.entities.slot_of(archer).unwrap();
    state.entities.row_mut(slot).attack_reach = 15.0;
    let mut battle = BattleState::default();
    let frame = Frame { entities: &state.entities, registry: &state.registry, world: &state.world, tick: i64::from(archer.0).rem_euclid(10) as u64 };
    targeting::update(&frame, &mut battle);
    assert_eq!(battle.targets[&archer].id, enemy);
    assert_eq!(battle.targets[&archer].distance, 10.0);
}

#[test]
fn faction_membership_survives_compaction_transfer_and_zero_reset() {
    let mut state = fixture();
    let first = spawn(&mut state, 0, "minecraft:cow", [1.0, 100.0, 1.0]);
    let last = spawn(&mut state, 7, "minecraft:cow", [2.0, 100.0, 1.0]);
    state.entities.remove(state.entities.slot_of(first).unwrap());
    let moved = state.entities.slot_of(last).unwrap();
    assert_eq!(state.entities.body(moved).faction, 7);
    assert_eq!(state.entities.battle_slots().collect::<Vec<_>>(), vec![moved]);
    let mut destination = EntityStore::default();
    let moved = state.entities.transfer_to(moved, &mut destination, [4.0, 100.0, 4.0], [0.0; 2]);
    assert!(state.entities.battle_slots().next().is_none());
    assert_eq!(destination.body(moved).faction, 7);
    assert_eq!(destination.battle_slots().collect::<Vec<_>>(), vec![moved]);
    destination.set_faction(moved, 0);
    assert!(destination.battle_slots().next().is_none());
}

#[test]
fn faction_fields_reject_invalid_native_and_json_values() {
    use mc_nbt::Tag;
    let state = fixture();
    for tag in [Tag::Int(-1), Tag::Int(65536), Tag::Short(1), Tag::String("2".into())] {
        let record = mc_data::entity_nbt::VanillaEntity {
            id: "minecraft:zombie".into(),
            extra_fields: vec![("solaris:Faction".into(), tag)],
            ..Default::default()
        };
        assert!(crate::journal::EntityStateImage::from_record(&state.registry, &record).is_err());
    }
    for value in [-1, 65536] {
        assert!(serde_json::from_str::<crate::journal::EntityStateImage>(&format!("{{\"faction\":{value}}}")).is_err());
    }
}
