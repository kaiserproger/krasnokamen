//! Preserve the owner's confirmed upgrade level while prerequisites change,
//! and check an upgrade before its admission work.
use super::buildings::BuildingId;
use super::{OwnerOrder, SettlementId, TickState};

pub(super) fn next_level(state: &TickState, id: SettlementId, order: &OwnerOrder) -> Option<u8> {
    let building = order.building?;
    let target = state.settlements.get(id)?.buildings.get(&building)?;
    Some(target.level.saturating_add(1))
}

/// The order names a later level than the building's next one, so it waits
/// for its predecessor.
pub(super) fn waits(state: &TickState, id: SettlementId, order: &OwnerOrder) -> bool {
    next_level(state, id, order).is_some_and(|next| next != order.level)
}

/// An upgrade's checks before its admission work: a free project slot, the
/// building's next blueprint level and its funding. Nothing is planned,
/// withdrawn or reserved: funding is proved again when the upgrade starts.
pub(super) fn upgradable(
    state: &TickState,
    id: SettlementId,
    building: BuildingId,
) -> Result<(), String> {
    super::claim_slot(state, id)?;
    let record = state.settlements.get(id).expect("claimed settlement");
    let building = record
        .buildings
        .get(&building)
        .ok_or("Building not found.")?;
    let (blueprint, ..) = super::upgrade_blueprint(state, id, building)
        .ok_or("No blueprint upgrade available for this building.")?;
    let level = blueprint.level(building.level + 1).expect("upgrade level");
    super::funding_error(state, id, &level.materials).map_or(Ok(()), Err)
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
