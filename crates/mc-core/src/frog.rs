//! Pregnant frogs seek land beside water and lay frogspawn.
use crate::boundary::BlockPos;
use crate::entities::ColdRow;
use crate::tick::TickState;

static TYPE: std::sync::LazyLock<u32> =
    std::sync::LazyLock::new(|| crate::piglin::type_id_of("minecraft:frog").expect("frog type"));

pub(crate) fn is_frog(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    if !row.entity_life.frog_pregnant {
        return None;
    }
    let target = row.entity_life.frog_spawn_target?;
    Some((
        [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]),
            f64::from(target[2]) + 0.5,
        ],
        f64::from(row.speed),
    ))
}

fn spawn_at(state: &TickState, shore: [i32; 3]) -> Option<BlockPos> {
    let [x, y, z] = shore;
    let ground = state.world.block_at(x, y - 1, z)?;
    if state.registry.materials.classify(ground) != mc_physics::BlockMaterial::Solid
        || state.world.block_at(x, y, z) != Some(state.registry.states.air)
    {
        return None;
    }
    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
        let water = state.world.block_at(x + dx, y - 1, z + dz);
        if water.is_some_and(|id| {
            state.registry.materials.classify(id) == mc_physics::BlockMaterial::Water
        }) && state.world.block_at(x + dx, y, z + dz) == Some(state.registry.states.air)
        {
            return Some(BlockPos {
                x: x + dx,
                y,
                z: z + dz,
            });
        }
    }
    None
}

fn site(state: &TickState, pos: [f64; 3]) -> Option<[i32; 3]> {
    let origin = pos.map(|v| v.floor() as i32);
    let mut best = None;
    for x in origin[0] - 8..=origin[0] + 8 {
        for z in origin[2] - 8..=origin[2] + 8 {
            for y in origin[1] - 3..=origin[1] + 3 {
                let shore = [x, y, z];
                if spawn_at(state, shore).is_none() {
                    continue;
                }
                let distance = (f64::from(x) + 0.5 - pos[0]).powi(2)
                    + (f64::from(y) - pos[1]).powi(2)
                    + (f64::from(z) + 0.5 - pos[2]).powi(2);
                if best.is_none_or(|(_, old)| distance < old) {
                    best = Some((shore, distance));
                }
            }
        }
    }
    best.map(|(shore, _)| shore)
}

pub(crate) fn run(state: &mut TickState) {
    let slots: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in slots {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        if !body.alive || row.controls.no_ai || !row.entity_life.frog_pregnant {
            continue;
        }
        let id = body.id;
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let here = pos.map(|v| v.floor() as i32);
        if state.entities.ground(slot)
            && let Some(at) = spawn_at(state, here)
        {
            let name = mc_data::Identifier::parse("minecraft:frogspawn").expect("frogspawn id");
            if let Some(block) = state.registry.blocks.block(&name) {
                let block = block.default.0;
                state.write_block(at, block);
                let life = &mut state.entities.row_mut(slot).entity_life;
                life.frog_pregnant = false;
                life.frog_spawn_target = None;
            }
        } else if state.tick.wrapping_add(id.0 as u64).is_multiple_of(20) {
            let target = site(state, pos);
            state.entities.row_mut(slot).entity_life.frog_spawn_target = target;
        }
    }
}
