//! Vehicle seats: passenger pairings and stair seats planned against the
//! snapshot.
use super::*;

/// 26.3 `ArmorStand` client flags; bit `0x10` turns the stand into a marker.
const ARMOR_STAND_DATA_INDEX: u8 = 15;
const ARMOR_STAND_MARKER_FLAG: i8 = 0x10;
/// Shared entity flags; `0x20` makes the stand invisible.
const INVISIBLE_FLAG: i8 = 0x20;

/// Release one seat: clear the passenger list, then remove the stand.
fn push_seat_release(viewer: &ViewerState, out: &mut Vec<PlannedPacket>, seat: i32) {
    push_best_effort(
        out,
        viewer.connection,
        &mc_protocol::packets::play::ClientboundSetPassengers {
            vehicle_id: seat,
            passenger_ids: Vec::new(),
        },
    );
    push_best_effort(
        out,
        viewer.connection,
        &RemoveEntities {
            entity_ids: vec![seat],
        },
    );
}

pub(super) fn clear_seats(
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) -> bool {
    while let Some((&rider, &(seat, _))) = viewer.seats.iter().next() {
        if !try_push(out, credit, |out| push_seat_release(viewer, out, seat)) {
            return false;
        }
        viewer.seats.remove(&rider);
    }
    true
}

pub(super) fn plan_seats(
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) {
    // Seats whose rider is no longer seated where the ride put them (or that
    // left tracking) are released before new seats are planned.
    while let Some((rider, id)) = viewer.seats.iter().find_map(|(&rider, &(id, pos))| {
        (viewer
            .tracked_entities
            .get(&rider)
            .and_then(|tracked| tracked.appearance.villager_seat)
            != Some(pos))
        .then_some((rider, id))
    }) {
        if !try_push(out, credit, |out| push_seat_release(viewer, out, id)) {
            return;
        }
        viewer.seats.remove(&rider);
    }
    for (&rider, tracked) in &viewer.tracked_entities {
        let Some(pos) = tracked.appearance.villager_seat else {
            continue;
        };
        if viewer.seats.contains_key(&rider) {
            continue;
        }
        if credit.spent() || credit.frames < 3 {
            return;
        }
        let Some(id) = viewer.labels.alloc() else {
            return;
        };
        let stand_type =
            mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:armor_stand")
                .expect("armor stand entity contract")
                .protocol_id as i32;
        if !try_push(out, credit, |out| {
            push_best_effort(
                out,
                viewer.connection,
                &AddEntity {
                    entity_id: id,
                    uuid: uuid::Uuid::from_u128((u128::from(id as u32) << 64) | 0x73656174),
                    entity_type_id: stand_type,
                    x: f64::from(pos.x) + 0.5,
                    y: f64::from(pos.y) + 0.5,
                    z: f64::from(pos.z) + 0.5,
                    movement: EntityVec3::ZERO,
                    pitch: 0.0,
                    yaw: 0.0,
                    head_yaw: 0.0,
                    data: 0,
                },
            );
            push_best_effort(
                out,
                viewer.connection,
                &ClientboundSetEntityData {
                    entity_id: id,
                    values: vec![
                        WireEntityDataValue::Byte {
                            index: ENTITY_DATA_SHARED_FLAGS_INDEX,
                            value: INVISIBLE_FLAG,
                        },
                        WireEntityDataValue::Byte {
                            index: ARMOR_STAND_DATA_INDEX,
                            value: ARMOR_STAND_MARKER_FLAG,
                        },
                    ],
                },
            );
            push_best_effort(
                out,
                viewer.connection,
                &mc_protocol::packets::play::ClientboundSetPassengers {
                    vehicle_id: id,
                    passenger_ids: vec![rider.0],
                },
            );
        }) {
            return;
        }
        viewer.seats.insert(rider, (id, pos));
    }
}
