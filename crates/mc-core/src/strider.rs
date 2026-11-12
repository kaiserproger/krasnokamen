//! Strider.tick warmth and transient suffocating movement modifier (local javap).
use crate::{boundary::EntityDataValue, tick::TickState};

/// `Strider.DATA_SUFFOCATING`: Animal ends at 17, boost time is 18 (local javap).
const SUFFOCATING_INDEX: u8 = 19;

pub(crate) fn cold(row: &crate::entities::ColdRow) -> bool {
    row.appearance
        .extra
        .contains(&(SUFFOCATING_INDEX, EntityDataValue::Boolean(true)))
}

pub(crate) fn run(state: &mut TickState) {
    let Some(kind) = crate::piglin::type_id_of("minecraft:strider") else {
        return;
    };
    let mut slots = std::mem::take(&mut state.serial_entity_slots);
    slots.extend(state.entities.mob_slots(kind));
    for &slot in &slots {
        let pos = state.entities.pos(slot);
        if !state.entities.body(slot).alive
            || state.entities.row(slot).controls.no_ai
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            continue;
        }
        let [x, y, z] = pos.map(|v| v.floor() as i32);
        let warm = [y, y - 1].into_iter().any(|y| {
            state.world.block_at(x, y, z).is_some_and(|b| {
                state.registry.materials.classify(b) == mc_physics::BlockMaterial::Lava
            })
        });
        if cold(state.entities.row(slot)) != !warm {
            crate::special_mobs::metadata(
                state,
                slot,
                SUFFOCATING_INDEX,
                EntityDataValue::Boolean(!warm),
            );
            crate::entity_loot::restore(state, slot);
        }
    }
    slots.clear();
    state.serial_entity_slots = slots;
}
