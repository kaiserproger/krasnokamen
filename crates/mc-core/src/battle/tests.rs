use super::*;
use rayon::prelude::*;

fn unit(id: i32, faction: u16, x: f64, y: f64, z: f64) -> Unit {
    Unit {
        id: EntityId(id),
        faction,
        pos: [x, y, z],
    }
}

fn oracle(units: &[Unit], actor: Unit, reach: f64) -> Option<EnemyTarget> {
    units
        .iter()
        .filter(|other| enemies(actor.faction, other.faction))
        .filter_map(|other| {
            let d2: f64 = (0..3)
                .map(|axis| (actor.pos[axis] - other.pos[axis]).powi(2))
                .sum();
            (d2 <= reach * reach).then_some((d2, *other))
        })
        .min_by(|(a, lhs), (b, rhs)| a.total_cmp(b).then(lhs.id.cmp(&rhs.id)))
        .map(|(d2, other)| EnemyTarget {
            id: other.id,
            pos: other.pos,
            distance: d2.sqrt(),
        })
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
    let mut units = vec![
        actor,
        unit(9, 2, -3.0, 64.0, 0.0),
        unit(2, 3, -5.0, 64.0, 0.0),
    ];
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
        if reverse {
            units.reverse();
        }
        grid.rebuild(units.iter().copied());
        assert_eq!(
            grid.nearest_enemy(actor, 1.0).target.unwrap().id,
            EntityId(2)
        );
        for actor in &units {
            for reach in [0.0, 1.0, 6.0, 15.0] {
                assert_eq!(
                    grid.nearest_enemy(*actor, reach).target,
                    oracle(&units, *actor, reach)
                );
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
            let ticks: Vec<_> = (start..start + 10)
                .filter(|tick| retarget_due(EntityId(id), *tick))
                .collect();
            assert_eq!(ticks.len(), 1, "id {id}, start {start}");
        }
    }
}

#[test]
fn target_results_are_identical_on_one_two_four_threads() {
    let units: Vec<_> = (0..1024)
        .map(|id| {
            unit(
                id,
                (id % 3 + 1) as u16,
                f64::from(id % 32) - 16.5,
                64.0,
                f64::from(id / 32),
            )
        })
        .collect();
    let mut grid = TargetGrid::default();
    grid.rebuild(units.iter().copied());
    let expected: Vec<_> = units
        .iter()
        .map(|unit| grid.nearest_enemy(*unit, 6.0))
        .collect();
    for threads in [1, 2, 4] {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        let actual: Vec<_> = pool.install(|| {
            units
                .par_iter()
                .map(|unit| grid.nearest_enemy(*unit, 6.0))
                .collect()
        });
        assert_eq!(actual, expected, "{threads} workers");
    }
}

fn army_units(gap: f64) -> Vec<Unit> {
    (0..20_000)
        .map(|id| {
            let team = id / 10_000;
            let local = id % 10_000;
            let x = f64::from(local % 100) + if team == 0 { -100.0 } else { gap };
            unit(id + 1, (team + 1) as u16, x, 64.0, f64::from(local / 100))
        })
        .collect()
}

/// Work, not wall time: deterministic counters survive CPU contention. Run with
/// --nocapture and retain the log as the 20k micro receipt.
#[test]
fn twenty_thousand_units_record_sliced_query_work() {
    for (scene, gap) in [("contact", 0.0), ("approach", 40.0)] {
        let units = army_units(gap);
        let mut grid = TargetGrid::default();
        grid.rebuild(units.iter().copied());
        for reach in [6.0, 15.0, 220.0] {
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
                assert!(
                    work.examined < 20_000_000,
                    "search degenerated to a full enemy scan: {work:?}"
                );
                if scene == "approach" && reach < gap {
                    assert_eq!(acquired, 0);
                }
                eprintln!(
                    "battle20k scene={scene} reach={reach} tick={tick} retargeted={} examined={} acquired={acquired}",
                    work.retargeted, work.examined
                );
                total += work.examined;
            }
            let unsliced: u64 = units
                .iter()
                .map(|actor| grid.nearest_enemy(*actor, reach).examined)
                .sum();
            assert_eq!(
                total, unsliced,
                "each decision executes exactly once in ten ticks"
            );
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
    state
        .entities
        .set_faction(state.entities.slot_of(id).unwrap(), faction);
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
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick,
        };
        battle
            .columns
            .prepare(&frame, &mut battle.formation, &mut battle.output.counters);
        battle.formation.finish_preparation();
        targeting::update(&frame, battle);
    };
    let due = i64::from(actor.0).rem_euclid(10) as u64;
    update(&state, &mut battle, due + 1);
    assert!(battle.columns.target(slot).is_none());
    update(&state, &mut battle, due);
    assert_eq!(battle.columns.target(slot).unwrap().id, enemy);
    assert_ne!(battle.columns.target(slot).unwrap().id, friend);
    state.entities.set_pos(enemy_slot, [8.0, 100.0, 4.0]);
    update(&state, &mut battle, due + 1);
    assert_eq!(battle.columns.target(slot).unwrap().pos, [8.0, 100.0, 4.0]);
    state.entities.set_faction(enemy_slot, 1);
    update(&state, &mut battle, due + 2);
    assert!(battle.columns.target(slot).is_none());
    state.entities.set_faction(enemy_slot, 2);
    update(&state, &mut battle, due + 3);
    assert!(
        battle.columns.target(slot).is_none(),
        "no off-slice reacquisition"
    );
    update(&state, &mut battle, due + 10);
    assert_eq!(battle.columns.target(slot).unwrap().id, enemy);
    state.entities.row_mut(enemy_slot).health = 0.0;
    update(&state, &mut battle, due + 11);
    assert!(battle.columns.target(slot).is_none());
    state.entities.row_mut(enemy_slot).health = 20.0;
    state.entities.set_pos(enemy_slot, [11.0, 100.0, 4.0]);
    update(&state, &mut battle, due + 20);
    assert!(
        battle.columns.target(slot).is_none(),
        "no 35-block follow-range targeting"
    );
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
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: i64::from(archer.0).rem_euclid(10) as u64,
    };
    battle
        .columns
        .prepare(&frame, &mut battle.formation, &mut battle.output.counters);
    battle.formation.finish_preparation();
    targeting::update(&frame, &mut battle);
    assert_eq!(battle.columns.target(slot).unwrap().id, enemy);
    assert_eq!(battle.columns.target(slot).unwrap().distance, 10.0);
}

#[test]
fn faction_membership_survives_compaction_transfer_and_zero_reset() {
    let mut state = fixture();
    let first = spawn(&mut state, 0, "minecraft:cow", [1.0, 100.0, 1.0]);
    let last = spawn(&mut state, 7, "minecraft:cow", [2.0, 100.0, 1.0]);
    state
        .entities
        .remove(state.entities.slot_of(first).unwrap());
    let moved = state.entities.slot_of(last).unwrap();
    assert_eq!(state.entities.body(moved).faction, 7);
    assert_eq!(
        state.entities.battle_slots().collect::<Vec<_>>(),
        vec![moved]
    );
    let mut destination = EntityStore::default();
    let moved = state
        .entities
        .transfer_to(moved, &mut destination, [4.0, 100.0, 4.0], [0.0; 2]);
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
    for tag in [
        Tag::Int(-1),
        Tag::Int(65536),
        Tag::Short(1),
        Tag::String("2".into()),
    ] {
        let record = mc_data::entity_nbt::VanillaEntity {
            id: "minecraft:zombie".into(),
            extra_fields: vec![("krasnokamen:Faction".into(), tag)],
            ..Default::default()
        };
        assert!(crate::journal::EntityStateImage::from_record(&state.registry, &record).is_err());
    }
    for value in [-1, 65536] {
        assert!(
            serde_json::from_str::<crate::journal::EntityStateImage>(&format!(
                "{{\"faction\":{value}}}"
            ))
            .is_err()
        );
    }
}

#[test]
fn paused_battle_keeps_its_clock_and_clears_motion() {
    let mut state = fixture();
    state.hostile.battle.elapsed_ticks = 41;
    state
        .hostile
        .battle
        .columns
        .drive_by_slot
        .push([1.0, 0.0, 0.0]);
    assert!(step(&mut state).is_empty());
    assert_eq!(state.hostile.battle.elapsed_ticks, 41);
    assert!(
        state
            .hostile
            .battle
            .columns
            .drive_by_slot
            .iter()
            .all(|drive| *drive == [0.0; 3])
    );
    state.hostile.battle.running = true;
    assert!(step(&mut state).is_empty());
    assert_eq!(state.hostile.battle.elapsed_ticks, 42);
    state.hostile.battle.running = false;
    assert!(step(&mut state).is_empty());
    assert_eq!(state.hostile.battle.elapsed_ticks, 42);
}

#[test]
fn per_tick_unit_counters_report_native_liveness_by_faction_even_when_paused() {
    let mut state = fixture();
    let friendly = spawn(&mut state, 1, "minecraft:zombie", [1.0, 100.0, 1.0]);
    let enemy = spawn(&mut state, 2, "minecraft:zombie", [2.0, 100.0, 1.0]);
    let dead = spawn(&mut state, 2, "minecraft:zombie", [3.0, 100.0, 1.0]);
    spawn(&mut state, 0, "minecraft:zombie", [4.0, 100.0, 1.0]);
    state
        .entities
        .body_mut(state.entities.slot_of(dead).unwrap())
        .alive = false;
    state.tick = 9;
    step(&mut state);
    let counts = unit_counters(&state.hostile);
    assert_eq!(counts.tick, 9);
    assert_eq!(counts.units, 3);
    assert_eq!(
        counts.alive_per_faction,
        vec![
            FactionAlive {
                faction: 1,
                alive: 1
            },
            FactionAlive {
                faction: 2,
                alive: 1
            },
        ]
    );
    state
        .entities
        .set_faction(state.entities.slot_of(friendly).unwrap(), 0);
    state
        .entities
        .row_mut(state.entities.slot_of(enemy).unwrap())
        .health = 0.0;
    state.tick = 10;
    step(&mut state);
    let counts = unit_counters(&state.hostile);
    assert_eq!(counts.tick, 10);
    assert_eq!(counts.units, 2);
    assert_eq!(
        counts.alive_per_faction,
        vec![FactionAlive {
            faction: 2,
            alive: 0
        }]
    );
}

#[test]
fn published_population_recounts_post_merge_state_and_keeps_zero_alive_factions() {
    let mut state = fixture();
    let actor = spawn(&mut state, 1, "minecraft:zombie", [1.0, 100.0, 1.0]);
    let enemy = spawn(&mut state, 2, "minecraft:zombie", [2.0, 100.0, 1.0]);
    step(&mut state);
    state
        .entities
        .row_mut(state.entities.slot_of(enemy).unwrap())
        .health = 0.0;
    let counts = publication_counts(&mut state).unwrap();
    assert_eq!(counts.units, 2);
    assert_eq!(counts.alive, vec![(1, 1), (2, 0)]);
    assert_eq!(unit_counters(&state.hostile).alive_per_faction[1].alive, 0);
    state
        .entities
        .set_faction(state.entities.slot_of(actor).unwrap(), 0);
    state
        .entities
        .set_faction(state.entities.slot_of(enemy).unwrap(), 0);
    assert!(publication_counts(&mut state).is_none());
}

#[test]
fn dense_slot_projection_survives_compaction_and_classification_changes() {
    let mut state = fixture();
    let removed = spawn(&mut state, 1, "minecraft:zombie", [1.0, 100.0, 1.0]);
    let actor = spawn(&mut state, 2, "minecraft:zombie", [2.0, 100.0, 1.0]);
    state
        .entities
        .remove(state.entities.slot_of(removed).unwrap());
    let slot = state.entities.slot_of(actor).unwrap();
    let bow = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bow").unwrap())
        .unwrap();
    let mut battle = BattleState::default();
    {
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick: 0,
        };
        battle
            .columns
            .prepare(&frame, &mut battle.formation, &mut battle.output.counters);
        battle.formation.finish_preparation();
    }
    let bi = battle.columns.index(slot).unwrap();
    assert_eq!(battle.columns.ids[bi], actor);
    assert!(!battle.columns.ranged[bi]);
    battle.columns.drive[bi] = [2.0, 0.0, 1.0];
    battle.columns.project();
    assert_eq!(
        battle.columns.drive_by_slot[slot.0 as usize],
        [2.0, 0.0, 1.0]
    );
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
        Some(mc_data::ItemStack::new(bow, 1));
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 1,
    };
    battle
        .columns
        .prepare(&frame, &mut battle.formation, &mut battle.output.counters);
    battle.formation.finish_preparation();
    assert!(battle.columns.ranged[bi]);
    assert_eq!(battle.columns.drive_by_slot[slot.0 as usize], [0.0; 3]);
}

fn native_cache_fixture() -> (crate::tick::TickState, EntityId, EntityId) {
    let mut state = fixture();
    state.tickets.insert(
        "battle-cache-regression".into(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 0),
    );
    let rider = spawn(&mut state, 1, "minecraft:husk", [4.5, 64.0, 4.5]);
    let horse = spawn(&mut state, 1, "minecraft:horse", [12.5, 64.0, 12.5]);
    for id in [rider, horse] {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.row_mut(slot).persistent = true;
    }
    state
        .hostile
        .battle
        .formation
        .add_regiment(
            formation::RegimentSpec {
                id: 1,
                faction: 1,
                origin: [4.5, 64.0, 4.5],
                facing: [0.0, 1.0],
                files: 1,
                frontage: 1.0,
                order: formation::Order::Hold,
            },
            &[rider],
        )
        .unwrap();
    state.hostile.battle.running = true;
    for _ in 0..3 {
        crate::tick::run_tick(&mut state, &[]);
    }
    (state, rider, horse)
}

#[test]
fn native_cached_facts_invalidate_no_ai_between_ticks() {
    let (mut state, rider, _) = native_cache_fixture();
    let slot = state.entities.slot_of(rider).unwrap();
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(state.hostile.battle.columns.active[bi]);
    state.entities.row_mut(slot).controls.no_ai = true;
    crate::tick::run_tick(&mut state, &[]);
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(!state.hostile.battle.columns.active[bi]);
}

#[test]
fn native_cached_facts_invalidate_equipment_and_latch_inactive_changes() {
    let (mut state, rider, _) = native_cache_fixture();
    let slot = state.entities.slot_of(rider).unwrap();
    state.entities.row_mut(slot).controls.no_ai = true;
    crate::tick::run_tick(&mut state, &[]);
    let bow = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bow").unwrap())
        .unwrap();
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[0] =
        Some(mc_data::ItemStack::new(bow, 1));
    crate::tick::run_tick(&mut state, &[]);
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(state.hostile.battle.columns.ranged[bi]);
    assert!(state.hostile.battle.columns.equipment_dirty[bi]);
    state.entities.row_mut(slot).controls.no_ai = false;
    crate::tick::run_tick(&mut state, &[]);
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert_eq!(
        state.hostile.battle.combat.fighters[bi].weapon,
        combat::Weapon::Bow
    );
    assert!(!state.hostile.battle.columns.equipment_dirty[bi]);
}

#[test]
fn native_cached_facts_invalidate_mounts_between_ticks() {
    let (mut state, rider, horse) = native_cache_fixture();
    let slot = state.entities.slot_of(horse).unwrap();
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(!state.hostile.battle.columns.carried[bi]);
    cavalry::mount(&mut state, rider, horse).unwrap();
    crate::tick::run_tick(&mut state, &[]);
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(state.hostile.battle.columns.carried[bi]);
}

#[test]
fn late_wire_fact_invalidation_precedes_the_views_drain() {
    let (mut state, rider, _) = native_cache_fixture();
    step(&mut state);
    let slot = state.entities.slot_of(rider).unwrap();
    // A canonical late-phase write after the frozen battle frame. Its wire
    // notification will be gone at the next battle frame, just as in run_tick.
    state.entities.row_mut(slot).controls.no_ai = true;
    invalidate_wire_facts(&mut state);
    let _ = state.entities.views();
    step(&mut state);
    let bi = state.hostile.battle.columns.index(slot).unwrap();
    assert!(!state.hostile.battle.columns.active[bi]);
}

#[test]
fn fused_population_matches_the_native_sparse_scan_after_roster_changes() {
    let (mut state, rider, horse) = native_cache_fixture();
    let dead = spawn(&mut state, 1, "minecraft:husk", [6.5, 64.0, 6.5]);
    let zero = spawn(&mut state, 2, "minecraft:husk", [7.5, 64.0, 7.5]);
    let mismatch = spawn(&mut state, 2, "minecraft:husk", [9.5, 64.0, 9.5]);
    let gone = spawn(&mut state, 1, "minecraft:husk", [10.5, 64.0, 10.5]);
    state
        .hostile
        .battle
        .formation
        .add_regiment(
            formation::RegimentSpec {
                id: 11,
                faction: 1,
                origin: [6.5, 64.0, 6.5],
                facing: [0.0, 1.0],
                files: 4,
                frontage: 1.0,
                order: formation::Order::Hold,
            },
            &[dead, zero, mismatch, gone],
        )
        .unwrap();
    state
        .entities
        .body_mut(state.entities.slot_of(dead).unwrap())
        .alive = false;
    state
        .entities
        .row_mut(state.entities.slot_of(zero).unwrap())
        .health = 0.0;
    state.entities.remove(state.entities.slot_of(gone).unwrap());
    let tail_dead = spawn(&mut state, 3, "minecraft:husk", [13.5, 64.0, 13.5]);
    state
        .entities
        .body_mut(state.entities.slot_of(tail_dead).unwrap())
        .alive = false;
    spawn(&mut state, 3, "minecraft:husk", [14.5, 64.0, 14.5]);
    spawn(&mut state, 0, "minecraft:husk", [15.5, 64.0, 15.5]);
    let mut expected = UnitCounters::default();
    for round in 0..3 {
        if round == 1 {
            state
                .entities
                .set_faction(state.entities.slot_of(rider).unwrap(), 0);
            state
                .entities
                .set_faction(state.entities.slot_of(horse).unwrap(), 2);
        } else if round == 2 {
            state
                .entities
                .remove(state.entities.slot_of(mismatch).unwrap());
        }
        state.tick += 1;
        step(&mut state);
        expected.sample(&state.entities, state.tick);
        assert_eq!(unit_counters(&state.hostile), &expected, "round {round}");
    }
}

#[test]
fn twenty_thousand_registered_archers_use_one_regiment_candidate_per_decision() {
    let mut state = fixture();
    let mut battle = BattleState::default();
    for side in 1..=2_u16 {
        let pos = [if side == 1 { 4.0 } else { 12.0 }, 100.0, 4.0];
        for regiment in 0..10_u32 {
            let ids: Vec<_> = (0..1000)
                .map(|_| spawn(&mut state, side, "minecraft:skeleton", pos))
                .collect();
            for id in &ids {
                state
                    .entities
                    .row_mut(state.entities.slot_of(*id).unwrap())
                    .attack_reach = 220.0;
            }
            battle
                .formation
                .add_regiment(
                    formation::RegimentSpec {
                        id: u32::from(side) * 100 + regiment,
                        faction: side,
                        origin: pos,
                        facing: [if side == 1 { 1.0 } else { -1.0 }, 0.0],
                        files: 20,
                        frontage: 1.0,
                        order: formation::Order::Hold,
                    },
                    &ids,
                )
                .unwrap();
        }
    }
    let mut examined = 0;
    for tick in 0..10 {
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick,
        };
        battle
            .columns
            .prepare(&frame, &mut battle.formation, &mut battle.output.counters);
        battle.formation.finish_preparation();
        targeting::update(&frame, &mut battle);
        assert_eq!(battle.work.retargeted, 2000);
        assert!(battle.work.examined <= battle.work.retargeted);
        assert!(battle.work.examined < 10_000);
        examined += battle.work.examined;
        eprintln!(
            "battle20k registered_archers tick={tick} retargeted={} examined={}",
            battle.work.retargeted, battle.work.examined
        );
        for (bi, target) in battle.columns.targets.iter().enumerate() {
            if let Some(target) = target {
                let slot = state.entities.slot_of(target.id).unwrap();
                assert!(enemies(
                    battle.columns.factions[bi],
                    state.entities.body(slot).faction
                ));
                assert!(target.distance <= 220.0);
            }
        }
    }
    assert!(
        examined > 0,
        "live enemy regiments must supply actual candidates"
    );
}

#[test]
fn tick_metrics_publish_staged_battle_counts_and_clear_them_after_reset() {
    let mut state = fixture();
    let first = spawn(&mut state, 1, "minecraft:cow", [1.0, 100.0, 1.0]);
    let second = spawn(&mut state, 2, "minecraft:cow", [2.0, 100.0, 1.0]);
    for id in [first, second] {
        let row = state.entities.row_mut(state.entities.slot_of(id).unwrap());
        row.persistent = true;
        row.controls.no_ai = true;
    }
    assert!(!state.hostile.battle.running);
    let (output, _, times) = crate::tick::run_tick(&mut state, &[]);
    let counts = times
        .battle
        .as_ref()
        .expect("staged rows are observable before start");
    assert_eq!(counts.units, 2);
    assert_eq!(counts.alive, vec![(1, 1), (2, 1)]);
    assert_eq!(unit_counters(&state.hostile).tick, output.tick);
    for id in [first, second] {
        state
            .entities
            .set_faction(state.entities.slot_of(id).unwrap(), 0);
    }
    let (_, _, times) = crate::tick::run_tick(&mut state, &[]);
    assert!(times.battle.is_none());
}

#[test]
fn native_battle_arrows_clear_allied_ranks_without_changing_unaffiliated_hits() {
    for (owner_faction, near_faction, near_hit, player) in [
        (1, 1, false, false),
        (1, 2, true, false),
        (0, 0, true, false),
        (1, 1, true, true),
    ] {
        let mut state = fixture();
        let chunks = (0..=8)
            .map(|z| (crate::boundary::ChunkPos { x: 0, z }, 0))
            .collect();
        state.world.ensure_blocking(&chunks);
        for z in 0..=8 {
            state.tickets.insert(
                format!("battle-arrow-regression-{z}"),
                (crate::boundary::ChunkPos { x: 0, z }, 0),
            );
        }
        state.refresh_active_chunks(state.tick);
        let owner = spawn(
            &mut state,
            owner_faction,
            "minecraft:husk",
            [8.5, 64.0, 8.5],
        );
        let near = if player {
            let connection = crate::boundary::ConnectionId(1);
            crate::tick::run_tick(
                &mut state,
                &[crate::boundary::PlayerInput::Join {
                    connection,
                    uuid: uuid::Uuid::new_v4(),
                    name: "battle-arrow-player".into(),
                    permission_level: 0,
                }],
            );
            let id = state.connections.entity_of(connection).unwrap();
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_pos(slot, [8.5, 64.0, 10.5]);
            state.entities.set_faction(slot, near_faction);
            state.entities.row_mut(slot).protected_until = 0;
            id
        } else {
            spawn(
                &mut state,
                near_faction,
                "minecraft:husk",
                [8.5, 64.0, 10.5],
            )
        };
        let enemy = spawn(&mut state, 2, "minecraft:husk", [8.5, 64.0, 128.5]);
        for id in [owner, near, enemy] {
            let slot = state.entities.slot_of(id).unwrap();
            let row = state.entities.row_mut(slot);
            row.controls.no_ai = true;
            row.persistent = true;
        }
        let owner_slot = state.entities.slot_of(owner).unwrap();
        let mut from = state.entities.pos(owner_slot);
        from[1] += state.entities.body(owner_slot).height * 0.85;
        let (velocity, _) = archery::aimed_arc(from, [8.5, 64.8, 128.5]).unwrap();
        crate::hostile::launch_arrow(&mut state, owner, from, velocity, 2.0).unwrap();
        let mut near_damage = 0;
        let mut enemy_damage = 0;
        for _ in 0..90 {
            let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
            for event in &output.changes.entity_events {
                if let crate::boundary::EntityEvent::Damage { entity, .. } = event {
                    near_damage += usize::from(*entity == near);
                    enemy_damage += usize::from(*entity == enemy);
                }
            }
        }
        assert_eq!(
            near_damage > 0,
            near_hit,
            "owner={owner_faction} near={near_faction}"
        );
        assert_eq!(
            enemy_damage > 0,
            !near_hit,
            "owner={owner_faction} near={near_faction}"
        );
    }
}

pub(super) fn staged_flat_fixture(side: usize) -> crate::tick::TickState {
    let mut state = fixture();
    staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &staging::BattleCommand::Setup {
            seed: 42,
            count: side,
        },
    )
    .unwrap();
    let desired = state.desired_chunks();
    state.world.ensure_blocking(&desired);
    state.refresh_active_chunks(state.tick);
    for _ in 0..8 {
        crate::tick::run_tick(&mut state, &[]);
    }
    state
}

#[test]
fn native_tick_pursues_routed_enemies_only_for_the_staged_scenario() {
    use formation::Order;
    for staged in [true, false] {
        let mut state = staged_flat_fixture(100);
        staging::execute(
            &mut state,
            &mut crate::boundary::ChangeSet::default(),
            [8.5, 64.0, 8.5],
            &staging::BattleCommand::Start,
        )
        .unwrap();
        if !staged {
            state.tickets.remove(staging::OWNER);
        }
        let regiment_id = state
            .hostile
            .battle
            .formation
            .regiments
            .iter()
            .filter(|regiment| regiment.spec.faction == 1 && regiment.spec.order == Order::Advance)
            .max_by_key(|regiment| regiment.len)
            .unwrap()
            .spec
            .id;
        // A controlled native morale state, not a mocked staging callback.
        for regiment in &mut state.hostile.battle.formation.regiments {
            if regiment.spec.faction == 2 {
                regiment.morale.broken = true;
            }
        }
        for _ in 0..21 {
            crate::tick::run_tick(&mut state, &[]);
        }
        let regiment = state
            .hostile
            .battle
            .formation
            .regiments
            .iter()
            .find(|regiment| regiment.spec.id == regiment_id)
            .unwrap();
        assert!(regiment.enemy_routing);
        assert_eq!(
            regiment.spec.order,
            if staged {
                Order::Pursue
            } else {
                Order::Advance
            }
        );
        assert!(
            state
                .hostile
                .battle
                .formation
                .regiments
                .iter()
                .any(|regiment| regiment.spec.faction == 1 && regiment.spec.order == Order::Hold),
            "holding archers lost their order"
        );
    }
}

fn melee_contact(battle: &BattleState) -> bool {
    battle.formation.regiments.iter().any(|regiment| {
        !battle.columns.ranged[regiment.start]
            && regiment.enemy.is_some()
            && regiment.gap <= 6.0
            && !regiment.morale.broken
            && regiment.live_count > 0
    })
}

fn native_battle_losses(state: &crate::tick::TickState) -> ([usize; 2], [usize; 2], usize) {
    let mut dead = [0; 2];
    let mut removed = [0; 2];
    let mut native_wounds = 0;
    for member in &state.hostile.battle.formation.members {
        let faction = state.hostile.battle.formation.regiments[member.regiment]
            .spec
            .faction;
        let i = usize::from(faction - 1);
        let health = state
            .entities
            .slot_of(member.id)
            .map(|slot| state.entities.row(slot).health)
            .filter(|health| *health > 0.0);
        dead[i] += usize::from(health.is_none());
        removed[i] += usize::from(health.is_none() || member.wounded);
        native_wounds += usize::from(member.wounded && health.is_some_and(|health| health < 20.0));
    }
    (dead, removed, native_wounds)
}

fn log_attrition(state: &crate::tick::TickState, elapsed: usize, engaged_ticks: u64) {
    let (dead, removed, native_wounds) = native_battle_losses(state);
    let minutes = engaged_ticks as f64 / (20.0 * 60.0);
    if minutes > 0.0 {
        eprintln!(
            "BATTLE_ATTRITION elapsed={elapsed} engaged_ticks={engaged_ticks} native_deaths={dead:?} removed={removed:?} native_wounds={native_wounds} deaths_per_engaged_minute={:?} removals_per_engaged_minute={:?}",
            dead.map(|count| count as f64 / minutes),
            removed.map(|count| count as f64 / minutes)
        );
    }
}

/// In-process native tick diagnostic, not a paced/networked TPS or checkpoint
/// gate. Build in the ordinary lane; run its frozen release test binary in -p
/// with MC_CORE_PHASE_LOG=1 and retain host load alongside the receipt.
#[test]
#[ignore = "release battle phase diagnostic; run in the perf lane"]
fn battle_phase_micro() {
    assert!(std::env::var_os("MC_CORE_PHASE_LOG").is_some());
    let side =
        std::env::var("BATTLE_UNITS_PER_SIDE").map_or(10_000, |value| value.parse().unwrap());
    let ticks =
        std::env::var("BATTLE_MICRO_TICKS").map_or(200, |value| value.parse::<usize>().unwrap());
    assert!(ticks > 0);
    let mut state = staged_flat_fixture(side);
    let mut changes = crate::boundary::ChangeSet::default();
    let origin = [8.5, 64.0, 8.5];
    for running in [false, true] {
        if running {
            staging::execute(
                &mut state,
                &mut changes,
                origin,
                &staging::BattleCommand::Start,
            )
            .unwrap();
        }
        let phase_ticks = if running { ticks } else { ticks.min(100) };
        let mut samples = Vec::with_capacity(phase_ticks);
        let mut swings = 0;
        let mut strikes = 0;
        let mut kills = 0;
        let mut arrows = 0;
        let mut parries = 0;
        let mut wounds = 0;
        let mut friendly_hits = 0;
        let mut enemy_hits = 0;
        let mut engaged_ticks = 0;
        let mut previous_losses = vec![(0, 0); state.hostile.battle.formation.regiments.len()];
        for elapsed in 1..=phase_ticks {
            let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
            for event in &output.changes.entity_events {
                let crate::boundary::EntityEvent::Damage {
                    entity,
                    cause: Some(cause),
                    ..
                } = event
                else {
                    continue;
                };
                let Some(victim) = state.entities.slot_of_any(*entity) else {
                    continue;
                };
                let Some(attacker) = state.entities.slot_of_any(*cause) else {
                    continue;
                };
                let a = state.entities.body(attacker).faction;
                let b = state.entities.body(victim).faction;
                if a != 0 && b != 0 {
                    friendly_hits += u64::from(a == b);
                    enemy_hits += u64::from(a != b);
                }
            }
            let work = state.hostile.battle.combat.work;
            swings += work.swings;
            strikes += work.strikes;
            kills += work.killed;
            arrows += work.arrows;
            parries += work.parries;
            wounds += work.wounded;
            if running {
                engaged_ticks += u64::from(melee_contact(&state.hostile.battle));
            }
            if elapsed % 200 == 0 || elapsed == phase_ticks {
                eprintln!(
                    "BATTLE_ACTIVITY running={running} elapsed={elapsed} swings={swings} strikes={strikes} parries={parries} melee_kills={kills} wounds={wounds} arrows={arrows} friendly_hits={friendly_hits} enemy_hits={enemy_hits}"
                );
                if running {
                    log_regiment_activity(&state, elapsed, &mut previous_losses);
                    log_attrition(&state, elapsed, engaged_ticks);
                }
            }
            samples.push(state.hostile.battle.timings);
        }
        let percentile = |pick: fn(Timings) -> u64, percentile: usize| {
            let mut values: Vec<_> = samples.iter().copied().map(pick).collect();
            values.sort_unstable();
            values[((values.len() - 1) * percentile) / 100]
        };
        eprintln!(
            "BATTLE_MICRO side={side} running={running} ticks={phase_ticks} units={} total_us_p50={} total_us_p95={} total_us_p99={} counters_us_p95={} formation_us_p95={} columns_us_p95={} targets_us_p95={} steering_us_p95={} combat_us_p95={} morale_us_p95={} projection_us_p95={} emit_us_p95={}",
            unit_counters(&state.hostile).units,
            percentile(|t| t.total_us, 50),
            percentile(|t| t.total_us, 95),
            percentile(|t| t.total_us, 99),
            percentile(|t| t.counters_us, 95),
            percentile(|t| t.formation_us, 95),
            percentile(|t| t.columns_us, 95),
            percentile(|t| t.targeting_us, 95),
            percentile(|t| t.steering_us, 95),
            percentile(|t| t.combat_us, 95),
            percentile(|t| t.morale_us, 95),
            percentile(|t| t.projection_us, 95),
            percentile(|t| t.emit_us, 95),
        );
    }
}

fn log_regiment_activity(
    state: &crate::tick::TickState,
    elapsed: usize,
    previous_losses: &mut [(usize, usize)],
) {
    let battle = &state.hostile.battle;
    for (ri, regiment) in battle.formation.regiments.iter().enumerate() {
        let mut alive = 0;
        let mut moving = 0;
        let mut targets = 0;
        let mut resting = 0;
        let mut front = [0.0; 3];
        let mut front_count = 0;
        let mut ammunition = 0_u64;
        let mut wounded = 0;
        for bi in regiment.start..regiment.start + regiment.len {
            let member = &battle.formation.members[bi];
            let Some(slot) = state.entities.slot_of(member.id) else {
                continue;
            };
            if state.entities.row(slot).health <= 0.0 {
                continue;
            }
            alive += 1;
            moving += usize::from(state.entities.vel(slot).iter().any(|v| *v != 0.0));
            resting +=
                usize::from(state.entities.motion_flags(slot) & crate::entities::MOTION_REST != 0);
            targets += usize::from(battle.columns.targets.get(bi).is_some_and(Option::is_some));
            wounded += usize::from(member.wounded);
            ammunition += u64::from(battle.combat.fighters[bi].arrows);
            if member.rank == 0 {
                let pos = state.entities.pos(slot);
                for axis in 0..3 {
                    front[axis] += pos[axis];
                }
                front_count += 1;
            }
        }
        if front_count != 0 {
            front = front.map(|v| v / front_count as f64);
        }
        let pulsing = regiment
            .sections
            .iter()
            .filter(|section| section.pulse_until > battle.elapsed_ticks)
            .count();
        eprintln!(
            "BATTLE_REGIMENT elapsed={elapsed} id={} side={} order={:?} alive={alive} dead={} moving={moving} rest_hint={resting} targets={targets} wounded={wounded} ammunition={ammunition} origin={:?} front={front:?} gap={} enemy={:?} speed={} stamina={} morale={} broken={} pulsing={pulsing}",
            regiment.spec.id,
            regiment.spec.faction,
            regiment.spec.order,
            regiment.len - alive,
            regiment.spec.origin,
            regiment.gap,
            regiment.enemy_regiment,
            regiment.speed,
            regiment.stamina,
            regiment.morale.value,
            regiment.morale.broken
        );
        let dead = regiment.len - alive;
        let removed = dead + wounded;
        eprintln!(
            "BATTLE_WINDOW elapsed={elapsed} id={} side={} deaths={} removals={}",
            regiment.spec.id,
            regiment.spec.faction,
            dead as i64 - previous_losses[ri].0 as i64,
            removed as i64 - previous_losses[ri].1 as i64
        );
        previous_losses[ri] = (dead, removed);
    }
}
