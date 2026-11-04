//! Pair compatibility is blueprint data; merge rechecks the same rule as AI.
//! Egg-laying species stage a block or item birth instead of a live child.
use crate::boundary::EntityId;
use crate::entities::EntityKind;
use crate::registry::CoreRegistry;
use crate::tick::TickState;

/// Vanilla egg-laying birth staged instead of an immediate live child.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EggBirth {
    /// A sniffer egg block is placed at the parents' midpoint.
    SnifferEgg,
    /// Turtle egg blocks accumulate on the sand below the midpoint.
    TurtleEgg,
    /// Frogspawn places on the water surface nearest the midpoint.
    Frogspawn,
}

/// Egg-laying birth for a same-species pair, if vanilla lays eggs.
pub(crate) fn egg_birth(registry: &CoreRegistry, first: u32, second: u32) -> Option<EggBirth> {
    if first != second {
        return None;
    }
    let blueprint = registry.blueprint(first)?;
    match blueprint.name.as_str() {
        "minecraft:sniffer" => Some(EggBirth::SnifferEgg),
        "minecraft:turtle" => Some(EggBirth::TurtleEgg),
        "minecraft:frog" => Some(EggBirth::Frogspawn),
        _ => None,
    }
}

pub(crate) fn offspring(registry: &CoreRegistry, first: u32, second: u32) -> Option<u32> {
    let a = registry.blueprint(first)?;
    let b = registry.blueprint(second)?;
    let child = if first == second {
        a.offspring?
    } else {
        let a = a.capabilities.crossbreed?;
        let b = b.capabilities.crossbreed?;
        if a.partner != second || b.partner != first || a.offspring != b.offspring {
            return None;
        }
        a.offspring
    };
    registry.blueprint(child).map(|_| child)
}

pub(crate) fn valid(state: &TickState, actor: EntityId, partner: EntityId, child: u32) -> bool {
    if actor == partner {
        return false;
    }
    let Some(a) = state.entities.slot_of(actor) else {
        return false;
    };
    let Some(b) = state.entities.slot_of(partner) else {
        return false;
    };
    for slot in [a, b] {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        if !crate::entity_life::breed_ready(body, row)
            || !crate::entity_life::can_breed(state, body, row)
        {
            return false;
        }
    }
    let (EntityKind::Mob { type_id: first }, EntityKind::Mob { type_id: second }) =
        (state.entities.body(a).kind, state.entities.body(b).kind)
    else {
        return false;
    };
    // Turtle nesting and frog pregnancy are not yet admitted by this path.
    if matches!(
        egg_birth(&state.registry, first, second),
        Some(EggBirth::TurtleEgg | EggBirth::Frogspawn)
    ) {
        return false;
    }
    if egg_birth(&state.registry, first, second) == Some(EggBirth::SnifferEgg) {
        let pa = state.entities.pos(a);
        let pb = state.entities.pos(b);
        let x = ((pa[0] + pb[0]) * 0.5).floor() as i32;
        let y = ((pa[1] + pb[1]) * 0.5).floor() as i32;
        let z = ((pa[2] + pb[2]) * 0.5).floor() as i32;
        if state.world.block_at(x, y, z) != Some(state.registry.states.air) {
            return false;
        }
    }
    // Vanilla refuses only a both-sitting cat/wolf pair; mobile + sitting breeds.
    if state.registry.blueprint(first).is_some_and(|blueprint| {
        matches!(
            blueprint.capabilities.breeding,
            crate::blueprint::BreedingRule::TamedStanding
                | crate::blueprint::BreedingRule::TamedOneStanding
        )
    }) && state.entities.row(a).entity_life.sitting
        && state.entities.row(b).entity_life.sitting
    {
        return false;
    }
    offspring(&state.registry, first, second) == Some(child)
        && crate::target_grid::distance(state.entities.pos(a), state.entities.pos(b)) < 3.0
}
