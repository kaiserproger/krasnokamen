//! Big dripleaf tilt (26.3 `BigDripleafBlock` javap). A projectile hit tilts
//! the leaf fully (`onProjectileHit`); its scheduled ticks walk `unstable` ->
//! `partial` -> `full` and reset `full` to `none`
//! (`DELAY_UNTIL_NEXT_TILT_STATE`: 10, 10, 100); a neighbour signal resets
//! the leaf (`tick`, `neighborChanged`).

use mc_data::builtin_sounds_26_3::{BIG_DRIPLEAF_TILT_DOWN, BIG_DRIPLEAF_TILT_UP};
use mc_world::ScheduledBlockTick;

use crate::boundary::BlockPos;
use crate::tick::TickState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tilt {
    None,
    Unstable,
    Partial,
    Full,
}

impl Tilt {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "none" => Self::None,
            "unstable" => Self::Unstable,
            "partial" => Self::Partial,
            "full" => Self::Full,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Unstable => "unstable",
            Self::Partial => "partial",
            Self::Full => "full",
        }
    }

    /// `Tilt.causesVibration`: every tilt but `unstable`.
    fn causes_vibration(self) -> bool {
        self != Self::Unstable
    }

    /// `DELAY_UNTIL_NEXT_TILT_STATE`; `none` schedules nothing.
    fn delay(self) -> Option<u64> {
        match self {
            Self::None => None,
            Self::Unstable | Self::Partial => Some(10),
            Self::Full => Some(100),
        }
    }
}

/// The leaf's state id and tilt; `None` for any other block.
fn leaf(state: &TickState, pos: BlockPos) -> Option<(u32, Tilt)> {
    let id = state.world.block_at(pos.x, pos.y, pos.z)?;
    let block = state.registry.blocks.by_id(mc_world::BlockStateId(id))?;
    if block.block.id.as_str() != mc_data::blocks::BIG_DRIPLEAF {
        return None;
    }
    let tilt = block
        .properties
        .iter()
        .find(|(key, _)| key == "tilt")
        .and_then(|(_, value)| Tilt::parse(value))?;
    Some((id, tilt))
}

/// `setTilt`: writes the tilt (a scheduled tick publishes through the redstone
/// pass's `writes`) and emits `BLOCK_CHANGE` when the tilt changes to one that
/// vibrates. Returns the previous tilt.
fn set_tilt(
    state: &mut TickState,
    pos: BlockPos,
    tilt: Tilt,
    writes: Option<&mut Vec<(BlockPos, u32)>>,
) -> Option<Tilt> {
    let (current, old) = leaf(state, pos)?;
    let next = crate::farming::with_prop(&state.registry.blocks, current, "tilt", tilt.name())?;
    match writes {
        Some(writes) => crate::redstone::set(state, writes, pos, next),
        None if next != current => state.write_block(pos, next),
        None => {}
    }
    if tilt.causes_vibration() && tilt != old {
        crate::vibration::emit(
            state,
            crate::vibration::centre(pos),
            crate::vibration::GameEvent::BlockChange,
            None,
        );
    }
    Some(old)
}

/// `playTiltSound`: volume 1, pitch `Mth.randomBetween(0.8, 1.2)`.
fn tilt_sound(state: &mut TickState, pos: BlockPos, sound: &str) {
    let mut rng = crate::natural_spawn::Rng(
        state.tick ^ ((pos.x as u64) << 40) ^ ((pos.z as u64) << 16) ^ pos.y as u64,
    );
    let pitch = 0.8 + rng.unit() as f32 * 0.4;
    crate::command_sound::play_block(state, pos, sound, 1.0, pitch);
}

/// `setTiltAndScheduleTick`. Vanilla keeps one scheduled tick per cell and
/// block, so a pending tick is not rescheduled.
fn tilt_and_schedule(
    state: &mut TickState,
    pos: BlockPos,
    tilt: Tilt,
    writes: Option<&mut Vec<(BlockPos, u32)>>,
) {
    if set_tilt(state, pos, tilt, writes).is_none() {
        return;
    }
    tilt_sound(state, pos, BIG_DRIPLEAF_TILT_DOWN);
    let Some(delay) = tilt.delay() else {
        return;
    };
    let Some(block) = state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
    else {
        return;
    };
    let id = block.block.id.clone();
    if !state.block_ticks.contains(pos, &id, 0) {
        state.block_ticks.schedule(ScheduledBlockTick::new(
            pos,
            id,
            state.tick.saturating_add(delay),
            0,
        ));
    }
}

/// `resetTilt`: back to `none`, with the tilt-up sound unless it was level.
fn reset(state: &mut TickState, pos: BlockPos, writes: Option<&mut Vec<(BlockPos, u32)>>) {
    if set_tilt(state, pos, Tilt::None, writes).is_some_and(|old| old != Tilt::None) {
        tilt_sound(state, pos, BIG_DRIPLEAF_TILT_UP);
    }
}

/// `BigDripleafBlock.onProjectileHit`: every projectile tilts the leaf fully.
pub(crate) fn projectile_hit(state: &mut TickState, pos: BlockPos) {
    if leaf(state, pos).is_some() {
        tilt_and_schedule(state, pos, Tilt::Full, None);
    }
}

/// `BigDripleafBlock.tick`.
pub(crate) fn scheduled(state: &mut TickState, pos: BlockPos, writes: &mut Vec<(BlockPos, u32)>) {
    let Some((_, tilt)) = leaf(state, pos) else {
        return;
    };
    if crate::redstone::power_at(state, pos) > 0 {
        reset(state, pos, Some(writes));
        return;
    }
    match tilt {
        Tilt::Unstable => tilt_and_schedule(state, pos, Tilt::Partial, Some(writes)),
        Tilt::Partial => tilt_and_schedule(state, pos, Tilt::Full, Some(writes)),
        Tilt::Full => reset(state, pos, Some(writes)),
        Tilt::None => {}
    }
}

/// `BigDripleafBlock.neighborChanged`: a neighbour signal resets the leaf.
pub(crate) fn changed(state: &mut TickState, pos: BlockPos) {
    if leaf(state, pos).is_some() && crate::redstone::power_at(state, pos) > 0 {
        reset(state, pos, None);
    }
}

#[cfg(test)]
#[path = "big_dripleaf_tests.rs"]
mod tests;
