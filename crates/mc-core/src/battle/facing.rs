//! Facing of standing battle soldiers (D36, design R2:
//! receipts/BattleTurnCapCause/D36-FACING-DESIGN.md; row
//! BATTLE-TARGET-LOOK-TURNS-WHOLE-BODY).
//!
//! Invariant: once per battle tick [`project`] publishes, for every projected
//! row (a battle row with a target and zero drive), the one value both the
//! rest guard and the step read: the rotation the step writes, admitted as a
//! rest only when bit-identical to the stored one. Rows that stop being
//! projected run their own rule in the step (a head left off the body returns
//! with `follow_head`); their drawn body is handed to that step first,
//! also when the battle pauses or stops ([`FacingState::release`]).
//!
//! The head tracks the target (a dead zone on a smoothed bearing triggers a
//! turn that lands on the current bearing at most [`RATE`] per tick and then
//! holds); the body is a replica of the client's `BodyRotationControl` (cone 75,
//! 15 degrees stable, 10 ticks, aligned over 10), which is what the client draws
//! for a standing mob from its head. Riders, rows whose body controller keeps
//! the head on the body and a dismounted rider whose charge bonus still lasts
//! keep the previous whole-body look (section 5 of the design): their step and
//! rest guard are the ones before D36 (`Facing::Look`).
//!
//! A standing row stores no yaw of its own (owner D48, the lazy policy of the
//! D36 comparison): the stored yaw changes only when the row really moves or
//! leaves the projection, so the wire keeps the yaw the row already had while
//! the head and the drawn body (`Facing::Set`'s `body`) turn.

use super::Frame;
use super::columns::Columns;
use super::combat::CombatRuntime;
use crate::boundary::EntityId;
use crate::entities::{EntityKind, EntitySlot};
use crate::entity_motion::{Facing, MOVING};
use mc_physics::{Vec3, mth};

/// Trigger: the smoothed bearing this far off the head starts a turn.
const TRIGGER: f32 = 20.0;
/// A turn ends within this angle of the current bearing.
const RELEASE: f32 = 5.0;
/// Head turn per tick during a turn.
const RATE: f32 = 15.0;
/// Weight of the current bearing in the trigger filter (time constant about 3 ticks).
const ALPHA: f32 = 0.3;
/// A target switch at least this far off the filter re-seeds the filter.
const SNAP: f32 = 45.0;
/// Pitch: its own trigger on the smoothed elevation, release on the current one.
const PITCH_TRIGGER: f32 = 8.0;
const PITCH_RELEASE: f32 = 2.0;
/// Vanilla `LookControl.setLookAt(x, y, z)`: `xMaxRotAngle` = `getMaxHeadXRot` 40.
const PITCH_RATE: f32 = 40.0;
/// Vanilla `Mob.getMaxHeadYRot`, the client's `max_head_yaw` for these mobs.
const CONE: f32 = 75.0;
/// Vanilla `BodyRotationControl`: `HEAD_STABLE_ANGLE`,
/// `DELAY_UNTIL_STARTING_TO_FACE_FORWARD`, `HOW_LONG_IT_TAKES_TO_FACE_FORWARD`.
const HEAD_STABLE: f32 = 15.0;
const ALIGN_DELAY: u8 = 10;
const ALIGN_TICKS: u8 = 10;
/// The previous rest guard's tolerance for the rows that keep the old look:
/// half a wire quantum (BATTLE-REST-SKIPS-TARGET-FACING).
const OLD_HOLD: f32 = 0.703_125;
/// Bearings and elevations need this much horizontal distance.
const MIN_HORIZONTAL: f64 = 1.0e-3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Law {
    /// Not projected last tick.
    #[default]
    Off,
    /// The previous whole-body look (riders, head-on-body mobs, charge bonus).
    Old,
    /// Head and body separate.
    Split,
    /// Left the split law with its drawn body handed to the step
    /// (`Facing::From`) while the stored rotation had these bits; republished
    /// until a step consumed it (a row whose chunk is not simulated keeps every
    /// stored bit).
    Leaving { at: [u32; 3] },
}

/// Per column row; carries its own id because tail rows shift when bodies die.
#[derive(Debug, Clone, Copy, Default)]
struct Row {
    id: Option<EntityId>,
    law: Law,
    target: Option<EntityId>,
    smooth: f32,
    turning: bool,
    pitch_smooth: f32,
    pitch_turning: bool,
    body: f32,
    stable: u8,
    last_stable: f32,
    position: [f64; 3],
}

#[derive(Debug, Default)]
pub(crate) struct FacingState {
    rows: Vec<Row>,
    by_slot: Vec<Option<Facing>>,
}

impl FacingState {
    /// The facing published for `slot` this battle tick.
    pub(crate) fn of(&self, slot: EntitySlot) -> Option<Facing> {
        self.by_slot.get(slot.0 as usize).copied().flatten()
    }

    /// Battle paused or stopped: nothing is projected. A row whose drawn body is
    /// not its stored yaw hands it to its step until a step consumed it; every
    /// other row is let go and a head left off its body returns in the step.
    /// The rows are kept, so a resumed battle finds them by column index.
    pub(crate) fn release(&mut self, entities: &crate::entities::EntityStore) {
        self.by_slot.clear();
        let rotations = entities.rotations();
        for row in &mut self.rows {
            if !matches!(row.law, Law::Split | Law::Leaving { .. }) {
                row.law = Law::Off;
                continue;
            }
            let Some(slot) = row.id.and_then(|id| entities.slot_of(id)) else {
                row.law = Law::Off;
                continue;
            };
            if let Some(facing) = leave(row, rotations[slot.0 as usize]) {
                if self.by_slot.len() < entities.len() {
                    self.by_slot.resize(entities.len(), None);
                }
                self.by_slot[slot.0 as usize] = Some(facing);
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn clear(&mut self) {
        self.rows.clear();
        self.by_slot.clear();
    }
}

/// Registration point: right after `Columns::project` in `battle::step`.
pub(crate) fn project(
    frame: &Frame<'_>,
    columns: &Columns,
    combat: &CombatRuntime,
    state: &mut FacingState,
) {
    // Indexed by slot so the guard and the step read it without a lookup; its
    // size follows the whole entity store, not the battle roster.
    state.by_slot.clear();
    state.by_slot.resize(frame.entities.len(), None);
    state.rows.resize(columns.ids.len(), Row::default());
    let rotations = frame.entities.rotations();
    for (bi, slot) in columns.slots.iter().enumerate() {
        let row = &mut state.rows[bi];
        if row.id != Some(columns.ids[bi]) {
            *row = Row {
                id: Some(columns.ids[bi]),
                ..Row::default()
            };
        }
        let Some(slot) = *slot else {
            row.law = Law::Off;
            continue;
        };
        let stored = rotations[slot.0 as usize];
        let position = columns.positions[bi];
        let facing = match columns.targets[bi] {
            Some(target) if columns.active[bi] && columns.drive[bi] == [0.0; 3] => {
                let law = if keeps_old_look(frame, combat, bi, slot) {
                    Law::Old
                } else {
                    Law::Split
                };
                Some(next(row, law, stored, position, target.id, target.pos))
            }
            _ => leave(row, stored),
        };
        row.position = position;
        state.by_slot[slot.0 as usize] = facing;
    }
}

/// Section 5: a rider, a mob whose own controller keeps the head on the body,
/// and a fighter whose charge bonus still lasts (its strike pushes along its
/// stored yaw) keep the previous whole-body look.
fn keeps_old_look(frame: &Frame<'_>, combat: &CombatRuntime, bi: usize, slot: EntitySlot) -> bool {
    frame.entities.row(slot).entity_life.mount.vehicle.is_some()
        || matches!(frame.entities.body(slot).kind, EntityKind::Mob { type_id }
            if frame.registry.blueprint(type_id).is_some_and(|blueprint| blueprint.capabilities.head_follows_body))
        || combat.ids.get(bi) == Some(&frame.entities.body(slot).id)
            && combat
                .fighters
                .get(bi)
                .is_some_and(|fighter| fighter.charge.bonus(frame.tick) > 0.0)
}

/// A row that stops being projected. Its stored yaw is the last one written,
/// not its drawn body: `From` hands the body to the step, republished while the
/// stored rotation keeps every bit it had when it was handed (the row's step
/// has not run: its chunk is not simulated, or its battle paused). Any change
/// of the stored rotation is a step that consumed it.
fn leave(row: &mut Row, stored: [f32; 3]) -> Option<Facing> {
    let handing = hands_the_body(row, stored);
    row.law = if handing {
        Law::Leaving { at: bits(stored) }
    } else {
        Law::Off
    };
    handing.then_some(Facing::From { yaw: row.body })
}

/// The row's drawn body is not its stored yaw and no step has taken it yet.
fn hands_the_body(row: &Row, stored: [f32; 3]) -> bool {
    match row.law {
        Law::Split => row.body.to_bits() != stored[0].to_bits(),
        Law::Leaving { at } => at == bits(stored),
        Law::Off | Law::Old => false,
    }
}

fn bits(rotation: [f32; 3]) -> [u32; 3] {
    rotation.map(f32::to_bits)
}

fn next(
    row: &mut Row,
    law: Law,
    stored: [f32; 3],
    position: [f64; 3],
    target: EntityId,
    aim: [f64; 3],
) -> Facing {
    if law == Law::Old {
        // A row coming off the split law first hands its drawn body to the
        // step; the old look then turns it from there.
        if hands_the_body(row, stored) {
            row.law = Law::Leaving { at: bits(stored) };
            return Facing::From { yaw: row.body };
        }
        row.law = Law::Old;
        return Facing::Look {
            rest: old_rest(stored, position, aim).then_some(stored),
        };
    }
    let (dx, dy, dz) = (
        aim[0] - position[0],
        aim[1] - position[1],
        aim[2] - position[2],
    );
    let horizontal = mth::length2(dx, dz);
    let bearing =
        (horizontal >= MIN_HORIZONTAL).then(|| -(mth::atan2(dx, dz) as f32) * mth::RAD_TO_DEG);
    let elevation = (horizontal >= MIN_HORIZONTAL)
        .then(|| -(mth::atan2(dy, horizontal) as f32) * mth::RAD_TO_DEG);
    let [_, mut pitch, mut head] = stored;
    let resumes = matches!(row.law, Law::Leaving { at } if at == bits(stored));
    if row.law != Law::Split && !resumes {
        // First sight, back from a driven step, or off the old look: start
        // from what is stored, without moving the head (section 3.3).
        *row = Row {
            id: row.id,
            law: Law::Split,
            target: Some(target),
            smooth: bearing.unwrap_or(head),
            turning: false,
            pitch_smooth: elevation.unwrap_or(pitch),
            pitch_turning: false,
            body: stored[0],
            stable: 0,
            last_stable: head,
            position,
        };
    } else {
        // Still split, or back before any step took the handed body (its
        // drawn body stands).
        row.law = Law::Split;
        if row.target != Some(target) {
            if let Some(bearing) = bearing
                && wrap(bearing - row.smooth).abs() >= SNAP
            {
                row.smooth = bearing;
            }
            row.target = Some(target);
        }
    }
    if let Some(bearing) = bearing {
        row.smooth = wrap(row.smooth + ALPHA * wrap(bearing - row.smooth));
        if !row.turning && wrap(row.smooth - head).abs() >= TRIGGER {
            row.turning = true;
        }
        if row.turning {
            head = rotate(head, bearing, RATE);
            row.turning = wrap(bearing - head).abs() > RELEASE;
        }
    }
    if let Some(elevation) = elevation {
        row.pitch_smooth += ALPHA * (elevation - row.pitch_smooth);
        if !row.pitch_turning && (row.pitch_smooth - pitch).abs() >= PITCH_TRIGGER {
            row.pitch_turning = true;
        }
        if row.pitch_turning {
            pitch = (pitch + (elevation - pitch).clamp(-PITCH_RATE, PITCH_RATE)).clamp(-90.0, 90.0);
            row.pitch_turning = (elevation - pitch).abs() > PITCH_RELEASE;
        }
    }
    let (x, z) = (position[0] - row.position[0], position[2] - row.position[2]);
    if x * x + z * z > MOVING {
        // The row moved last tick: the client took the sent yaw as the body
        // and kept the head inside the cone around it.
        row.stable = 0;
        head = within(head, row.body, CONE);
        row.last_stable = head;
    } else {
        row.body = body_rule(row, head);
    }
    // Standing, the stored yaw is the last one written: the step writes it back
    // (a rest when nothing else changed) and the drawn body turns beside it.
    Facing::Set {
        rotation: [stored[0], pitch, head],
        body: row.body,
    }
}

/// The client's standing-body rule (`render_living/state.rs`), stable count
/// capped. The cone also holds while the head turns at exactly the
/// [`HEAD_STABLE`] rate, which the client sees as larger 3-tick steps.
fn body_rule(row: &mut Row, head: f32) -> f32 {
    if wrap(head - row.last_stable).abs() > HEAD_STABLE {
        row.stable = 0;
        row.last_stable = head;
        return within(row.body, head, CONE);
    }
    row.stable = (row.stable + 1).min(ALIGN_DELAY + ALIGN_TICKS);
    if row.stable <= ALIGN_DELAY {
        return within(row.body, head, CONE);
    }
    let fraction = f32::from(row.stable - ALIGN_DELAY) / f32::from(ALIGN_TICKS);
    within(row.body, head, CONE * (1.0 - fraction))
}

/// The previous rest guard of a row that keeps the old look: its head on its
/// body, and the look its step would take less than half a wire quantum away
/// in yaw and in pitch. Its step still runs the old look from where the body
/// ends up, with no tolerance.
fn old_rest(stored: [f32; 3], position: [f64; 3], aim: [f64; 3]) -> bool {
    if stored[2] != stored[0] {
        return false;
    }
    let turned = crate::entity_motion::look(
        stored,
        Vec3::new(position[0], position[1], position[2]),
        Vec3::ZERO,
        Some(aim),
    );
    wrap(turned[0] - stored[0]).abs() < OLD_HOLD && (turned[1] - stored[1]).abs() < OLD_HOLD
}

/// `Mth.rotateIfNecessary(current, target, max)` with its identity inside the
/// cone returned as the very same bits (design 3.4.1): never re-expressed.
fn within(current: f32, target: f32, max: f32) -> f32 {
    let off = wrap(target - current);
    if off.abs() <= max {
        current
    } else {
        target - off.clamp(-max, max)
    }
}

fn wrap(angle: f32) -> f32 {
    (angle + 180.0).rem_euclid(360.0) - 180.0
}

fn rotate(current: f32, wanted: f32, limit: f32) -> f32 {
    current + wrap(wanted - current).clamp(-limit, limit)
}

/// Test seam for the rest-guard fixtures, whose battle is not running: the
/// per-row law of [`project`] for one row, published the same way.
#[cfg(test)]
pub(crate) fn project_row(
    state: &mut crate::tick::TickState,
    slot: EntitySlot,
    target: Option<(EntityId, [f64; 3])>,
) {
    let mut facing = std::mem::take(&mut state.hostile.battle.facing);
    let combat = &state.hostile.battle.combat;
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: state.hostile.battle.elapsed_ticks,
    };
    let id = frame.entities.body(slot).id;
    let fighter = combat
        .ids
        .iter()
        .position(|&fighter| fighter == id)
        .unwrap_or(usize::MAX);
    facing.rows.resize(1, Row::default());
    let row = &mut facing.rows[0];
    if row.id != Some(id) {
        *row = Row {
            id: Some(id),
            ..Row::default()
        };
    }
    let stored = frame.entities.rot(slot);
    let position = frame.entities.pos(slot);
    let published = match target {
        Some((target, aim)) => {
            let law = if keeps_old_look(&frame, combat, fighter, slot) {
                Law::Old
            } else {
                Law::Split
            };
            Some(next(row, law, stored, position, target, aim))
        }
        None => leave(row, stored),
    };
    row.position = position;
    facing.by_slot.clear();
    facing.by_slot.resize(frame.entities.len(), None);
    facing.by_slot[slot.0 as usize] = published;
    state.hostile.battle.facing = facing;
}

#[cfg(test)]
#[path = "facing_tests.rs"]
mod tests;
