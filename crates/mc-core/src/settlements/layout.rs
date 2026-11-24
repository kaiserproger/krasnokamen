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
use super::buildings::{Building, BuildingKind};
use super::construction::{self, Project};
use crate::plugin_structures::settlement_builds::{self as executor, PlannedCell};
use crate::tick::TickState;

/// Street and fortification project kinds; none is a blueprint or a certified
/// building.
pub(crate) const ROAD: &str = "road";
pub(crate) const LAMP: &str = "lamp";
/// Free grass clearing that gathers wheat seeds for the first Farm.
pub(crate) const MEADOW: &str = "meadow";
/// One stretch of the perimeter wall, or one gatehouse where a street crosses it.
pub(crate) const PERIMETER: &str = "perimeter";
pub(crate) const GATE: &str = "perimeter_gate";
/// The plaza well: a cobblestone rim around one water source.
pub(crate) const WELL: &str = "well";
/// Street feature kind of finished road: not work, so never a project.
pub(crate) const PAVED: &str = "paved";

/// First lot row, counted from a road centre line (road ±1, verge 2).
/// What a villager walks between two neighbouring columns: up one block, down
/// three (`allocator` path search). Street steps, verge and door steps keep it.
const STEP_UP: i32 = 1;
const STEP_DOWN: i32 = 3;
const LOT_FRONT: i32 = 3;
/// Deepest lot: the largest blueprint extent across its front (the wall's
/// 13-block length runs along the road).
pub(crate) const LOT_DEPTH: i32 = blueprints::MAX_SIZE[2] as i32;
/// Street grid period: two lot rows back to back between two roads.
pub(crate) const PERIOD: i32 = 2 * (LOT_FRONT + LOT_DEPTH);
/// Chebyshev radius of the open square around the bell.
pub(crate) const PLAZA: i32 = 4;
/// Chebyshev distance of the perimeter wall from the bell: the outer verge of
/// the first ring road. The ring road runs inside the wall, every lot of the
/// four blocks around the bell keeps its frontage, and no lot reaches the wall.
pub(crate) const RING: i32 = PERIOD + 2;
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
    matches!(blueprint, ROAD | LAMP | MEADOW | PERIMETER | GATE | WELL)
}

/// The blocks a settlement structure places at no material cost: paving turns
/// the soil already in place into a dirt path, as a shovel does, and the well's
/// one water source is the water table the shaft reaches (no storage holds a
/// filled bucket, and a bucket is not craftable from the stock).
#[must_use]
pub(crate) fn free_paving(state: &TickState, blueprint: &str, block: u32) -> bool {
    let free = match blueprint {
        ROAD => "dirt_path",
        WELL => "water",
        _ => return false,
    };
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| definition.block.id.path() == free)
}

/// Square side of one meadow clearing window.
const MEADOW_WINDOW: i32 = 16;
/// Windows examined either side of the bell, in each axis (48 blocks).
const MEADOW_REACH: i32 = 3;
/// Fewest seed-bearing plants worth a Builder's trip: at one in eight each,
/// a window of this size is expected to yield one wheat seed. Seed-24's
/// savanna village holds about two plants per 8 x 8 block of ground.
const MEADOW_MIN_PLANTS: usize = 8;

/// Plants whose vanilla loot table drops wheat seeds when broken by hand.
fn seed_plant(state: &TickState, block: u32) -> bool {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .is_some_and(|definition| {
            matches!(
                definition.block.id.path(),
                "short_grass" | "tall_grass" | "fern" | "large_fern"
            )
        })
}

/// The nearest window around the bell holding enough seed-bearing grass,
/// cleared at no material cost: its loot is credited to storage like any
/// cleared plant. Buildings, containers and farmland are never touched.
#[must_use]
pub(crate) fn meadow_work(state: &TickState, record: &SettlementRecord) -> Option<Work> {
    let anchor = record.anchor;
    let air = state.registry.states.air;
    let mut windows: Vec<[i32; 2]> = (-MEADOW_REACH..MEADOW_REACH)
        .flat_map(|i| (-MEADOW_REACH..MEADOW_REACH).map(move |j| [i, j]))
        .map(|[i, j]| [anchor[0] + i * MEADOW_WINDOW, anchor[2] + j * MEADOW_WINDOW])
        .collect();
    let centre = |[x, z]: [i32; 2]| {
        i64::from(x + MEADOW_WINDOW / 2 - anchor[0]).pow(2)
            + i64::from(z + MEADOW_WINDOW / 2 - anchor[2]).pow(2)
    };
    windows.sort_by_key(|&window| (centre(window), window));
    for [wx, wz] in windows {
        let mut cells = Vec::new();
        let mut plants = 0;
        let (mut low, mut high) = (i32::MAX, i32::MIN);
        for x in wx..wx + MEADOW_WINDOW {
            for z in wz..wz + MEADOW_WINDOW {
                if !level_inside(record, x, z)
                    || record.buildings.values().any(|building| {
                        !building.natural_zone
                            && (building.bounds.min[0]..=building.bounds.max[0]).contains(&x)
                            && (building.bounds.min[2]..=building.bounds.max[2]).contains(&z)
                    })
                {
                    continue;
                }
                let Some(y) = construction::surface_y(state, anchor[1], x, z) else {
                    continue;
                };
                if state.block_entities.contains_key(&(x, y + 1, z)) {
                    continue;
                }
                let Some(first) = state.world.physics_block_at(x, y + 1, z) else {
                    continue;
                };
                if !seed_plant(state, first) {
                    continue;
                }
                plants += 1;
                for above in [y + 1, y + 2] {
                    if let Some(block) = state.world.physics_block_at(x, above, z)
                        && !vacant(state, block)
                        && construction::clearable(state, block)
                    {
                        cells.push(cleared([x, above, z], air));
                        low = low.min(above);
                        high = high.max(above);
                    }
                }
            }
        }
        if plants < MEADOW_MIN_PLANTS {
            continue;
        }
        let bounds = ScriptWorldBounds::new(
            [wx, low, wz],
            [wx + MEADOW_WINDOW - 1, high, wz + MEADOW_WINDOW - 1],
        )
        .ok()?;
        if executor::area_reserved(state, bounds) {
            continue;
        }
        return Some(Work { bounds, cells });
    }
    None
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
    /// The perimeter wall line on the first ring road's outer verge: no lot
    /// reaches it and no frontage forms across it from outside.
    Wall,
    Lot,
}

/// Whether `(x, z)` is on the square wall line [`RING`] blocks from the bell.
fn on_ring(anchor: [i32; 3], x: i32, z: i32) -> bool {
    (x - anchor[0]).abs().max((z - anchor[2]).abs()) == RING
}

#[must_use]
pub(crate) fn cell(anchor: [i32; 3], x: i32, z: i32) -> Cell {
    let (dx, dz) = (signed(x, anchor[0]), signed(z, anchor[2]));
    if dx.abs() <= 1 || dz.abs() <= 1 {
        Cell::Road
    } else if (x - anchor[0]).abs().max((z - anchor[2]).abs()) <= PLAZA {
        Cell::Plaza
    } else if on_ring(anchor, x, z) {
        Cell::Wall
    } else if dx.abs() == 2 || dz.abs() == 2 {
        Cell::Verge
    } else {
        Cell::Lot
    }
}

/// Lamp post columns: the crossing corner and every nine blocks along the
/// street's south verge and the avenue's east verge, never on the wall line.
#[must_use]
pub(crate) fn lamp_at(anchor: [i32; 3], x: i32, z: i32) -> bool {
    let ox = (x - anchor[0]).rem_euclid(PERIOD);
    let oz = (z - anchor[2]).rem_euclid(PERIOD);
    ((oz == 2 && STREET_TILES.iter().any(|tile| tile.2 == ox))
        || (ox == 2 && AVENUE_TILES.iter().any(|tile| tile.2 == oz)))
        && !on_ring(anchor, x, z)
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

pub(super) fn sample(
    state: &TickState,
    record: &SettlementRecord,
    x: i32,
    z: i32,
    prepare_forest: bool,
) -> Column {
    sample_near(state, record, x, z, prepare_forest, None)
}

/// A column's ground. Within eight blocks of the bell's elevation it is the
/// highest standing surface there; beyond that band (a hillside village) only a
/// column next to a reached one is found, within what a villager walks from
/// `hint`: one block up, three down (owner 2026-11-17, bell-elevation limit).
pub(super) fn sample_near(
    state: &TickState,
    record: &SettlementRecord,
    x: i32,
    z: i32,
    prepare_forest: bool,
    hint: Option<i32>,
) -> Column {
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
    // Owner D27.4: inside the territory a player's history neither hides the
    // street column's ground nor blocks its headroom. Owner D7: a registered
    // planted tree on the route is felled whole like a tree on the lot, so a
    // construction forecast walks through it (the proposed crown of a planting
    // preview still blocks, see `Network::column`).
    let mut street = super::forestry::Clearance::street(record, prepare_forest);
    let Some(y) = construction::surface_y_for_lot(state, anchor_y, x, z, None, &mut street)
        .or_else(|| {
            hint.and_then(|near| {
                construction::surface_y_between(
                    state,
                    near - STEP_DOWN..=near + STEP_UP,
                    x,
                    z,
                    None,
                    &mut street,
                )
            })
        })
    else {
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
    let headroom = [y + 1, y + 2].into_iter().all(|above| {
        state
            .world
            .physics_block_at(x, above, z)
            .is_some_and(|block| {
                construction::clearable(state, block)
                    || (prepare_forest && street.clearable(state, [x, above, z], None))
            })
    });
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

/// A new crown may change the preferred route or lot, but must not remove
/// existing building access or the last feasible lot of a housing blueprint.
/// The proof takes one slice of the tick's site `budget` (owner D42): its
/// walks and at most the slice's lot checks. Per house blueprint the crown
/// spares the blueprint's housing when
/// - the last free lot found (the village's hint) is still free on the
///   crowned walk: one check; or
/// - no lot the crown takes from the plain walk is free now
///   (`construction::takes_free_lot`), all checked in this proof.
///
/// Otherwise the crowned walk must keep another free lot: this crown's own
/// scan looks for one with a fair share of the slice and its next proof
/// continues it (`construction::crowned_house_lot`); a walk without one
/// refuses. Every verdict that plants is proved in the proof that plants.
/// Without a slice, or while a scan runs, the verdict is "not yet".
pub(super) fn planting_verdict(
    state: &TickState,
    record: &SettlementRecord,
    crown: ScriptWorldBounds,
    budget: &mut construction::site_turn::SiteBudget,
) -> construction::Replant {
    let lots = budget.take();
    if lots == 0 {
        return construction::Replant::NotYet;
    }
    crown_verdict(state, record, crown, lots, budget)
}

fn crown_verdict(
    state: &TickState,
    record: &SettlementRecord,
    crown: ScriptWorldBounds,
    lots: u32,
    budget: &mut construction::site_turn::SiteBudget,
) -> construction::Replant {
    use construction::{HouseLot, Replant};
    let misses = forest_misses(state);
    let crowned = Network::build(state, record, true, Some(crown));
    let mut proof = CrownProof {
        crown,
        plain: None,
        fronts: crowned.frontage(),
        crowned,
        checks: construction::LotChecks::new(lots),
    };
    if record.buildings.values().any(|building| {
        !building.natural_zone
            && proof.crowned.access(building).is_none()
            && proof
                .plain
                .get_or_insert_with(|| Network::for_construction(state, record))
                .access(building)
                .is_some()
    }) {
        return Replant::Refused;
    }
    let scans: Vec<&'static Blueprint> = blueprints::all(&state.registry)
        .iter()
        .filter(|blueprint| blueprint.kind == BuildingKind::House)
        .filter(|&blueprint| !spares_housing(state, record, &mut proof, blueprint, budget, misses))
        .collect();
    let mut verdict = Replant::Plant;
    let mut waiting = u32::try_from(scans.len()).unwrap_or(u32::MAX);
    for blueprint in scans {
        // Each scan takes a fair share of what the slice has left, so one
        // long scan cannot starve the next blueprint's scan.
        let share = proof.checks.lots / waiting;
        let rest = proof.checks.lots - share;
        proof.checks.lots = share;
        waiting -= 1;
        let (hint, progress) = budget.house(record.id, &blueprint.id, crown.min, state.tick);
        let found = construction::crowned_house_lot(
            state,
            &mut proof.crowned,
            &proof.fronts,
            blueprint,
            progress,
            &mut proof.checks,
        );
        proof.checks.lots += rest;
        match found {
            Some(HouseLot::Free(front)) => *hint = Some(front),
            Some(HouseLot::None) => return Replant::Refused,
            None => verdict = Replant::NotYet,
        }
    }
    // A proved remaining lot and its loaded access are enough. Unrelated
    // cache misses elsewhere must not discard this grove's renewal.
    verdict
}

fn forest_misses(state: &TickState) -> u64 {
    state
        .settlements
        .forest
        .lock()
        .expect("forest cache poisoned")
        .misses()
}

/// A crown proof: the crown, the crowned walk and its frontages, the plain
/// walk (built only when needed) and the proof's lot checks.
struct CrownProof<'a> {
    crown: ScriptWorldBounds,
    plain: Option<Network<'a>>,
    crowned: Network<'a>,
    fronts: Vec<construction::Front>,
    checks: construction::LotChecks,
}

/// Whether the crown spares `blueprint`'s housing without a scan: the hint
/// lot is still free on the crowned walk, or the crown takes no lot of the
/// blueprint that is free now (proved with forest evidence at hand). A crown
/// whose scan for another free lot runs already took a free lot.
fn spares_housing<'a>(
    state: &'a TickState,
    record: &'a SettlementRecord,
    proof: &mut CrownProof<'a>,
    blueprint: &'static Blueprint,
    budget: &mut construction::site_turn::SiteBudget,
    misses: u64,
) -> bool {
    let (hint, progress) = budget.house(record.id, &blueprint.id, proof.crown.min, state.tick);
    if let Some(front) = *hint {
        match construction::keeps_free_lot(
            state,
            &mut proof.crowned,
            &proof.fronts,
            blueprint,
            front,
            &mut proof.checks,
        ) {
            Some(true) => return true,
            Some(false) => *hint = None,
            None => return false,
        }
    }
    if !progress.idle() {
        return false;
    }
    let plain = proof
        .plain
        .get_or_insert_with(|| Network::for_construction(state, record));
    construction::takes_free_lot(
        state,
        plain,
        &proof.fronts,
        proof.crown,
        blueprint,
        &mut proof.checks,
    ) == Some(false)
        && forest_misses(state) == misses
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

/// The state street paving turns soil into: the one `dirt_path` block.
pub(crate) fn paved_state(state: &TickState) -> Option<u32> {
    default_state(state, "minecraft:dirt_path")
}

fn default_state(state: &TickState, name: &str) -> Option<u32> {
    let id = mc_data::Identifier::parse(name).ok()?;
    Some(state.registry.blocks.block(&id)?.default.0)
}

fn paid(state: &TickState, at: [i32; 3], block: u32) -> Option<PlannedCell> {
    let (item, quantity) =
        blueprints::material_for(&state.registry, mc_world::BlockStateId(block))?;
    Some(PlannedCell {
        portion: crate::plugin_structures::settlement_builds::PortionKind::Build,
        at,
        state: block,
        resource: Some(state.registry.items.name_of(item)?.to_string()),
        quantity,
    })
}

fn free(at: [i32; 3], block: u32) -> PlannedCell {
    PlannedCell {
        portion: crate::plugin_structures::settlement_builds::PortionKind::Build,
        at,
        state: block,
        resource: None,
        quantity: 0,
    }
}

fn cleared(at: [i32; 3], air: u32) -> PlannedCell {
    let mut cell = free(at, air);
    cell.portion = crate::plugin_structures::settlement_builds::PortionKind::Cut;
    cell
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
pub(super) fn front(blueprint: &Blueprint) -> Option<usize> {
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

/// The geometry of the blueprint's lot facing a frontage across the verge,
/// before any terrain is read.
struct LotPlan {
    rotation: u8,
    /// Minimum x and z of the footprint.
    origin: [i32; 2],
    /// The rotated footprint.
    size: [i32; 3],
    verge: [i32; 2],
    door: [i32; 2],
}

fn lot_plan(blueprint: &Blueprint, front: [i32; 3], normal: [i32; 2]) -> Option<LotPlan> {
    let verge = [front[0] + normal[0], front[2] + normal[1]];
    let door = [front[0] + 2 * normal[0], front[2] + 2 * normal[1]];
    let rotation = facing(blueprint, normal)?;
    let turn = Rotation::from_quarter_turns(rotation);
    let entrance = blueprint.level(1)?.rotated(turn).entrance.map(i32::from);
    let size = blueprint.footprint(turn).map(i32::from);
    let [sx, _, sz] = size;
    let origin = match normal {
        [0, -1] => [door[0] - entrance[0], door[1] - (sz - 1)],
        [0, _] => [door[0] - entrance[0], door[1]],
        [-1, _] => [door[0] - (sx - 1), door[1] - entrance[2]],
        _ => [door[0], door[1] - entrance[2]],
    };
    Some(LotPlan {
        rotation,
        origin,
        size,
        verge,
        door,
    })
}

/// Whether `crown` covers a column the blueprint's lot facing `front` reads
/// (its footprint, door or verge): only such a lot can differ between a walk
/// and the same walk with the crown planted ([`Network::lot`]).
pub(crate) fn crown_touches_lot(
    blueprint: &Blueprint,
    front: [i32; 3],
    normal: [i32; 2],
    crown: ScriptWorldBounds,
) -> bool {
    let Some(LotPlan {
        origin: [ox, oz],
        size: [sx, _, sz],
        verge,
        door,
        ..
    }) = lot_plan(blueprint, front, normal)
    else {
        return false;
    };
    let covers = |[x, z]: [i32; 2]| {
        (crown.min[0]..=crown.max[0]).contains(&x) && (crown.min[2]..=crown.max[2]).contains(&z)
    };
    covers(verge)
        || covers(door)
        || (crown.min[0] < ox + sx
            && ox <= crown.max[0]
            && crown.min[2] < oz + sz
            && oz <= crown.max[2])
}

/// Street columns reachable on foot from the bell, including verge detours.
pub(crate) struct Network<'a> {
    state: &'a TickState,
    record: &'a SettlementRecord,
    columns: HashMap<(i32, i32), Column>,
    cells: Vec<[i32; 3]>,
    parents: Vec<usize>,
    index: HashMap<(i32, i32), usize>,
    prepare_forest: bool,
    planting: Option<ScriptWorldBounds>,
}

impl<'a> Network<'a> {
    /// Breadth-first walk over roads, the plaza and their walkable verges:
    /// steps of at most one block, inside the territory, on loaded ground.
    /// An existing village obstacle may block the road without its shoulder.
    pub(crate) fn walk(state: &'a TickState, record: &'a SettlementRecord) -> Self {
        Self::build(state, record, false, None)
    }

    /// Forecast only: every obstructing natural tree must become paid-project
    /// removal work before this corridor is actually walkable.
    pub(crate) fn for_construction(state: &'a TickState, record: &'a SettlementRecord) -> Self {
        Self::build(state, record, true, None)
    }

    /// The settlement this walk belongs to.
    pub(crate) fn record(&self) -> &'a SettlementRecord {
        self.record
    }

    fn build(
        state: &'a TickState,
        record: &'a SettlementRecord,
        prepare_forest: bool,
        planting: Option<ScriptWorldBounds>,
    ) -> Self {
        let mut network = Self {
            state,
            record,
            prepare_forest,
            planting,
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
                        && let Column::Ground { y, .. } = network.column(ax + dx, az + dz, None)
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
                    || !matches!(
                        cell(record.anchor, nx, nz),
                        Cell::Road | Cell::Plaza | Cell::Verge
                    )
                    || !level_inside(record, nx, nz)
                {
                    continue;
                }
                if let Column::Ground { y: ny, .. } = network.column(nx, nz, Some(y))
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

    fn column(&mut self, x: i32, z: i32, hint: Option<i32>) -> Column {
        let (state, record, prepare_forest, planting) =
            (self.state, self.record, self.prepare_forest, self.planting);
        *self.columns.entry((x, z)).or_insert_with(|| {
            let column = sample_near(state, record, x, z, prepare_forest, hint);
            if let Column::Ground { y, .. } = column
                && planting.is_some_and(|crown| {
                    (crown.min[0]..=crown.max[0]).contains(&x)
                        && (crown.min[2]..=crown.max[2]).contains(&z)
                        && y <= crown.max[1]
                        && y + 2 >= crown.min[1]
                })
            {
                Column::Occupied
            } else {
                column
            }
        })
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
        let LotPlan {
            rotation,
            origin: [ox, oz],
            size: [sx, sy, sz],
            verge,
            door,
        } = lot_plan(blueprint, front, normal)?;
        if !(ox..ox + sx).all(|x| (oz..oz + sz).all(|z| cell(anchor, x, z) == Cell::Lot)) {
            return None;
        }
        let Column::Ground { y: verge_y, .. } = self.column(verge[0], verge[1], Some(front[1]))
        else {
            return None;
        };
        let Column::Ground { y: floor, .. } = self.column(door[0], door[1], Some(verge_y)) else {
            return None;
        };
        if self.planting.is_some_and(|crown| {
            let min = [ox, floor, oz];
            let max = [ox + sx - 1, floor + sy - 1, oz + sz - 1];
            (0..3).all(|axis| crown.min[axis] <= max[axis] && min[axis] <= crown.max[axis])
        }) {
            return None;
        }
        // Owner 2026-11-17: inside the settlement the one-block step limit is
        // lifted; what stays is what a villager walks from the road across the
        // verge to the door (up one, down three), so the door stays reachable.
        let walkable = |from: i32, to: i32| to - from <= STEP_UP && from - to <= STEP_DOWN;
        (walkable(front[1], verge_y) && walkable(verge_y, floor)).then_some(Lot {
            origin: [ox, floor, oz],
            rotation,
        })
    }

    /// The funded building's future walking corridor, plaza first. Trees on
    /// this route become ordinary executor removal cells, never admission edits.
    pub(crate) fn construction_access(&self, project: &Project) -> Option<Vec<[i32; 3]>> {
        let blueprint = blueprints::get(&self.state.registry, &project.blueprint)?;
        let turn = Rotation::from_quarter_turns(project.rotation);
        let [ex, _, ez] = blueprint.level(1)?.rotated(turn).entrance.map(i32::from);
        let [nx, nz] = COMPASS[(front(blueprint)? + usize::from(project.rotation)) % 4];
        let (x, z) = (project.origin[0] + ex, project.origin[2] + ez);
        let mut at = (1..=LOT_FRONT + LOT_DEPTH)
            .find_map(|step| self.street_index(x + step * nx, z + step * nz))?;
        let mut route = Vec::new();
        loop {
            route.push(self.cells[at]);
            if self.parents[at] == SEED {
                break;
            }
            at = self.parents[at];
        }
        route.reverse();
        // Owner D7: the forecast walks through a registered planted tree on the
        // door step as it does on the street (see `sample`).
        let mut clearance = if self.prepare_forest {
            super::forestry::Clearance::street(self.record, true)
        } else {
            super::forestry::Clearance::prepared()
        };
        for [cx, cz] in [[x + nx, z + nz], [x, z]] {
            let y = construction::surface_y_for_lot(
                self.state,
                self.record.anchor[1],
                cx,
                cz,
                None,
                &mut clearance,
            )?;
            route.push([cx, y, cz]);
        }
        Some(route)
    }

    /// The reached street column behind `building`'s door, exactly the column
    /// [`Self::access`] indexes: the road graph a route must keep connected.
    pub(crate) fn access_column(&self, building: &Building) -> Option<[i32; 3]> {
        self.access(building).map(|index| self.cells[index])
    }

    /// A verge may carry a detour, but a door still fronts an actual street.
    fn street_index(&self, x: i32, z: i32) -> Option<usize> {
        matches!(cell(self.record.anchor, x, z), Cell::Road | Cell::Plaza)
            .then(|| self.index.get(&(x, z)).copied())
            .flatten()
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
                .find_map(|step| self.street_index(x + step * nx, z + step * nz));
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
                        if let Some(index) = self.street_index(x + step * nx, z + step * nz) {
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
        let path = paved_state(state)?;
        let mut cells = Vec::new();
        let (mut low, mut high) = (i32::MAX, i32::MIN);
        for x in tile.min[0]..=tile.max[0] {
            for z in tile.min[1]..=tile.max[1] {
                let Some(&index) = self.index.get(&(x, z)) else {
                    continue;
                };
                let y = self.cells[index][1];
                let Column::Ground { soil, .. } = self.column(x, z, None) else {
                    continue;
                };
                low = low.min(y);
                high = high.max(y + 2);
                for above in [y + 1, y + 2] {
                    if let Some(block) = state.world.physics_block_at(x, above, z)
                        && !vacant(state, block)
                        && construction::clearable(state, block)
                    {
                        cells.push(cleared([x, above, z], air));
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
    /// column. A ring road's lamp column on the wall line has no post. Species
    /// and funding are the construction planner's choice.
    pub(crate) fn lamp_work(&mut self, tile: &Tile) -> Option<Work> {
        let (state, record) = (self.state, self.record);
        let [x, z] = tile.lamp;
        let stand = stand_column(record.anchor, x, z);
        if !lamp_at(record.anchor, x, z)
            || !self.index.contains_key(&(stand[0], stand[1]))
            || !level_inside(record, x, z)
            || lamp_standing(state, record, x, z)
        {
            return None;
        }
        let Column::Ground { y, .. } = self.column(x, z, None) else {
            return None;
        };
        let top = state.world.physics_block_at(x, y + 3, z)?;
        if !construction::clearable(state, top) || !inside(record, [x, y + 3, z]) {
            return None;
        }
        let fence = default_state(state, "minecraft:oak_fence")?;
        let torch = state.registry.states.torch;
        let air = state.registry.states.air;
        let mut cells = Vec::new();
        for above in y + 1..=y + 3 {
            let block = state.world.physics_block_at(x, above, z)?;
            if !vacant(state, block) {
                cells.push(cleared([x, above, z], air));
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

/// Where a builder stands for the next street or fortification cell: on the
/// road column itself while paving, on the road beside a lamp post's column,
/// inside the wall beside a wall or gate column, and on the road beside the
/// well's rim.
///
/// A street plan follows the terrain, so a road or lamp on a slope lies well
/// below the bell's own level: its stand is searched around the cell's level.
/// Searching around the anchor instead left every street cell more than
/// `SITE_BAND` below the bell without a stand (seed 24, 2026-11-23: cells 9
/// blocks below the anchor, `worksite_unreached` with `navigation=None` and the
/// road and lamp at 0/N for the whole 96 000-tick window). The perimeter, a
/// gatehouse and the well are planned on the bell's level and keep the anchor.
#[must_use]
pub(crate) fn stand(state: &TickState, project: &Project, next: [i32; 3]) -> Option<[i32; 3]> {
    let record = state.settlements.get(project.settlement)?;
    let centre = if street(project.blueprint.as_str()) {
        next[1]
    } else {
        record.anchor[1]
    };
    stand_columns(record.anchor, project.blueprint.as_str(), next[0], next[2])
        .into_iter()
        .find_map(|[x, z]| {
            let y = construction::surface_y(state, centre, x, z)?;
            Some([x, y + 1, z])
        })
}

/// Street work: pavement, a lamp post and a meadow clearing. These follow the
/// terrain and raise no authored body above their floor.
#[must_use]
pub(crate) fn street(blueprint: &str) -> bool {
    matches!(blueprint, ROAD | LAMP | MEADOW)
}

/// Columns a builder may stand on to place `(x, z)` of a `blueprint` project,
/// in preference order: the road beside a lamp post's column, inside the wall
/// beside a wall or gate column, the road beside the well's rim, or the column
/// itself.
fn stand_columns(anchor: [i32; 3], blueprint: &str, x: i32, z: i32) -> Vec<[i32; 2]> {
    match blueprint {
        LAMP => vec![stand_column(anchor, x, z)],
        PERIMETER | GATE => (1..=2).map(|step| inward(anchor, x, z, step)).collect(),
        WELL => vec![[anchor[0] + (x - anchor[0]).signum(), z]],
        _ => vec![[x, z]],
    }
}

/// Wall columns are cobblestone, this many blocks tall.
const WALL_HEIGHT: usize = 3;
/// A gatehouse stands one block above the wall it pierces.
const GATE_HEIGHT: usize = 4;
/// Half width of a gatehouse: the road's three columns, a pillar column each side.
const GATE_HALF: i32 = 3;
/// Longest wall stretch: one project, one Builder visit.
const STRETCH: i32 = 11;

/// One project of the fortification plan: a wall stretch, a gatehouse or the
/// well, as an inclusive `[x, z]` extent in world columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Span {
    pub(crate) kind: &'static str,
    pub(crate) min: [i32; 2],
    pub(crate) max: [i32; 2],
}

/// The perimeter: the square wall line [`RING`] blocks from the bell, split
/// into projects, with a gatehouse where each of the four roads through the bell
/// crosses it. Sides run north, east, south, west; each is its gatehouse, then
/// the wall outward from it in stretches of at most [`STRETCH`] columns,
/// alternating sides of the gatehouse. Other roads crossing the line get plain
/// wall; the ring road inside connects them. The north and south sides own the
/// corner columns. A pure function of the anchor.
#[must_use]
pub(crate) fn perimeter_spans(anchor: [i32; 3]) -> Vec<Span> {
    let mut spans = Vec::new();
    for (fixed, side) in [(1_usize, -1), (0, 1), (1, 1), (0, -1)] {
        let along = 1 - fixed;
        let centre = [anchor[0], anchor[2]];
        let reach = if fixed == 1 { RING } else { RING - 1 };
        let at = |offset: i32| {
            let mut column = [0; 2];
            column[fixed] = centre[fixed] + side * RING;
            column[along] = centre[along] + offset;
            column
        };
        spans.push(Span {
            kind: GATE,
            min: at(-GATE_HALF),
            max: at(GATE_HALF),
        });
        let mut from = GATE_HALF + 1;
        while from <= reach {
            let to = (from + STRETCH - 1).min(reach);
            spans.push(Span {
                kind: PERIMETER,
                min: at(from),
                max: at(to),
            });
            spans.push(Span {
                kind: PERIMETER,
                min: at(-to),
                max: at(-from),
            });
            from = to + 1;
        }
    }
    spans
}

/// The three plaza corners a well may stand on, each a 3 x 3 square between
/// the plaza's two road bands. The fourth holds the crossing's corner lamp.
fn well_spans(anchor: [i32; 3]) -> [Span; 3] {
    let range = |side: i32, at: i32| {
        if side < 0 {
            [at - 4, at - 2]
        } else {
            [at + 2, at + 4]
        }
    };
    [(-1, -1), (1, -1), (-1, 1)].map(|(sx, sz)| {
        let [x0, x1] = range(sx, anchor[0]);
        let [z0, z1] = range(sz, anchor[2]);
        Span {
            kind: WELL,
            min: [x0, z0],
            max: [x1, z1],
        }
    })
}

/// `(x, z)` moved `step` columns toward the bell off the wall line.
fn inward(anchor: [i32; 3], x: i32, z: i32, step: i32) -> [i32; 2] {
    let pull = |value: i32, at: i32| {
        let offset = value - at;
        if offset.abs() == RING {
            value - step * offset.signum()
        } else {
            value
        }
    };
    [pull(x, anchor[0]), pull(z, anchor[2])]
}

fn built_over(record: &SettlementRecord, x: i32, z: i32) -> bool {
    record.buildings.values().any(|building| {
        !building.natural_zone
            && (building.bounds.min[0]..=building.bounds.max[0]).contains(&x)
            && (building.bounds.min[2]..=building.bounds.max[2]).contains(&z)
    })
}

fn block_path(state: &TickState, block: u32) -> Option<&str> {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .map(|definition| definition.block.id.path())
}

fn is_door(state: &TickState, block: u32) -> bool {
    block_path(state, block).is_some_and(|path| path.ends_with("_door"))
}

/// An oak door half in the wall line, facing out of the settlement.
fn door(state: &TickState, facing: &str, upper: bool) -> Option<u32> {
    let name = mc_data::Identifier::parse("minecraft:oak_door").ok()?;
    let default = state.registry.blocks.block(&name)?.default;
    let mut properties = state.registry.blocks.by_id(default)?.properties.clone();
    for (key, value) in [
        ("facing", facing),
        ("half", if upper { "upper" } else { "lower" }),
        ("hinge", "left"),
        ("open", "false"),
        ("powered", "false"),
    ] {
        if let Some(entry) = properties.iter_mut().find(|(known, _)| known == key) {
            entry.1 = value.to_owned();
        }
    }
    state
        .registry
        .blocks
        .by_name_and_props(&name, &properties)
        .map(|id| id.0)
}

/// A cell placed from stock, or free when the block has no item cost.
fn placed(state: &TickState, at: [i32; 3], block: u32) -> PlannedCell {
    paid(state, at, block).unwrap_or_else(|| free(at, block))
}

fn work_of(cells: Vec<PlannedCell>) -> Option<Work> {
    let first = cells.first()?.at;
    let (mut min, mut max) = (first, first);
    for cell in &cells {
        for axis in 0..3 {
            min[axis] = min[axis].min(cell.at[axis]);
            max[axis] = max[axis].max(cell.at[axis]);
        }
    }
    Some(Work {
        bounds: ScriptWorldBounds::new(min, max).ok()?,
        cells,
    })
}

/// What one wall column still owes to hold `profile`, bottom up from the
/// ground. `None`: the column is not loaded, so nothing is known about it. A
/// column the wall cannot stand on (water, a cliff, a tree, a building, ground
/// outside the territory) owes nothing: terrain-blocked stays blocked.
/// Cobblestone and doors already stacked on the ground count as built, so the
/// plan is read from the world and never stored.
fn column_cells(
    state: &TickState,
    record: &SettlementRecord,
    [x, z]: [i32; 2],
    cobble: u32,
    profile: &[u32],
) -> Option<Vec<PlannedCell>> {
    let anchor_y = record.anchor[1];
    state.world.physics_block_at(x, anchor_y, z)?;
    let nothing = Some(Vec::new());
    if !level_inside(record, x, z) || built_over(record, x, z) {
        return nothing;
    }
    let Some(top) = construction::surface_y(state, anchor_y, x, z) else {
        return nothing;
    };
    let mut ground = top;
    while top - ground < profile.len() as i32 {
        let block = state.world.physics_block_at(x, ground, z)?;
        if block != cobble && !is_door(state, block) {
            break;
        }
        ground -= 1;
    }
    let air = state.registry.states.air;
    let mut cells = Vec::new();
    for (index, &want) in profile.iter().enumerate() {
        let y = ground + 1 + index as i32;
        let have = state.world.physics_block_at(x, y, z)?;
        if have == want || (is_door(state, have) && is_door(state, want)) {
            continue;
        }
        if state.block_entities.contains_key(&(x, y, z)) {
            return nothing;
        }
        if !vacant(state, have) {
            if !construction::clearable(state, have) {
                return nothing;
            }
            cells.push(cleared([x, y, z], air));
        }
        cells.push(placed(state, [x, y, z], want));
    }
    Some(cells)
}

/// The cells one wall stretch or gatehouse still owes: `Some(None)` when it
/// stands, `None` when part of it is not loaded or when a column it still owes
/// has no builder stand beside it (such a stretch is not planned at all).
fn span_work(state: &TickState, record: &SettlementRecord, span: Span) -> Option<Option<Work>> {
    let anchor = record.anchor;
    let cobble = state.registry.states.cobblestone;
    let x_fixed = span.min[0] == span.max[0];
    let facing = match (x_fixed, span.min[0] > anchor[0], span.min[1] > anchor[2]) {
        (true, true, _) => "east",
        (true, false, _) => "west",
        (false, _, true) => "south",
        (false, _, false) => "north",
    };
    let mut cells = Vec::new();
    for x in span.min[0]..=span.max[0] {
        for z in span.min[1]..=span.max[1] {
            let across = if x_fixed {
                z - anchor[2]
            } else {
                x - anchor[0]
            };
            let profile: Vec<u32> = if span.kind != GATE {
                vec![cobble; WALL_HEIGHT]
            } else if across == 0 {
                vec![
                    door(state, facing, false)?,
                    door(state, facing, true)?,
                    cobble,
                    cobble,
                ]
            } else {
                vec![cobble; GATE_HEIGHT]
            };
            let owed = column_cells(state, record, [x, z], cobble, &profile)?;
            // A column with no builder stand beside it would prepare and then
            // stall the whole stretch until the next cancellation admits it
            // again, so a stretch that owes work on such a column is not
            // planned at all.
            if !owed.is_empty()
                && stand_columns(anchor, span.kind, x, z)
                    .into_iter()
                    .all(|[sx, sz]| construction::surface_y(state, anchor[1], sx, sz).is_none())
            {
                return None;
            }
            cells.extend(owed);
        }
    }
    Some(work_of(cells))
}

/// The cells one plaza corner still owes for the well: a cobblestone rim one
/// block high around a single water source set into the ground. `Some(None)`
/// when the well stands; `None` when the corner is not loaded or cannot hold one.
fn well_cells(state: &TickState, record: &SettlementRecord, span: Span) -> Option<Option<Work>> {
    let anchor_y = record.anchor[1];
    let [cx, cz] = [span.min[0] + 1, span.min[1] + 1];
    let rim: Vec<[i32; 2]> = (span.min[0]..=span.max[0])
        .flat_map(|x| (span.min[1]..=span.max[1]).map(move |z| [x, z]))
        .filter(|&column| column != [cx, cz])
        .collect();
    state.world.physics_block_at(cx, anchor_y, cz)?;
    let cobble = state.registry.states.cobblestone;
    let water = state.registry.states.water;
    let band = construction::SITE_BAND;
    let source = (anchor_y - band..=anchor_y + band)
        .find(|&y| state.world.physics_block_at(cx, y, cz) == Some(water));
    if let Some(y) = source
        && rim
            .iter()
            .all(|&[x, z]| state.world.physics_block_at(x, y + 1, z) == Some(cobble))
    {
        return Some(None);
    }
    let centre = construction::surface_y(state, anchor_y, cx, cz)?;
    let air = state.registry.states.air;
    let mut cells = Vec::new();
    for [x, z] in span_columns(span) {
        if !level_inside(record, x, z)
            || built_over(record, x, z)
            || state.block_entities.contains_key(&(x, centre, z))
        {
            return None;
        }
    }
    for [x, z] in rim {
        let top = construction::surface_y(state, anchor_y, x, z)?;
        let ground = if state.world.physics_block_at(x, top, z) == Some(cobble) {
            top - 1
        } else {
            top
        };
        if (ground - centre).abs() > 1 {
            return None;
        }
        let have = state.world.physics_block_at(x, ground + 1, z)?;
        if have == cobble {
            continue;
        }
        if !vacant(state, have) {
            if !construction::clearable(state, have) {
                return None;
            }
            cells.push(cleared([x, ground + 1, z], air));
        }
        cells.push(placed(state, [x, ground + 1, z], cobble));
    }
    let above = state.world.physics_block_at(cx, centre + 1, cz)?;
    if !vacant(state, above) {
        if !construction::clearable(state, above) {
            return None;
        }
        cells.push(cleared([cx, centre + 1, cz], air));
    }
    cells.push(free([cx, centre, cz], water));
    Some(work_of(cells))
}

fn span_columns(span: Span) -> impl Iterator<Item = [i32; 2]> {
    (span.min[0]..=span.max[0]).flat_map(move |x| (span.min[1]..=span.max[1]).map(move |z| [x, z]))
}

/// The well still owed: none once any plaza corner holds one, else the corner
/// closest to done, the first by compass order on a tie.
fn well_work(state: &TickState, record: &SettlementRecord) -> Option<Work> {
    let mut best: Option<Work> = None;
    for span in well_spans(record.anchor) {
        match well_cells(state, record, span) {
            Some(None) => return None,
            Some(Some(work))
                if best
                    .as_ref()
                    .is_none_or(|known| work.cells.len() < known.cells.len()) =>
            {
                best = Some(work);
            }
            Some(Some(_)) | None => {}
        }
    }
    best
}

/// Fortification work the settlement still owes, in building order: the well,
/// then each side's gatehouse and wall. Lazy: a pass looks only as far as the
/// first project it can fund. Read-only; unloaded stretches are skipped.
pub(crate) fn fortification<'a>(
    state: &'a TickState,
    record: &'a SettlementRecord,
) -> impl Iterator<Item = (&'static str, Work)> + 'a {
    well_work(state, record)
        .map(|work| (WELL, work))
        .into_iter()
        .chain(
            perimeter_spans(record.anchor)
                .into_iter()
                .filter_map(move |span| {
                    span_work(state, record, span)
                        .flatten()
                        .map(|work| (span.kind, work))
                }),
        )
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
            match network.column(x, z, None) {
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
