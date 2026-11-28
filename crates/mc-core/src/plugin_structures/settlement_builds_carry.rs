//! Released construction stock remains on its resident until a bank accepts it
//! or the next builder collects it. A return duty is not another material bill.
use super::*;
use crate::entities::EntitySlot;

fn source(state: &TickState, id: SettlementId, uuid: uuid::Uuid) -> Option<EntitySlot> {
    let slot = state.entities.slot_of_uuid(uuid)?;
    let v = state.entities.row(slot).villager.as_ref()?;
    (state.entities.body(slot).alive
        && v.settlement == Some(id)
        && !v.wandering
        && v.conversion_time.is_none()
        && !crate::settlements::economy::worker_reserved(state, Some(id), uuid)
        && !crate::settlements::construction::bound_builder(&state.settlements, uuid))
    .then_some(slot)
}

fn available(state: &TickState, slot: EntitySlot, item: u32, owed: u32) -> u32 {
    state
        .entities
        .row(slot)
        .villager
        .as_ref()
        .unwrap()
        .inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item && stack.count > 0)
        .map(|stack| stack.count as u32)
        .sum::<u32>()
        .min(owed)
}

/// What one released body owes for this item, capped by what it actually
/// carries right now. `None` when the body is not an addressable custody source.
fn owed(
    state: &TickState,
    id: SettlementId,
    uuid: uuid::Uuid,
    item: u32,
) -> Option<(EntitySlot, u32)> {
    let slot = source(state, id, uuid)?;
    let duty = state
        .settlements
        .get(id)?
        .carry_pending
        .get(&uuid)?
        .iter()
        .find(|(known, _)| *known == item)?
        .1;
    let count = available(state, slot, item, duty);
    (count > 0).then_some((slot, count))
}

/// Reserve only what storage still needs to supply. Already-carried stock must
/// not require a duplicate recipe (and an empty bank slot) to prove this haul.
pub(super) fn deduct(state: &TickState, id: SettlementId, bill: &mut [(u32, u32)]) {
    let Some(record) = state.settlements.get(id) else {
        return;
    };
    for (uuid, owed) in &record.carry_pending {
        let Some(slot) = source(state, id, *uuid) else {
            continue;
        };
        for (item, count) in bill.iter_mut() {
            if let Some((_, debt)) = owed.iter().find(|(known, _)| known == item) {
                *count = count.saturating_sub(available(state, slot, *item, *debt));
            }
        }
    }
}

fn point(state: &TickState, slot: EntitySlot) -> [i32; 3] {
    state.entities.pos(slot).map(|v| v.floor() as i32)
}

/// The nearest addressable body that owes this item, with its floored cell.
/// The returned uuid, not the cell, is what the caller must debit.
pub(super) fn pickup_point(
    state: &TickState,
    id: SettlementId,
    item: u32,
    origin: [f64; 3],
) -> Option<(uuid::Uuid, [i32; 3])> {
    state
        .settlements
        .get(id)?
        .carry_pending
        .keys()
        .filter_map(|uuid| {
            let (slot, _) = owed(state, id, *uuid, item)?;
            let pos = state.entities.pos(slot);
            let distance = (origin[0] - pos[0]).powi(2) + (origin[2] - pos[2]).powi(2);
            Some((*uuid, point(state, slot), distance))
        })
        .min_by(|(_, _, a), (_, _, b)| a.total_cmp(b))
        .map(|(uuid, at, _)| (uuid, at))
}

fn near_bodies(state: &TickState, a: EntitySlot, b: EntitySlot) -> bool {
    state
        .entities
        .pos(a)
        .iter()
        .zip(state.entities.pos(b))
        .map(|(a, b)| (a - b).powi(2))
        .sum::<f64>()
        <= 2.5_f64.powi(2)
}

/// Donor routing and the transfer fence use exactly the same body-to-body
/// distance. An unaddressable debtor is never in reach.
pub(super) fn in_reach(
    state: &TickState,
    id: SettlementId,
    builder: EntitySlot,
    donor: uuid::Uuid,
    item: u32,
) -> bool {
    let Some((slot, _)) = owed(state, id, donor, item) else {
        return false;
    };
    near_bodies(state, builder, slot)
}

/// The caller has authorized the bound builder and capped `wanted` by its
/// remaining haul. This debits exactly this donor, never "whatever body stands
/// in the route's cell", and moves at most what the duty still owes.
pub(super) fn take(
    state: &mut TickState,
    id: SettlementId,
    builder: uuid::Uuid,
    donor_uuid: uuid::Uuid,
    item: u32,
    wanted: u32,
) -> Result<(), Failure> {
    let (slot, owed) = owed(state, id, donor_uuid, item).ok_or(Failure::Forbidden)?;
    let receiver = state
        .entities
        .slot_of_uuid(builder)
        .ok_or(Failure::NotFound)?;
    if !near_bodies(state, receiver, slot) {
        return Err(Failure::Blocked);
    }
    let donor = inventory::load(
        state,
        &inventory::Endpoint::VillagerCarry { uuid: donor_uuid },
    )?;
    let receiver = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid: builder })?;
    let mut donor_after = donor.slots.clone();
    let mut receiver_after = [receiver.slots.clone()];
    let mut left = owed.min(wanted);
    let mut moved = 0;
    for stack in donor_after
        .iter_mut()
        .filter(|stack| stack.item_id == item && stack.count > 0)
    {
        if left == 0 {
            break;
        }
        let mut piece = stack.clone();
        piece.count = left.min(stack.count as u32) as i32;
        let taken = piece.count
            - crate::settlements::construction_crafting::add_stack_partial(
                state,
                &mut receiver_after,
                &piece,
            );
        stack.count -= taken;
        left -= taken as u32;
        moved += taken as u32;
        if stack.count == 0 {
            *stack = mc_data::ItemStack::EMPTY;
        }
    }
    if moved == 0 {
        return Err(Failure::Capacity);
    }
    let [receiver_after] = receiver_after;
    inventory::commit(
        state,
        vec![
            inventory::Change {
                before: donor,
                after: donor_after,
            },
            inventory::Change {
                before: receiver,
                after: receiver_after,
            },
        ],
    )?;
    let record = state
        .settlements
        .get_mut(id)
        .expect("authorized settlement");
    let duty = record
        .carry_pending
        .get_mut(&donor_uuid)
        .expect("source duty");
    let (_, count) = duty
        .iter_mut()
        .find(|(known, _)| *known == item)
        .expect("source material");
    *count -= moved;
    duty.retain(|(_, count)| *count > 0);
    if duty.is_empty() {
        record.carry_pending.remove(&donor_uuid);
    }
    record.revision = record.revision.saturating_add(1);
    Ok(())
}
