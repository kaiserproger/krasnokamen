//! Native settlement production: one honest job per living member body.
//!
//! Admission checks the live town owner, the live district certificate (whose
//! generic owner may differ from the town owner), the live workstation block
//! and the startup-native [`WorkRecipe`](mc_data::recipes::work::WorkRecipe).
//! Progress is delayed and causal: at most one due portion per job per tick,
//! applied atomically through the parent inventory bridge. Bodies walk via
//! [`target`]; this module never teleports and never fabricates catch-up.

use std::collections::BTreeMap;

use mc_data::recipes::work::{WorkRecipe, WorkRecipeAlternative};
use mc_data::{Identifier, ItemStack};
use mc_script::{
    ScriptOperationFailure as Failure, ScriptSettlementBuildingRef, ScriptWorldBounds,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::{SettlementId, inventory};
use crate::boundary::BlockPos;
use crate::tick::TickState;

/// Manual-hold reason. Auto pauses use native reasons below; only this exact
/// value makes [`target`] hide the job and [`active`] skip it.
const MANUAL: &str = "paused";
/// Retry delay for paused jobs: causal, never backfills missed portions.
const RETRY_TICKS: u64 = 20;
/// Largest autocraft chain one order may admit.
pub(super) const MAX_ORDER_STEPS: usize = 32;

#[path = "production_delivery.rs"]
mod delivery;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ToolWear {
    pub item: String,
    pub wear: u32,
    pub broke: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Receipt {
    pub job: String,
    pub sequence: u32,
    pub recipe: String,
    pub recipe_revision: String,
    pub inputs: Vec<ItemStack>,
    pub outputs: Vec<ItemStack>,
    pub tool_wear: Vec<ToolWear>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Job {
    pub id: String,
    pub owner: String,
    pub worker: Uuid,
    pub recipe: String,
    pub recipe_revision: String,
    pub duration: u64,
    pub count: u32,
    pub done: u32,
    /// Only standalone/final order jobs owe authored output to real storage.
    /// None means this historical/intermediate job never tracked delivery;
    /// never infer delivery of old carry merely from a recipe receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<delivery::Delivery>,
    pub station: [i32; 3],
    pub stand_at: [i32; 3],
    pub building: ScriptSettlementBuildingRef,
    pub dimension: String,
    pub accepted_tick: u64,
    pub next_due: u64,
    pub paused: Option<String>,
    pub cancelled: bool,
    pub receipts: Vec<Receipt>,
    /// Autocraft order identity: the final step's job id, shared by every step.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub order: Option<String>,
    /// Previous order step. While set this step neither ticks nor routes;
    /// the tick that completes the previous step clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// Derived navigation only; stock and job progress remain the authorities.
    #[serde(skip)]
    fetch_at: Option<[i32; 3]>,
}

impl Job {
    pub(super) fn delivery_pending(&self) -> bool {
        self.delivery
            .as_ref()
            .is_some_and(delivery::Delivery::pending)
    }

    pub(super) fn unfinished(&self) -> bool {
        !self.cancelled && (self.done < self.count || self.delivery_pending())
    }

    pub(crate) fn delivery_complete(&self) -> bool {
        !self.cancelled
            && self.done == self.count
            && self
                .delivery
                .as_ref()
                .is_some_and(|delivery| !delivery.invalidated && !delivery.pending())
    }
}

/// One planned autocraft step: a recipe run `count` times at a live station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Step {
    pub recipe: String,
    pub count: u32,
    pub station: [i32; 3],
    pub stand_at: [i32; 3],
    pub building: ScriptSettlementBuildingRef,
}

/// Policy only: named job map plus admission/completion counters for the
/// parent WAL. No inventory mirrors, no JSON fragments.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub jobs: BTreeMap<String, Job>,
    pub sequence: u64,
    pub completed: u64,
    pub cancelled: u64,
    #[serde(skip)]
    cursor: super::economy::WorkCursor,
}

/// Private admission parameters mirroring the flat `Request::Start` fields.
struct Start {
    job: String,
    worker: Uuid,
    recipe_name: String,
    count: u32,
    station: [i32; 3],
    stand_at: [i32; 3],
    building: ScriptSettlementBuildingRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Request {
    Start {
        job: String,
        worker: Uuid,
        recipe: String,
        count: u32,
        station: [i32; 3],
        stand_at: [i32; 3],
        building: ScriptSettlementBuildingRef,
    },
    Pause {
        job: String,
    },
    Resume {
        job: String,
    },
    Cancel {
        job: String,
    },
    /// Planned dependency chain, executed in order by one worker. The final
    /// step's id is `order`; earlier steps are `order-1`, `order-2`, ...
    Order {
        order: String,
        worker: Uuid,
        steps: Vec<Step>,
    },
}

pub(super) fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

pub(super) fn recipe_of<'a>(world: &'a TickState, name: &str) -> Result<&'a WorkRecipe, Failure> {
    let id = Identifier::parse(name).map_err(|_| Failure::NotFound)?;
    world
        .registry
        .item_facts
        .work_recipe(&id)
        .ok_or(Failure::NotFound)
}

/// One atomic portion plan against worker carry + equipment copies.
struct PortionPlan {
    carry_before: inventory::Image,
    equipment_before: inventory::Image,
    inputs: Vec<ItemStack>,
    outputs: Vec<ItemStack>,
    wear: Vec<ToolWear>,
    carry_after: Vec<ItemStack>,
    equipment_after: Vec<ItemStack>,
}

/// Live worker membership: exact native body, alive, bound to this town.
pub(super) fn worker_slot(
    world: &TickState,
    settlement: SettlementId,
    worker: Uuid,
) -> Result<(), Failure> {
    let slot = world
        .entities
        .slot_of_uuid(worker)
        .ok_or(Failure::NotFound)?;
    let row = world.entities.row(slot);
    if row.villager.as_ref().is_some_and(|v| {
        matches!(
            v.data.profession,
            crate::villager::VillagerProfession::Builder
                | crate::villager::VillagerProfession::Soldier
        ) || v.soldier.is_some()
    }) {
        return Err(Failure::Busy);
    }
    if super::economy::caravan_reserved(world, Some(settlement), worker) {
        return Err(Failure::Busy);
    }
    if !world.entities.body(slot).alive {
        return Err(Failure::NotFound);
    }
    match row.villager.as_ref().map(|v| v.settlement) {
        Some(Some(id)) if id == settlement => Ok(()),
        Some(_) => Err(Failure::Forbidden),
        None => Err(Failure::NotFound),
    }
}

fn worker_pos(world: &TickState, worker: Uuid) -> Option<[f64; 3]> {
    let slot = world.entities.slot_of_uuid(worker)?;
    world.entities.row(slot).villager.as_ref()?;
    Some(world.entities.pos(slot))
}

/// Player-built certificate (`kind == "building"`): the live record's
/// production building (`Workshop`/`Mine`/`LumberCamp`) bounds are the
/// footprint; the station is checked against the live block by [`station_live`].
fn native_building_facts(
    world: &TickState,
    settlement: SettlementId,
    reference: &ScriptSettlementBuildingRef,
) -> Result<crate::plugin_structures::BuildingFacts, Failure> {
    let id = super::buildings::BuildingId(reference.id.parse().map_err(|_| Failure::NotFound)?);
    let building = world
        .settlements
        .get(settlement)
        .and_then(|record| record.buildings.get(&id))
        .ok_or(Failure::NotFound)?;
    if building.kind.profession().is_none()
        || reference.purpose.strip_prefix("solaris:") != Some(building.kind.name())
    {
        return Err(Failure::NotFound);
    }
    Ok(crate::plugin_structures::BuildingFacts {
        dimension: world.dimension.name().to_owned(),
        footprint: building.bounds,
        points: Vec::new(),
        blueprint: None,
    })
}

/// Live certificate + intact native building. A plugin-ledger certificate must
/// be in the live district plan; its generic owner (`DistrictPlan.owner`)
/// queries the native ledger, never the town owner.
fn building_facts(
    world: &mut TickState,
    settlement: SettlementId,
    reference: &ScriptSettlementBuildingRef,
) -> Result<crate::plugin_structures::BuildingFacts, Failure> {
    reference.validate().map_err(|_| Failure::InvalidRequest)?;
    if reference.kind == "building" {
        return native_building_facts(world, settlement, reference);
    }
    let record = world.settlements.get(settlement).ok_or(Failure::NotFound)?;
    let district = record.district.as_ref().ok_or(Failure::NotFound)?;
    if !district.buildings.iter().any(|bound| {
        bound.kind == reference.kind
            && bound.id == reference.id
            && bound.purpose == reference.purpose
    }) {
        return Err(Failure::NotFound);
    }
    let mut facts = crate::plugin_structures::building_facts(
        world,
        &district.owner,
        std::iter::once((reference.id.as_str(), reference.kind == "worksite")),
    )?;
    let facts = facts.pop().ok_or(Failure::NotFound)?;
    if facts.dimension != world.dimension.name()
        || !crate::plugin_structures::check_building(world, &facts)?
    {
        return Err(Failure::Blocked);
    }
    Ok(facts)
}

fn footprint_contains(facts: &crate::plugin_structures::BuildingFacts, at: [i32; 3]) -> bool {
    (0..3)
        .all(|axis| facts.footprint.min[axis] <= at[axis] && at[axis] <= facts.footprint.max[axis])
}

/// The requested station must be live recipe station inside the certified
/// footprint; the stand post must sit next to it.
fn station_live(
    world: &TickState,
    recipe: &WorkRecipe,
    facts: &crate::plugin_structures::BuildingFacts,
    station: [i32; 3],
    stand_at: [i32; 3],
) -> Result<(), Failure> {
    if facts.dimension != world.dimension.name() {
        return Err(Failure::Unloaded);
    }
    if !footprint_contains(facts, station) {
        return Err(Failure::Blocked);
    }
    let near: i64 = station
        .into_iter()
        .zip(stand_at)
        .map(|(a, b)| (i64::from(a) - i64::from(b)).pow(2))
        .sum();
    if near > 64 {
        return Err(Failure::InvalidRequest);
    }
    station_block(world, recipe, station)
}

/// The live block at `station` is this recipe's station (a lit campfire).
pub(super) fn station_block(
    world: &TickState,
    recipe: &WorkRecipe,
    station: [i32; 3],
) -> Result<(), Failure> {
    let native = world
        .world
        .block_at(station[0], station[1], station[2])
        .ok_or(Failure::Unloaded)?;
    let block = world
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(native))
        .ok_or(Failure::Blocked)?;
    if block.block.id != recipe.station
        || (recipe.station.path() == "campfire"
            && block
                .properties
                .iter()
                .any(|(key, value)| key == "lit" && value == "false"))
    {
        return Err(Failure::Blocked);
    }
    Ok(())
}

/// A free cell beside `station` to stand on: air at feet and head, support below.
/// Cells inside the building win; order is fixed for determinism.
pub(super) fn stand_at(
    state: &TickState,
    station: [i32; 3],
    bounds: ScriptWorldBounds,
) -> Option<[i32; 3]> {
    let air = state.registry.states.air;
    let is_air = |at: [i32; 3]| state.world.block_at(at[0], at[1], at[2]) == Some(air);
    let solid_below = |at: [i32; 3]| {
        state
            .world
            .block_at(at[0], at[1] - 1, at[2])
            .is_some_and(|block| block != air)
    };
    let inside = |at: [i32; 3]| (0..3).all(|i| bounds.min[i] <= at[i] && at[i] <= bounds.max[i]);
    let mut candidates = Vec::new();
    for dz in -1..=1 {
        for dx in -1..=1 {
            if (dx, dz) != (0, 0) {
                candidates.push([station[0] + dx, station[1], station[2] + dz]);
            }
        }
    }
    let usable =
        |at: &[i32; 3]| is_air(*at) && is_air([at[0], at[1] + 1, at[2]]) && solid_below(*at);
    candidates
        .iter()
        .copied()
        .find(|at| inside(*at) && usable(at))
        .or_else(|| candidates.iter().copied().find(usable))
}

pub(super) fn resource<'a>(world: &'a TickState, stack: &ItemStack) -> Option<&'a str> {
    world
        .registry
        .items
        .name_of(stack.item_id)
        .map(Identifier::as_str)
}

pub(super) fn accepts(
    world: &TickState,
    stack: &ItemStack,
    input: &mc_data::recipes::work::WorkRecipeInput,
) -> bool {
    if stack.is_empty()
        || input
            .components
            .as_ref()
            .is_some_and(|components| !components.matches(stack))
    {
        return false;
    }
    let identity = resource(world, stack);
    let item = input
        .item
        .as_ref()
        .is_some_and(|name| identity == Some(name.as_str()));
    let tag = input.tag.as_ref().is_some_and(|name| {
        world
            .registry
            .tags
            .contains_raw_id("minecraft:item", name.as_str(), stack.item_id as i32)
    });
    item || tag
        || input
            .alternatives
            .iter()
            .any(|alternative| match alternative {
                WorkRecipeAlternative::Item(name) => identity == Some(name.as_str()),
                WorkRecipeAlternative::Tag(name) => world.registry.tags.contains_raw_id(
                    "minecraft:item",
                    name.as_str(),
                    stack.item_id as i32,
                ),
            })
}

/// Quantities already present in detached carry that the next active portion
/// will select. This is derived from the job, not persisted inventory policy.
pub(super) fn protected_carry(
    world: &TickState,
    settlement: SettlementId,
    slot: crate::entities::EntitySlot,
    carry: &[Option<ItemStack>; 8],
) -> [u32; 8] {
    let mut protected = [0_u32; 8];
    let row = world.entities.row(slot);
    let Some(job) = world
        .settlements
        .get(settlement)
        .and_then(|record| record.economy.production_job(row.uuid))
    else {
        return protected;
    };
    if let Some(delivery) = &job.delivery {
        delivery.protect(carry, &mut protected);
    }
    if job.done >= job.count || target_for_job(job).is_none() {
        return protected;
    }
    let Some(recipe) = world
        .registry
        .item_facts
        .work_recipes()
        .find(|recipe| recipe.id.as_str() == job.recipe)
    else {
        return protected;
    };
    let mut equipment_used = [0_u32; 6];
    for input in &recipe.inputs {
        let mut remaining = input.count;
        for (index, stack) in carry.iter().enumerate() {
            let Some(stack) = stack else { continue };
            if remaining == 0 {
                break;
            }
            if accepts(world, stack, input) {
                let available = (stack.count.max(0) as u32).saturating_sub(protected[index]);
                let take = remaining.min(available);
                protected[index] += take;
                remaining -= take;
            }
        }
        for (index, equipment_slot) in [0, 1, 5, 4, 3, 2].into_iter().enumerate() {
            let Some(stack) = &row.appearance.equipment[equipment_slot] else {
                continue;
            };
            if remaining == 0 {
                break;
            }
            if accepts(world, stack, input) {
                let available = (stack.count.max(0) as u32).saturating_sub(equipment_used[index]);
                let take = remaining.min(available);
                equipment_used[index] += take;
                remaining -= take;
            }
        }
    }
    protected
}

fn maximum_damage(world: &TickState, stack: &ItemStack, name: &str) -> Option<u32> {
    stack
        .extended()
        .max_damage
        .or_else(|| {
            world
                .registry
                .item_facts
                .get(world.registry.items.name_of(stack.item_id)?)
                .and_then(|facts| facts.max_damage)
        })
        .or_else(|| {
            mc_data::item_semantics_26_1_2::max_tool_damage_for_path(
                name.strip_prefix("minecraft:").unwrap_or(name),
            )
            .map(|value| value as u32)
        })
        .filter(|maximum| *maximum > 0 && *maximum <= i32::MAX as u32)
}

enum Shortage {
    Input,
    Tool,
    Storage,
}

#[allow(clippy::too_many_lines)]
fn plan_portion(
    world: &TickState,
    worker: Uuid,
    recipe: &WorkRecipe,
) -> Result<PortionPlan, Shortage> {
    let carry_image = inventory::load(world, &inventory::Endpoint::VillagerCarry { uuid: worker })
        .map_err(|_| Shortage::Input)?;
    let equipment_image = inventory::load(
        world,
        &inventory::Endpoint::VillagerEquipment { uuid: worker },
    )
    .map_err(|_| Shortage::Input)?;
    let mut carry = carry_image.slots.clone();
    let mut equipment = equipment_image.slots.clone();
    let mut inputs = Vec::new();
    for input in &recipe.inputs {
        let mut remaining = input.count;
        for slots in [&mut carry, &mut equipment] {
            for stack in slots.iter_mut() {
                if remaining == 0 {
                    break;
                }
                if !accepts(world, stack, input) {
                    continue;
                }
                let take = remaining.min(stack.count.max(0) as u32);
                let mut taken = stack.clone();
                taken.count = take as i32;
                inputs.push(taken);
                stack.count -= take as i32;
                if stack.is_empty() {
                    *stack = ItemStack::EMPTY;
                }
                remaining -= take;
            }
        }
        if remaining != 0 {
            return Err(Shortage::Input);
        }
    }
    let mut wear = Vec::new();
    for tool in &recipe.tools {
        let mut placed = None;
        for (slots, equipment_side) in [(&mut equipment, true), (&mut carry, false)] {
            if let Some(index) = slots.iter().position(|stack| {
                !stack.is_empty()
                    && resource(world, stack) == Some(tool.item.as_str())
                    && tool
                        .components
                        .as_ref()
                        .is_none_or(|components| components.matches(stack))
            }) {
                placed = Some((index, equipment_side));
                break;
            }
        }
        let Some((index, equipment_side)) = placed else {
            return Err(Shortage::Tool);
        };
        let slots = if equipment_side {
            &mut equipment
        } else {
            &mut carry
        };
        let name = resource(world, &slots[index]).ok_or(Shortage::Tool)?;
        let maximum = maximum_damage(world, &slots[index], name).ok_or(Shortage::Tool)?;
        let damage = (slots[index].damage.unwrap_or(0).max(0) as u32).saturating_add(tool.wear);
        let broke = damage >= maximum;
        if broke {
            slots[index] = ItemStack::EMPTY;
        } else {
            slots[index].damage = Some(damage as i32);
        }
        wear.push(ToolWear {
            item: name.to_owned(),
            wear: tool.wear,
            broke,
        });
    }
    let mut outputs = Vec::new();
    for output in recipe.outputs.iter().chain(&recipe.remainders) {
        outputs.push(
            output
                .to_stack(&world.registry.items, &world.registry.item_facts)
                .ok_or(Shortage::Input)?,
        );
    }
    for stack in &outputs {
        let mut rest = stack.clone();
        for slot in &mut carry {
            if rest.is_empty() {
                break;
            }
            let maximum = world.registry.item_rules().max_stack(&rest).min(99);
            if slot.is_empty() {
                let take = rest.count.min(maximum);
                *slot = rest.clone();
                slot.count = take;
                rest.count -= take;
            } else if mc_data::inventory_semantics_26_1_2::can_stack(slot, &rest) {
                let take = rest.count.min((maximum - slot.count).max(0));
                slot.count += take;
                rest.count -= take;
            }
        }
        if !rest.is_empty() {
            return Err(Shortage::Storage);
        }
    }
    Ok(PortionPlan {
        carry_before: carry_image,
        equipment_before: equipment_image,
        inputs,
        outputs,
        wear,
        carry_after: carry,
        equipment_after: equipment,
    })
}

fn select_available(
    world: &TickState,
    input: &mc_data::recipes::work::WorkRecipeInput,
    stacks: &[ItemStack],
    used: &mut [u32],
    mut remaining: u32,
) -> u32 {
    for (stack, used) in stacks.iter().zip(used) {
        if remaining == 0 {
            break;
        }
        if accepts(world, stack, input) {
            let take = remaining.min((stack.count.max(0) as u32).saturating_sub(*used));
            *used += take;
            remaining -= take;
        }
    }
    remaining
}

type SupplyPlan = (Vec<ItemStack>, Vec<ItemStack>);

/// One source's answer to the next-portion deficit.
enum Supply {
    /// The source holds none of the missing inputs (or none unreserved).
    Empty,
    /// The source holds them but the carry has no room for them.
    Crowded,
    Plan(SupplyPlan),
}

/// Plan this source's share of the next-portion deficit, in recipe input order.
/// Selected overlapping tags/components cannot be selected a second time.
fn plan_supply(
    world: &TickState,
    recipe: &WorkRecipe,
    carry: &inventory::Image,
    equipment: &inventory::Image,
    source: &inventory::Image,
) -> Result<Supply, Failure> {
    let mut carry_used = [0; 8];
    let mut equipment_used = [0; 6];
    let mut source_after = source.slots.clone();
    let mut available = inventory::unreserved_stock(world, source)?;
    for input in &recipe.inputs {
        let remaining = select_available(world, input, &carry.slots, &mut carry_used, input.count);
        let mut remaining = select_available(
            world,
            input,
            &equipment.slots,
            &mut equipment_used,
            remaining,
        );
        for stack in &mut source_after {
            if remaining == 0 {
                break;
            }
            if accepts(world, stack, input) {
                let free = available
                    .get_mut(&stack.item_id)
                    .ok_or(Failure::InvalidRequest)?;
                let take = u64::from(remaining)
                    .min(stack.count.max(0) as u64)
                    .min(*free) as u32;
                *free -= u64::from(take);
                stack.count -= take as i32;
                remaining -= take;
                if stack.is_empty() {
                    *stack = ItemStack::EMPTY;
                }
            }
        }
    }
    if source_after == source.slots {
        return Ok(Supply::Empty);
    }
    let mut carry_after = carry.slots.clone();
    for (before, after) in source.slots.iter().zip(&source_after) {
        let count = before.count - after.count;
        if count <= 0 {
            continue;
        }
        let mut moving = before.clone();
        moving.count = count;
        let maximum = world.registry.item_rules().max_stack(&moving).min(99);
        for empty in [false, true] {
            for stored in &mut carry_after {
                if moving.is_empty() {
                    break;
                }
                if stored.is_empty() != empty
                    || (!empty && !mc_data::inventory_semantics_26_1_2::can_stack(stored, &moving))
                {
                    continue;
                }
                let take = moving.count.min((maximum - stored.count).max(0));
                if take > 0 {
                    if empty {
                        *stored = moving.clone();
                        stored.count = 0;
                    }
                    stored.count += take;
                    moving.count -= take;
                }
            }
        }
        if !moving.is_empty() {
            return Ok(Supply::Crowded);
        }
    }
    Ok(Supply::Plan((source_after, carry_after)))
}

fn supply_stand(world: &TickState, worker: Uuid, chest: [i32; 3]) -> Option<[i32; 3]> {
    let slot = world.entities.slot_of_uuid(worker)?;
    let body = world.entities.body(slot);
    let pos = world.entities.pos(slot);
    let mut best = None;
    for dy in [0, 1, -1] {
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let cell = BlockPos {
                x: chest[0] + dx,
                y: chest[1] + dy,
                z: chest[2] + dz,
            };
            let Some(stand) = super::allocator::standing(
                &world.world,
                &world.registry,
                cell,
                body.half_width,
                body.height,
            ) else {
                continue;
            };
            let distance: f64 = stand.iter().zip(pos).map(|(a, b)| (a - b).powi(2)).sum();
            if best.is_none_or(|(_, nearest)| distance < nearest) {
                best = Some(([cell.x, cell.y, cell.z], distance));
            }
        }
    }
    best.map(|(at, _)| at)
}

/// Squared distance from `pos` to the centre of block `at`; reach is 2.5.
fn reach(pos: [f64; 3], at: [i32; 3]) -> f64 {
    pos.into_iter()
        .zip([
            f64::from(at[0]) + 0.5,
            f64::from(at[1]),
            f64::from(at[2]) + 0.5,
        ])
        .map(|(a, b)| (a - b).powi(2))
        .sum()
}

/// Native production tick owns the attached carry. Enumerate current native
/// town storage again at every attempt, then use the existing fenced bridge;
/// cached navigation never grants authority or bypasses reservation floors.
/// Stock that a full carry cannot take sends the worker to put surplus away.
fn fetch_inputs(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
) -> Result<bool, Failure> {
    let recipe = recipe_of(world, &job.recipe)?;
    let carry = inventory::load(
        world,
        &inventory::Endpoint::VillagerCarry { uuid: job.worker },
    )?;
    let equipment = inventory::load(
        world,
        &inventory::Endpoint::VillagerEquipment { uuid: job.worker },
    )?;
    let retry = world.tick.saturating_add(RETRY_TICKS);
    let mut crowded = false;
    for (position, source) in inventory::town_storage(world, settlement) {
        let (source_after, carry_after) =
            match plan_supply(world, recipe, &carry, &equipment, &source)? {
                Supply::Plan(plan) => plan,
                Supply::Crowded => {
                    crowded = true;
                    continue;
                }
                Supply::Empty => continue,
            };
        let pos = worker_pos(world, job.worker).ok_or(Failure::NotFound)?;
        if reach(pos, position) > 6.25 {
            let Some(stand) = supply_stand(world, job.worker, position) else {
                continue;
            };
            job.fetch_at = Some(stand);
            return Ok(park(job, "awaiting input reach", retry));
        }
        inventory::commit(
            world,
            vec![
                inventory::Change {
                    before: source,
                    after: source_after,
                },
                inventory::Change {
                    before: carry,
                    after: carry_after,
                },
            ],
        )?;
        job.fetch_at = None;
        park(job, "awaiting reach", retry);
        return Ok(true);
    }
    if crowded {
        return store_surplus(world, settlement, job);
    }
    Ok(park(job, "missing input", retry))
}

/// Carry units the next portion of `recipe` uses: its inputs, selected like
/// [`plan_portion`] (carry before equipment), and a carried tool the
/// equipment does not already provide.
fn kept_carry(
    world: &TickState,
    recipe: &WorkRecipe,
    carry: &[ItemStack],
    equipment: &[ItemStack],
) -> Vec<u32> {
    let mut kept = vec![0_u32; carry.len()];
    let mut worn = vec![0_u32; equipment.len()];
    for input in &recipe.inputs {
        let remaining = select_available(world, input, carry, &mut kept, input.count);
        select_available(world, input, equipment, &mut worn, remaining);
    }
    for tool in &recipe.tools {
        let fits = |stack: &ItemStack| {
            !stack.is_empty()
                && resource(world, stack) == Some(tool.item.as_str())
                && tool
                    .components
                    .as_ref()
                    .is_none_or(|components| components.matches(stack))
        };
        if !equipment.iter().any(&fits)
            && let Some(index) = carry.iter().position(&fits)
        {
            kept[index] = carry[index].count.max(0) as u32;
        }
    }
    kept
}

/// Move every unkept carry unit into `target`, merging into matching stacks
/// before using empty slots. `None` when nothing fits there.
fn plan_store(
    world: &TickState,
    carry: &[ItemStack],
    kept: &[u32],
    target: &[ItemStack],
) -> Option<SupplyPlan> {
    let mut target_after = target.to_vec();
    let mut carry_after = carry.to_vec();
    let mut moved = false;
    for (moving, kept) in carry_after.iter_mut().zip(kept) {
        let keep = i32::try_from(*kept).unwrap_or(i32::MAX);
        if moving.is_empty() || moving.count <= keep {
            continue;
        }
        let maximum = world.registry.item_rules().max_stack(moving);
        for empty in [false, true] {
            for stored in &mut target_after {
                let spare = moving.count - keep;
                if spare <= 0 {
                    break;
                }
                if stored.is_empty() != empty
                    || (!empty && !mc_data::inventory_semantics_26_1_2::can_stack(stored, moving))
                {
                    continue;
                }
                let take = spare.min((maximum - stored.count).max(0));
                if take > 0 {
                    if empty {
                        *stored = moving.clone();
                        stored.count = 0;
                    }
                    stored.count += take;
                    moving.count -= take;
                    moved = true;
                }
            }
        }
        if moving.count == 0 {
            *moving = ItemStack::EMPTY;
        }
    }
    moved.then_some((target_after, carry_after))
}

/// The carry has no room for the next portion's inputs or outputs. Put away
/// everything that portion does not use (products, by-products, other cargo)
/// in town storage: walk to the nearest physical container with room, then
/// move it through the same fenced bridge as a fetch. Nothing is dropped;
/// with no room anywhere the job parks on "no storage" until there is.
fn store_surplus(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
) -> Result<bool, Failure> {
    let recipe = recipe_of(world, &job.recipe)?;
    let carry = inventory::load(
        world,
        &inventory::Endpoint::VillagerCarry { uuid: job.worker },
    )?;
    let equipment = inventory::load(
        world,
        &inventory::Endpoint::VillagerEquipment { uuid: job.worker },
    )?;
    let kept = kept_carry(world, recipe, &carry.slots, &equipment.slots);
    let pos = worker_pos(world, job.worker).ok_or(Failure::NotFound)?;
    let retry = world.tick.saturating_add(RETRY_TICKS);
    let mut walk: Option<([i32; 3], f64)> = None;
    for (position, target) in inventory::town_storage(world, settlement) {
        let Some((target_after, carry_after)) =
            plan_store(world, &carry.slots, &kept, &target.slots)
        else {
            continue;
        };
        let distance = reach(pos, position);
        if distance <= 6.25 {
            inventory::commit(
                world,
                vec![
                    inventory::Change {
                        before: target,
                        after: target_after,
                    },
                    inventory::Change {
                        before: carry,
                        after: carry_after,
                    },
                ],
            )?;
            job.fetch_at = None;
            park(job, "awaiting reach", retry);
            return Ok(true);
        }
        if walk.is_none_or(|(_, nearest)| distance < nearest)
            && let Some(stand) = supply_stand(world, job.worker, position)
        {
            walk = Some((stand, distance));
        }
    }
    let Some((stand, _)) = walk else {
        return Ok(park(job, "no storage", retry));
    };
    job.fetch_at = Some(stand);
    Ok(park(job, "awaiting storage reach", retry))
}

fn pause_reason(failure: Failure) -> &'static str {
    match failure {
        Failure::Unloaded => "unloaded",
        Failure::Blocked => "missing station",
        Failure::NotFound => "missing station",
        Failure::Capacity => "no storage",
        Failure::InsufficientItems => "missing input",
        _ => "blocked route",
    }
}

fn admit(
    world: &mut TickState,
    settlement: SettlementId,
    owner: &str,
    state: &mut State,
    start: Start,
) -> Result<bool, Failure> {
    let Start {
        job,
        worker,
        recipe_name,
        count,
        station,
        stand_at,
        building,
    } = start;
    if !valid_id(&job) || count == 0 || worker.is_nil() {
        return Err(Failure::InvalidRequest);
    }
    let record = world.settlements.get(settlement).ok_or(Failure::NotFound)?;
    if record.owner.as_deref() != Some(owner) {
        return Err(Failure::Forbidden);
    }
    worker_slot(world, settlement, worker)?;
    if let Some(existing) = state.jobs.get(&job) {
        let same = existing.worker == worker
            && existing.recipe == recipe_name
            && existing.count == count
            && existing.station == station
            && existing.stand_at == stand_at
            && existing.building.kind == building.kind
            && existing.building.id == building.id
            && existing.building.purpose == building.purpose;
        return if same {
            Ok(false)
        } else {
            Err(Failure::OperationConflict)
        };
    }
    if state
        .jobs
        .values()
        .any(|other| other.unfinished() && other.worker == worker)
    {
        return Err(Failure::Busy);
    }
    let facts = building_facts(world, settlement, &building)?;
    let recipe = recipe_of(world, &recipe_name)?;
    station_live(world, recipe, &facts, station, stand_at)?;
    let duration = recipe.duration_ticks.max(1);
    let tick = world.tick;
    state.jobs.insert(
        job.clone(),
        Job {
            id: job,
            owner: owner.to_owned(),
            worker,
            recipe: recipe_name,
            recipe_revision: recipe.revision.clone(),
            duration,
            count,
            done: 0,
            delivery: Some(delivery::Delivery::default()),
            station,
            stand_at,
            building,
            dimension: facts.dimension,
            accepted_tick: tick,
            next_due: tick.saturating_add(duration),
            paused: None,
            cancelled: false,
            receipts: Vec::new(),
            order: None,
            after: None,
            fetch_at: None,
        },
    );
    state.sequence = state.sequence.saturating_add(1);
    Ok(true)
}

/// Admit a planned chain atomically: every step passes the [`admit`] checks
/// before any insert, and step `n` waits for step `n - 1`. The final step's
/// id is the order id, so a retried order is idempotent, never a duplicate.
fn admit_order(
    world: &mut TickState,
    settlement: SettlementId,
    owner: &str,
    state: &mut State,
    order: String,
    worker: Uuid,
    steps: Vec<Step>,
) -> Result<bool, Failure> {
    let Some(last) = steps.last() else {
        return Err(Failure::InvalidRequest);
    };
    // Leave room for the `-NN` suffix of earlier steps inside `valid_id`.
    if !valid_id(&order)
        || order.len() > 60
        || steps.len() > MAX_ORDER_STEPS
        || worker.is_nil()
        || steps.iter().any(|step| step.count == 0)
    {
        return Err(Failure::InvalidRequest);
    }
    let record = world.settlements.get(settlement).ok_or(Failure::NotFound)?;
    if record.owner.as_deref() != Some(owner) {
        return Err(Failure::Forbidden);
    }
    worker_slot(world, settlement, worker)?;
    if let Some(existing) = state.jobs.get(&order) {
        let same = existing.order.as_deref() == Some(order.as_str())
            && existing.worker == worker
            && existing.recipe == last.recipe
            && existing.count == last.count;
        return if same {
            Ok(false)
        } else {
            Err(Failure::OperationConflict)
        };
    }
    let ids: Vec<String> = (1..steps.len())
        .map(|n| format!("{order}-{n}"))
        .chain(std::iter::once(order.clone()))
        .collect();
    if ids.iter().any(|id| state.jobs.contains_key(id)) {
        return Err(Failure::OperationConflict);
    }
    if state
        .jobs
        .values()
        .any(|other| other.unfinished() && other.worker == worker)
    {
        return Err(Failure::Busy);
    }
    let tick = world.tick;
    let mut admitted: Vec<Job> = Vec::with_capacity(steps.len());
    for (id, step) in ids.into_iter().zip(steps) {
        let facts = building_facts(world, settlement, &step.building)?;
        let recipe = recipe_of(world, &step.recipe)?;
        station_live(world, recipe, &facts, step.station, step.stand_at)?;
        let duration = recipe.duration_ticks.max(1);
        let after = admitted.last().map(|previous| previous.id.clone());
        let delivery = (id == order).then(delivery::Delivery::default);
        admitted.push(Job {
            id,
            owner: owner.to_owned(),
            worker,
            recipe: step.recipe,
            recipe_revision: recipe.revision.clone(),
            duration,
            count: step.count,
            done: 0,
            delivery,
            station: step.station,
            stand_at: step.stand_at,
            building: step.building,
            dimension: facts.dimension,
            accepted_tick: tick,
            next_due: tick.saturating_add(duration),
            paused: None,
            cancelled: false,
            receipts: Vec::new(),
            order: Some(order.clone()),
            after,
            fetch_at: None,
        });
    }
    state.sequence = state.sequence.saturating_add(admitted.len() as u64);
    for job in admitted {
        state.jobs.insert(job.id.clone(), job);
    }
    Ok(true)
}

pub(crate) fn execute(
    world: &mut TickState,
    settlement: SettlementId,
    owner: &str,
    state: &mut State,
    request: Request,
) -> Result<bool, Failure> {
    match request {
        Request::Start {
            job,
            worker,
            recipe: recipe_name,
            count,
            station,
            stand_at,
            building,
        } => admit(
            world,
            settlement,
            owner,
            state,
            Start {
                job,
                worker,
                recipe_name,
                count,
                station,
                stand_at,
                building,
            },
        ),
        Request::Pause { job } => {
            let Some(entry) = state.jobs.get_mut(&job) else {
                return Err(Failure::NotFound);
            };
            if !entry.unfinished() || entry.paused.as_deref() == Some(MANUAL) {
                return Ok(false);
            }
            entry.paused = Some(MANUAL.to_owned());
            entry.fetch_at = None;
            Ok(true)
        }
        Request::Resume { job } => {
            if let Some(entry) = state.jobs.get(&job) {
                worker_slot(world, settlement, entry.worker)?;
            }
            let tick = world.tick;
            let Some(entry) = state.jobs.get_mut(&job) else {
                return Err(Failure::NotFound);
            };
            if entry.cancelled {
                return Err(Failure::InvalidRequest);
            }
            if !entry.unfinished() || entry.paused.is_none() {
                return Ok(false);
            }
            entry.paused = None;
            entry.fetch_at = None;
            entry.next_due = tick;
            Ok(true)
        }
        Request::Cancel { job } => {
            let Some(entry) = state.jobs.get(&job) else {
                return Err(Failure::NotFound);
            };
            // An order stops as a whole, whichever step is named (a finished
            // dependency included): later steps could never start, and
            // earlier ones would only produce for a cancelled target.
            // Finished steps keep their receipts; only open steps cancel.
            let order = entry.order.clone();
            let mut cancelled = 0_u64;
            for entry in state
                .jobs
                .values_mut()
                .filter(|entry| entry.id == job || (order.is_some() && entry.order == order))
            {
                if !entry.unfinished() {
                    continue;
                }
                entry.cancelled = true;
                entry.paused = None;
                entry.fetch_at = None;
                cancelled += 1;
            }
            state.cancelled = state.cancelled.saturating_add(cancelled);
            Ok(cancelled > 0)
        }
        Request::Order {
            order,
            worker,
            steps,
        } => admit_order(world, settlement, owner, state, order, worker, steps),
    }
}

fn reached(world: &TickState, job: &Job) -> bool {
    let Some(pos) = worker_pos(world, job.worker) else {
        return false;
    };
    let centre = [
        f64::from(job.stand_at[0]) + 0.5,
        f64::from(job.stand_at[1]),
        f64::from(job.stand_at[2]) + 0.5,
    ];
    pos.iter()
        .zip(centre)
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        <= 6.25
}

fn park(job: &mut Job, reason: &str, next_due: u64) -> bool {
    let changed = job.paused.as_deref() != Some(reason) || job.next_due != next_due;
    if job.paused.as_deref() != Some(reason) {
        job.paused = Some(reason.to_owned());
    }
    job.next_due = next_due;
    changed
}

fn tick_job(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
) -> Result<bool, Failure> {
    if job.delivery_pending() {
        return delivery::store(world, settlement, job);
    }
    let route_changed = job.fetch_at.take().is_some();
    // Resolve mutable physical evidence before borrowing the native recipe.
    let facts = building_facts(world, settlement, &job.building)?;
    let recipe = recipe_of(world, &job.recipe)?;
    let metadata_changed = route_changed
        || job.recipe_revision != recipe.revision
        || job.duration != recipe.duration_ticks.max(1);
    if job.recipe_revision != recipe.revision {
        job.recipe_revision.clone_from(&recipe.revision);
    }
    job.duration = recipe.duration_ticks.max(1);
    let retry = world.tick.saturating_add(RETRY_TICKS);
    if let Err(reason) = station_live(world, recipe, &facts, job.station, job.stand_at) {
        return Ok(park(job, pause_reason(reason), retry) | metadata_changed);
    }
    let plan = match plan_portion(world, job.worker, recipe) {
        Ok(plan) => plan,
        Err(Shortage::Input) => return Ok(fetch_inputs(world, settlement, job)? | metadata_changed),
        Err(Shortage::Tool) => return Ok(park(job, "missing tool", retry) | metadata_changed),
        Err(Shortage::Storage) => {
            return Ok(store_surplus(world, settlement, job)? | metadata_changed);
        }
    };
    if !reached(world, job) {
        return Ok(park(job, "awaiting reach", retry) | metadata_changed);
    }
    let delivery_after = job
        .delivery
        .as_ref()
        .map(|previous| {
            delivery::after_production(
                previous,
                &plan.carry_after,
                &plan.outputs[..recipe.outputs.len()],
            )
        })
        .transpose()?;
    let changes = vec![
        inventory::Change {
            before: plan.carry_before,
            after: plan.carry_after,
        },
        inventory::Change {
            before: plan.equipment_before,
            after: plan.equipment_after,
        },
    ];
    if let Err(failure) = inventory::commit(world, changes) {
        return Ok(park(job, pause_reason(failure), retry) | metadata_changed);
    }
    job.delivery = delivery_after;
    job.done += 1;
    job.receipts.push(Receipt {
        job: job.id.clone(),
        sequence: job.done,
        recipe: job.recipe.clone(),
        recipe_revision: job.recipe_revision.clone(),
        inputs: plan.inputs,
        outputs: plan.outputs,
        tool_wear: plan.wear,
    });
    job.paused = job
        .delivery_pending()
        .then(|| "awaiting output storage".to_owned());
    job.next_due = world.tick.saturating_add(job.duration);
    Ok(true)
}

pub(crate) fn tick(world: &mut TickState, settlement: SettlementId, state: &mut State) -> bool {
    let tick_now = world.tick;
    let mut finished = Vec::new();
    let changed = state.cursor.run(
        &mut state.jobs,
        |job| {
            job.unfinished()
                && job.after.is_none()
                && job.paused.as_deref() != Some(MANUAL)
                && job.next_due <= tick_now
        },
        |_, job| {
            if worker_slot(world, settlement, job.worker).is_err() {
                return park(job, "missing worker", tick_now.saturating_add(RETRY_TICKS)).into();
            }
            let produced_before = job.done;
            match tick_job(world, settlement, job) {
                Ok(changed) => {
                    if produced_before < job.count && job.done >= job.count {
                        finished.push(job.id.clone());
                    }
                    changed
                }
                Err(failure) => park(
                    job,
                    pause_reason(failure),
                    tick_now.saturating_add(RETRY_TICKS),
                ),
            }
            .into()
        },
    );
    // Release the next order step in the same tick (and policy image) that
    // completed its predecessor, so a restart never sees a stale gate.
    if !finished.is_empty() {
        for job in state.jobs.values_mut() {
            if job
                .after
                .as_ref()
                .is_some_and(|after| finished.contains(after))
            {
                job.after = None;
                job.next_due = tick_now.saturating_add(job.duration);
            }
        }
    }
    state.completed = state.completed.saturating_add(finished.len() as u64);
    changed || !finished.is_empty()
}

/// Any nonterminal, noncancelled, ungated job needing tick. Auto-paused jobs
/// that can resume are included; only the manual hold is parked.
pub(crate) fn active(state: &State) -> bool {
    state
        .jobs
        .values()
        .any(|job| job.unfinished() && job.after.is_none() && job.paused.as_deref() != Some(MANUAL))
}

/// Native brain routing target for one worker: its live stand post, hidden
/// while manually held or terminal.
#[cfg(test)]
pub(crate) fn target(state: &State, worker: Uuid) -> Option<BlockPos> {
    state
        .jobs
        .values()
        .filter(|job| job.worker == worker)
        .find_map(target_for_job)
}

pub(super) fn target_for_job(job: &Job) -> Option<BlockPos> {
    let at = job.fetch_at.unwrap_or(job.stand_at);
    (job.unfinished() && job.after.is_none() && job.paused.as_deref() != Some(MANUAL)).then_some(
        BlockPos {
            x: at[0],
            y: at[1],
            z: at[2],
        },
    )
}

pub(crate) fn validate(state: &State) -> Result<(), &'static str> {
    if state.completed > state.sequence || state.cancelled > state.sequence {
        return Err("invalid production counters");
    }
    for (id, job) in &state.jobs {
        if !valid_id(id) || id != &job.id {
            return Err("invalid production job identity");
        }
        if job.owner.is_empty()
            || job.recipe.is_empty()
            || job.recipe_revision.is_empty()
            || job.dimension.is_empty()
            || job.count == 0
            || job.done > job.count
            || job.duration == 0
        {
            return Err("invalid production job");
        }
        if let Some(delivery) = &job.delivery {
            delivery.validate()?;
            if job.order.as_ref().is_some_and(|order| order != &job.id)
                || (delivery.pending() && job.done == 0)
            {
                return Err("invalid delivery job");
            }
        }
        if job.cancelled && job.done >= job.count && !job.delivery_pending() {
            return Err("invalid cancelled production job");
        }
        if job.receipts.len() != job.done as usize {
            return Err("invalid production receipts");
        }
        for (index, receipt) in job.receipts.iter().enumerate() {
            if receipt.sequence != index as u32 + 1
                || receipt.job != job.id
                || receipt.recipe != job.recipe
                || receipt.recipe_revision.is_empty()
                || receipt.outputs.is_empty()
            {
                return Err("invalid production receipt");
            }
        }
        job.building
            .validate()
            .map_err(|_| "invalid production certificate")?;
        if let Some(order) = &job.order
            && state
                .jobs
                .get(order)
                .is_none_or(|last| last.order.as_ref() != Some(order))
        {
            return Err("invalid production order");
        }
        if let Some(after) = &job.after
            && (job.order.is_none()
                || job.done != 0
                || after == id
                || state
                    .jobs
                    .get(after)
                    .is_none_or(|previous| previous.order != job.order))
        {
            return Err("invalid production order step");
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "production_tests.rs"]
mod tests;
