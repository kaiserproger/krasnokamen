//! Entity position, rotation and velocity wire encoders.
//!
//! Exact-baseline move frames; the baseline stays in the parent so one
//! publish can share a frame between viewers with identical baselines.
use super::*;

/// Smallest squared entity move sent to clients (vanilla `ServerEntity`).
const MIN_MOVE_DISTANCE_SQ: f64 = 7.629_394_5e-6;

/// Local 26.1.2 javap: leash knots and item frames publish their BlockPos,
/// not the offset internal rendering position.
pub(super) fn entity_spawn_position(entity: &EntityView) -> [f64; 3] {
    if entity_type_flags::is_leash_knot(entity.type_id)
        || entity_type_flags::is_item_frame(entity.type_id)
    {
        entity.position.map(f64::floor)
    } else {
        entity.position
    }
}

/// One entity movement packet against the client's baseline.
pub(super) enum MovePacket {
    Pos(MoveEntityPos),
    Rot(MoveEntityRot),
    PosRot(MoveEntityPosRot),
    Sync(EntityPositionSync),
}

#[cfg(test)]
impl MovePacket {
    pub(super) fn push(&self, out: &mut Vec<PlannedPacket>, conn: ConnectionId) {
        match self {
            Self::Pos(packet) => push_best_effort(out, conn, packet),
            Self::Rot(packet) => push_best_effort(out, conn, packet),
            Self::PosRot(packet) => push_best_effort(out, conn, packet),
            Self::Sync(packet) => push_best_effort(out, conn, packet),
        }
    }
}

/// The move `entity` needs relative to `baseline`, with the baseline the
/// client holds once it is delivered; `None` below vanilla `ServerEntity`
/// thresholds (a move under ~0.003 blocks or a turn within one packed angle
/// step accumulates against the unchanged baseline until it is sent).
pub(super) fn entity_movement(
    entity: &EntityView,
    baseline: MoveBaseline,
) -> Option<(MovePacket, MoveBaseline)> {
    let (last_pos, last_yaw, last_pitch, _) = baseline;
    let dx = entity.position[0] - last_pos[0];
    let dy = entity.position[1] - last_pos[1];
    let dz = entity.position[2] - last_pos[2];
    let moved = dx * dx + dy * dy + dz * dz >= MIN_MOVE_DISTANCE_SQ;
    let rotated = pack_degrees(entity.yaw) != pack_degrees(last_yaw)
        || pack_degrees(entity.pitch) != pack_degrees(last_pitch);
    if !moved && !rotated {
        return None;
    }
    let rel = if moved {
        (
            relative_move_delta(entity.position[0], last_pos[0]),
            relative_move_delta(entity.position[1], last_pos[1]),
            relative_move_delta(entity.position[2], last_pos[2]),
        )
    } else {
        (Some(0), Some(0), Some(0))
    };
    let fits = rel.0.is_some() && rel.1.is_some() && rel.2.is_some();
    let packet = match (moved, rotated, fits) {
        (true, false, true) => MovePacket::Pos(MoveEntityPos {
            entity_id: entity.id.0,
            delta_x: rel.0.unwrap_or(0),
            delta_y: rel.1.unwrap_or(0),
            delta_z: rel.2.unwrap_or(0),
            on_ground: entity.on_ground,
        }),
        (false, true, _) => MovePacket::Rot(MoveEntityRot {
            entity_id: entity.id.0,
            yaw: pack_degrees(entity.yaw),
            pitch: pack_degrees(entity.pitch),
            on_ground: entity.on_ground,
        }),
        (true, true, true) => MovePacket::PosRot(MoveEntityPosRot {
            entity_id: entity.id.0,
            delta_x: rel.0.unwrap_or(0),
            delta_y: rel.1.unwrap_or(0),
            delta_z: rel.2.unwrap_or(0),
            yaw: pack_degrees(entity.yaw),
            pitch: pack_degrees(entity.pitch),
            on_ground: entity.on_ground,
        }),
        _ => MovePacket::Sync(EntityPositionSync {
            entity_id: entity.id.0,
            position: EntityVec3 {
                x: entity.position[0],
                y: entity.position[1],
                z: entity.position[2],
            },
            yaw: entity.yaw,
            pitch: entity.pitch,
            on_ground: entity.on_ground,
        }),
    };
    // Keep the last sent authoritative endpoint: relative moves subtract its
    // quantized value, not the raw displacement. Position-only moves preserve
    // rotation and rotation-only moves preserve position; sync carries both.
    let synced = moved && !fits;
    let position = if moved { entity.position } else { last_pos };
    let (yaw, pitch) = if rotated || synced {
        (entity.yaw, entity.pitch)
    } else {
        (last_yaw, last_pitch)
    };
    Some((packet, (position, yaw, pitch, entity.head_yaw)))
}

/// Core velocities are blocks per second; the wire carries blocks per tick.
pub(super) fn per_tick(velocity: [f64; 3]) -> EntityVec3 {
    EntityVec3 {
        x: velocity[0] / 20.0,
        y: velocity[1] / 20.0,
        z: velocity[2] / 20.0,
    }
}

/// `value.floor() as i32` (saturating, NaN to zero) without a libm call.
///
/// Bottleneck: baseline x86-64 has no `roundsd`, so `f64::floor` is a `floor` call, and `mul_add` an
/// `fmaf` call; per-entity-per-viewer planning made them 11 % of the fanout threads' self time on
/// native-100 (FP perf leg, window c). Fence: bit-identical to the floor form for every input
/// (`floor_matches_the_libm_form`). Removal path: none needed; delete when the build baseline gains SSE4.1.
#[inline]
pub(super) fn floor_i32(value: f64) -> i32 {
    let integer = value as i32;
    integer.saturating_sub(i32::from(value < f64::from(integer)))
}

/// [`floor_i32`] for `f32`.
#[inline]
pub(super) fn floor_f32_i32(value: f32) -> i32 {
    let integer = value as i32;
    integer.saturating_sub(i32::from(value < integer as f32))
}

/// Vanilla `Mth.packDegrees`: floor(yaw * 256 / 360) as a byte.
///
/// `degrees * 256.0` equals `fma(degrees, 256.0, 0.0)` for every input: the product is a power-of-two
/// scaling, exact unless it overflows (both give infinity), and the sign of a zero is lost to the floor.
pub(super) fn pack_degrees(degrees: f32) -> u8 {
    floor_f32_i32(degrees * 256.0 / 360.0) as u8
}

/// Vanilla VecDeltaCodec (775–777): subtract endpoints rounded to 1/4096
/// blocks, with ties toward positive infinity. Retain the raw baseline for
/// movement thresholds; an out-of-range difference needs absolute sync.
fn relative_move_delta(position: f64, baseline: f64) -> Option<i16> {
    let quantize = |coordinate: f64| {
        let scaled = coordinate * 4096.0;
        let lower = scaled.floor();
        // Adding 0.5 before flooring would round the float immediately below
        // a positive half-step up as well. Compare the fraction instead.
        lower as i64 + i64::from(scaled - lower >= 0.5)
    };
    i16::try_from(quantize(position).checked_sub(quantize(baseline))?).ok()
}
