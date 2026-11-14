//! Four-block, faction-partitioned broad phase. Rebuilt from frozen rows once
//! per tick by count/prefix/scatter; queries never mutate shared state.

use crate::boundary::EntityId;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

pub const CELL_BLOCKS: f64 = 4.0;
const TARGET_CHUNK: usize = 256;

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

struct Search {
    nearest: Option<Unit>,
    best: f64,
    examined: u64,
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
        self.members.resize(
            start,
            Unit {
                id: EntityId(0),
                faction: 0,
                pos: [0.0; 3],
            },
        );
        for &unit in &self.input {
            let key = (unit.faction, cell(unit.pos[0]), cell(unit.pos[2]));
            let bucket = self.cells.get_mut(&key).expect("counted battle cell");
            self.members[bucket.start + bucket.filled] = unit;
            bucket.filled += 1;
        }
    }

    fn consider_cell(&self, actor: Unit, faction: u16, x: i32, z: i32, search: &mut Search) {
        if cell_distance_squared(actor.pos, x, z) > search.best {
            return;
        }
        let Some(bucket) = self.cells.get(&(faction, x, z)) else {
            return;
        };
        for unit in &self.members[bucket.start..bucket.start + bucket.len] {
            search.examined += 1;
            let d2 = distance_squared(actor.pos, unit.pos);
            if d2 <= search.best
                && search
                    .nearest
                    .is_none_or(|old| d2 < search.best || unit.id < old.id)
            {
                search.best = d2;
                search.nearest = Some(*unit);
            }
        }
    }

    /// Exact nearest enemy in 3D, inclusive reach. Equal distances select the
    /// lowest stable entity id, independent of bucket order, slots and workers.
    /// A cell's horizontal lower bound prunes work, never a possible winner.
    pub fn nearest_enemy(&self, actor: Unit, reach: f64) -> Query {
        if actor.faction == 0 || !reach.is_finite() || reach < 0.0 {
            return Query::default();
        }
        let mut search = Search {
            nearest: None,
            best: reach * reach,
            examined: 0,
        };
        let (cx, cz) = (cell(actor.pos[0]), cell(actor.pos[2]));
        let radius = (reach / CELL_BLOCKS).ceil() as i32;
        for &faction in &self.factions {
            if faction == actor.faction {
                continue;
            }
            for ring in 0..=radius {
                // This ring is at least (ring-1)*cell_size away, even at a
                // cell edge. Strict > preserves equal-distance boundary ties.
                let lower = f64::from((ring - 1).max(0)) * CELL_BLOCKS;
                if lower * lower > search.best {
                    break;
                }
                for dx in -ring..=ring {
                    let x = cx.saturating_add(dx);
                    self.consider_cell(actor, faction, x, cz.saturating_sub(ring), &mut search);
                    if ring != 0 {
                        self.consider_cell(actor, faction, x, cz.saturating_add(ring), &mut search);
                    }
                }
                for dz in (-ring + 1)..ring {
                    let z = cz.saturating_add(dz);
                    self.consider_cell(actor, faction, cx.saturating_sub(ring), z, &mut search);
                    self.consider_cell(actor, faction, cx.saturating_add(ring), z, &mut search);
                }
            }
        }
        Query {
            target: search.nearest.map(|unit| EnemyTarget {
                id: unit.id,
                pos: unit.pos,
                distance: search.best.sqrt(),
            }),
            examined: search.examined,
        }
    }

    /// The first enemy within `reach` (3D, inclusive) that `accept` takes,
    /// cells visited ring by ring outwards: a melee range test needs any enemy
    /// in range, not only the nearest (inside a spear's minimum, or across a
    /// height step, the nearest one can fail where another passes).
    pub fn any_enemy(
        &self,
        actor: Unit,
        reach: f64,
        mut accept: impl FnMut(&Unit) -> bool,
    ) -> Option<Unit> {
        if actor.faction == 0 || !reach.is_finite() || reach < 0.0 {
            return None;
        }
        let limit = reach * reach;
        let (cx, cz) = (cell(actor.pos[0]), cell(actor.pos[2]));
        let radius = (reach / CELL_BLOCKS).ceil() as i32;
        for &faction in &self.factions {
            if faction == actor.faction {
                continue;
            }
            for ring in 0..=radius {
                for dx in -ring..=ring {
                    for dz in -ring..=ring {
                        if dx.abs() != ring && dz.abs() != ring {
                            continue;
                        }
                        let (x, z) = (cx.saturating_add(dx), cz.saturating_add(dz));
                        if cell_distance_squared(actor.pos, x, z) > limit {
                            continue;
                        }
                        let Some(bucket) = self.cells.get(&(faction, x, z)) else {
                            continue;
                        };
                        for unit in &self.members[bucket.start..bucket.start + bucket.len] {
                            if distance_squared(actor.pos, unit.pos) <= limit && accept(unit) {
                                return Some(*unit);
                            }
                        }
                    }
                }
            }
        }
        None
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

fn eligible(
    formation: &super::formation::FormationRuntime,
    columns: &super::Columns,
    bi: usize,
) -> bool {
    if !columns.active[bi] || columns.carried[bi] {
        return false;
    }
    if bi >= formation.members.len() {
        return true;
    }
    // Every steady soldier looks for an opponent within reach, whatever his
    // rank: mixing, flanks and gaps bring enemies to rear ranks, and a man
    // with an enemy at his shoulder who never strikes is no fighter (G2 had
    // 0.13 melee swings/s per fighter in contact). Fugitives search only at
    // bay against the battle bounds.
    !formation.context_at(bi).routing || formation.cornered_at(bi)
}

fn retained(
    entities: &crate::entities::EntityStore,
    columns: &super::Columns,
    bi: usize,
    target: EntityId,
) -> Option<EnemyTarget> {
    let slot = entities.slot_of(target)?;
    let enemy = columns.index(slot)?;
    if !columns.targetable[enemy] || !super::enemies(columns.factions[bi], columns.factions[enemy])
    {
        return None;
    }
    let distance = distance_squared(columns.positions[bi], columns.positions[enemy]).sqrt();
    (distance <= columns.reach[bi]).then_some(EnemyTarget {
        id: target,
        pos: columns.positions[enemy],
        distance,
    })
}

/// One immutable grid per tick; one tenth of eligible units reacquires.
/// Retained opponents are O(1)-validated every tick and replaced only on the
/// assigned phase. Fugitives and carried mounts do not pay for search.
pub(crate) fn update(frame: &super::Frame<'_>, battle: &mut super::BattleState) {
    battle
        .grid
        .rebuild((0..battle.columns.ids.len()).filter_map(|bi| {
            battle.columns.targetable[bi].then_some(Unit {
                id: battle.columns.ids[bi],
                faction: battle.columns.factions[bi],
                pos: battle.columns.positions[bi],
            })
        }));
    let mut targets = std::mem::take(&mut battle.columns.targets);
    let columns = &battle.columns;
    let formation = &battle.formation;
    let grid = &battle.grid;
    let entities = frame.entities;
    let tick = frame.tick;
    let sample = |(chunk_index, chunk): (usize, &mut [Option<EnemyTarget>])| {
        let mut work = Work::default();
        for (offset, target) in chunk.iter_mut().enumerate() {
            let bi = chunk_index * TARGET_CHUNK + offset;
            if !eligible(formation, columns, bi) {
                *target = None;
                continue;
            }
            let id = columns.ids[bi];
            if !retarget_due(id, tick) {
                *target = (*target).and_then(|target| retained(entities, columns, bi, target.id));
                continue;
            }
            work.retargeted += 1;
            *target = if columns.ranged[bi] && bi < formation.members.len() {
                let roll = super::combat::roll(id, tick / RETARGET_TICKS, 0x4152_4348) as u64;
                formation.archer_target(bi, roll).and_then(|target| {
                    work.examined += 1;
                    retained(entities, columns, bi, target)
                })
            } else {
                let query = grid.nearest_enemy(
                    Unit {
                        id,
                        faction: columns.factions[bi],
                        pos: columns.positions[bi],
                    },
                    columns.reach[bi],
                );
                work.examined += query.examined;
                query.target
            };
        }
        work
    };
    let merge = |a: Work, b: Work| Work {
        retargeted: a.retargeted + b.retargeted,
        examined: a.examined + b.examined,
    };
    battle.work = if targets.len() >= TARGET_CHUNK * 2 && rayon::current_num_threads() > 1 {
        targets
            .par_chunks_mut(TARGET_CHUNK)
            .enumerate()
            .map(sample)
            .reduce(Work::default, merge)
    } else {
        targets
            .chunks_mut(TARGET_CHUNK)
            .enumerate()
            .map(sample)
            .fold(Work::default(), merge)
    };
    battle.columns.targets = targets;
}
