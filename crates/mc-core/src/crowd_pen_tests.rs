use super::*;

/// Pen layout of the coreperf scale fixture (`tools/coreperf/profiles.py`,
/// `scale_fixture`): interior `x0..x0+63`, `z0..z0+63`, floor top at y=64
/// (grass at y=63), one-block-thick stone walls at `x0-1`/`x0+64` and
/// `z0-1`/`z0+64` from y=64 up, summons on a two-block grid at y=64.
const PEN_X0: i32 = 0;
const PEN_Z0: i32 = 512;

fn stone(state: &TickState) -> u32 {
    state.registry.states.stone
}

fn solid_at(state: &TickState, x: i32, y: i32, z: i32) -> bool {
    state
        .world
        .block_at(x, y, z)
        .is_some_and(|id| state.registry.materials.classify(id) == mc_physics::BlockMaterial::Solid)
}

/// Block cells whose interior a body's own box overlaps (open-interval sense:
/// a face exactly on a cell boundary does not touch that cell).
fn buried_cells(state: &TickState, slot: EntitySlot) -> Vec<(i32, i32, i32)> {
    let body = state.entities.body(slot);
    let pos = state.entities.pos(slot);
    let half = body.half_width;
    let low = |value: f64| (value + 1.0e-9).floor() as i32;
    let high = |value: f64| (value - 1.0e-9).ceil() as i32 - 1;
    let mut cells = Vec::new();
    for x in low(pos[0] - half)..=high(pos[0] + half) {
        for y in low(pos[1])..=high(pos[1] + body.height) {
            for z in low(pos[2] - half)..=high(pos[2] + half) {
                if solid_at(state, x, y, z) {
                    cells.push((x, y, z));
                }
            }
        }
    }
    cells
}

fn build_pen(state: &mut TickState) {
    let stone = stone(state);
    for x in (PEN_X0 - 1)..=(PEN_X0 + 64) {
        for y in 64..=70 {
            state.world.set_block(x, y, PEN_Z0 - 1, stone);
            state.world.set_block(x, y, PEN_Z0 + 64, stone);
        }
    }
    for z in PEN_Z0..=(PEN_Z0 + 63) {
        for y in 64..=70 {
            state.world.set_block(PEN_X0 - 1, y, z, stone);
            state.world.set_block(PEN_X0 + 64, y, z, stone);
        }
    }
    state.world.fold_overlay();
}

fn pen_world() -> TickState {
    let registry = Arc::new(CoreRegistry::load(None).expect("registry"));
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    let chunk_x = (PEN_X0.div_euclid(16) - 1)..=(PEN_X0.div_euclid(16) + 5);
    let chunk_z = (PEN_Z0.div_euclid(16) - 1)..=(PEN_Z0.div_euclid(16) + 5);
    let chunks: Vec<ChunkPos> = chunk_x
        .flat_map(|x| chunk_z.clone().map(move |z| ChunkPos { x, z }))
        .collect();
    state
        .world
        .ensure_blocking(&chunks.iter().map(|pos| (*pos, 0)).collect());
    state
        .world
        .set_active_chunks(chunks.into_iter().collect(), 1);
    build_pen(&mut state);
    state.day_time = 14_000;
    state
}

fn join_target(state: &mut TickState, pos: [f64; 3]) -> crate::boundary::EntityId {
    input::session::join(
        state,
        (
            &ConnectionId(996),
            &uuid::Uuid::from_u128(996),
            &"crowd-pen-target".to_owned(),
            &0,
        ),
        &mut ChangeSet::default(),
    );
    let player = *state.players.keys().next().unwrap();
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, pos);
    state.entities.set_ground(slot, true);
    player
}

fn summon_row(state: &mut TickState, kind: &str, count: usize, z: f64) -> Vec<EntitySlot> {
    let type_id = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(kind).unwrap())
        .unwrap();
    (0..count)
        .map(|index| {
            let position = [
                f64::from(PEN_X0) + f64::from(index as i32 % 32) * 2.0 + 1.0,
                64.0,
                z,
            ];
            let id = state.spawn_mob(type_id, position, 20.0);
            let slot = state.entities.slot_of(id).unwrap();
            state.entities.set_ground(slot, true);
            slot
        })
        .collect()
}

/// A crowded pen pressed against its wall must never resolve a body into the
/// wall blocks: the movement kernel clips every step, so no body AABB may
/// overlap a solid cell, however long the crowd pushes.
#[test]
fn crowded_pen_bodies_never_enter_the_wall() {
    let mut state = pen_world();
    let mobs = summon_row(&mut state, "minecraft:zombie", 32, f64::from(PEN_Z0) + 1.0);
    // Chase target behind the `-z` wall, inside a loaded chunk (z 500.5 is in
    // chunk 31, the wall's own chunk) so the crowd presses the wall every tick.
    let _target = join_target(&mut state, [32.5, 64.0, f64::from(PEN_Z0) - 11.5]);
    let mut failures = Vec::new();
    for _ in 0..240 {
        state.tick += 1;
        let _ = run_entity_phase(&mut state, true);
        for &slot in &mobs {
            let cells = buried_cells(&state, slot);
            if !cells.is_empty() && failures.len() < 8 {
                let body = state.entities.body(slot);
                failures.push(format!(
                    "tick {} slot {} pos {:?} half_width {} height {} cells {:?}",
                    state.tick,
                    slot.0,
                    state.entities.pos(slot),
                    body.half_width,
                    body.height,
                    cells
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "bodies resolved inside pen blocks:\n{}",
        failures.join("\n")
    );
}
