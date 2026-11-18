//! Preserve the owner's confirmed upgrade level while prerequisites change.
use super::{OwnerOrder, SettlementId, TickState};

pub(super) fn next_level(state: &TickState, id: SettlementId, order: &OwnerOrder) -> Option<u8> {
    let building = order.building?;
    let target = state.settlements.get(id)?.buildings.get(&building)?;
    Some(target.level.saturating_add(1))
}

pub(super) fn pending_reason(
    state: &TickState,
    id: SettlementId,
    order: &OwnerOrder,
) -> Option<String> {
    let next = next_level(state, id, order)?;
    (next != order.level).then(|| {
        format!(
            "Upgrade L{} is waiting: building #{} needs L{} next.",
            order.level,
            order.building.expect("upgrade target").0,
            next,
        )
    })
}
