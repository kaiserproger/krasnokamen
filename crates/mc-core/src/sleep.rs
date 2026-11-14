//! Beds, vanilla `BedBlock.useWithoutItem` + `Player.startSleepInBed`: a
//! player lies down in a free bed at night (or in a thunderstorm) with no
//! monster near. A valid bed also sets respawn during daytime. Once the
//! configured quorum has slept long enough, the night is skipped and rain
//! clears. Waking and respawning share the bed's safe-exit search.

use std::sync::Arc;

use crate::blueprint::MobCategory;
use crate::boundary::{BlockPos, ConnectionId, EntityDataValue, EntityId, PlayerChange};
use crate::cat::{Relax, TYPE};
use crate::commands::Weather;
use crate::entities::EntitySlot;
use crate::tick::TickState;

/// Vanilla `Pose.SLEEPING` ordinal.
const SLEEPING_POSE: u8 = 2;
/// `LivingEntity.SLEEPING_POS_ID` entity data index; the edge's
/// `mc_protocol` `LIVING_ENTITY_DATA_SLEEPING_POS_INDEX` is the wire evidence.
pub const SLEEPING_POS_DATA_INDEX: u8 = 14;
/// Ticks a player must have slept before the night may be skipped.
pub(crate) const SLEEP_TICKS: u32 = 100;
/// Day times (ticks into the day) at which beds work (vanilla `BedRule`).
const NIGHT: std::ops::Range<i64> = 12_542..23_460;
/// Monsters this close to a bed keep its sleeper awake.
const MONSTER_RANGE: [f64; 2] = [8.0, 5.0];
/// Vanilla bed height above its block's floor, and the sleeper's offset in it.
const BED_TOP: f64 = 0.5625;
const SLEEPER_HEIGHT: f64 = 0.6875;

/// A player lying in a bed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sleeper {
    /// Head cell of the bed.
    pub bed: BlockPos,
    /// Ticks slept so far, capped at [`SLEEP_TICKS`].
    pub ticks: u32,
    /// Health when lying down: a hit wakes the sleeper.
    health: f32,
}

/// Whether the clock or the weather allows sleeping.
pub(crate) fn can_sleep_now(state: &TickState) -> bool {
    NIGHT.contains(&state.day_time.rem_euclid(24_000)) || state.weather.0 == Weather::Thunder
}

fn say(state: &mut TickState, connection: ConnectionId, message: &str) {
    state.push_player(
        connection,
        PlayerChange::Chat {
            message: message.to_owned(),
        },
    );
}

/// Whether `pos` holds a bed block.
fn is_bed(state: &TickState, pos: BlockPos) -> bool {
    state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|current| state.registry.blocks.by_id(mc_world::BlockStateId(current)))
        .is_some_and(|block| block.block.id.path().ends_with("_bed"))
}

/// A monster whose box meets the box around `bed` that keeps sleepers up.
/// Vanilla exempts slimes/magma cubes entirely and zombified piglins unless
/// angered by a player; endermen still block sleep.
fn monster_near(state: &TickState, bed: BlockPos) -> bool {
    let centre = [f64::from(bed.x) + 0.5, f64::from(bed.z) + 0.5];
    let floor = f64::from(bed.y);
    state.entities.slots().any(|slot| {
        let row = state.entities.body(slot);
        if !row.alive || crate::natural_spawn::category_of(state, slot) != MobCategory::Monster {
            return false;
        }
        if mob_exempt_from_bed_check(state, slot) {
            return false;
        }
        let at = state.entities.pos(slot);
        (at[0] - centre[0]).abs() < MONSTER_RANGE[0] + row.half_width
            && (at[2] - centre[1]).abs() < MONSTER_RANGE[0] + row.half_width
            && at[1] + row.height > floor - MONSTER_RANGE[1]
            && at[1] < floor + MONSTER_RANGE[1]
    })
}

/// Vanilla BedBlock monster exceptions: slimes and magma cubes never block
/// sleep; zombified piglins block only while hostile to a player.
fn mob_exempt_from_bed_check(state: &TickState, slot: crate::entities::EntitySlot) -> bool {
    let crate::entities::EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
        return false;
    };
    let Some(blueprint) = state.registry.blueprint(type_id) else {
        return false;
    };
    match blueprint.name.as_str() {
        "minecraft:slime" | "minecraft:magma_cube" => true,
        "minecraft:zombified_piglin" => !mob_hostile_to_players(state, slot),
        _ => false,
    }
}

/// Whether the mob is currently hostile to a player (its combat credit or
/// recent player damage). Neutral mobs at peace do not block sleep.
fn mob_hostile_to_players(state: &TickState, slot: crate::entities::EntitySlot) -> bool {
    let row = state.entities.row(slot);
    // Vanilla player-kill credit persists 100 ticks after a player hit: a
    // piglin hurt by a player in that window counts as angered.
    // (last_hurt is a damage magnitude, not recency — not used here.)
    row.last_player_hit
        .is_some_and(|(_, at)| state.tick.saturating_sub(at) <= 100)
}

/// Publish the sleeping position (and pose) of `entity`.
fn set_lying(state: &mut TickState, entity: EntityId, bed: Option<BlockPos>) {
    let Some(slot) = state.entities.slot_of_any(entity) else {
        return;
    };
    state
        .entities
        .set_half_width(slot, if bed.is_some() { 0.1 } else { 0.3 });
    state.entities.body_mut(slot).height = if bed.is_some() { 0.2 } else { 1.8 };
    let row = state.entities.row_mut(slot);
    let appearance = Arc::make_mut(&mut row.appearance);
    appearance.pose = if bed.is_some() { SLEEPING_POSE } else { 0 };
    if bed.is_some() {
        appearance.shared_flags &= !crate::tick::SNEAKING_FLAG;
    }
    let value = EntityDataValue::OptionalBlockPos(bed.map(|bed| [bed.x, bed.y, bed.z]));
    match appearance
        .extra
        .iter_mut()
        .find(|(index, _)| *index == SLEEPING_POS_DATA_INDEX)
    {
        Some(entry) => entry.1 = value,
        None => appearance.extra.push((SLEEPING_POS_DATA_INDEX, value)),
    }
    state.appearance_changed.push(entity);
}

/// A hand use of the block at `clicked`. `false` when it is not a bed;
/// otherwise the use is consumed: the player lies down or is told why not.
pub(crate) fn use_bed(
    state: &mut TickState,
    entity: EntityId,
    connection: ConnectionId,
    clicked: BlockPos,
) -> bool {
    let Some(current) = state.world.block_at(clicked.x, clicked.y, clicked.z) else {
        return false;
    };
    if !is_bed(state, clicked) {
        return false;
    }
    // Either half sleeps in the head cell.
    let Some((other, head)) =
        crate::placement::other_half(&state.registry.blocks, clicked, current)
    else {
        return false;
    };
    let Some(player) = state.players.get(&entity) else {
        return true;
    };
    if player.spectator() || state.sleepers.contains_key(&entity) {
        return true;
    }
    let creative = player.creative();
    let Some(slot) = state.entities.slot_of(entity) else {
        return true;
    };
    // Vanilla BedBlock.useWithoutItem: an occupied bed only wakes the villager
    // asleep in it; the player is not put to bed on that click.
    if is_bed(state, head) && crate::villager::brain::kick_out_of_bed(state, head) {
        return true;
    }
    if !is_bed(state, head) || state.sleepers.values().any(|other| other.bed == head) {
        say(state, connection, "This bed is occupied");
        return true;
    }
    let [yaw, _, _] = state.entities.rot(slot);
    // Vanilla ServerPlayer.isReachableBedBlock: within 3/2/3 blocks of either
    // half's bottom centre. Packet reach (4.5+1) can target farther beds.
    if !creative {
        let at = state.entities.pos(slot);
        let near_half = |half: BlockPos| {
            (f64::from(half.x) + 0.5 - at[0]).abs() <= 3.0
                && (f64::from(half.y) - at[1]).abs() <= 2.0
                && (f64::from(half.z) + 0.5 - at[2]).abs() <= 3.0
        };
        if !near_half(clicked) && !near_half(other) {
            say(state, connection, "You are too far away");
            return true;
        }
    }
    // Vanilla tests the block above each bed half for suffocation, not
    // whether a wake-up position exists around the bed.
    if [clicked, other].into_iter().any(|half| {
        state
            .world
            .block_at(half.x, half.y + 1, half.z)
            .and_then(|id| state.registry.light.suffocating(id))
            != Some(false)
    }) {
        say(state, connection, "This bed is obstructed");
        return true;
    }
    if let Some(player) = state.players.get_mut(&entity) {
        player.respawn_point = Some([head.x, head.y, head.z]);
        player.respawn_dimension = state.dimension;
        player.respawn_forced = false;
        player.respawn_yaw = yaw;
        player.respawn_pitch = 0.0;
    }
    if !can_sleep_now(state) {
        say(state, connection, "You can sleep only at night");
        return true;
    }
    if !creative && monster_near(state, head) {
        say(
            state,
            connection,
            "You may not rest now; there are monsters nearby",
        );
        return true;
    }
    let lying = [
        f64::from(head.x) + 0.5,
        f64::from(head.y) + SLEEPER_HEIGHT,
        f64::from(head.z) + 0.5,
    ];
    let [yaw, pitch, _] = state.entities.rot(slot);
    let health = state.entities.row(slot).health;
    state.entities.set_pos(slot, lying);
    state.entities.set_vel(slot, [0.0; 3]);
    state.sleepers.insert(
        entity,
        Sleeper {
            bed: head,
            ticks: 0,
            health,
        },
    );
    if let Some(player) = state.players.get_mut(&entity) {
        // The spawn was set even if sleeping would have been refused above.
        player
            .statistics
            .increment_custom(mc_data::statistics::CustomStat::SleepInBed, 1);
        player
            .statistics
            .set_custom(mc_data::statistics::CustomStat::TimeSinceRest, 0);
    }
    set_lying(state, entity, Some(head));
    crate::advancements::trigger(
        state,
        entity,
        "minecraft:slept_in_bed",
        serde_json::json!({}),
    );
    state.correct_position(connection, lying, yaw, pitch);
    true
}

/// Get `entity` out of its bed (leave-bed command, morning, hit, death).
pub(crate) fn wake(state: &mut TickState, entity: EntityId) {
    if let Some(sleeper) = state.sleepers.remove(&entity) {
        wake_cats(state, entity, sleeper.ticks);
        if let Some(slot) = state.entities.slot_of(entity) {
            let [yaw, pitch, _] = state.entities.rot(slot);
            if let Some(exit) = bed_exit(state, sleeper.bed, yaw) {
                state.entities.set_pos(slot, exit);
                state.entities.set_vel(slot, [0.0; 3]);
                if let Some(connection) = state.connections.entity_connection(entity) {
                    state.correct_position(connection, exit, yaw, pitch);
                }
            }
        }
        set_lying(state, entity, None);
    }
}

/// Once per tick: count sleep time, wake those whose bed is gone, whose
/// night is over or who were hurt, and skip the night when the gamerule's
/// percentage of active non-spectator players sleeps soundly.
pub(crate) fn run(state: &mut TickState) {
    run_cats(state);
    if state.sleepers.is_empty() {
        return;
    }
    let can_sleep = can_sleep_now(state);
    let mut sleepers: Vec<EntityId> = state.sleepers.keys().copied().collect();
    sleepers.sort_unstable();
    for entity in sleepers {
        let Some(sleeper) = state.sleepers.get_mut(&entity) else {
            continue;
        };
        sleeper.ticks = (sleeper.ticks + 1).min(SLEEP_TICKS);
        let (bed, health) = (sleeper.bed, sleeper.health);
        let hurt = state
            .entities
            .slot_of(entity)
            .is_none_or(|slot| state.entities.row(slot).health < health);
        if hurt || !can_sleep || !is_bed(state, bed) {
            wake(state, entity);
        }
    }
    let mut active = 0_u64;
    let mut deep_sleepers = 0_u64;
    for (id, player) in &state.players {
        if player.spectator() || state.entities.slot_of(*id).is_none() {
            continue;
        }
        active += 1;
        if state
            .sleepers
            .get(id)
            .is_some_and(|sleeper| sleeper.ticks >= SLEEP_TICKS)
        {
            deep_sleepers += 1;
        }
    }
    let percentage = crate::command_rules::integer(
        state,
        crate::command_rules::IntRule::PLAYERS_SLEEPING_PERCENTAGE,
    ) as u64;
    let required = (u128::from(active) * u128::from(percentage))
        .div_ceil(100)
        .max(1);
    if u128::from(deep_sleepers) >= required {
        if crate::command_rules::boolean(state, crate::command_rules::BoolRule::ADVANCE_TIME) {
            state.day_time = (state.day_time.div_euclid(24_000) + 1) * 24_000;
        }
        if crate::command_rules::boolean(state, crate::command_rules::BoolRule::ADVANCE_WEATHER)
            && state.weather.0 != Weather::Clear
        {
            state.weather = (Weather::Clear, 0);
            state.thundering = false;
            state.thunder_remaining_ticks = 0;
            state.clear_remaining_ticks = 0;
        }
        let sleepers: Vec<EntityId> = state.sleepers.keys().copied().collect();
        for entity in sleepers {
            wake(state, entity);
        }
    }
}

/// Forced command points allow a safe non-bed cell; ordinary bed points
/// remain valid only while the bed and an exit stand.
pub(crate) fn respawn_position(state: &TickState, entity: EntityId) -> Option<[f64; 3]> {
    let player = state.players.get(&entity)?;
    respawn_position_for(state, player)
}

pub(crate) fn respawn_position_for(
    state: &TickState,
    player: &crate::player::PlayerState,
) -> Option<[f64; 3]> {
    let [x, y, z] = player.respawn_point?;
    if player.respawn_dimension != state.dimension {
        return None;
    }
    if let Some(position) = crate::dimensions::rules::anchor_spawn(state, BlockPos { x, y, z }) {
        return Some(position);
    }
    if !state.dimension.beds_work() && is_bed(state, BlockPos { x, y, z }) {
        return None;
    }
    if !is_bed(state, BlockPos { x, y, z }) {
        let safe = |y| {
            state.world.block_at(x, y, z).is_some_and(|block| {
                state.registry.materials.classify(block) != mc_physics::BlockMaterial::Solid
                    && state
                        .registry
                        .blocks
                        .by_id(mc_world::BlockStateId(block))
                        .is_some_and(|state| !state.has_fluid())
            })
        };
        return (player.respawn_forced && safe(y) && safe(y + 1)).then_some([
            f64::from(x) + 0.5,
            f64::from(y) + 0.1,
            f64::from(z) + 0.5,
        ]);
    }
    bed_exit(state, BlockPos { x, y, z }, player.respawn_yaw)
}

/// Fit a standing player above the collision surface in a candidate column.
pub(crate) fn stand_up_position(state: &TickState, cell: BlockPos) -> Option<[f64; 3]> {
    let shapes = mc_data::collision_shapes::vanilla_collision_shapes();
    let surface = |pos: BlockPos| {
        let id = state.world.block_at(pos.x, pos.y, pos.z)?;
        let block = state.registry.blocks.by_id(mc_world::BlockStateId(id))?;
        if block.has_fluid() || state.registry.hazard_flags.get(id as usize) != Some(&0) {
            return None;
        }
        let shape = shapes.get_for_state(id, &block.block.id, &block.properties)?;
        Some(
            shape
                .iter()
                .map(|bounds| bounds.as_blocks()[4])
                .fold(0.0_f64, f64::max),
        )
    };
    let height = surface(cell)?;
    let height = if height == 0.0 {
        surface(BlockPos {
            y: cell.y - 1,
            ..cell
        })? - 1.0
    } else {
        height
    };
    if height <= -1.0 {
        return None;
    }
    let position = [
        f64::from(cell.x) + 0.5,
        f64::from(cell.y) + height,
        f64::from(cell.z) + 0.5,
    ];
    stand_up_clear(state, position).then_some(position)
}

fn stand_up_clear(state: &TickState, [x, y, z]: [f64; 3]) -> bool {
    let shapes = mc_data::collision_shapes::vanilla_collision_shapes();
    for by in y.floor() as i32..=(y + 1.8 - 1.0e-6).floor() as i32 {
        let bx = x.floor() as i32;
        let bz = z.floor() as i32;
        let Some(id) = state.world.block_at(bx, by, bz) else {
            return false;
        };
        let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
            return false;
        };
        if block.has_fluid() || state.registry.hazard_flags.get(id as usize) != Some(&0) {
            return false;
        }
        let Some(shape) = shapes.get_for_state(id, &block.block.id, &block.properties) else {
            return false;
        };
        for bounds in shape.iter() {
            let [min_x, min_y, min_z, max_x, max_y, max_z] = bounds.as_blocks();
            if x + 0.3 > f64::from(bx) + min_x
                && x - 0.3 < f64::from(bx) + max_x
                && y + 1.8 > f64::from(by) + min_y
                && y < f64::from(by) + max_y
                && z + 0.3 > f64::from(bz) + min_z
                && z - 0.3 < f64::from(bz) + max_z
            {
                return false;
            }
        }
    }
    true
}

/// Both bed halves, all perimeter cells and the lower floor tier. The search is
/// bounded to 36 cells and ordered toward the player's saved facing.
fn bed_exit(state: &TickState, head: BlockPos, yaw: f32) -> Option<[f64; 3]> {
    let current = state.world.block_at(head.x, head.y, head.z)?;
    let foot = crate::placement::other_half(&state.registry.blocks, head, current)
        .map_or(head, |(other, _)| other);
    let offsets = [
        [1, 0],
        [1, 1],
        [0, 1],
        [-1, 1],
        [-1, 0],
        [-1, -1],
        [0, -1],
        [1, -1],
    ];
    let first = (((f64::from(yaw).to_radians() + std::f64::consts::FRAC_PI_2)
        / std::f64::consts::FRAC_PI_4)
        .round() as i32)
        .rem_euclid(8) as usize;
    // Bed and anchor respawns share the same player-shaped collision check.
    for dy in [0, -1] {
        for anchor in [head, foot] {
            for index in 0..8 {
                let [dx, dz] = offsets[(first + index) % 8];
                let (x, y, z) = (anchor.x + dx, head.y + dy, anchor.z + dz);
                if let Some(position) = stand_up_position(state, BlockPos { x, y, z }) {
                    return Some(position);
                }
            }
        }
    }
    [head, foot].into_iter().find_map(|pos| {
        let position = [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + BED_TOP,
            f64::from(pos.z) + 0.5,
        ];
        stand_up_clear(state, position).then_some(position)
    })
}

pub(crate) fn respawn_rotation(state: &TickState, entity: EntityId) -> [f32; 2] {
    if respawn_position(state, entity).is_some() {
        let player = &state.players[&entity];
        [player.respawn_yaw, player.respawn_pitch]
    } else {
        state.world.world_spawn_rotation()
    }
}

/// Respawn waits for its off-tick loads rather than treating an unloaded
/// personal point as an invalid bed and silently falling back.
pub(crate) fn respawn_chunks_loaded(state: &TickState, entity: EntityId) -> bool {
    let loaded = |spawn: [i32; 3]| {
        let x = spawn[0].div_euclid(16);
        let z = spawn[2].div_euclid(16);
        (-1..=1).all(|dx| (-1..=1).all(|dz| state.world.has_chunk(x + dx, z + dz)))
    };
    loaded(state.world.world_spawn())
        && state
            .players
            .get(&entity)
            .and_then(|player| player.respawn_point)
            .is_none_or(loaded)
}

fn metadata(state: &mut TickState, slot: EntitySlot, lying: bool, relaxing: bool) {
    let extra = &state.entities.row(slot).appearance.extra;
    if [(21, lying), (22, relaxing)].iter().all(|(index, value)| {
        extra
            .iter()
            .find(|(i, _)| i == index)
            .map(|(_, v)| v)
            .unwrap_or(&EntityDataValue::Boolean(false))
            == &EntityDataValue::Boolean(*value)
    }) {
        return;
    }
    let row = state.entities.row_mut(slot);
    let appearance = Arc::make_mut(&mut row.appearance);
    let mut changed = false;
    for (index, value) in [(21, lying), (22, relaxing)] {
        let value = EntityDataValue::Boolean(value);
        if let Some((_, old)) = appearance.extra.iter_mut().find(|(i, _)| *i == index) {
            if *old != value {
                *old = value;
                changed = true;
            }
        } else if value == EntityDataValue::Boolean(true) {
            appearance.extra.push((index, value));
            changed = true;
        }
    }
    if changed {
        state.appearance_changed.push(state.entities.body(slot).id);
    }
}

fn run_cats(state: &mut TickState) {
    let cats: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in cats {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let owner = row.entity_life.taming.owner;
        let eligible =
            body.alive && !row.controls.no_ai && row.entity_life.tamed && !row.entity_life.sitting;
        let Some(owner_slot) = owner.and_then(|owner| state.entities.slot_of_uuid(owner)) else {
            if row.entity_life.cat_relax.is_some() {
                state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
                metadata(state, slot, false, false);
            }
            continue;
        };
        let owner_id = state.entities.body(owner_slot).id;
        let Some(sleeper) = state.sleepers.get(&owner_id).copied().filter(|_| eligible) else {
            if state.entities.row(slot).entity_life.cat_relax.is_some() {
                state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
                metadata(state, slot, false, false);
            }
            continue;
        };
        let pos = state.entities.pos(slot);
        let owner_pos = state.entities.pos(owner_slot);
        let distance = (0..3).map(|i| (pos[i] - owner_pos[i]).powi(2)).sum::<f64>();
        let existing = state.entities.row(slot).entity_life.cat_relax.clone();
        if existing.is_none() && distance > 100.0 {
            continue;
        }
        let bed = sleeper.bed;
        let facing = state
            .world
            .block_at(bed.x, bed.y, bed.z)
            .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
            .and_then(|block| block.properties.iter().find(|(key, _)| key == "facing"))
            .map(|(_, value)| value.as_str())
            .unwrap_or("north");
        let (dx, dz) = match facing {
            "north" => (0, 1),
            "south" => (0, -1),
            "east" => (-1, 0),
            "west" => (1, 0),
            _ => (0, 0),
        };
        let target = [
            f64::from(bed.x + dx) + 0.5,
            f64::from(bed.y),
            f64::from(bed.z + dz) + 0.5,
        ];
        let occupied = state.entities.mob_slots(*TYPE).any(|other| {
            if other == slot {
                return false;
            }
            let row = state.entities.row(other);
            let Some(relax) = &row.entity_life.cat_relax else {
                return false;
            };
            relax.on_bed_ticks > 0
                && (0..3).all(|i| (state.entities.pos(other)[i] - target[i]).abs() <= 2.0)
        });
        if occupied {
            state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
            metadata(state, slot, false, false);
            continue;
        }
        let mut relax = existing.unwrap_or(Relax {
            owner: owner.expect("owner"),
            target,
            on_bed_ticks: 0,
        });
        relax.target = target;
        if distance < 2.5 {
            relax.on_bed_ticks = relax.on_bed_ticks.saturating_add(1);
        } else {
            relax.on_bed_ticks = 0;
        }
        metadata(
            state,
            slot,
            relax.on_bed_ticks > 16,
            (1..=16).contains(&relax.on_bed_ticks),
        );
        state.entities.row_mut_unmarked(slot).entity_life.cat_relax = Some(relax);
    }
}

fn gift_teleport(state: &mut TickState, slot: EntitySlot) {
    let original = state.entities.pos(slot);
    let base = match state.entities.row(slot).entity_life.leash.as_ref() {
        Some(crate::leash::Holder::Entity(uuid)) => state
            .entities
            .slot_of_uuid(*uuid)
            .map(|holder| state.entities.pos(holder))
            .unwrap_or(original),
        Some(crate::leash::Holder::Fence(pos)) => pos.map(f64::from),
        None => original,
    };
    let row = state.entities.row_mut(slot);
    let x = base[0].floor() as i32 + crate::taming::roll(row, 11) as i32 - 5;
    let y = base[1].floor() as i32 + crate::taming::roll(row, 5) as i32 - 2;
    let z = base[2].floor() as i32 + crate::taming::roll(row, 11) as i32 - 5;
    let body = state.entities.body(slot);
    let (width, height, id) = (body.half_width, body.height, body.id);
    for floor in (state.world.geometry().min_y() + 1..=y).rev() {
        let Some(ground) = state.world.block_at(x, floor - 1, z) else {
            return;
        };
        if state.registry.materials.classify(ground) != mc_physics::BlockMaterial::Solid {
            continue;
        }
        let at = [f64::from(x) + 0.5, f64::from(floor), f64::from(z) + 0.5];
        let clear = (mc_physics::mth::floor_i32(at[0] - width)
            ..=mc_physics::mth::floor_i32(at[0] + width))
            .all(|bx| {
                (mc_physics::mth::floor_i32(at[2] - width)
                    ..=mc_physics::mth::floor_i32(at[2] + width))
                    .all(|bz| {
                        (floor..=mc_physics::mth::floor_i32(at[1] + height - 1.0e-6)).all(|by| {
                            state.world.block_at(bx, by, bz).is_some_and(|block| {
                                state.registry.collision_boxes[block as usize].is_empty()
                                    && state.registry.materials.classify(block)
                                        != mc_physics::BlockMaterial::Water
                            })
                        })
                    })
            });
        if clear {
            state.entities.set_pos(slot, at);
            state.entities.set_vel(slot, [0.0; 3]);
            state.entities.set_ground(slot, true);
            state.appearance_changed.push(id);
        }
        break;
    }
}

/// Resolve each active relaxation when its owner wakes.
fn wake_cats(state: &mut TickState, owner: EntityId, slept: u32) {
    let Some(owner_slot) = state.entities.slot_of_any(owner) else {
        return;
    };
    let uuid = state.entities.row(owner_slot).uuid;
    let cats: Vec<_> = state
        .entities
        .mob_slots(*TYPE)
        .filter(|slot| {
            state
                .entities
                .row(*slot)
                .entity_life
                .cat_relax
                .as_ref()
                .is_some_and(|relax| relax.owner == uuid)
        })
        .collect();
    for slot in cats {
        state.entities.row_mut_unmarked(slot).entity_life.cat_relax = None;
        metadata(state, slot, false, false);
        let time = state.day_time.rem_euclid(24_000);
        if slept < 100
            || (362..23_667).contains(&time)
            || crate::taming::roll(state.entities.row_mut(slot), 10) >= 7
        {
            continue;
        }
        let id =
            mc_data::Identifier::parse("minecraft:gameplay/cat_morning_gift").expect("gift id");
        gift_teleport(state, slot);
        let seed = state.tick ^ state.entities.row(slot).uuid.as_u128() as u64;
        let Some(table) = state.registry.chest_loot.get(&id) else {
            continue;
        };
        let Ok(stacks) = table.roll(
            &mut mc_data::loot::chest_26_1_2::ChestRng::new(seed),
            &state.registry.items,
            0.0,
        ) else {
            continue;
        };
        let pos = state.entities.pos(slot);
        state
            .merge_life_drops
            .extend(stacks.into_iter().map(|stack| (pos, stack)));
    }
}

#[cfg(test)]
#[path = "sleep_tests.rs"]
mod tests;
