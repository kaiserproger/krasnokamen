//! Settlement residents shear adult sheep with a real tool and carry wool to storage.
//! Eligible adults share the same bounded sheep search after their own job.
//! Shears are taken from settlement storage or crafted from two iron ingots.
use super::mining;
use super::{Activity, VillagerProfession, VillagerState};
use crate::boundary::{BlockPos, ChangeSet, ChunkPos};
use crate::entities::{EntityKind, EntitySlot};
use crate::settlements::{buildings, inventory};
use crate::tick::TickState;
use mc_data::ItemStack;

/// Ticks between shear attempts of one working shepherd.
const PERIOD: u64 = 100;
/// Shear reach in blocks, matching the interact geometry other workers use.
const REACH: f64 = 3.0;

/// Nearest eligible sheep among at most 256 chunk-local sensor entries.
/// The chunk range is settlement-bounded; unrelated world populations never
/// contribute to this work attempt.
fn find_sheep(
    state: &TickState,
    anchor: [i32; 3],
    radius: u32,
    from: [f64; 3],
) -> Option<EntitySlot> {
    let sheep = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:sheep").ok()?)?;
    let span = radius as i32;
    let limit = f64::from(radius) * f64::from(radius);
    let (min_cx, max_cx) = (
        anchor[0].saturating_sub(span).div_euclid(16),
        anchor[0].saturating_add(span).div_euclid(16),
    );
    let (min_cz, max_cz) = (
        anchor[2].saturating_sub(span).div_euclid(16),
        anchor[2].saturating_add(span).div_euclid(16),
    );
    let mut best: Option<(EntitySlot, f64)> = None;
    let mut examined = 0;
    for cx in min_cx..=max_cx {
        for cz in min_cz..=max_cz {
            if !state.world.is_active(cx, cz) {
                continue;
            }
            let Some(ids) = state
                .entities
                .brain_sensor_chunks()
                .get(&ChunkPos { x: cx, z: cz })
            else {
                continue;
            };
            for id in ids {
                if examined == 256 {
                    return best.map(|(slot, _)| slot);
                }
                examined += 1;
                let Some(slot) = state.entities.slot_of(*id) else {
                    continue;
                };
                let body = state.entities.body(slot);
                if !body.alive
                    || !matches!(body.kind, EntityKind::Mob { type_id } if type_id == sheep)
                {
                    continue;
                }
                let row = state.entities.row(slot);
                if !crate::entity_life::adult(row)
                    || crate::tick::wool_of(&row.appearance) & 0x10 != 0
                {
                    continue;
                }
                let p = state.entities.pos(slot);
                let inside = (p[0] - f64::from(anchor[0])).powi(2)
                    + (p[1] - f64::from(anchor[1])).powi(2)
                    + (p[2] - f64::from(anchor[2])).powi(2);
                if inside > limit {
                    continue;
                }
                let distance =
                    (p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2) + (p[2] - from[2]).powi(2);
                if best.is_none_or(|(_, kept)| distance < kept) {
                    best = Some((slot, distance));
                }
            }
        }
    }
    best.map(|(slot, _)| slot)
}

fn wool(state: &TickState, stack: &ItemStack) -> bool {
    state
        .registry
        .items
        .name_of(stack.item_id)
        .is_some_and(|name| name.namespace() == "minecraft" && name.path().ends_with("_wool"))
}

fn deposit(state: &mut TickState, slot: EntitySlot, v: &mut VillagerState) -> Option<BlockPos> {
    let mut protected = crate::settlements::economy::protected_carry(state, slot, v);
    for (index, stack) in v.inventory.iter().enumerate() {
        if let Some(stack) = stack
            && !wool(state, stack)
        {
            protected[index] = stack.count.max(0) as u32;
        }
    }
    mining::deposit_with_protected(state, slot, v, protected)
}

fn acquire_tool(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    shears: u32,
    iron: u32,
) -> Option<BlockPos> {
    let iron_count: i32 = v
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == iron)
        .map(|stack| stack.count)
        .sum();
    if iron_count >= 2 {
        let mut carry = v.inventory.clone();
        let mut remaining = 2;
        for stack in &mut carry {
            if let Some(item) = stack.as_mut().filter(|item| item.item_id == iron) {
                let taken = remaining.min(item.count);
                item.count -= taken;
                remaining -= taken;
                if item.count == 0 {
                    *stack = None;
                }
            }
        }
        if let Some(empty) = carry.iter_mut().find(|stack| stack.is_none()) {
            *empty = Some(ItemStack::new(shears, 1));
            v.inventory = carry;
        }
        return None;
    }
    let settlement = v.settlement?;
    for position in buildings::storage_points(state, settlement) {
        let endpoint = inventory::Endpoint::Container {
            dimension: state.dimension.name().into(),
            position,
        };
        let Ok(before) = inventory::load(state, &endpoint) else {
            continue;
        };
        let Ok(free) = inventory::unreserved_stock(state, &before) else {
            continue;
        };
        let selected = [shears, iron].into_iter().find_map(|item| {
            let available = free.get(&item).copied().unwrap_or(0);
            before
                .slots
                .iter()
                .position(|stack| stack.item_id == item && stack.count > 0 && available > 0)
                .map(|index| (item, index, available))
        });
        let Some((item, index, available)) = selected else {
            continue;
        };
        let pos = BlockPos {
            x: position[0],
            y: position[1],
            z: position[2],
        };
        if !mining::near(state, slot, pos, 2.5) {
            return Some(pos);
        }
        let mut carried = v.inventory.clone();
        let count = if item == shears {
            1
        } else {
            (2 - iron_count).min(before.slots[index].count)
        }
        .min(available.min(i32::MAX as u64) as i32);
        let Some(empty) = carried.iter_mut().find(|stack| stack.is_none()) else {
            return deposit(state, slot, v);
        };
        let mut moving = before.slots[index].clone();
        moving.count = count;
        *empty = Some(moving);
        let mut after = before.slots.clone();
        after[index].count -= count;
        if after[index].count == 0 {
            after[index] = ItemStack::EMPTY;
        }
        if inventory::commit(state, vec![inventory::Change { before, after }]).is_ok() {
            v.inventory = carried;
            return None;
        }
    }
    None
}

pub(crate) fn eligible(state: &TickState, slot: EntitySlot, v: &VillagerState) -> bool {
    v.age >= 0
        && v.conversion_time.is_none()
        && !matches!(
            v.data.profession,
            VillagerProfession::Nitwit | VillagerProfession::Builder | VillagerProfession::Soldier
        )
        && !crate::settlements::economy::worker_reserved(
            state,
            v.settlement,
            state.entities.row(slot).uuid,
        )
}

pub(super) fn work(
    state: &mut TickState,
    slot: EntitySlot,
    v: &mut VillagerState,
    _changes: &mut ChangeSet,
) -> Option<Option<BlockPos>> {
    if !eligible(state, slot, v) || v.sleeping || v.activity == Activity::Panic {
        return None;
    }
    let settlement = v.settlement?;
    let record = state.settlements.get(settlement)?;
    let (anchor, radius) = (record.anchor, record.radius);
    let working = v.activity == Activity::Work
        && (2_000..9_000).contains(&state.day_time.rem_euclid(24_000))
        && crate::command_rules::boolean(state, crate::command_rules::BoolRule::MOB_GRIEFING);
    let shears = mining::item_id(state, "shears")?;
    let iron = mining::item_id(state, "iron_ingot")?;
    if v.inventory.iter().flatten().any(|stack| wool(state, stack)) {
        return Some(deposit(state, slot, v));
    }
    if !working || state.tick < v.shepherd_at {
        return None;
    }
    if !matches!(
        v.data.profession,
        VillagerProfession::Shepherd | VillagerProfession::None
    ) && !crate::settlements::construction::wool_demand(state, settlement)
    {
        v.shepherd_at = state.tick + PERIOD;
        return None;
    }
    let from = state.entities.pos(slot);
    let Some(target) = find_sheep(state, anchor, radius, from) else {
        v.shepherd_at = state.tick + PERIOD;
        return None;
    };
    if !v
        .inventory
        .iter()
        .flatten()
        .any(|stack| stack.item_id == shears)
    {
        let before: i32 = v.inventory.iter().flatten().map(|stack| stack.count).sum();
        let destination = acquire_tool(state, slot, v, shears, iron);
        let after: i32 = v.inventory.iter().flatten().map(|stack| stack.count).sum();
        if destination.is_none() && before == after {
            v.shepherd_at = state.tick + PERIOD;
            return None;
        }
        return Some(destination);
    }
    let feet = state.entities.pos(target);
    let block = BlockPos {
        x: feet[0].floor() as i32,
        y: feet[1].floor() as i32,
        z: feet[2].floor() as i32,
    };
    if !mining::near(state, slot, block, REACH) {
        // Walk without consuming the shear cooldown: the next attempt
        // re-scans and keeps navigating until within reach.
        return Some(Some(block));
    }
    // The whole yield must fit before the sheep is touched; otherwise haul
    // first and the wool stays on the sheep.
    let wool = crate::tick::wool_of(&state.entities.row(target).appearance);
    let color = usize::from(wool & 15);
    let name = format!(
        "{}_wool",
        crate::blueprint::DYE_COLORS
            .get(color)
            .copied()
            .unwrap_or("white")
    );
    let Some(item) = mining::item_id(state, &name) else {
        v.shepherd_at = state.tick + PERIOD;
        return None;
    };
    let count = 1 + (u64::from(target.0) ^ state.tick).wrapping_mul(0x9E37_79B9_7F4A_7C15) % 3;
    let stack = ItemStack::new(item, count as i32);
    if mining::room(state, v, &stack) < count as i32
        || !mining::collect(state, v, item, count as i32)
    {
        return Some(deposit(state, slot, v));
    }
    let id = state.entities.body(target).id;
    let appearance = std::sync::Arc::make_mut(&mut state.entities.row_mut(target).appearance);
    crate::tick::set_wool(appearance, wool | 0x10);
    state.appearance_changed.push(id);
    if let Some(index) = v
        .inventory
        .iter()
        .position(|stack| stack.as_ref().is_some_and(|stack| stack.item_id == shears))
    {
        let tool = v.inventory[index].as_mut().expect("tool exists");
        let rules = state.registry.item_rules();
        if let Some(maximum) = rules
            .facts
            .facts_for_stack(tool, rules.items)
            .and_then(|facts| facts.max_damage)
        {
            let mut seed = (state.tick ^ u64::from(target.0)) as i32;
            let wear = crate::enchantments::durability_damage(tool, 1, false, &mut seed);
            let damage = tool.damage.unwrap_or(0).saturating_add(wear);
            if u32::try_from(damage).is_ok_and(|damage| damage >= maximum) {
                v.inventory[index] = None;
            } else if wear > 0 {
                tool.damage = Some(damage);
            }
        }
    }
    v.shepherd_at = state.tick + PERIOD;
    Some(None)
}
