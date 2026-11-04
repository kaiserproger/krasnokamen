//! Pillager patrol admission using the vanilla 26.1.2 custom-spawner cadence.
use super::{MobCategory, Rng, observe_slot};
use crate::boundary::{ChangeSet, EntityId};
use crate::tick::TickState;
use mc_data::{Identifier, ItemStack};

fn banner(state: &TickState) -> ItemStack {
    let id = state
        .registry
        .items
        .id_of(&Identifier::parse("minecraft:white_banner").unwrap())
        .expect("white banner");
    let mut stack = ItemStack::new(id, 1);
    stack.extended_mut().banner_patterns = [
        ("rhombus", 9),
        ("stripe_bottom", 8),
        ("stripe_center", 7),
        ("border", 8),
        ("stripe_middle", 15),
        ("half_horizontal", 8),
        ("circle", 8),
        ("border", 15),
    ]
    .into_iter()
    .map(|(name, color)| mc_data::item_stack::BannerPattern {
        pattern: Identifier::parse(format!("minecraft:{name}")).expect("banner pattern"),
        color,
    })
    .collect();
    stack
}

pub(super) fn run(
    state: &mut TickState,
    observers: &[(EntityId, [f64; 3])],
    rng: &mut Rng,
    changes: &mut ChangeSet,
) {
    if state.dimension != crate::dimensions::Dimension::Overworld
        || !crate::command_rules::boolean(state, crate::command_rules::BoolRule::SPAWN_PATROLS)
        || state.tick < state.spawn_census.patrol_next_tick
    {
        return;
    }
    state.spawn_census.patrol_next_tick = state.tick + 12_000 + rng.below(1_200);
    if state.day_time < 120_000 || super::is_night(state.day_time) || rng.below(5) != 0 {
        return;
    }
    let pos = observers[rng.below(observers.len() as u64) as usize].1;
    if crate::raids::village_center(state, pos).is_some() {
        return;
    }
    let offset = |rng: &mut Rng| {
        let magnitude = 24 + rng.below(24) as i32;
        if rng.below(2) == 0 {
            magnitude
        } else {
            -magnitude
        }
    };
    let mut x = pos[0].floor() as i32 + offset(rng);
    let mut z = pos[2].floor() as i32 + offset(rng);
    for cx in (x - 10).div_euclid(16)..=(x + 10).div_euclid(16) {
        for cz in (z - 10).div_euclid(16)..=(z + 10).div_euclid(16) {
            if !state.world.is_active(cx, cz) {
                return;
            }
        }
    }
    let Some(y) = super::spawn_floor(state, x, z, MobCategory::Monster) else {
        return;
    };
    if state
        .world
        .biome_at(x, y, z)
        .is_none_or(|biome| biome.as_str() == "minecraft:mushroom_fields")
    {
        return;
    }
    let Some(type_id) = state
        .registry
        .entity_types
        .id_of(&Identifier::parse("minecraft:pillager").unwrap())
    else {
        return;
    };
    let Some(blueprint) = state.registry.blueprint(type_id) else {
        return;
    };
    let health = blueprint.params.max_health;
    let count = 1
        + crate::mob_equipment::effective_at(state, [x as f64, y as f64, z as f64]).ceil() as usize;
    for index in 0..count {
        let Some(y) = super::spawn_floor(state, x, z, MobCategory::Monster) else {
            if index == 0 {
                return;
            } else {
                continue;
            }
        };
        let valid = state
            .world
            .light_at(x, y, z)
            .is_some_and(|(_, block)| block <= 8)
            && (y..=y + 1).all(|y| {
                state.world.block_at(x, y, z).is_some_and(|block| {
                    state.registry.materials.classify(block) == mc_physics::BlockMaterial::Air
                })
            });
        if !valid {
            if index == 0 {
                return;
            } else {
                continue;
            }
        }
        let entity = state.spawn_mob(type_id, [x as f64 + 0.5, y as f64, z as f64 + 0.5], health);
        let slot = state.entities.slot_of(entity).expect("new patrol member");
        crate::mob_equipment::natural(state, slot);
        if index == 0 {
            let banner = banner(state);
            let row = state.entities.row_mut(slot);
            row.loot.patrol_leader = true;
            row.loot.drop_chances[5] = 2.0;
            std::sync::Arc::make_mut(&mut row.appearance).equipment[5] = Some(banner);
        }
        observe_slot(state, slot);
        changes.entities_spawned.push(entity);
        x += rng.below(5) as i32 - rng.below(5) as i32;
        z += rng.below(5) as i32 - rng.below(5) as i32;
    }
}
