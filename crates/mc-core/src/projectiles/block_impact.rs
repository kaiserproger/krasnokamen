//! Vanilla 26.3 `onProjectileHit` block reactions (server jar javap:
//! `Projectile`, `DecoratedPotBlock`, `ChorusFlowerBlock`, `SpeleothemBlock`,
//! `TntBlock`, `CampfireBlock`, `BellBlock`, `BigDripleafBlock`,
//! `AmethystBlock`, `AbstractCandleBlock`).

use super::{ProjectileKind, ProjectileState};
use crate::boundary::BlockPos;
use crate::command_rules::{BoolRule, boolean};
use crate::entities::{EntityKind, EntitySlot};
use crate::tick::TickState;

/// `Projectile.mayInteract`: a player owner needs build access at `pos`
/// (`ServerLevel.mayInteract`: spawn protection and
/// `WorldBorder.isWithinBounds(pos)`), any other loaded owner needs
/// `mob_griefing`, and an ownerless projectile may always interact.
pub(super) fn may_interact(state: &TickState, record: &ProjectileState, pos: BlockPos) -> bool {
    match super::owner_id(state, record) {
        None => true,
        Some(owner) if state.players.contains_key(&owner) => {
            !crate::spawn_protection::protected(state, owner, pos)
                && state
                    .border
                    .is_within_bounds(f64::from(pos.x), f64::from(pos.z))
        }
        Some(_) => boolean(state, BoolRule::MOB_GRIEFING),
    }
}

/// `Projectile.mayBreak`: an `impact_projectiles` entity, the
/// `projectiles_can_break_blocks` rule, and `canBreakBlockInAdventureMode`.
fn may_break(state: &TickState, slot: EntitySlot, record: &ProjectileState, pos: BlockPos) -> bool {
    let EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
        return false;
    };
    state.registry.tags.contains_raw_id(
        mc_data::tags::ENTITY_TYPE_REGISTRY,
        mc_data::tags::IMPACT_PROJECTILES,
        type_id as i32,
    ) && boolean(state, BoolRule::PROJECTILES_CAN_BREAK_BLOCKS)
        && can_break_in_adventure_mode(state, record, pos)
}

/// `Projectile.canBreakBlockInAdventureMode`: an ownerless shot (the default
/// game type is survival) and a non-adventure player owner pass; a mob owner
/// or an adventure player needs the projectile's `can_break` predicate, taken
/// from the arrow stack (`AbstractArrow` applies its pickup stack's components;
/// other projectiles carry none here).
fn can_break_in_adventure_mode(state: &TickState, record: &ProjectileState, pos: BlockPos) -> bool {
    let exempt = super::owner_id(state, record).is_none_or(|owner| {
        state
            .players
            .get(&owner)
            .is_some_and(|player| player.game_mode != crate::commands::GameMode::Adventure)
    });
    exempt || (record.kind.arrow() && super::can_break::can_break(state, &record.stack, pos))
}

/// `DecoratedPotBlock`, `ChorusFlowerBlock` and `SpeleothemBlock` (pointed
/// dripstone, sulfur spike) `onProjectileHit`: `Level.destroyBlock` with
/// empty-tool drops. A pot cracks first, so it drops its sherds; a speleothem
/// breaks only for a trident moving faster than 0.6 blocks per tick.
fn destroy(
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
        || !may_break(state, slot, record, pos)
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
    // `Level.destroyBlock` emits BLOCK_DESTROY at the cell.
    crate::vibration::emit(
        state,
        crate::vibration::centre(pos),
        crate::vibration::GameEvent::BlockDestroy,
        super::owner_id(state, record),
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

/// `Projectile.onHitBlock` -> `BlockState.onProjectileHit` of the struck block.
/// `face` is the struck face's outward normal; `impact` the exact hit point.
/// Reactions that change the world respect plugin protected zones like breaks.
pub(super) fn react(
    state: &mut TickState,
    slot: EntitySlot,
    record: &ProjectileState,
    pos: BlockPos,
    impact: [f64; 3],
    face: Option<[i32; 3]>,
    velocity: [f64; 3],
) {
    let Some(id) = state.world.block_at(pos.x, pos.y, pos.z) else {
        return;
    };
    let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
        return;
    };
    let path = block.block.id.path();
    match path {
        "decorated_pot" | "chorus_flower" | "pointed_dripstone" | "sulfur_spike" => {
            destroy(state, slot, record, pos, velocity);
        }
        "bell" => ring_bell(state, record, pos, id, impact, face),
        "big_dripleaf" => {
            if zone_allows(state, record, pos) {
                crate::redstone::big_dripleaf::projectile_hit(state, pos);
            }
        }
        "amethyst_block"
        | "budding_amethyst"
        | "amethyst_cluster"
        | "small_amethyst_bud"
        | "medium_amethyst_bud"
        | "large_amethyst_bud" => {
            let mut rng = crate::natural_spawn::Rng(
                state.tick ^ ((pos.x as u64) << 40) ^ ((pos.z as u64) << 16) ^ pos.y as u64,
            );
            let pitch = 0.5 + rng.unit() as f32 * 1.2;
            crate::command_sound::play_block(
                state,
                pos,
                mc_data::builtin_sounds_26_3::AMETHYST_BLOCK_CHIME,
                1.0,
                pitch,
            );
        }
        "tnt" => {
            if burning(state, slot, record)
                && may_interact(state, record, pos)
                && adventure_may_prime(state, record, pos)
                && zone_allows(state, record, pos)
            {
                crate::tnt::prime_block(state, pos, 80);
            }
        }
        "campfire" | "soul_campfire" => {
            if burning(state, slot, record)
                && may_interact(state, record, pos)
                && zone_allows(state, record, pos)
            {
                crate::campfire::relight(state, pos);
            }
        }
        _ if path == "candle" || path.ends_with("_candle") || path.ends_with("candle_cake") => {
            light_candle(state, slot, record, pos, id);
        }
        _ => {}
    }
}

/// `Projectile.isOnFire` as Solaris tracks it: a flame or burning arrow
/// (tridents carry no fire here), and a small or large fireball, which
/// `AbstractHurtingProjectile.tick` ignites before it resolves a hit
/// (`shouldBurn`; neither type is fire immune, while dragon fireballs, wither
/// skulls and wind charges opt out).
fn burning(state: &TickState, slot: EntitySlot, record: &ProjectileState) -> bool {
    match record.kind {
        ProjectileKind::SmallFireball | ProjectileKind::Fireball => true,
        ProjectileKind::Arrow | ProjectileKind::SpectralArrow => {
            record.flame || state.entities.body(slot).burn_ticks != 0
        }
        _ => false,
    }
}

/// `TntBlock.prime`: an adventure-mode player igniter needs the arrow's pickup
/// stack `can_break` to admit the TNT.
fn adventure_may_prime(state: &TickState, record: &ProjectileState, pos: BlockPos) -> bool {
    let adventure = super::owner_id(state, record).is_some_and(|owner| {
        state
            .players
            .get(&owner)
            .is_some_and(|player| player.game_mode == crate::commands::GameMode::Adventure)
    });
    !adventure || super::can_break::can_break(state, &record.stack, pos)
}

/// `AbstractCandleBlock.onProjectileHit`: a burning projectile lights an unlit
/// candle (`CandleBlock.canBeLit` also needs it dry) or candle cake with a
/// plain state write (`setLit`, flags 11; no game event).
fn light_candle(
    state: &mut TickState,
    slot: EntitySlot,
    record: &ProjectileState,
    pos: BlockPos,
    id: u32,
) {
    let blocks = &state.registry.blocks;
    let unlit = crate::farming::prop(blocks, id, "lit") == Some("false");
    let dry = crate::farming::prop(blocks, id, "waterlogged") != Some("true");
    if !unlit || !dry || !burning(state, slot, record) || !zone_allows(state, record, pos) {
        return;
    }
    if let Some(lit) = crate::farming::with_prop(blocks, id, "lit", "true") {
        state.write_block(pos, lit);
    }
}

/// `BellBlock.onProjectileHit` -> `onHit(.., player owner, true)`: only a
/// proper hit (`isProperHit`) rings (`attemptToRing`): the block event and
/// bell sound, `BLOCK_CHANGE` with the player owner, and that player's
/// `bell_ring` statistic.
fn ring_bell(
    state: &mut TickState,
    record: &ProjectileState,
    pos: BlockPos,
    id: u32,
    impact: [f64; 3],
    face: Option<[i32; 3]>,
) {
    let Some(face) = face else {
        return;
    };
    let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
        return;
    };
    let property = |name: &str| {
        block
            .properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    // `isProperHit`: a horizontal face, struck no higher than 0.8124f above the
    // cell floor, on a side the attachment exposes.
    if face[1] != 0 || impact[1] - f64::from(pos.y) > f64::from(0.8124_f32) {
        return;
    }
    let facing_x = matches!(property("facing"), Some("east" | "west"));
    let face_x = face[0] != 0;
    let proper = match property("attachment") {
        Some("floor") => facing_x == face_x,
        Some("single_wall" | "double_wall") => facing_x != face_x,
        Some("ceiling") => true,
        _ => false,
    };
    let Ok(raw) = u32::try_from(block.block.raw_id) else {
        return;
    };
    if !proper {
        return;
    }
    // `Direction.get3DDataValue`: down, up, north, south, west, east.
    let direction = match face {
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    };
    crate::redstone::ring_bell(state, pos, raw, id, direction);
    let player = super::owner_id(state, record).filter(|owner| state.players.contains_key(owner));
    crate::vibration::emit(
        state,
        crate::vibration::centre(pos),
        crate::vibration::GameEvent::BlockChange,
        player,
    );
    if let Some(player) = player.and_then(|owner| state.players.get_mut(&owner)) {
        player
            .statistics
            .increment_custom(mc_data::statistics::CustomStat::BellRing, 1);
    }
}
