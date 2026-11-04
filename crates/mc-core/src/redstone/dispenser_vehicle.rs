//! Dispenser vehicle placement: boats onto water, minecarts onto rails.

use crate::boundary::BlockPos;
use crate::tick::TickState;

/// Place a vehicle on valid water or rails; otherwise use ordinary ejection.
pub(super) fn place(
    state: &mut TickState,
    pos: BlockPos,
    target: BlockPos,
    index: usize,
    spawned: &mut Vec<crate::boundary::EntityId>,
) -> bool {
    let held = state.block_entities[&(pos.x, pos.y, pos.z)].slots[index].clone();
    let Some(item) = state.registry.items.name_of(held.item_id) else {
        return false;
    };
    let name = item.as_str();
    let path = name.strip_prefix("minecraft:").unwrap_or(name);
    let boat = path.ends_with("_boat") || path == "bamboo_raft" || path == "bamboo_chest_raft";
    let minecart = matches!(
        path,
        "minecart"
            | "chest_minecart"
            | "hopper_minecart"
            | "furnace_minecart"
            | "tnt_minecart"
            | "command_block_minecart"
    );
    if !boat && !minecart {
        return false;
    }
    let Some(type_id) = state.registry.entity_types.id_of(item) else {
        return true;
    };
    let Some(bp) = state.registry.blueprint(type_id) else {
        return true;
    };
    // A vehicle can also be dispensed through air above its supporting surface.
    let at = if minecart {
        let rail = crate::vehicles::motion::rail_at(state, target).or_else(|| {
            let block = state.world.block_at(target.x, target.y, target.z)?;
            if block != state.registry.states.air {
                return None;
            }
            let below = super::offset(target, (0, -1, 0))?;
            crate::vehicles::motion::rail_at(state, below)
        });
        let Some(rail) = rail else {
            return false;
        };
        rail.point(0.5)
    } else {
        let Some(block) = state.world.block_at(target.x, target.y, target.z) else {
            return true;
        };
        let water = |id| {
            crate::fluids::contained(state, id)
                .is_some_and(|(fluid, _)| fluid == crate::fluids::Fluid::Water)
        };
        let height = if water(block) {
            1.0
        } else if block == state.registry.states.air
            && super::offset(target, (0, -1, 0))
                .and_then(|below| state.world.block_at(below.x, below.y, below.z))
                .is_some_and(water)
        {
            0.0
        } else {
            return false;
        };
        let distance = 0.5625 + bp.half_width;
        [
            f64::from(pos.x) + 0.5 + f64::from(target.x - pos.x) * distance,
            f64::from(pos.y) + 0.5 + f64::from(target.y - pos.y) * 1.125 + height,
            f64::from(pos.z) + 0.5 + f64::from(target.z - pos.z) * distance,
        ]
    };
    let id = state.spawn_mob(type_id, at, 4.0);
    if let Some(slot) = state.entities.slot_of(id) {
        if boat {
            let yaw = match (target.x - pos.x, target.z - pos.z) {
                (1, 0) => -90.0,
                (-1, 0) => 90.0,
                (0, -1) => 180.0,
                _ => 0.0,
            };
            state.entities.set_rot(slot, yaw, 0.0);
        }
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).custom_name =
            held.custom_name.as_deref().map(str::to_owned);
    }
    let key = (pos.x, pos.y, pos.z);
    mc_data::inventory_semantics_26_1_2::decrement_stack(
        &mut state
            .block_entities
            .get_mut(&key)
            .expect("vehicle custody")
            .slots[index],
    );
    spawned.push(id);
    true
}
