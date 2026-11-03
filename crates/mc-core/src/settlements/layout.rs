//! Settlement street plan: a fixed street grid through the bell, densely
//! packed road-fronting lots, verge lamp posts, and the road and lamp work
//! each standing building needs to be reached.
//!
//! Geometry is a pure function of the anchor. Roads are three blocks wide
//! every [`PERIOD`] blocks on both axes; a one-block verge on each side
//! carries lamp posts; every block holds two back-to-back rows of lots of the
//! deepest blueprint ([`LOT_DEPTH`]) with a one-block gap between them. The
//! square within [`PLAZA`] blocks of the bell stays open. Nothing about the
//! plan is stored: feasibility is read from the live terrain on every pass,
//! and progress is the world itself plus the structure ledger, so a blocked or
//! unloaded column is reported, never assumed buildable.
//!
//! Objective: every new building takes the first free lot in walking order
//! from the plaza (first fit along each frontage, one-block gaps between
//! buildings, entrance facing the road), so blocks fill densely outward and
//! every door faces a road that is connected on foot to the bell.

use std::collections::{HashMap, HashSet};

use mc_script::ScriptWorldBounds;

use super::SettlementRecord;
use super::blueprints::{self, Blueprint, Rotation};
use super::buildings::Building;
use super::construction::{self, Project};
use crate::plugin_structures::settlement_builds::{self as executor, PlannedCell};
use crate::tick::TickState;

/// Street project kinds; neither is a blueprint nor a certified building.
pub(crate) const ROAD: &str = "road";
pub(crate) const LAMP: &str = "lamp";
/// Street feature kind of finished road: not work, so never a project.
pub(crate) const PAVED: &str = "paved";

/// First lot row, counted from a road centre line (road ±1, verge 2).
const LOT_FRONT: i32 = 3;
/// Deepest lot: the largest blueprint extent, on either axis.
pub(crate) const LOT_DEPTH: i32 = blueprints::MAX_SIZE[2] as i32;
/// Street grid period: two lot rows back to back between two roads.
pub(crate) const PERIOD: i32 = 2 * (LOT_FRONT + LOT_DEPTH);
/// Chebyshev radius of the open square around the bell.
pub(crate) const PLAZA: i32 = 4;
/// Street stretches along x per period as `(first, last, lamp)` offsets from
/// an avenue centre. The first owns the crossing and its corner lamp.
const STREET_TILES: [(i32, i32, i32); 3] = [(-1, 6, 2), (7, 16, 11), (17, 26, 20)];
/// Avenue stretches along z per period, as offsets from a street centre.
const AVENUE_TILES: [(i32, i32, i32); 2] = [(2, 15, 11), (16, 26, 20)];
/// Natural surfaces a road turns into a dirt path; other walkable ground
/// (stone, sand, existing paths) is used as it is.
const SOIL: [&str; 6] = [
    "grass_block",
    "dirt",
    "coarse_dirt",
    "podzol",
    "mycelium",
    "rooted_dirt",
];
/// Surfaces nobody walks a street on: tree crowns and trunks, posts, beds.
const NOT_FOOTING: [&str; 12] = [
    "_leaves",
    "_log",
    "_wood",
    "_stem",
    "_hyphae",
    "_fence",
    "_fence_gate",
    "_wall",
    "_door",
    "_bed",
    "cactus",
    "bell",
];
const SEED: usize = usize::MAX;

#[must_use]
pub(crate) fn infra(blueprint: &str) -> bool {
    blueprint == ROAD || blueprint == LAMP
}

/// Paving turns the soil already in place into a dirt path, as a shovel does:
/// the one block a settlement structure places at no material cost.
#[must_use]
pub(crate) fn free_paving(state: &TickState, blueprint: &str, block: u32) -> bool {
    blueprint == ROAD
        && state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(block))
            .is_some_and(|definition| definition.block.id.path() == "dirt_path")
}

/// Signed offset of `value` from the nearest grid line through `anchor`.
fn signed(value: i32, anchor: i32) -> i32 {
    let offset = (value - anchor).rem_euclid(PERIOD);
    if offset > PERIOD / 2 {
        offset - PERIOD
    } else {
        offset
    }
}

/// What the plan reserves a column for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Cell {
    Road,
    Plaza,
    Verge,
    Lot,
}

#[must_use]
pub(crate) fn cell(anchor: [i32; 3], x: i32, z: i32) -> Cell {
    let (dx, dz) = (signed(x, anchor[0]), signed(z, anchor[2]));
    if dx.abs() <= 1 || dz.abs() <= 1 {
        Cell::Road
    } else if (x - anchor[0]).abs().max((z - anchor[2]).abs()) <= PLAZA {
        Cell::Plaza
    } else if dx.abs() == 2 || dz.abs() == 2 {
        Cell::Verge
    } else {
        Cell::Lot
    }
}

/// Lamp post columns: the crossing corner and every nine blocks along the
/// street's south verge and the avenue's east verge.
#[must_use]
pub(crate) fn lamp_at(anchor: [i32; 3], x: i32, z: i32) -> bool {
    let ox = (x - anchor[0]).rem_euclid(PERIOD);
    let oz = (z - anchor[2]).rem_euclid(PERIOD);
    (oz == 2 && STREET_TILES.iter().any(|tile| tile.2 == ox))
        || (ox == 2 && AVENUE_TILES.iter().any(|tile| tile.2 == oz))
}

/// One road stretch with its lamp post: the structure-ledger unit of street
/// work. Stretches never overlap each other, lots or another stretch's lamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Tile {
    /// 0: street (runs along x); 1: avenue (runs along z).
    pub(crate) axis: u8,
    /// Inclusive `[x, z]` extent of the road band.
    pub(crate) min: [i32; 2],
    pub(crate) max: [i32; 2],
    /// Verge lamp column `[x, z]`.
    pub(crate) lamp: [i32; 2],
}

/// The stretch owning road column `(x, z)`.
#[must_use]
pub(crate) fn tile(anchor: [i32; 3], x: i32, z: i32) -> Option<Tile> {
    let (dx, dz) = (signed(x, anchor[0]), signed(z, anchor[2]));
    if dz.abs() <= 1 {
        let centre = z - dz;
        let mut offset = (x - anchor[0]).rem_euclid(PERIOD);
        if offset == PERIOD - 1 {
            offset = -1;
        }
        let base = x - offset;
        let &(first, last, lamp) = STREET_TILES
            .iter()
            .find(|tile| (tile.0..=tile.1).contains(&offset))?;
        return Some(Tile {
            axis: 0,
            min: [base + first, centre - 1],
            max: [base + last, centre + 1],
            lamp: [base + lamp, centre + 2],
        });
    }
    if dx.abs() <= 1 {
        let centre = x - dx;
        let offset = (z - anchor[2]).rem_euclid(PERIOD);
        let base = z - offset;
        let &(first, last, lamp) = AVENUE_TILES
            .iter()
            .find(|tile| (tile.0..=tile.1).contains(&offset))?;
        return Some(Tile {
            axis: 1,
            min: [centre - 1, base + first],
            max: [centre + 1, base + last],
            lamp: [centre + 2, base + lamp],
        });
    }
    None
}

/// One sampled column of the plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Column {
    Unloaded,
    /// A settlement building, block entity or farmland holds the surface.
    Occupied,
    /// No walkable surface with two clear blocks above it in the band:
    /// water, cliffs, trees, rock, or outside the territory.
    Blocked,
    /// Walkable surface at `y`; `soil` is paved into a dirt path.
    Ground {
        y: i32,
        soil: bool,
    },
}

fn inside(record: &SettlementRecord, at: [i32; 3]) -> bool {
    at.into_iter()
        .zip(record.anchor)
        .map(|(value, anchor)| (i64::from(value) - i64::from(anchor)).pow(2))
        .sum::<i64>()
        <= i64::from(record.radius).pow(2)
}

fn level_inside(record: &SettlementRecord, x: i32, z: i32) -> bool {
    inside(record, [x, record.anchor[1], z])
}

fn sample(state: &TickState, record: &SettlementRecord, x: i32, z: i32) -> Column {
    let anchor_y = record.anchor[1];
    if state.world.physics_block_at(x, anchor_y, z).is_none() {
        return Column::Unloaded;
    }
    // Natural work zones (a certified forest or quarry) stay walkable terrain.
    if record.buildings.values().any(|building| {
        !building.natural_zone
            && (building.bounds.min[0]..=building.bounds.max[0]).contains(&x)
            && (building.bounds.min[2]..=building.bounds.max[2]).contains(&z)
    }) {
        return Column::Occupied;
    }
    let Some(y) = construction::surface_y(state, anchor_y, x, z) else {
        return Column::Blocked;
    };
    if state.block_entities.contains_key(&(x, y, z))
        || state.block_entities.contains_key(&(x, y + 1, z))
    {
        return Column::Occupied;
    }
    let Some(ground) = state
        .world
        .physics_block_at(x, y, z)
        .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
    else {
        return Column::Blocked;
    };
    let name = ground.block.id.path();
    if name == "farmland" {
        return Column::Occupied;
    }
    let headroom = state
        .world
        .physics_block_at(x, y + 2, z)
        .is_some_and(|block| construction::clearable(state, block));
    if ground.has_fluid()
        || NOT_FOOTING.iter().any(|suffix| name.ends_with(suffix))
        || !headroom
        || !inside(record, [x, y + 1, z])
    {
        return Column::Blocked;
    }
    Column::Ground {
        y,
        soil: SOIL.contains(&name),
    }
}

fn vacant(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            matches!(definition.block.id.path(), "air" | "cave_air" | "void_air")
        })
}

fn default_state(state: &TickState, name: &str) -> Option<u32> {
    let id = mc_data::Identifier::parse(name).ok()?;
    Some(state.registry.blocks.block(&id)?.default.0)
}

fn paid(state: &TickState, at: [i32; 3], block: u32) -> Option<PlannedCell> {
    let (item, quantity) =
        blueprints::material_for(&state.registry, mc_world::BlockStateId(block))?;
    Some(PlannedCell {
        at,
        state: block,
        resource: Some(state.registry.items.name_of(item)?.to_string()),
        quantity,
    })
}

fn free(at: [i32; 3], block: u32) -> PlannedCell {
    PlannedCell {
        at,
        state: block,
        resource: None,
        quantity: 0,
    }
}

/// A finished lamp post: a fence topped by a torch anywhere in the band.
fn lamp_standing(state: &TickState, record: &SettlementRecord, x: i32, z: i32) -> bool {
    let name = |y: i32| {
        state
            .world
            .physics_block_at(x, y, z)
            .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
            .map(|definition| definition.block.id.path())
    };
    let band = construction::SITE_BAND;
    (record.anchor[1] - band..=record.anchor[1] + band + 3).any(|y| {
        name(y).is_some_and(|path| path.ends_with("_fence")) && name(y + 1) == Some("torch")
    })
}

/// A blueprint placement on the plan: footprint origin and quarter turns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lot {
    pub(crate) origin: [i32; 3],
    pub(crate) rotation: u8,
}

/// Street work ready for the structure executor.
pub(crate) struct Work {
    pub(crate) bounds: ScriptWorldBounds,
    pub(crate) cells: Vec<PlannedCell>,
}

/// Unit steps of the four compass directions in clockwise quarter turns:
/// north 0, east 1, south 2, west 3.
const COMPASS: [[i32; 2]; 4] = [[0, -1], [1, 0], [0, 1], [-1, 0]];

/// The catalogue front of a blueprint, as a [`COMPASS`] index: the extent
/// edge holding the level-one entrance; an entrance inside the extent (the
/// farm's door between house and field) keeps the catalogue's south front.
fn front(blueprint: &Blueprint) -> Option<usize> {
    let [fx, _, _] = blueprint.footprint(Rotation(0)).map(i32::from);
    let [ex, _, ez] = blueprint.level(1)?.entrance.map(i32::from);
    Some(if ez == 0 {
        0
    } else if ex == fx - 1 {
        1
    } else if ex == 0 {
        3
    } else {
        2
    })
}

/// Quarter turns that make the blueprint's front face the road behind
/// `normal` (pointing from the road into the lot).
fn facing(blueprint: &Blueprint, normal: [i32; 2]) -> Option<u8> {
    let outward = COMPASS
        .iter()
        .position(|step| *step == [-normal[0], -normal[1]])?;
    u8::try_from((outward + 4 - front(blueprint)?) % 4).ok()
}

/// Road and plaza columns reachable on foot from the bell, in walking order.
pub(crate) struct Network<'a> {
    state: &'a TickState,
    record: &'a SettlementRecord,
    columns: HashMap<(i32, i32), Column>,
    cells: Vec<[i32; 3]>,
    parents: Vec<usize>,
    index: HashMap<(i32, i32), usize>,
}

impl<'a> Network<'a> {
    /// Breadth-first walk over road and plaza columns starting on the open
    /// plaza: steps of at most one block up or down, inside the territory,
    /// on loaded walkable ground only.
    pub(crate) fn walk(state: &'a TickState, record: &'a SettlementRecord) -> Self {
        let mut network = Self {
            state,
            record,
            columns: HashMap::new(),
            cells: Vec::new(),
            parents: Vec::new(),
            index: HashMap::new(),
        };
        let [ax, _, az] = record.anchor;
        // Every walkable plaza column starts the walk, nearest the bell first:
        // a building may stand on the bell's own neighbours.
        for ring in 0..=PLAZA {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if dx.abs().max(dz.abs()) == ring
                        && level_inside(record, ax + dx, az + dz)
                        && let Column::Ground { y, .. } = network.column(ax + dx, az + dz)
                    {
                        network.reach([ax + dx, y, az + dz], SEED);
                    }
                }
            }
        }
        let mut next = 0;
        while next < network.cells.len() {
            let [x, y, z] = network.cells[next];
            for (dx, dz) in [(1, 0), (0, 1), (-1, 0), (0, -1)] {
                let (nx, nz) = (x + dx, z + dz);
                if network.index.contains_key(&(nx, nz))
                    || !matches!(cell(record.anchor, nx, nz), Cell::Road | Cell::Plaza)
                    || !level_inside(record, nx, nz)
                {
                    continue;
                }
                if let Column::Ground { y: ny, .. } = network.column(nx, nz)
                    && (ny - y).abs() <= 1
                {
                    network.reach([nx, ny, nz], next);
                }
            }
            next += 1;
        }
        network
    }

    fn reach(&mut self, at: [i32; 3], parent: usize) {
        self.index.insert((at[0], at[2]), self.cells.len());
        self.cells.push(at);
        self.parents.push(parent);
    }

    fn column(&mut self, x: i32, z: i32) -> Column {
        let (state, record) = (self.state, self.record);
        *self
            .columns
            .entry((x, z))
            .or_insert_with(|| sample(state, record, x, z))
    }

    /// Lot candidates in walking order: a reached road edge column and the
    /// normal pointing from it across a lamp-free verge column onto a lot
    /// column, the door step. Road columns inside the plaza front nothing, so
    /// a bounded planning pass never spends attempts on them.
    pub(crate) fn frontage(&self) -> Vec<([i32; 3], [i32; 2])> {
        let anchor = self.record.anchor;
        let mut fronts = Vec::new();
        for &at in &self.cells {
            let (dx, dz) = (signed(at[0], anchor[0]), signed(at[2], anchor[2]));
            for (edge, normal) in [(dz.abs() == 1, [0, dz]), (dx.abs() == 1, [dx, 0])] {
                let verge = [at[0] + normal[0], at[2] + normal[1]];
                if edge
                    && cell(anchor, verge[0], verge[1]) == Cell::Verge
                    && !lamp_at(anchor, verge[0], verge[1])
                    && cell(anchor, verge[0] + normal[0], verge[1] + normal[1]) == Cell::Lot
                {
                    fronts.push((at, normal));
                }
            }
        }
        fronts
    }

    /// The blueprint's lot whose entrance column faces a [`Self::frontage`]
    /// candidate across the verge: footprint wholly on lot columns and
    /// one-block steps from road to verge to floor. Terrain preparation,
    /// buildings, reservations and funding are the construction planner's
    /// checks.
    pub(crate) fn lot(
        &mut self,
        blueprint: &Blueprint,
        front: [i32; 3],
        normal: [i32; 2],
    ) -> Option<Lot> {
        let anchor = self.record.anchor;
        let verge = [front[0] + normal[0], front[2] + normal[1]];
        let door = [front[0] + 2 * normal[0], front[2] + 2 * normal[1]];
        let rotation = facing(blueprint, normal)?;
        let turn = Rotation::from_quarter_turns(rotation);
        let entrance = blueprint.level(1)?.rotated(turn).entrance.map(i32::from);
        let [sx, _, sz] = blueprint.footprint(turn).map(i32::from);
        let [ox, oz] = match normal {
            [0, -1] => [door[0] - entrance[0], door[1] - (sz - 1)],
            [0, _] => [door[0] - entrance[0], door[1]],
            [-1, _] => [door[0] - (sx - 1), door[1] - entrance[2]],
            _ => [door[0], door[1] - entrance[2]],
        };
        if !(ox..ox + sx).all(|x| (oz..oz + sz).all(|z| cell(anchor, x, z) == Cell::Lot)) {
            return None;
        }
        let Column::Ground { y: verge_y, .. } = self.column(verge[0], verge[1]) else {
            return None;
        };
        let Column::Ground { y: floor, .. } = self.column(door[0], door[1]) else {
            return None;
        };
        ((verge_y - front[1]).abs() <= 1 && (floor - verge_y).abs() <= 1).then_some(Lot {
            origin: [ox, floor, oz],
            rotation,
        })
    }

    /// A standing building's street access, as a reached column index. A
    /// blueprint build is reached through its door: the first reached road or
    /// plaza column straight out of its level-one entrance. A player-certified
    /// building has no door on record; its access is the nearest reached
    /// column two to four blocks out from any side.
    fn access(&self, building: &Building) -> Option<usize> {
        if let Some((blueprint, origin, rotation)) =
            construction::placement(self.state, self.record.id, building)
        {
            let turn = Rotation::from_quarter_turns(rotation);
            let [ex, _, ez] = blueprint.level(1)?.rotated(turn).entrance.map(i32::from);
            let [nx, nz] = COMPASS[(front(blueprint)? + usize::from(rotation)) % 4];
            let (x, z) = (origin[0] + ex, origin[2] + ez);
            return (1..=LOT_FRONT + LOT_DEPTH)
                .find_map(|step| self.index.get(&(x + step * nx, z + step * nz)).copied());
        }
        let bounds = building.bounds;
        let mut best: Option<usize> = None;
        for x in bounds.min[0]..=bounds.max[0] {
            for z in bounds.min[2]..=bounds.max[2] {
                if x != bounds.min[0]
                    && x != bounds.max[0]
                    && z != bounds.min[2]
                    && z != bounds.max[2]
                {
                    continue;
                }
                for [nx, nz] in [[1, 0], [-1, 0], [0, 1], [0, -1]] {
                    for step in 2..=4 {
                        if let Some(&index) = self.index.get(&(x + step * nx, z + step * nz)) {
                            best = Some(best.map_or(index, |known| known.min(index)));
                        }
                    }
                }
            }
        }
        best
    }

    /// Stretches on the walks from the plaza to every standing building's
    /// street access, nearest the plaza first: the roads the village uses.
    pub(crate) fn served(&self) -> Vec<Tile> {
        let anchor = self.record.anchor;
        let mut fronts: Vec<usize> = self
            .record
            .buildings
            .values()
            .filter(|building| !building.natural_zone)
            .filter_map(|building| self.access(building))
            .collect();
        fronts.sort_unstable();
        fronts.dedup();
        let mut tiles: Vec<Tile> = Vec::new();
        for front in fronts {
            let mut walk = Vec::new();
            let mut at = front;
            loop {
                walk.push(at);
                if self.parents[at] == SEED {
                    break;
                }
                at = self.parents[at];
            }
            for &step in walk.iter().rev() {
                let [x, _, z] = self.cells[step];
                if let Some(found) = tile(anchor, x, z)
                    && !tiles.contains(&found)
                {
                    tiles.push(found);
                }
            }
        }
        tiles
    }

    /// Every stretch with a reached column, in walking order.
    pub(crate) fn tiles(&self) -> Vec<Tile> {
        let mut seen = HashSet::new();
        self.cells
            .iter()
            .filter_map(|at| tile(self.record.anchor, at[0], at[2]))
            .filter(|found| seen.insert(*found))
            .collect()
    }

    /// The reached columns of one stretch by state, as exact rectangles: the
    /// columns the planner still owes work on (`work`, `ROAD`) and the columns
    /// already surfaced with a dirt path (`PAVED`). A column is one or the
    /// other; an unreached, blocked, unpaved or other column is neither, so no
    /// rectangle spans a gap (water, trees, stone left as it is). Runs are
    /// maximal along the stretch and merged across adjacent lines of equal
    /// extent.
    fn road_runs(
        &self,
        tile: &Tile,
        work: Option<&Work>,
    ) -> Vec<(&'static str, ScriptWorldBounds)> {
        let state = self.state;
        let owed: HashSet<(i32, i32)> = work
            .into_iter()
            .flat_map(|work| work.cells.iter().map(|cell| (cell.at[0], cell.at[2])))
            .collect();
        let kind_at = |x: i32, z: i32| -> Option<(&'static str, i32)> {
            let y = self.cells[*self.index.get(&(x, z))?][1];
            if owed.contains(&(x, z)) {
                return Some((ROAD, y));
            }
            state
                .world
                .physics_block_at(x, y, z)
                .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
                .filter(|definition| definition.block.id.path() == "dirt_path")
                .map(|_| (PAVED, y))
        };
        let (along, across) = if tile.axis == 0 { (0, 1) } else { (1, 0) };
        // One run per line across the stretch: (kind, first, last, line, low y, high y).
        let mut runs: Vec<(&'static str, i32, i32, i32, i32, i32)> = Vec::new();
        for line in tile.min[across]..=tile.max[across] {
            let mut open: Option<(&'static str, i32, i32, i32, i32)> = None;
            for step in tile.min[along]..=tile.max[along] {
                let (x, z) = if along == 0 {
                    (step, line)
                } else {
                    (line, step)
                };
                let here = kind_at(x, z);
                if let (Some(run), Some((kind, y))) = (open.as_mut(), here)
                    && run.0 == kind
                {
                    run.2 = step;
                    run.3 = run.3.min(y);
                    run.4 = run.4.max(y);
                    continue;
                }
                if let Some(run) = open.take() {
                    runs.push((run.0, run.1, run.2, line, run.3, run.4));
                }
                open = here.map(|(kind, y)| (kind, step, step, y, y));
            }
            if let Some(run) = open {
                runs.push((run.0, run.1, run.2, line, run.3, run.4));
            }
        }
        runs.sort_unstable();
        // (kind, first, last, first line, last line, low y, high y)
        let mut merged: Vec<(&'static str, i32, i32, i32, i32, i32, i32)> = Vec::new();
        for (kind, first, last, line, low, high) in runs {
            if let Some(prev) = merged.last_mut()
                && (prev.0, prev.1, prev.2) == (kind, first, last)
                && prev.4 + 1 == line
            {
                prev.4 = line;
                prev.5 = prev.5.min(low);
                prev.6 = prev.6.max(high);
            } else {
                merged.push((kind, first, last, line, line, low, high));
            }
        }
        merged
            .into_iter()
            .filter_map(|(kind, first, last, from, to, low, high)| {
                let (min, max) = if along == 0 {
                    ([first, low, from], [last, high, to])
                } else {
                    ([from, low, first], [to, high, last])
                };
                Some((kind, ScriptWorldBounds::new(min, max).ok()?))
            })
            .collect()
    }

    /// Paving still owed on a stretch's reached columns: soil becomes a dirt
    /// path and plants are cleared from the two blocks above. Free work.
    pub(crate) fn road_work(&mut self, tile: &Tile) -> Option<Work> {
        let state = self.state;
        let air = state.registry.states.air;
        let path = default_state(state, "minecraft:dirt_path")?;
        let mut cells = Vec::new();
        let (mut low, mut high) = (i32::MAX, i32::MIN);
        for x in tile.min[0]..=tile.max[0] {
            for z in tile.min[1]..=tile.max[1] {
                let Some(&index) = self.index.get(&(x, z)) else {
                    continue;
                };
                let y = self.cells[index][1];
                let Column::Ground { soil, .. } = self.column(x, z) else {
                    continue;
                };
                low = low.min(y);
                high = high.max(y + 2);
                for above in [y + 1, y + 2] {
                    if let Some(block) = state.world.physics_block_at(x, above, z)
                        && !vacant(state, block)
                        && construction::clearable(state, block)
                    {
                        cells.push(free([x, above, z], air));
                    }
                }
                if soil {
                    cells.push(free([x, y, z], path));
                }
            }
        }
        if cells.is_empty() {
            return None;
        }
        let bounds = ScriptWorldBounds::new(
            [tile.min[0], low, tile.min[1]],
            [tile.max[0], high, tile.max[1]],
        )
        .ok()?;
        Some(Work { bounds, cells })
    }

    /// The stretch's lamp post when it is still missing and buildable: two
    /// oak fences and a torch on walkable verge ground beside a reached road
    /// column. Species and funding are the construction planner's choice.
    pub(crate) fn lamp_work(&mut self, tile: &Tile) -> Option<Work> {
        let (state, record) = (self.state, self.record);
        let [x, z] = tile.lamp;
        let stand = stand_column(record.anchor, x, z);
        if !self.index.contains_key(&(stand[0], stand[1]))
            || !level_inside(record, x, z)
            || lamp_standing(state, record, x, z)
        {
            return None;
        }
        let Column::Ground { y, .. } = self.column(x, z) else {
            return None;
        };
        let top = state.world.physics_block_at(x, y + 3, z)?;
        if !construction::clearable(state, top) || !inside(record, [x, y + 3, z]) {
            return None;
        }
        let fence = default_state(state, "minecraft:oak_fence")?;
        let torch = default_state(state, "minecraft:torch")?;
        let air = state.registry.states.air;
        let mut cells = Vec::new();
        for above in y + 1..=y + 3 {
            let block = state.world.physics_block_at(x, above, z)?;
            if !vacant(state, block) {
                cells.push(free([x, above, z], air));
            }
        }
        cells.push(paid(state, [x, y + 1, z], fence)?);
        cells.push(paid(state, [x, y + 2, z], fence)?);
        cells.push(paid(state, [x, y + 3, z], torch)?);
        let bounds = ScriptWorldBounds::new([x, y + 1, z], [x, y + 3, z]).ok()?;
        Some(Work { bounds, cells })
    }
}

/// The road column a builder stands on to set the lamp post at `(x, z)`.
fn stand_column(anchor: [i32; 3], x: i32, z: i32) -> [i32; 2] {
    if (z - anchor[2]).rem_euclid(PERIOD) == 2 {
        [x, z - 1]
    } else {
        [x - 1, z]
    }
}

/// Where a builder stands for the next street cell: on the road column itself
/// while paving, or on the road beside a lamp post's column.
#[must_use]
pub(crate) fn stand(state: &TickState, project: &Project, next: [i32; 3]) -> Option<[i32; 3]> {
    let record = state.settlements.get(project.settlement)?;
    let [x, z] = if project.blueprint == LAMP {
        stand_column(record.anchor, next[0], next[2])
    } else {
        [next[0], next[2]]
    };
    let y = construction::surface_y(state, record.anchor[1], x, z)?;
    Some([x, y + 1, z])
}

/// The structure-ledger box of any active project: blueprint footprint with
/// its foundation, or a street stretch or lamp post.
#[must_use]
pub(crate) fn project_bounds(state: &TickState, project: &Project) -> Option<ScriptWorldBounds> {
    let snapshot = executor::status(state, project.settlement, &project.structure).ok()?;
    let max: [i32; 3] =
        std::array::from_fn(|axis| snapshot.origin[axis] + snapshot.reserved_footprint[axis] - 1);
    ScriptWorldBounds::new(snapshot.origin, max).ok()
}

/// Street features for the settlement map, as exact rectangles: on every
/// reached stretch the columns the planner still owes road work on (`ROAD`,
/// only on the walks from the plaza to standing buildings, the stretches it
/// paves) and the columns already surfaced with a dirt path (`PAVED`), then the
/// missing buildable lamp posts of those walks (`LAMP`, whether or not storage
/// funds them yet). Columns that are unreached, blocked or neither are in no
/// rectangle. Read-only; unloaded columns are skipped.
#[must_use]
pub(crate) fn street_features(
    state: &TickState,
    record: &SettlementRecord,
) -> Vec<(&'static str, ScriptWorldBounds)> {
    let mut network = Network::walk(state, record);
    let served = network.served();
    let mut features = Vec::new();
    for tile in network.tiles() {
        let work = if served.contains(&tile) {
            network.road_work(&tile)
        } else {
            None
        };
        features.extend(network.road_runs(&tile, work.as_ref()));
    }
    for tile in &served {
        if let Some(work) = network.lamp_work(tile) {
            features.push((LAMP, work.bounds));
        }
    }
    features
}

/// `/settlement layout`: the plan, its objective, what it has reached and
/// what blocks it, from the same live walk the planner uses.
#[must_use]
pub(crate) fn report(state: &TickState, record: &SettlementRecord) -> String {
    let mut network = Network::walk(state, record);
    let anchor = record.anchor;
    let radius = record.radius.min(256) as i32;
    let (mut territory, mut road) = (0_u64, 0_u64);
    let (mut unloaded, mut occupied, mut blocked) = (0_u64, 0_u64, 0_u64);
    for x in anchor[0] - radius..=anchor[0] + radius {
        for z in anchor[2] - radius..=anchor[2] + radius {
            if !level_inside(record, x, z) {
                continue;
            }
            territory += 1;
            if cell(anchor, x, z) != Cell::Road {
                continue;
            }
            road += 1;
            if network.index.contains_key(&(x, z)) {
                continue;
            }
            match network.column(x, z) {
                Column::Unloaded => unloaded += 1,
                Column::Occupied => occupied += 1,
                Column::Blocked | Column::Ground { .. } => blocked += 1,
            }
        }
    }
    let reached = network
        .cells
        .iter()
        .filter(|at| cell(anchor, at[0], at[2]) == Cell::Road)
        .count();
    let tiles = network.tiles();
    let (mut paved, mut lamps, mut buildable) = (0, 0, 0);
    for tile in &tiles {
        if network.road_work(tile).is_none() {
            paved += 1;
        }
        if lamp_standing(state, record, tile.lamp[0], tile.lamp[1]) {
            lamps += 1;
        } else if network.lamp_work(tile).is_some() {
            buildable += 1;
        }
    }
    let standing: Vec<&Building> = record
        .buildings
        .values()
        .filter(|building| !building.natural_zone)
        .collect();
    let fronting = standing
        .iter()
        .filter(|building| network.access(building).is_some())
        .count();
    let covered: i64 = record
        .buildings
        .values()
        .map(|building| {
            i64::from(building.bounds.max[0] - building.bounds.min[0] + 1)
                * i64::from(building.bounds.max[2] - building.bounds.min[2] + 1)
        })
        .sum();
    let percent = |part: i64| part * 100 / i64::try_from(territory.max(1)).unwrap_or(1);
    let served = network.served();
    let next = served
        .iter()
        .find_map(|tile| {
            network.road_work(tile).map(|work| {
                format!(
                    "pave road x {}..{} z {}..{}",
                    work.bounds.min[0], work.bounds.max[0], work.bounds.min[2], work.bounds.max[2]
                )
            })
        })
        .or_else(|| {
            served.iter().find_map(|tile| {
                network.lamp_work(tile).map(|work| {
                    format!(
                        "lamp post at {} {} {} (2 fences + 1 torch from storage)",
                        work.bounds.min[0], work.bounds.min[1], work.bounds.min[2]
                    )
                })
            })
        })
        .unwrap_or_else(|| "none".to_owned());
    let plaza = 2 * PLAZA + 1;
    [
        format!(
            "Street plan of {}: roads 3 wide every {PERIOD} blocks through the bell, lots up to {LOT_DEPTH} deep facing them, lamp posts on the verge every 9 blocks, an open {plaza}x{plaza} plaza at the bell.",
            record.name,
        ),
        "Objective: each new building takes the first free road-fronting lot in walking order from the plaza (first fit, one-block gaps, entrance to the road).".to_owned(),
        format!(
            "Territory {territory} columns: buildings cover {covered} ({}%), planned roads {road} ({}%).",
            percent(covered),
            percent(i64::try_from(road).unwrap_or(i64::MAX)),
        ),
        format!(
            "Roads: {reached}/{road} columns reachable on foot from the bell; {paved}/{} reachable stretches paved. Cut off: {blocked} by terrain (water, slopes, trees, rock), {occupied} by buildings or containers, {unloaded} unloaded.",
            tiles.len(),
        ),
        format!(
            "Lamps: {lamps} standing, {buildable} buildable on reachable streets."
        ),
        format!(
            "Buildings: {fronting}/{} face a reachable road.",
            standing.len()
        ),
        format!("Next street work: {next}."),
    ]
    .join("\n")
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
