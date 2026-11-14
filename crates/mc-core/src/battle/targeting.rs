//! Four-block, faction-partitioned broad phase. Rebuilt from frozen rows once
//! per tick by count/prefix/scatter; queries never mutate shared state.

use crate::boundary::EntityId;
use rustc_hash::FxHashMap;

pub const CELL_BLOCKS: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    pub id: EntityId,
    pub faction: u16,
    pub pos: [f64; 3],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EnemyTarget {
    pub id: EntityId,
    pub pos: [f64; 3],
    pub distance: f64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Query {
    pub target: Option<EnemyTarget>,
    /// Actual enemy bodies distance-tested (not cells or friendly bodies).
    pub examined: u64,
}

#[derive(Debug, Default)]
struct Cell {
    start: usize,
    len: usize,
    filled: usize,
}

#[derive(Debug, Default)]
pub struct TargetGrid {
    cells: FxHashMap<(u16, i32, i32), Cell>,
    factions: Vec<u16>,
    input: Vec<Unit>,
    members: Vec<Unit>,
}

fn cell(value: f64) -> i32 {
    (value / CELL_BLOCKS).floor() as i32
}

fn distance_squared(a: [f64; 3], b: [f64; 3]) -> f64 {
    let [x, y, z] = std::array::from_fn(|axis| a[axis] - b[axis]);
    x * x + y * y + z * z
}

fn cell_distance_squared(pos: [f64; 3], x: i32, z: i32) -> f64 {
    let dx = (f64::from(x) * CELL_BLOCKS - pos[0])
        .max(pos[0] - (f64::from(x) + 1.0) * CELL_BLOCKS)
        .max(0.0);
    let dz = (f64::from(z) * CELL_BLOCKS - pos[2])
        .max(pos[2] - (f64::from(z) + 1.0) * CELL_BLOCKS)
        .max(0.0);
    dx * dx + dz * dz
}

impl TargetGrid {
    /// Inputs are live, valid mob rows supplied by the tick owner. Zero-faction
    /// bodies are not battle targets. Allocation capacity survives every rebuild.
    pub fn rebuild(&mut self, units: impl Iterator<Item = Unit>) {
        self.cells.clear();
        self.factions.clear();
        self.input.clear();
        for unit in units.filter(|unit| unit.faction != 0) {
            let key = (unit.faction, cell(unit.pos[0]), cell(unit.pos[2]));
            self.cells.entry(key).or_default().len += 1;
            if !self.factions.contains(&unit.faction) {
                self.factions.push(unit.faction);
            }
            self.input.push(unit);
        }
        self.factions.sort_unstable();
        let mut start = 0;
        for bucket in self.cells.values_mut() {
            bucket.start = start;
            start += bucket.len;
        }
        self.members.resize(start, Unit { id: EntityId(0), faction: 0, pos: [0.0; 3] });
        for &unit in &self.input {
            let key = (unit.faction, cell(unit.pos[0]), cell(unit.pos[2]));
            let bucket = self.cells.get_mut(&key).expect("counted battle cell");
            self.members[bucket.start + bucket.filled] = unit;
            bucket.filled += 1;
        }
    }

    /// Exact nearest enemy in 3D, inclusive reach. Equal distances select the
    /// lowest stable entity id, independent of bucket order, slots and workers.
    /// A cell's horizontal lower bound prunes work, never a possible winner.
    pub fn nearest_enemy(&self, actor: Unit, reach: f64) -> Query {
        let mut result = Query::default();
        if actor.faction == 0 || !reach.is_finite() || reach < 0.0 {
            return result;
        }
        let mut best = reach * reach;
        let mut nearest: Option<Unit> = None;
        for &faction in &self.factions {
            if faction == actor.faction {
                continue;
            }
            for z in cell(actor.pos[2] - reach)..=cell(actor.pos[2] + reach) {
                for x in cell(actor.pos[0] - reach)..=cell(actor.pos[0] + reach) {
                    if cell_distance_squared(actor.pos, x, z) > best {
                        continue;
                    }
                    let Some(bucket) = self.cells.get(&(faction, x, z)) else {
                        continue;
                    };
                    for unit in &self.members[bucket.start..bucket.start + bucket.len] {
                        result.examined += 1;
                        let d2 = distance_squared(actor.pos, unit.pos);
                        if d2 <= best
                            && nearest.is_none_or(|old| d2 < best || unit.id < old.id)
                        {
                            best = d2;
                            nearest = Some(*unit);
                        }
                    }
                }
            }
        }
        result.target = nearest.map(|unit| EnemyTarget {
            id: unit.id,
            pos: unit.pos,
            distance: best.sqrt(),
        });
        result
    }
}

pub const MELEE_ENGAGEMENT_REACH: f64 = 6.0;
pub const RETARGET_TICKS: u64 = 10;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Work {
    pub retargeted: u64,
    pub examined: u64,
}

/// Stable phase independent of dense-slot compaction or worker scheduling.
pub fn retarget_due(id: EntityId, tick: u64) -> bool {
    tick % RETARGET_TICKS == i64::from(id.0).rem_euclid(RETARGET_TICKS as i64) as u64
}

/// Archers use attack range, never the vanilla follow range.
pub fn engagement_reach(ranged: bool, attack_reach: f32) -> f64 {
    if ranged {
        f64::from(attack_reach)
    } else {
        MELEE_ENGAGEMENT_REACH
    }
}

fn ranged(frame: &super::Frame<'_>, slot: crate::entities::EntitySlot) -> bool {
    matches!(frame.entities.body(slot).kind, crate::entities::EntityKind::Mob { type_id }
        if frame.registry.blueprint(type_id)
            .is_some_and(|bp| bp.has_goal(crate::blueprint::GoalKind::RangedAttack)))
}

fn eligible(
    frame: &super::Frame<'_>,
    battle: &super::BattleState,
    slot: crate::entities::EntitySlot,
) -> bool {
    let body = frame.entities.body(slot);
    let row = frame.entities.row(slot);
    !row.controls.no_ai
        && row.entity_life.mount.rider.is_none()
        && !battle.formation.is_wounded(body.id)
        && if ranged(frame, slot) {
            battle.formation.context(body.id).is_none_or(|context| !context.routing)
        } else {
            battle.formation.can_engage(body.id)
        }
}

/// One immutable grid per tick; one tenth of eligible units reacquires.
/// Retained opponents are O(1)-validated every tick and replaced only on the
/// assigned phase. Rear ranks and carried mounts do not pay for search.
pub(crate) fn update(frame: &super::Frame<'_>, battle: &mut super::BattleState) {
    battle.work = Work::default();
    battle.grid.rebuild(frame.entities.battle_slots()
        .filter_map(|slot| super::faction::live_unit(frame.entities, slot)));
    battle.targets.retain(|actor, target| {
        let Some(slot) = frame.entities.slot_of(*actor) else {
            return false;
        };
        let reach = engagement_reach(ranged(frame, slot), frame.entities.row(slot).attack_reach);
        if let Some(current) = super::retained_enemy(frame.entities, slot, target.id, reach) {
            *target = current;
            true
        } else {
            false
        }
    });
    for slot in frame.entities.battle_slots() {
        let body = frame.entities.body(slot);
        let Some(unit) = super::faction::live_unit(frame.entities, slot) else {
            battle.targets.remove(&body.id);
            continue;
        };
        let chunk = frame.entities.chunk_of(slot);
        if !frame.world.is_active_position(chunk.x, chunk.z) || !eligible(frame, battle, slot) {
            battle.targets.remove(&body.id);
            continue;
        }
        if !retarget_due(body.id, frame.tick) {
            continue;
        }
        battle.work.retargeted += 1;
        let reach = engagement_reach(ranged(frame, slot), frame.entities.row(slot).attack_reach);
        let query = battle.grid.nearest_enemy(unit, reach);
        battle.work.examined += query.examined;
        if let Some(target) = query.target {
            battle.targets.insert(body.id, target);
        } else {
            battle.targets.remove(&body.id);
        }
    }
}
