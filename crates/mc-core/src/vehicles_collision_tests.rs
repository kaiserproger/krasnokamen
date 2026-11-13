use super::*;

/// The former full slot scan of `motion::entity_collisions`, applied to plain
/// arrays: the reference the chunk-indexed candidate walk must reproduce exactly.
fn full_scan_reference(
    state: &TickState,
    slot: crate::entities::EntitySlot,
) -> (Vec<[f64; 3]>, Vec<[f64; 2]>) {
    use crate::entities::{EntityKind, EntitySlot};
    let mut vel: Vec<[f64; 3]> = (0..state.entities.len())
        .map(|i| state.entities.vel(EntitySlot(i as u32)))
        .collect();
    let mut impulse: Vec<[f64; 2]> = (0..state.entities.len())
        .map(|i| state.entities.horizontal_impulse(EntitySlot(i as u32)))
        .collect();
    let p = state.entities.pos(slot);
    let body = state.entities.body(slot);
    let (width, height) = (body.half_width, body.height);
    let rider = crate::mounts::rider(state, slot);
    for i in 0..state.entities.len() {
        let other = EntitySlot(i as u32);
        if other == slot || Some(other) == rider {
            continue;
        }
        let body = state.entities.body(other);
        if !body.alive
            || matches!(
                body.kind,
                EntityKind::Item { .. } | EntityKind::ExperienceOrb { .. }
            )
        {
            continue;
        }
        let q = state.entities.pos(other);
        let (dx, dz) = (q[0] - p[0], q[2] - p[2]);
        if dx.abs() > width + body.half_width
            || dz.abs() > width + body.half_width
            || q[1] >= p[1] + height
            || p[1] >= q[1] + body.height
        {
            continue;
        }
        let length = mc_physics::mth::length2(dx, dz);
        if length < 1e-6 {
            continue;
        }
        let push = [dx / length * 0.05, dz / length * 0.05];
        vel[slot.0 as usize][0] -= push[0] * 20.0;
        vel[slot.0 as usize][2] -= push[1] * 20.0;
        vel[i][0] += push[0] * 20.0;
        vel[i][2] += push[1] * 20.0;
        impulse[i] = [
            impulse[i][0] + push[0] * 20.0,
            impulse[i][1] + push[1] * 20.0,
        ];
    }
    (vel, impulse)
}

/// Negative control: narrowing the chunk range in `entity_collisions` (for
/// example to the vehicle's own chunk) misses the neighbours across chunk
/// borders, in negative coordinates and the wide ghast, and this goes red.
#[test]
fn entity_collisions_chunk_index_matches_full_scan() {
    let mut state = fixture();
    // The boat sits on a chunk corner so overlapping bodies live in all four chunks.
    let boat = mob(&mut state, "oak_boat", [15.95, 65.0, -0.05]);
    let kinds = ["cow", "zombie", "chicken", "ghast", "sheep"];
    let mut seed = 0x2545_F491_4F6C_DD1D_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1_u64 << 53) as f64
    };
    for index in 0..400 {
        let at = [
            15.95 + (next() - 0.5) * 5.0,
            64.5 + next() * 1.5,
            -0.05 + (next() - 0.5) * 5.0,
        ];
        mob(&mut state, kinds[index % kinds.len()], at);
    }
    // Far bodies and a dead overlapping body never move the boat.
    mob(&mut state, "cow", [15.95 + 90.0, 65.0, -0.05]);
    let dead = mob(&mut state, "cow", [15.5, 65.0, -0.2]);
    let dead_slot = state.entities.slot_of(dead).unwrap();
    state.entities.body_mut(dead_slot).alive = false;

    let slot = state.entities.slot_of(boat).unwrap();
    let (expected_vel, expected_impulse) = full_scan_reference(&state, slot);
    crate::vehicles::motion::entity_collisions(&mut state, slot);
    let touched = (0..state.entities.len())
        .filter(|&i| expected_vel[i] != [0.0; 3])
        .count();
    assert!(touched > 20, "fixture must overlap many bodies: {touched}");
    for i in 0..state.entities.len() {
        let s = crate::entities::EntitySlot(i as u32);
        assert_eq!(
            state.entities.vel(s),
            expected_vel[i],
            "velocity of slot {i}"
        );
        assert_eq!(
            state.entities.horizontal_impulse(s),
            expected_impulse[i],
            "impulse of slot {i}"
        );
    }
}
