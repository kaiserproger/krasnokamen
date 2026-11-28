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

fn donor(
    state: &TickState,
    id: SettlementId,
    position: [i32; 3],
    item: u32,
) -> Option<(uuid::Uuid, EntitySlot, u32)> {
    state
        .settlements
        .get(id)?
        .carry_pending
        .iter()
        .find_map(|(uuid, owed)| {
            let (_, owed) = owed.iter().find(|(known, _)| *known == item)?;
            let slot = source(state, id, *uuid)?;
            let count = available(state, slot, item, *owed);
            (count > 0 && point(state, slot) == position).then_some((*uuid, slot, count))
        })
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

/// `None` is an ordinary bank pickup. Donor routing and the transfer fence use
/// exactly the same body-to-body distance, not the donor's floored cell centre.
pub(crate) fn in_reach(
    state: &TickState,
    id: SettlementId,
    builder: EntitySlot,
    position: [i32; 3],
    item: u32,
) -> Option<bool> {
    let (_, slot, _) = donor(state, id, position, item)?;
    Some(near_bodies(state, builder, slot))
}

pub(super) fn pickup_point(
    state: &TickState,
    id: SettlementId,
    item: u32,
    origin: [f64; 3],
) -> Option<[i32; 3]> {
    state
        .settlements
        .get(id)?
        .carry_pending
        .iter()
        .filter_map(|(uuid, owed)| {
            let (_, owed) = owed.iter().find(|(known, _)| *known == item)?;
            let slot = source(state, id, *uuid)?;
            (available(state, slot, item, *owed) > 0).then(|| {
                let pos = state.entities.pos(slot);
                let distance = (origin[0] - pos[0]).powi(2) + (origin[2] - pos[2]).powi(2);
                (point(state, slot), distance)
            })
        })
        .min_by(|(_, a), (_, b)| a.total_cmp(b))
        .map(|(point, _)| point)
}

/// The caller has authorized the bound builder and capped `wanted` by its
/// remaining haul. No remote transfer and no crafting inside a resident's bag.
pub(super) fn fetch(
    state: &mut TickState,
    id: SettlementId,
    builder: uuid::Uuid,
    position: [i32; 3],
    item: u32,
    wanted: u32,
) -> Option<Result<(), Failure>> {
    let (uuid, slot, owed) = donor(state, id, position, item)?;
    Some((|| {
        let receiver = state
            .entities
            .slot_of_uuid(builder)
            .ok_or(Failure::NotFound)?;
        if !near_bodies(state, receiver, slot) {
            return Err(Failure::Blocked);
        }
        let donor = inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid })?;
        let receiver =
            inventory::load(state, &inventory::Endpoint::VillagerCarry { uuid: builder })?;
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
        let duty = record.carry_pending.get_mut(&uuid).expect("source duty");
        let (_, count) = duty
            .iter_mut()
            .find(|(known, _)| *known == item)
            .expect("source material");
        *count -= moved;
        duty.retain(|(_, count)| *count > 0);
        if duty.is_empty() {
            record.carry_pending.remove(&uuid);
        }
        record.revision = record.revision.saturating_add(1);
        Ok(())
    })())
}
