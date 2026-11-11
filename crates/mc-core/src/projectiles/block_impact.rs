//! Vanilla 26.3 `onProjectileHit` block reactions that depend on the
//! projectile's `mayInteract`/`mayBreak` (server jar javap: `Projectile`,
//! `DecoratedPotBlock`, `ChorusFlowerBlock`, `SpeleothemBlock`, `TntBlock`,
//! `CampfireBlock`).

use super::{ProjectileKind, ProjectileState};
use crate::boundary::BlockPos;
use crate::command_rules::{BoolRule, boolean};
use crate::entities::{EntityKind, EntitySlot};
use crate::tick::TickState;

/// `Projectile.mayInteract`: a player owner needs build access at `pos`
/// (spawn protection), any other loaded owner needs `mob_griefing`, and an
/// ownerless projectile may always interact.
pub(super) fn may_interact(state: &TickState, record: &ProjectileState, pos: BlockPos) -> bool {
    match super::owner_id(state, record) {
        None => true,
        Some(owner) if state.players.contains_key(&owner) => {
            !crate::spawn_protection::protected(state, owner, pos)
        }
        Some(_) => boolean(state, BoolRule::MOB_GRIEFING),
    }
}

/// `Projectile.mayBreak`: an `impact_projectiles` entity, the
/// `projectiles_can_break_blocks` rule, and `canBreakBlockInAdventureMode`.
/// The last passes for an ownerless shot (the default game type is survival)
/// and a non-adventure player owner; mob owners and adventure players need a
/// `can_break` predicate, which Solaris does not evaluate anywhere.
fn may_break(state: &TickState, slot: EntitySlot, record: &ProjectileState) -> bool {
    let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
        return false;
    };
    state.registry.tags.contains_raw_id(
        mc_data::tags::ENTITY_TYPE_REGISTRY,
        mc_data::tags::IMPACT_PROJECTILES,
        type_id as i32,
    ) && boolean(state, BoolRule::PROJECTILES_CAN_BREAK_BLOCKS)
        && super::owner_id(state, record).is_none_or(|owner| {
            state
                .players
                .get(&owner)
                .is_some_and(|player| player.game_mode != crate::commands::GameMode::Adventure)
        })
}

/// `DecoratedPotBlock`, `ChorusFlowerBlock` and `SpeleothemBlock` (pointed
/// dripstone, sulfur spike) `onProjectileHit`: `Level.destroyBlock` with
/// empty-tool drops. A pot cracks first, so it drops its sherds; a speleothem
/// breaks only for a trident moving faster than 0.6 blocks per tick.
pub(super) fn destroy(
    state: &mut TickState,
    slot: EntitySlot,
    record: &ProjectileState,
    pos: BlockPos,
    velocity: [f64; 3],
) {
    let Some(old) = state.world.block_at(pos.x, pos.y, pos.z) else {
        return;
    };
    let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(old)) else {
        return;
    };
    let broken = match block.block.id.path() {
        "decorated_pot" => {
            let mut properties = block.properties.clone();
            let Some((_, value)) = properties.iter_mut().find(|(key, _)| key == "cracked") else {
                return;
            };
            *value = "true".to_owned();
            let Some(cracked) = state
                .registry
                .blocks
                .by_name_and_props(&block.block.id, &properties)
            else {
                return;
            };
            cracked.0
        }
        "chorus_flower" => old,
        "pointed_dripstone" | "sulfur_spike"
            if record.kind == ProjectileKind::Trident
                && velocity.iter().map(|v| v * v).sum::<f64>() > 0.6 * 0.6 =>
        {
            old
        }
        _ => return,
    };
    let replacement = if block
        .properties
        .iter()
        .any(|(key, value)| key == "waterlogged" && value == "true")
    {
        state.registry.states.water
    } else {
        state.registry.states.air
    };
    if !may_interact(state, record, pos)
        || !may_break(state, slot, record)
        || !zone_allows(state, record, pos)
        || !state.stage_command_break(pos, broken)
    {
        return;
    }
    state.write_block(pos, replacement);
    // LevelEvent 2001: block destroy particles and break sound.
    super::payloads::level_event(
        state,
        [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z) + 0.5,
        ],
        2001,
        broken as i32,
    );
}

/// Plugin protected zones: a player's shot breaks only where the zone admits
/// that player; any other shot never breaks a protected cell (as explosions).
fn zone_allows(state: &TickState, record: &ProjectileState, pos: BlockPos) -> bool {
    match super::owner_id(state, record) {
        Some(owner) if state.players.contains_key(&owner) => {
            crate::plugin_player::allows_block(state, owner, pos)
        }
        _ => !crate::plugin_player::protects_cell(state, pos),
    }
}
