//! Vehicle seats: passenger pairings and stair seats planned against the
//! snapshot.
use super::*;

pub(super) fn clear_seats(
    viewer: &mut ViewerState,
    out: &mut Vec<PlannedPacket>,
    credit: &mut WorldCredit,
) -> bool {
    while let Some((&rider, &(seat, _))) = viewer.seats.iter().next() {
        if !try_push(out, credit, |out| {
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
        }) {
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
    loop {
        let removed = viewer.seats.iter().find_map(|(&rider, &(id, pos))| {
            (viewer
                .tracked_entities
                .get(&rider)
                .and_then(|tracked| tracked.appearance.villager_seat)
                != Some(pos))
            .then_some((rider, id))
        });
        let Some((rider, id)) = removed else { break };
        if !try_push(out, credit, |out| {
            push_best_effort(
                out,
                viewer.connection,
                &mc_protocol::packets::play::ClientboundSetPassengers {
                    vehicle_id: id,
                    passenger_ids: Vec::new(),
                },
            );
            push_best_effort(
                out,
                viewer.connection,
                &RemoveEntities {
                    entity_ids: vec![id],
                },
            );
        }) {
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
                            value: 0x20,
                        },
                        WireEntityDataValue::Byte {
                            index: 15,
                            value: 0x10,
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
