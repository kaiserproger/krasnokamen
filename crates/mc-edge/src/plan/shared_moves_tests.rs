//! Shared move-frame oracle: [`SharedMoves`] matches the per-viewer
//! `entity_movement` + `MovePacket::push` bytes exactly and shares frames
//! across identical `(entity id, baseline)` pairs.
use super::*;
use crate::plan::shared_moves::SharedMoves;

type Baseline = ([f64; 3], f32, f32, f32);

fn baseline_at(pos: [f64; 3], yaw: f32, pitch: f32, head: f32) -> Baseline {
    (pos, yaw, pitch, head)
}

/// Old per-viewer path: `entity_movement` framed for one connection.
fn oracle(entity: &EntityView, baseline: Baseline) -> Option<(i32, bytes::Bytes, Baseline)> {
    let (packet, next) = entity_movement(entity, baseline)?;
    let mut out = Vec::new();
    packet.push(&mut out, ConnectionId(7));
    assert_eq!(out.len(), 1);
    let framed = out.pop().expect("move frame fits the framing layer");
    Some((framed.id, framed.data, next))
}

fn frame_body(data: &bytes::Bytes) -> bytes::Bytes {
    let mut framed = bytes::BytesMut::from(&data[..]);
    mc_protocol::frame::try_decode_frame(
        &mut framed,
        mc_protocol::frame::Compression::Threshold(256),
    )
    .expect("shared frame decodes")
    .expect("shared frame is complete")
    .body
}

#[test]
fn shared_moves_is_send_sync_for_rayon_viewers() {
    fn bound<T: Send + Sync>() {}
    bound::<SharedMoves>();
}

#[test]
fn relative_pos_move_matches_oracle_and_shares_frame_bytes() {
    let shared = SharedMoves::default();
    let entity = entity(2, [8.5, 64.0, 8.0]);
    let baseline = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    let first = shared.movement(&entity, baseline).expect("moved");
    let second = shared.movement(&entity, baseline).expect("moved");
    let (id, data, next) = oracle(&entity, baseline).expect("moved");
    assert_eq!(first.id, id);
    assert_eq!(first.data, data);
    assert_eq!(first.next, next);
    // Second hit shares the frame without copying body bytes.
    assert_eq!(first.data.as_ptr(), second.data.as_ptr());
    assert_eq!(second.data, data);
    assert_eq!(second.next, next);
}

#[test]
fn rotation_matches_oracle() {
    let shared = SharedMoves::default();
    let mut entity = entity(3, [8.0, 64.0, 8.0]);
    entity.yaw = 90.0;
    let baseline = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    let got = shared.movement(&entity, baseline).expect("rotated");
    let (id, data, next) = oracle(&entity, baseline).expect("rotated");
    assert_eq!(id, MoveEntityRot::ID);
    assert_eq!(got.id, id);
    assert_eq!(got.data, data);
    assert_eq!(got.next, next);
}

#[test]
fn absolute_sync_matches_oracle() {
    let shared = SharedMoves::default();
    let entity = entity(4, [108.0, 64.0, 8.0]);
    let baseline = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    let got = shared.movement(&entity, baseline).expect("teleported");
    let (id, data, next) = oracle(&entity, baseline).expect("teleported");
    assert_eq!(id, EntityPositionSync::ID);
    assert_eq!(got.id, id);
    assert_eq!(got.data, data);
    assert_eq!(got.next, next);
}

#[test]
fn distinct_baselines_give_valid_distinct_deltas() {
    let shared = SharedMoves::default();
    let mut entity = entity(5, [8.5, 64.0, 8.0]);
    entity.yaw = 10.0;
    // Baselines differ in position (distinct deltas) and in yaw bits that
    // still pack to one step (10.0/10.5 both pack to 7: no rotation, but
    // the unrotated `next` yaw keeps each baseline's own value).
    let near = baseline_at([8.0, 64.0, 8.0], 10.0, 0.0, 0.0);
    let mid = baseline_at([8.25, 64.0, 8.0], 10.5, 0.0, 0.0);
    let from_near = shared.movement(&entity, near).expect("moved");
    let from_mid = shared.movement(&entity, mid).expect("moved");
    // Both match the old path for their own baseline.
    let (_, near_data, near_next) = oracle(&entity, near).expect("moved");
    let (_, mid_data, mid_next) = oracle(&entity, mid).expect("moved");
    assert_eq!(
        (from_near.data.clone(), from_near.next),
        (near_data, near_next)
    );
    assert_eq!((from_mid.data.clone(), from_mid.next), (mid_data, mid_next));
    // Distinct baselines stay distinct: 0.5 -> 2048, 0.25 -> 1024, and the
    // kept yaw values differ.
    assert_ne!(from_near.data, from_mid.data);
    assert_ne!(from_near.next, from_mid.next);
    let delta_near =
        MoveEntityPos::decode(&mut &frame_body(&from_near.data)[..]).expect("pos body");
    let delta_mid = MoveEntityPos::decode(&mut &frame_body(&from_mid.data)[..]).expect("pos body");
    assert_eq!((delta_near.delta_x, delta_mid.delta_x), (2048, 1024));
}

#[test]
fn head_baseline_field_never_conflates_movement_frames() {
    let shared = SharedMoves::default();
    let mut entity = entity(6, [8.5, 64.0, 8.0]);
    entity.head_yaw = 45.0;
    // Same move baselines, different viewer head baselines: the move math
    // ignores head, so both frames stay valid and equal.
    let low = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    let high = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 180.0);
    let from_low = shared.movement(&entity, low).expect("moved");
    let from_high = shared.movement(&entity, high).expect("moved");
    let (_, data, next) = oracle(&entity, low).expect("moved");
    assert_eq!(from_low.data, data);
    assert_eq!(from_high.data, data);
    assert_eq!(from_low.next, next);
    assert_eq!(from_high.next, next);
}

#[test]
fn no_move_returns_none_and_caches_it() {
    let shared = SharedMoves::default();
    let entity = entity(7, [8.0, 64.0, 8.0]);
    let baseline = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    assert!(oracle(&entity, baseline).is_none());
    assert!(shared.movement(&entity, baseline).is_none());
    // Second call hits the cached `None`, skipping threshold math again.
    assert!(shared.movement(&entity, baseline).is_none());
}

#[test]
fn head_frame_matches_oracle_and_is_shared() {
    let shared = SharedMoves::default();
    let mut entity = entity(8, [8.0, 64.0, 8.0]);
    entity.head_yaw = 45.0;
    let (id, data) = shared.head(&entity);
    let (again_id, again_data) = shared.head(&entity);
    let mut out = Vec::new();
    push_best_effort(
        &mut out,
        ConnectionId(9),
        &RotateHead {
            entity_id: entity.id.0,
            head_yaw: entity.head_yaw,
        },
    );
    assert_eq!(out.len(), 1);
    assert_eq!(id, RotateHead::ID);
    assert_eq!(id, out[0].id);
    assert_eq!(data, out[0].data);
    assert_eq!(again_id, id);
    assert_eq!(again_data.as_ptr(), data.as_ptr());
}

#[test]
fn concurrent_first_users_share_one_encoded_frame() {
    let shared = SharedMoves::default();
    let entity = entity(2, [8.5, 64.0, 8.0]);
    let baseline = baseline_at([8.0, 64.0, 8.0], 0.0, 0.0, 0.0);
    let barrier = std::sync::Barrier::new(8);
    let frames = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    shared.movement(&entity, baseline).unwrap().data
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });
    assert!(
        frames
            .iter()
            .all(|frame| frame.as_ptr() == frames[0].as_ptr())
    );
}

/// Consume production shared frames, independently of the encoder's baseline.
/// VecDeltaCodec's 775–777 wire contract rounds the consumer base before adding
/// a nonzero delta, preserves zero-delta axes, and replaces the base on sync.
struct MovementConsumer {
    baseline: Baseline,
    position: [f64; 3],
    rotation: [f32; 2],
}

impl MovementConsumer {
    fn at(position: [f64; 3]) -> Self {
        Self {
            baseline: baseline_at(position, 0.0, 0.0, 0.0),
            position,
            rotation: [0.0, 0.0],
        }
    }

    fn receive(&mut self, entity: &EntityView) -> Option<i32> {
        // A cache is scoped to one immutable publication, not this trajectory.
        let movement = SharedMoves::default().movement(entity, self.baseline)?;
        let mut input = bytes::BytesMut::from(&movement.data[..]);
        let frame = mc_protocol::frame::try_decode_frame(&mut input, Compression::Threshold(256))
            .unwrap()
            .expect("one complete production movement frame");
        assert!(input.is_empty());
        assert_eq!(frame.id, movement.id);
        let mut body = &frame.body[..];
        let (id, on_ground, delta) = match frame.id {
            MoveEntityPos::ID => {
                let packet = MoveEntityPos::decode(&mut body).unwrap();
                (
                    packet.entity_id,
                    packet.on_ground,
                    Some([packet.delta_x, packet.delta_y, packet.delta_z]),
                )
            }
            MoveEntityPosRot::ID => {
                let packet = MoveEntityPosRot::decode(&mut body).unwrap();
                self.rotation =
                    [packet.yaw, packet.pitch].map(|angle| f32::from(angle) * 360.0 / 256.0);
                (
                    packet.entity_id,
                    packet.on_ground,
                    Some([packet.delta_x, packet.delta_y, packet.delta_z]),
                )
            }
            MoveEntityRot::ID => {
                let packet = MoveEntityRot::decode(&mut body).unwrap();
                self.rotation =
                    [packet.yaw, packet.pitch].map(|angle| f32::from(angle) * 360.0 / 256.0);
                (packet.entity_id, packet.on_ground, None)
            }
            EntityPositionSync::ID => {
                let packet = EntityPositionSync::decode(&mut body).unwrap();
                self.position = [packet.position.x, packet.position.y, packet.position.z];
                self.rotation = [packet.yaw, packet.pitch];
                (packet.entity_id, packet.on_ground, None)
            }
            id => panic!("unexpected movement packet {id}"),
        };
        assert_eq!(id, entity.id.0);
        assert_eq!(on_ground, entity.on_ground);
        assert!(body.is_empty());
        if let Some(delta) = delta {
            for (position, delta) in self.position.iter_mut().zip(delta) {
                if delta != 0 {
                    let units = *position * 4096.0;
                    // Independent nearest-integer formulation; Java's negative
                    // half ties differ from Rust's away-from-zero round().
                    let rounded = units.round() + f64::from(u8::from(units.fract() == -0.5));
                    *position = (rounded + f64::from(delta)) / 4096.0;
                }
            }
        }
        self.baseline = movement.next;
        Some(frame.id)
    }
}

#[test]
fn relative_movement_cycles_return_the_decoded_consumer_to_origin() {
    for origin in [
        [8.0, 69.0, 8.0],
        [-8.0, -69.0, -8.0],
        [29_999_990.0, 69.0, -29_999_990.0],
    ] {
        for rotate in [false, true] {
            let mut consumer = MovementConsumer::at(origin);
            let mut mob = entity(42, origin);
            let id = if rotate {
                MoveEntityPosRot::ID
            } else {
                MoveEntityPos::ID
            };
            let mut outbound = origin;
            for _ in 0..1024 {
                mob.position = [origin[0] + 0.01, origin[1] + 0.01, origin[2] - 0.01];
                mob.yaw = if rotate { 90.0 } else { 0.0 };
                assert_eq!(consumer.receive(&mob), Some(id));
                outbound = consumer.position;
                mob.position = origin;
                mob.yaw = 0.0;
                assert_eq!(consumer.receive(&mob), Some(id));
                assert_eq!(consumer.rotation, [0.0, 0.0]);
            }
            assert_eq!(consumer.position, origin, "1024 closed trajectories");
            assert_eq!(
                outbound,
                [
                    origin[0] + 41.0 / 4096.0,
                    origin[1] + 41.0 / 4096.0,
                    origin[2] - 41.0 / 4096.0,
                ],
                "last outbound endpoint"
            );
        }
    }
}

#[test]
fn relative_movement_rounds_endpoints_at_signed_half_step_boundaries() {
    // Values are in wire units; each motion exceeds the send threshold.
    // The nearest floats on either side of a half test ties without epsilon.
    for (from, to, expected) in [
        (-16.0, 0.5_f64.next_down(), 0.0),
        (-16.0, 0.5, 1.0),
        (-16.0, 0.5_f64.next_up(), 1.0),
        (16.0, (-0.5_f64).next_down(), -1.0),
        (16.0, -0.5, 0.0),
        (16.0, (-0.5_f64).next_up(), 0.0),
        (0.49, 16.51, 17.0),
        (0.51, 16.49, 16.0),
        (-0.49, -16.51, -17.0),
        (-0.51, -16.49, -16.0),
        (8192.5, 8208.5, 8209.0),
        (-8192.5, -8208.5, -8208.0),
    ] {
        let origin = [from / 4096.0, 69.0001, -8.0001];
        let mut consumer = MovementConsumer::at(origin);
        let mob = entity(42, [to / 4096.0, origin[1], origin[2]]);
        assert_eq!(consumer.receive(&mob), Some(MoveEntityPos::ID));
        assert_eq!(consumer.position, [expected / 4096.0, origin[1], origin[2]]);
    }
}

#[test]
fn relative_movement_preserves_unsent_components_and_accumulates_small_steps() {
    let origin = [8.0001, 69.0001, -8.0001];
    let mut consumer = MovementConsumer::at(origin);
    let mut mob = entity(42, origin);
    mob.position[0] += 0.001;
    mob.yaw = 90.0;
    assert_eq!(consumer.receive(&mob), Some(MoveEntityRot::ID));
    assert_eq!(consumer.position, origin);
    assert_eq!(consumer.baseline.0, origin);
    assert_eq!(consumer.rotation, [90.0, 0.0]);
    mob.position[0] = origin[0] + 0.002;
    assert_eq!(consumer.receive(&mob), None);
    assert_eq!(consumer.baseline.0, origin);
    mob.position[0] = origin[0] + 0.004;
    mob.yaw = 90.1;
    mob.pitch = 0.1;
    assert_eq!(consumer.receive(&mob), Some(MoveEntityPos::ID));
    assert_eq!(
        consumer.position,
        [8.0 + 17.0 / 4096.0, origin[1], origin[2]]
    );
    assert_eq!(consumer.rotation, [90.0, 0.0]);
    assert_eq!((consumer.baseline.1, consumer.baseline.2), (90.0, 0.0));
    mob.position = origin;
    mob.yaw = 0.0;
    mob.pitch = 0.0;
    assert_eq!(consumer.receive(&mob), Some(MoveEntityPosRot::ID));
    assert_eq!(consumer.position, [8.0, origin[1], origin[2]]);
    assert_eq!(consumer.rotation, [0.0, 0.0]);
}

#[test]
fn relative_movement_range_uses_quantized_endpoints_and_sync_resets_the_consumer() {
    for (from, to, relative) in [
        (0.49, 32767.49, true),
        (0.49, -32768.49, true),
        (0.49, 32767.51, false),
        (0.51, -32767.51, false),
    ] {
        for axis in 0..3 {
            let mut origin = [0.0, 0.0, 0.0];
            origin[axis] = from / 4096.0;
            let mut consumer = MovementConsumer::at(origin);
            let mut mob = entity(42, origin);
            mob.position[axis] = to / 4096.0;
            let id = if relative {
                MoveEntityPos::ID
            } else {
                EntityPositionSync::ID
            };
            assert_eq!(consumer.receive(&mob), Some(id));
            assert!((consumer.position[axis] - mob.position[axis]).abs() <= 0.5 / 4096.0);
        }
    }
    let origin = [8.0001, 69.0001, -8.0001];
    let remote = [100.12345, -80.0001, 0.0001];
    let mut consumer = MovementConsumer::at(origin);
    let mut mob = entity(42, remote);
    // Even sub-packed rotation is carried by absolute sync.
    mob.yaw = 0.1;
    mob.pitch = 0.1;
    assert_eq!(consumer.receive(&mob), Some(EntityPositionSync::ID));
    assert_eq!(consumer.position, remote);
    assert_eq!(consumer.rotation, [0.1, 0.1]);
    for target in [remote.map(|v| v + 0.01), remote] {
        mob.position = target;
        assert_eq!(consumer.receive(&mob), Some(MoveEntityPos::ID));
        for (actual, expected) in consumer.position.into_iter().zip(target) {
            assert!((actual - expected).abs() <= 0.5 / 4096.0);
        }
    }
    mob.position = origin;
    mob.yaw = 0.0;
    mob.pitch = 0.0;
    assert_eq!(consumer.receive(&mob), Some(EntityPositionSync::ID));
    assert_eq!(consumer.position, origin);
    assert_eq!(consumer.rotation, [0.0, 0.0]);
    mob.position[1] += 0.01;
    assert_eq!(consumer.receive(&mob), Some(MoveEntityPos::ID));
    mob.position = origin;
    assert_eq!(consumer.receive(&mob), Some(MoveEntityPos::ID));
    assert_eq!(consumer.position, [origin[0], 69.0, origin[2]]);
}
