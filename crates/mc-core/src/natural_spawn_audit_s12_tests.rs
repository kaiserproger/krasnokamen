//! H16: pinned exhaustive reference from 6b594d8f. Keep independent of the
//! typed admission path: compare complete census and maintenance effects.
use super::*;
use std::sync::Arc;

thread_local! {
    static VISITS: std::cell::Cell<[usize; 3]> = const { std::cell::Cell::new([0; 3]) };
}

pub(super) fn visit(pass: usize) {
    VISITS.with(|visits| {
        let mut counts = visits.get();
        counts[pass] += 1;
        visits.set(counts);
    });
}

fn visits() -> [usize; 3] {
    VISITS.with(|visits| visits.replace([0; 3]))
}

fn old_run_with(
    state: &mut TickState,
    changes: &mut ChangeSet,
    admitted: Option<Admitted>,
    min_parallel: usize,
) {
    // Reuse the census-owned observer scratch across ticks: the only
    // per-observer borrow is the entity position read, so the table keeps
    // sorted-by-id order for the RNG/local-cap accounting below.
    let mut observers = std::mem::take(&mut state.spawn_census.observer_table);
    let mut positions = std::mem::take(&mut state.spawn_census.observer_positions);
    observers.clear();
    positions.clear();
    observers.extend(
        state
            .players
            .iter()
            .filter(|(_, player)| !player.spectator())
            .filter_map(|(id, _)| {
                state
                    .entities
                    .slot_of(*id)
                    .map(|slot| (*id, state.entities.pos(slot)))
            }),
    );
    let mut rng = Rng((state.world.seed() as u64) ^ state.tick.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    // No observers pauses distance-based despawning, not Peaceful or item expiry.
    if observers.is_empty() {
        state.spawn_census.observer_table = observers;
        state.spawn_census.observer_positions = positions;
        despawn(
            state,
            &crate::target_grid::TargetGrid::linear(&[]),
            &mut rng,
            changes,
            admitted,
            min_parallel,
        );
        return;
    }
    observers.sort_unstable_by_key(|(id, _)| id.0);
    let grid = crate::target_grid::TargetGrid::for_consumers(&observers, state.entities.len());
    despawn(state, &grid, &mut rng, changes, admitted, min_parallel);

    // The disabled-spawn path only needs burn/graze: keep the position
    // projection (and its census/spawn consumers) out of it entirely.
    let cycle = state.tick.is_multiple_of(CYCLE);
    if !crate::command_rules::boolean(state, crate::command_rules::BoolRule::SPAWN_MOBS) {
        if cycle {
            old_burn(state, is_night(state.day_time), changes);
            old_graze(state, &mut rng, changes);
        }
        state.spawn_census.observer_table = observers;
        state.spawn_census.observer_positions = positions;
        return;
    }
    // Project positions once per pass from the same sorted table instead of
    // re-collecting a second allocation.
    positions.extend(observers.iter().map(|(_, pos)| *pos));

    let night = is_night(state.day_time);
    let spawn_monsters =
        crate::command_rules::boolean(state, crate::command_rules::BoolRule::SPAWN_MONSTERS);
    old_census(state, &positions);
    if spawn_monsters && state.difficulty != crate::survival::Difficulty::Peaceful {
        phantoms::run(state, &observers, &mut rng, changes);
        patrols::run(state, &observers, &mut rng, changes);
        hostile::run(state, &positions, &mut rng, changes);
    }
    if !cycle {
        state.spawn_census.observer_table = observers;
        state.spawn_census.observer_positions = positions;
        return;
    }
    let eligible = state.spawn_census.eligible.len();
    for (observer, around) in positions.iter().enumerate() {
        for (index, category) in CATEGORIES.into_iter().enumerate() {
            if category == MobCategory::Monster
                || (category == MobCategory::Creature && !state.tick.is_multiple_of(CREATURE_CYCLE))
            {
                continue;
            }
            let cap = category.max_instances_per_chunk() as usize;
            let global_cap = cap * eligible / 289;
            let local = state.spawn_census.local_count(observer, index);
            let room = global_cap
                .saturating_sub(state.spawn_census.global[index])
                .min(cap.saturating_sub(local));
            if room != 0 {
                spawn_group(
                    state, &positions, *around, category, room, &mut rng, changes,
                );
            }
        }
    }

    old_burn(state, night, changes);
    old_graze(state, &mut rng, changes);
    state.spawn_census.observer_table = observers;
    state.spawn_census.observer_positions = positions;
}

fn old_observe_slot(state: &mut TickState, slot: EntitySlot) {
    let row = state.entities.body(slot);
    if !row.alive {
        return;
    }
    let entity = row.id;
    let position = state.entities.pos(slot);
    let width = row.half_width;
    // Native rows outlive residency, unlike vanilla's loaded-entity census.
    // Keep their bodies indexed for overlap checks, but not in loaded mob caps.
    let cell = chunk_of(position);
    let category = (!persistent(state.entities.row(slot)) && state.world.has_chunk(cell.0, cell.1))
        .then(|| category_of(state, slot))
        .and_then(category_index);
    state
        .spawn_census
        .observe(entity, position, width, category);
    if category.is_some() {
        crate::dimensions::spawning::observe(state, slot);
    }
}

/// Build the one population census and all observer-local category counters.
/// Direct pack callers must rebuild after changes outside their spawn pass.
pub(crate) fn old_census(state: &mut TickState, players: &[[f64; 3]]) -> [usize; 7] {
    state.spawn_census.begin();
    // Build chunk totals first; do not multiply the population scan by players.
    for index in 0..state.entities.len() {
        visit(0);
        old_observe_slot(state, EntitySlot(index as u32));
    }
    state.spawn_census.chunks.retain(|_, chunk| chunk.active);
    for player in players {
        let cell = chunk_of(*player);
        let mut counts = [0; 7];
        for x in cell.0 - 8..=cell.0 + 8 {
            for z in cell.1 - 8..=cell.1 + 8 {
                if state.world.is_active(x, z) {
                    state.spawn_census.eligible.insert((x, z));
                }
                if local_in_range(*player, (x, z))
                    && let Some(chunk) = state.spawn_census.chunks.get(&(x, z))
                {
                    for (count, population) in counts.iter_mut().zip(&chunk.counts) {
                        *count += *population;
                    }
                }
            }
        }
        state.spawn_census.local_observers.push(*player);
        state.spawn_census.local_counts.push(counts);
    }
    state.spawn_census.tick = Some(state.tick);
    state.spawn_census.global
}

/// Vanilla `isSunBurnTick` mobs: the blueprint says whether it ignites by day.
fn burns_in_daylight(state: &TickState, slot: EntitySlot) -> bool {
    match state.entities.body(slot).kind {
        EntityKind::Mob { type_id } => state
            .registry
            .blueprint(type_id)
            .is_some_and(|blueprint| blueprint.capabilities.burns_in_daylight),
        _ => false,
    }
}

fn old_burn(state: &mut TickState, night: bool, changes: &mut ChangeSet) {
    if night || state.rain_level > 0.0 {
        return;
    }
    for index in 0..state.entities.len() as u32 {
        visit(1);
        let slot = EntitySlot(index);
        // Daylight burn is live simulation: freeze resident-inactive.
        let bpos = state.entities.pos(slot);
        if !state.world.is_active(
            (bpos[0].floor() as i32).div_euclid(16),
            (bpos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let row = state.entities.row(slot);
        if state.entities.body(slot).alive
            && burns_in_daylight(state, slot)
            && open_sky(
                state,
                state.entities.pos(slot),
                state.entities.body(slot).height,
            )
        {
            let id = state.entities.body(slot).id;
            // Mob.burnUndead protects with any nonempty head item, including
            // nondamageable pumpkins. A damageable head item takes nextInt(2)
            // wear; even its breaking pass is protected (javap-daylight.log).
            if let Some(head) = row.appearance.equipment[5]
                .as_ref()
                .filter(|stack| !stack.is_empty())
            {
                let maximum = head.extended().max_damage.or_else(|| {
                    state
                        .registry
                        .item_facts
                        .facts_for_stack(head, &state.registry.items)
                        .and_then(|facts| facts.max_damage)
                });
                let wear = Rng((state.world.seed() as u64)
                    ^ state.tick.wrapping_mul(0xBF58_476D_1CE4_E5B9)
                    ^ id.0 as u64)
                .below(2) as i32;
                if let Some(maximum) = maximum.filter(|maximum| *maximum != 0)
                    && wear != 0
                {
                    let head =
                        &mut std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance)
                            .equipment[5];
                    let stack = head.as_mut().expect("protected by nonempty head item");
                    let damage = stack.damage.unwrap_or(0).saturating_add(wear);
                    if u32::try_from(damage).is_ok_and(|damage| damage >= maximum) {
                        *head = None;
                    } else {
                        *stack = std::mem::take(stack).with_damage(damage);
                    }
                    changes.entities_appearance_changed.push(id);
                }
                continue;
            }
            crate::contact_hazards::ignite(state, id, 160);
        }
    }
}

fn old_graze(state: &mut TickState, rng: &mut Rng, changes: &mut ChangeSet) {
    let slots = 0..state.entities.len() as u32;
    old_graze_in_order(state, rng, changes, slots);
}

// The iterator seam is test-only: the second caller injects the RNG-order mutant.
fn old_graze_in_order(
    state: &mut TickState,
    rng: &mut Rng,
    changes: &mut ChangeSet,
    slots: impl Iterator<Item = u32>,
) {
    let mob_griefing =
        crate::command_rules::mob_griefing(state, crate::command_rules::MobGrief::SheepGrazing);
    for index in slots {
        visit(2);
        let slot = EntitySlot(index);
        // Graze/block-grief is live simulation: freeze resident-inactive.
        let gpos = state.entities.pos(slot);
        if !state.world.is_active(
            (gpos[0].floor() as i32).div_euclid(16),
            (gpos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let row = state.entities.row(slot);
        let wool = crate::tick::wool_of(&row.appearance);
        let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
            continue;
        };
        if !state.entities.body(slot).alive
            || wool & 0x10 == 0
            || !state
                .registry
                .blueprint(type_id)
                .is_some_and(|blueprint| blueprint.wool)
            || rng.below(1000 / CYCLE) != 0
        {
            continue;
        }
        let id = state.entities.body(slot).id;
        let pos = state.entities.pos(slot);
        let below = crate::boundary::BlockPos {
            x: pos[0].floor() as i32,
            y: pos[1].floor() as i32 - 1,
            z: pos[2].floor() as i32,
        };
        if state.world.block_at(below.x, below.y, below.z)
            != Some(state.registry.states.grass_block)
        {
            continue;
        }
        if mob_griefing {
            let dirt = state.registry.states.dirt;
            state.world.set_block(below.x, below.y, below.z, dirt);
            changes.blocks.push(crate::boundary::BlockChange {
                position: below,
                state: dirt,
            });
        }
        let row = state.entities.row_mut(slot);
        crate::tick::set_wool(std::sync::Arc::make_mut(&mut row.appearance), wool & 0x0F);
        state.appearance_changed.push(id);
    }
}

struct Flat {
    air: u32,
    grass: u32,
    geometry: mc_world::ChunkGeometry,
}

impl mc_world::ChunkGenerator for Flat {
    fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
        let mut chunk = mc_world::Chunk::empty_with_geometry(
            pos,
            mc_world::BlockStateId(self.air),
            mc_data::Identifier::parse("minecraft:plains").unwrap(),
            self.geometry,
        );
        for x in 0..16 {
            for z in 0..16 {
                chunk.set_block(x, 249, z, mc_world::BlockStateId(self.grass));
            }
        }
        chunk
    }
}

fn fixture() -> TickState {
    fixture_in(crate::dimensions::Dimension::Overworld)
}

fn fixture_in(dimension: crate::dimensions::Dimension) -> TickState {
    let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let generator = Arc::new(Flat {
        air: registry.states.air,
        grass: registry.states.grass_block,
        geometry: dimension.geometry(),
    });
    let mut world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [0, 250, 0],
        dimension.geometry(),
    );
    let wanted = crate::world::chunks_in_radius(mc_world::ChunkPos { x: 0, z: 0 }, 4)
        .into_iter()
        .map(|pos| (pos, 0))
        .collect();
    world.ensure_blocking(&wanted);
    world.set_active_chunks([mc_world::ChunkPos { x: 0, z: 0 }].into_iter().collect(), 0);
    let mut state = TickState::new(registry, world, 4, 512);
    state.dimension = dimension;
    state.day_time = 6_000;
    state.tick = 400;
    state
}

fn mob(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntitySlot {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap();
    let id = state.spawn_mob(type_id, pos, 20.0);
    state.entities.slot_of(id).unwrap()
}

fn mixed(state: &mut TickState) {
    for name in [
        "zombie",
        "cow",
        "bat",
        "axolotl",
        "glow_squid",
        "squid",
        "cod",
    ] {
        mob(state, name, [8.5, 250.0, 8.5]);
    }
    for index in 0..9 {
        let slot = mob(state, "sheep", [f64::from(index) + 0.5, 250.0, 0.5]);
        crate::tick::set_wool(
            Arc::make_mut(&mut state.entities.row_mut(slot).appearance),
            0x13,
        );
        match index {
            0 => state.entities.body_mut(slot).alive = false,
            1 => state.entities.row_mut(slot).persistent = true,
            2 => {
                Arc::make_mut(&mut state.entities.row_mut(slot).appearance).custom_name =
                    Some("named".into())
            }
            3 | 4 => {
                let mut life = state.entities.row(slot).entity_life.clone();
                life.tamed = index == 3;
                life.mounted = index == 4;
                state.entities.set_entity_life(slot, life);
            }
            5 => state.entities.set_pos(slot, [64.5, 250.0, 0.5]),
            6 => state.entities.set_pos(slot, [656.5, 250.0, 0.5]),
            _ => {}
        }
    }
    for (index, name) in ["stone", "iron_helmet", "iron_helmet"]
        .into_iter()
        .enumerate()
    {
        let kind = if index == 1 { "skeleton" } else { "zombie" };
        let slot = mob(state, kind, [index as f64 + 0.5, 250.0, 3.5]);
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
            .unwrap();
        let mut stack = mc_data::ItemStack::new(item, 1);
        if index == 2 {
            stack = stack.with_max_damage(1).unwrap();
        }
        Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[5] = Some(stack);
    }
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        state,
        [-0.5, 250.0, -0.5],
        mc_data::ItemStack::new(item, 1),
    );
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_half_width(slot, 20.0);
}

fn assert_census(actual: &TickState, expected: &TickState) {
    let a = &actual.spawn_census;
    let b = &expected.spawn_census;
    assert_eq!(a.global, b.global);
    assert_eq!(a.local_counts, b.local_counts);
    assert_eq!(a.local_observers, b.local_observers);
    assert_eq!(a.eligible, b.eligible);
    assert_eq!(a.max_half_width, b.max_half_width);
    assert_eq!(a.tick, b.tick);
    assert_eq!(a.observer_table, b.observer_table);
    assert_eq!(a.observer_positions, b.observer_positions);
    assert_eq!(a.phantom_next_tick, b.phantom_next_tick);
    assert_eq!(a.patrol_next_tick, b.patrol_next_tick);
    assert_eq!(
        format!("{:?}", a.dimension_charges),
        format!("{:?}", b.dimension_charges),
    );
    assert_eq!(a.chunks.len(), b.chunks.len());
    for (cell, chunk) in &a.chunks {
        let old = &b.chunks[cell];
        assert_eq!(chunk.counts, old.counts);
        assert_eq!(chunk.entities, old.entities);
        assert_eq!(chunk.active, old.active);
    }
    for slot in expected.entities.slots() {
        let pos = expected.entities.pos(slot);
        for dx in [-16.0, -0.3, 0.0, 0.3, 16.0] {
            let probe = [pos[0] + dx, pos[1], pos[2]];
            assert_eq!(
                a.overlaps(&actual.entities, probe, 0.3, 1.8),
                b.overlaps(&expected.entities, probe, 0.3, 1.8),
                "overlap at {probe:?}",
            );
        }
    }
}

fn assert_state(actual: &TickState, expected: &TickState, a: &ChangeSet, b: &ChangeSet) {
    assert_eq!(format!("{a:?}"), format!("{b:?}"), "ordered changes");
    assert_eq!(actual.appearance_changed, expected.appearance_changed);
    assert_eq!(actual.entities.len(), expected.entities.len());
    for slot in actual.entities.slots() {
        assert_eq!(actual.entities.body(slot), expected.entities.body(slot));
        assert_eq!(actual.entities.pos(slot), expected.entities.pos(slot));
        assert_eq!(actual.entities.row(slot), expected.entities.row(slot));
    }
    for x in -1..16 {
        for z in -1..16 {
            assert_eq!(
                actual.world.block_at(x, 249, z),
                expected.world.block_at(x, 249, z)
            );
        }
    }
}

#[test]
fn full_census_matches_exhaustive_across_lifecycle() {
    let mut actual = fixture();
    let mut expected = fixture();
    mixed(&mut actual);
    mixed(&mut expected);
    let observers = [[0.5, 250.0, 0.5], [8.5, 250.0, 8.5], [-16.5, 250.0, -0.5]];
    for stage in 0..4 {
        if stage == 1 {
            actual.remove_entity(EntitySlot(1));
            expected.remove_entity(EntitySlot(1));
        } else if stage == 2 {
            for state in [&mut actual, &mut expected] {
                mob(state, "cow", [-0.5, 250.0, 1.5]);
                state.entities.set_pos(EntitySlot(0), [656.5, 250.0, 0.5]);
            }
        } else if stage == 3 {
            for state in [&mut actual, &mut expected] {
                state.entities.body_mut(EntitySlot(0)).alive = false;
                state.world.set_active_chunks(
                    [mc_world::ChunkPos { x: 4, z: 0 }].into_iter().collect(),
                    state.tick,
                );
            }
        }
        assert_eq!(
            census(&mut actual, &observers),
            old_census(&mut expected, &observers)
        );
        assert_census(&actual, &expected);
        if stage == 0 {
            assert_eq!(actual.spawn_census.global, [4, 4, 1, 1, 1, 1, 1]);
        }
    }
}

#[test]
fn maintenance_matches_exhaustive_rng_wool_blocks_and_head_wear() {
    for (night, rain, grief) in [
        (false, 0.0, true),
        (false, 1.0, true),
        (true, 0.0, true),
        (false, 0.0, false),
    ] {
        let mut actual = fixture();
        let mut expected = fixture();
        for state in [&mut actual, &mut expected] {
            mixed(state);
            state.rain_level = rain;
            state.mob_griefing.sheep_grazing = grief;
        }
        for seed in 0..128 {
            actual.tick = 400 + seed * CYCLE;
            expected.tick = actual.tick;
            let mut a = ChangeSet::default();
            let mut b = ChangeSet::default();
            let mut ra = Rng(seed);
            let mut rb = Rng(seed);
            burn(&mut actual, night, &mut a);
            old_burn(&mut expected, night, &mut b);
            graze(&mut actual, &mut ra, &mut a);
            old_graze(&mut expected, &mut rb, &mut b);
            assert_eq!(ra.0, rb.0, "RNG consumption");
            assert_state(&actual, &expected, &a, &b);
        }
        assert!(
            actual.appearance_changed.len() >= 4,
            "eligible wool must regrow"
        );
        let broken_head = EntitySlot(18);
        if !night && rain == 0.0 {
            assert!(actual.entities.row(broken_head).appearance.equipment[5].is_none());
            assert!(actual.entities.body(broken_head).burn_ticks > 0);
        }
    }
}

fn observer(state: &mut TickState) {
    let number = state.players.len() as u64 + 1;
    let connection = crate::boundary::ConnectionId(number);
    crate::tick::run_tick(
        state,
        &[crate::boundary::PlayerInput::Join {
            connection,
            uuid: uuid::Uuid::from_u128(u128::from(number)),
            name: format!("observer{number}"),
            permission_level: 0,
        }],
    );
    let id = state.connections.entity_of(connection).unwrap();
    state.players.get_mut(&id).unwrap().game_mode = crate::commands::GameMode::Creative;
    let slot = state.entities.slot_of(id).unwrap();
    state
        .entities
        .set_pos(slot, [number as f64 + 0.5, 250.0, 0.5]);
}

#[test]
fn whole_pass_matches_old_spawn_output_and_cadence() {
    for observer_count in [0, 1, 3] {
        for enabled in [false, true] {
            let mut actual = fixture();
            let mut expected = fixture();
            for state in [&mut actual, &mut expected] {
                for _ in 0..observer_count {
                    observer(state);
                }
                mixed(state);
                state
                    .gamerules
                    .insert("spawn_mobs".into(), enabled.to_string());
            }
            let mut births = 0;
            for tick in (399..=528).chain([800]) {
                actual.tick = tick;
                expected.tick = tick;
                actual.day_time = if tick % 20 == 0 { 6_000 } else { 18_000 };
                expected.day_time = actual.day_time;
                let mut a = ChangeSet::default();
                let mut b = ChangeSet::default();
                run(&mut actual, &mut a);
                births += a.entities_spawned.len();
                old_run_with(&mut expected, &mut b, None, MIN_PARALLEL_ROWS);
                assert_state(&actual, &expected, &a, &b);
                assert_census(&actual, &expected);
            }
            if observer_count != 0 && enabled {
                assert!(
                    births > 0,
                    "the exact spawn-output oracle must observe births"
                );
            }
        }
    }
}

/// Focused debug attribution, not an M1/M3 capacity or tick-p99 claim.
/// Run the built test executable under mem-run -x with --ignored --nocapture.
#[test]
#[ignore = "serialized focused performance attribution"]
fn focused_census_burn_graze_attribution() {
    for population in [100, 10_000] {
        for dense in [false, true] {
            let mut state = fixture();
            for index in 0..population {
                let name = if index < 8 || (dense && index % 5 != 0) {
                    match index % 4 {
                        0 | 2 => "sheep",
                        1 => "zombie",
                        _ => "skeleton",
                    }
                } else {
                    "cow"
                };
                let pos = if name == "cow" {
                    [656.5, 250.0, 0.5]
                } else {
                    [
                        (index % 16) as f64 + 0.5,
                        250.0,
                        ((index / 16) % 16) as f64 + 0.5,
                    ]
                };
                let slot = mob(&mut state, name, pos);
                // Unsheared wool keeps the timed fixture stationary; regrowth
                // and RNG-order coverage live in the differential tests.
                crate::tick::set_wool(
                    Arc::make_mut(&mut state.entities.row_mut(slot).appearance),
                    0,
                );
            }
            old_burn(&mut state, false, &mut ChangeSet::default());
            state.appearance_changed.clear();
            for count in [0, 1, 100, 1000] {
                let observers = vec![[0.5, 250.0, 0.5]; count];
                for (repeat, old) in [true, true, false, false, true, false]
                    .into_iter()
                    .enumerate()
                {
                    visits();
                    let start = std::time::Instant::now();
                    if old {
                        old_census(&mut state, &observers);
                    } else {
                        census(&mut state, &observers);
                    }
                    let census_us = start.elapsed().as_micros();
                    let start = std::time::Instant::now();
                    if old {
                        old_burn(&mut state, false, &mut ChangeSet::default());
                    } else {
                        burn(&mut state, false, &mut ChangeSet::default());
                    }
                    let burn_us = start.elapsed().as_micros();
                    let start = std::time::Instant::now();
                    if old {
                        old_graze(&mut state, &mut Rng(42), &mut ChangeSet::default());
                    } else {
                        graze(&mut state, &mut Rng(42), &mut ChangeSet::default());
                    }
                    let graze_us = start.elapsed().as_micros();
                    eprintln!(
                        "H16 n={population} dense={dense} observers={count} repeat={repeat} old={old} census_us={census_us} burn_us={burn_us} graze_us={graze_us} visits={:?}",
                        visits()
                    );
                }
            }
        }
    }
}

#[test]
fn typed_maintenance_work_is_independent_of_unrelated_population() {
    let mut state = fixture();
    for index in 0..10_000 {
        let name = match index {
            0..4 => "sheep",
            4..8 => "zombie",
            _ => "cow",
        };
        mob(&mut state, name, [656.5, 250.0, 0.5]);
    }
    visits();
    census(&mut state, &[[0.5, 250.0, 0.5]]);
    burn(&mut state, false, &mut ChangeSet::default());
    graze(&mut state, &mut Rng(42), &mut ChangeSet::default());
    assert_eq!(visits(), [10_000, 4, 4]);
}

#[test]
fn frozen_ticks_do_not_visit_census_or_maintenance() {
    let mut state = fixture();
    observer(&mut state);
    mixed(&mut state);
    state.tick_control.set_frozen(true);
    state.tick = 399;
    visits();
    crate::tick::run_tick(&mut state, &[]);
    crate::tick::run_tick(&mut state, &[]);
    assert_eq!(visits(), [0; 3]);
}

#[test]
fn transfer_and_type_change_keep_census_and_maintenance_equivalent() {
    let mut actual = fixture();
    let mut expected = fixture();
    let mut actual_destination = fixture_in(crate::dimensions::Dimension::End);
    let mut expected_destination = fixture_in(crate::dimensions::Dimension::End);
    mixed(&mut actual);
    mixed(&mut expected);
    for (source, destination) in [
        (&mut actual, &mut actual_destination),
        (&mut expected, &mut expected_destination),
    ] {
        source.entities.transfer_to(
            EntitySlot(8),
            &mut destination.entities,
            [3.5, 250.0, 0.5],
            [0.0; 2],
        );
        source.entities.set_kind(
            EntitySlot(0),
            EntityKind::Mob {
                type_id: source
                    .registry
                    .entity_types
                    .id_of(&mc_data::Identifier::parse("minecraft:sheep").unwrap())
                    .unwrap(),
            },
        );
        crate::tick::set_wool(
            Arc::make_mut(&mut source.entities.row_mut(EntitySlot(0)).appearance),
            0x14,
        );
    }
    for (a, b) in [
        (&mut actual, &mut expected),
        (&mut actual_destination, &mut expected_destination),
    ] {
        census(a, &[[0.5, 250.0, 0.5]]);
        old_census(b, &[[0.5, 250.0, 0.5]]);
        assert_census(a, b);
        let mut ca = ChangeSet::default();
        let mut cb = ChangeSet::default();
        burn(a, false, &mut ca);
        old_burn(b, false, &mut cb);
        graze(a, &mut Rng(42), &mut ca);
        old_graze(b, &mut Rng(42), &mut cb);
        assert_state(a, b, &ca, &cb);
    }
}

#[test]
fn census_oracle_rejects_non_cap_body_omission() {
    let mut mutant = fixture();
    let mut expected = fixture();
    mixed(&mut mutant);
    mixed(&mut expected);
    census(&mut mutant, &[[0.5, 250.0, 0.5]]);
    old_census(&mut expected, &[[0.5, 250.0, 0.5]]);
    assert_census(&mutant, &expected);
    // Inject the exact forbidden result: caps stay correct, only the item's
    // overlap membership disappears.
    let item = mutant.entities.item_slots().next().unwrap();
    let id = mutant.entities.body(item).id;
    for chunk in mutant.spawn_census.chunks.values_mut() {
        chunk.entities.retain(|entity| *entity != id);
    }
    let probe = [-19.0, 250.0, -19.0];
    assert!(
        expected
            .spawn_census
            .overlaps(&expected.entities, probe, 0.3, 1.8)
    );
    assert!(
        !mutant
            .spawn_census
            .overlaps(&mutant.entities, probe, 0.3, 1.8)
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_census(&mutant, &expected);
        }))
        .is_err(),
        "full census oracle accepted a missing non-cap body"
    );
}

#[test]
fn maintenance_oracle_rejects_reversed_grazer_rng_order() {
    let mut mutant = fixture();
    let mut expected = fixture();
    for state in [&mut mutant, &mut expected] {
        for index in 0..8 {
            let slot = mob(state, "sheep", [index as f64 + 0.5, 250.0, 0.5]);
            crate::tick::set_wool(
                Arc::make_mut(&mut state.entities.row_mut(slot).appearance),
                0x13,
            );
        }
    }
    let seed = (0..10_000)
        .find(|seed| {
            let mut rng = Rng(*seed);
            let draws: Vec<_> = (0..8).map(|_| rng.below(1000 / CYCLE) == 0).collect();
            draws.iter().ne(draws.iter().rev())
        })
        .expect("asymmetric grazing draws");
    let mut a = ChangeSet::default();
    let mut b = ChangeSet::default();
    let mut ra = Rng(seed);
    let mut rb = Rng(seed);
    old_graze_in_order(&mut mutant, &mut ra, &mut a, (0..8).rev());
    old_graze(&mut expected, &mut rb, &mut b);
    assert_eq!(ra.0, rb.0, "same number of draws is insufficient");
    assert!(!b.blocks.is_empty(), "mutant must exercise real grazing");
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            assert_state(&mutant, &expected, &a, &b);
        }))
        .is_err(),
        "maintenance oracle accepted reordered RNG consumers"
    );
}
