//! Rest proofs: whether a row's feet, footprint and wander physics are
//! provably unchanged since the last committed tick. Byte-exact move out of
//! `entity_phase.rs`; the proof rules are unchanged.

use super::*;

pub(super) fn unchanged_entity_footprint(
    pos: [f64; 3],
    half_width: f64,
    chunk_revision: impl Fn(i32, i32) -> u64,
    pending: &[ChunkPos],
    unchanged_since: u64,
) -> bool {
    let half = half_width + 0.001;
    let chunk = |v: f64| mc_physics::mth::floor_i32(v).div_euclid(16);
    let (x0, x1) = (chunk(pos[0] - half), chunk(pos[0] + half));
    let (z0, z1) = (chunk(pos[2] - half), chunk(pos[2] + half));
    (x0..=x1).all(|cx| {
        (z0..=z1).all(|cz| {
            chunk_revision(cx, cz) <= unchanged_since
                && !pending.contains(&ChunkPos { x: cx, z: cz })
        })
    })
}

pub(super) fn motion_thinks(tick: u64, flags: u16, period: u8) -> bool {
    let phase = ((flags & MOTION_PHASE_MASK) >> MOTION_PHASE_SHIFT) as u8;
    (tick as u8).wrapping_add(phase) & (period - 1) == 0
}

/// Rest before the wide snapshot. Decision ticks additionally prove that the
/// final wander branch's complete motion step is bit-identical to the input.
pub(super) fn early_entity_rest(
    view: &PrepView,
    slot: EntitySlot,
    period: u8,
    unchanged_since: u64,
    special_idle: bool,
    targets_empty: bool,
) -> bool {
    let flags = view.entities.motion_flags(slot);
    let battle = crate::battle::battle_row(view.bodies[slot.0 as usize].faction);
    let battle_thrust = view
        .hostile
        .battle
        .columns
        .drive_by_slot
        .get(slot.0 as usize)
        .is_some_and(|drive| *drive != [0.0; 3]);
    if battle && battle_thrust
        || flags & MOTION_REST == 0
        || matches!(view.bodies[slot.0 as usize].kind, EntityKind::Mob { type_id } if crate::goat::is_goat(type_id))
            && crate::goat::drive(view.entities.row(slot)).is_some()
        || matches!(view.bodies[slot.0 as usize].kind, EntityKind::Mob { type_id } if crate::turtle::is_turtle(type_id))
            && crate::turtle::drive(view.entities.row(slot)).is_some()
        || matches!(view.bodies[slot.0 as usize].kind, EntityKind::Mob { type_id } if crate::frog::is_frog(type_id))
            && crate::frog::drive(view.entities.row(slot)).is_some()
        || matches!(view.bodies[slot.0 as usize].kind, EntityKind::Mob { type_id } if crate::cat::is_cat(type_id))
            && crate::cat::drive(
                view.entities.row(slot).entity_life.cat_relax.as_ref(),
                view.entities.row(slot).speed,
            )
            .is_some()
        || view
            .entities
            .row(slot)
            .entity_life
            .sniffer
            .as_ref()
            .is_some_and(|s| s.target.is_some())
        || !special_idle
        || (!battle
            && motion_thinks(view.tick, flags, period)
            && flags & crate::entities::MOTION_WANDER_REST == 0
            && !(targets_empty && flags & crate::entities::MOTION_TARGETLESS_WANDER_REST != 0))
        || (flags & MOTION_EFFECTS != 0
            && view
                .entities
                .row(slot)
                .active_effects
                .has(crate::effects::EffectId::Levitation))
        || !view.ground[slot.0 as usize]
        || view.velocities[slot.0 as usize] != [0.0; 3]
    {
        return false;
    }
    let body = &view.bodies[slot.0 as usize];
    if view.hostile.targets.contains_key(&body.id)
        || body
            .hurt_at
            .is_some_and(|at| view.tick.saturating_sub(at) < 100)
    {
        return false;
    }
    if !battle
        && motion_thinks(view.tick, flags, period)
        && let EntityKind::Mob { type_id } = body.kind
        && view.registry.blueprint(type_id).is_some_and(|blueprint| {
            (blueprint.capabilities.temptable && view.has_tempting_items)
                || (blueprint.capabilities.follows_adults
                    && !crate::entity_life::adult(view.entities.row(slot)))
        })
    {
        // Player food and adult positions may change inside the same chunk.
        // Decision ticks must observe them even when geometry is unchanged.
        return false;
    }
    if !battle && motion_thinks(view.tick, flags, period) {
        // The proof samples current geometry, so unrelated edits anywhere in
        // the footprint's chunk need not reject an otherwise exact no-op.
        return idle_rest_proved(view, slot, flags);
    }
    unchanged_entity_footprint(
        view.positions[slot.0 as usize],
        body.half_width,
        |cx, cz| view.world.chunk_revision(cx, cz),
        view.world.pending_block_chunks(),
        unchanged_since,
    )
}

/// Every input of the idle motion step except the blocks and the tick, folded
/// into one 64-bit key (see [`RestProofs`]).
fn rest_proof_key(view: &PrepView, slot: EntitySlot, flags: u16) -> u64 {
    use std::hash::Hasher;
    let index = slot.0 as usize;
    let body = &view.bodies[index];
    let row = view.entities.row(slot);
    let mut hash = rustc_hash::FxHasher::default();
    hash.write_i32(body.id.0);
    if let EntityKind::Mob { type_id } = body.kind {
        hash.write_u32(type_id);
    }
    hash.write_u8(body.movement as u8);
    hash.write_i32(body.air_supply);
    hash.write_u64(body.half_width.to_bits());
    hash.write_u64(body.height.to_bits());
    for value in view.positions[index].iter().chain(&view.velocities[index]) {
        hash.write_u64(value.to_bits());
    }
    for value in view.impulses[index] {
        hash.write_u64(value.to_bits());
    }
    for value in view.rotations[index] {
        hash.write_u32(value.to_bits());
    }
    hash.write_u8(u8::from(view.ground[index]) | u8::from(row.controls.no_gravity) << 1);
    if flags & MOTION_EFFECTS != 0 {
        hash.write_u32(
            row.active_effects
                .level(crate::effects::EffectId::Levitation)
                .to_bits(),
        );
        hash.write_u8(u8::from(
            row.active_effects
                .has(crate::effects::EffectId::SlowFalling),
        ));
    }
    hash.finish()
}

/// [`idle_wander_motion_unchanged`] through the [`RestProofs`] memo.
fn idle_rest_proved(view: &PrepView, slot: EntitySlot, flags: u16) -> bool {
    let index = slot.0 as usize;
    let body = &view.bodies[index];
    let Some(proofs) = view
        .rest_proofs
        .filter(|_| body.movement != crate::blueprint::MovementClass::Swimmer)
    else {
        return idle_wander_motion_unchanged(view, slot);
    };
    if crate::entity_motion::wander(
        body.movement,
        body.id.0,
        view.tick,
        view.entities.row(slot).speed,
    ) != [0.0; 3]
    {
        return false;
    }
    let key = rest_proof_key(view, slot, flags);
    let pos = view.positions[index];
    let pending = view.world.pending_block_chunks();
    if let Some((stamped, revision)) = proofs.get(slot)
        && stamped == key
        && unchanged_entity_footprint(
            pos,
            body.half_width,
            |cx, cz| view.world.chunk_revision(cx, cz),
            pending,
            revision,
        )
    {
        return true;
    }
    let proved = idle_wander_motion_unchanged(view, slot);
    // A pending write is visible to the sampler but not yet in any revision.
    if proved && unchanged_entity_footprint(pos, body.half_width, |_, _| 0, pending, 0) {
        proofs.set(slot, key);
    }
    proved
}

#[cfg(test)]
thread_local! {
    /// Full physics proofs run on this thread (memo hits do not count).
    pub(super) static REST_PHYSICS_PROOFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Keep the same physics, breathing and effect integration as `step_entity`.
/// In particular, an imported on-ground flag is not itself a support proof.
pub(super) fn idle_wander_motion_unchanged(view: &PrepView, slot: EntitySlot) -> bool {
    #[cfg(test)]
    REST_PHYSICS_PROOFS.with(|runs| runs.set(runs.get() + 1));
    let body = &view.bodies[slot.0 as usize];
    let row = view.entities.row(slot);
    if crate::entity_motion::wander(body.movement, body.id.0, view.tick, row.speed) != [0.0; 3] {
        return false;
    }
    let pos = view.positions[slot.0 as usize];
    let vel = view.velocities[slot.0 as usize];
    let rotation = view.rotations[slot.0 as usize];
    let physics_body = mc_physics::EntityBody {
        position: mc_physics::Vec3::new(pos[0], pos[1], pos[2]),
        velocity: mc_physics::Vec3::new(vel[0], vel[1], vel[2]),
        aabb: mc_physics::Aabb {
            half_width: body.half_width,
            height: body.height,
        },
        on_ground: view.ground[slot.0 as usize],
    };
    let reader = view.world.physics_reader();
    let sampler = WorkerSampler {
        block_at: &|x, y, z| reader.block_at(x, y, z),
        registry: view.registry,
    };
    let capabilities = match body.kind {
        EntityKind::Mob { type_id } => view
            .registry
            .blueprint(type_id)
            .map(|blueprint| blueprint.capabilities)
            .unwrap_or_default(),
        _ => return false,
    };
    let effects =
        (view.entities.motion_flags(slot) & MOTION_EFFECTS != 0).then_some(&row.active_effects);
    let result = crate::entity_motion::step(
        crate::entity_motion::MotionInput {
            body: physics_body,
            movement: body.movement,
            drive: [0.0; 3],
            impulse: view.impulses[slot.0 as usize],
            rotation,
            look_target: None,
            battle_facing: false,
            climbs_walls: capabilities.climbs_walls,
            aquatic_control: capabilities.aquatic,
            air_supply: body.air_supply,
            levitation: effects.map_or(0.0, |effects| {
                effects.level(crate::effects::EffectId::Levitation)
            }),
            slow_falling: effects
                .is_some_and(|effects| effects.has(crate::effects::EffectId::SlowFalling)),
            no_gravity: row.controls.no_gravity,
            id: body.id.0,
            tick: view.tick,
        },
        &sampler,
        |x, z| view.world.has_chunk(x.div_euclid(16), z.div_euclid(16)),
    );
    result.body.position.x.to_bits() == pos[0].to_bits()
        && result.body.position.y.to_bits() == pos[1].to_bits()
        && result.body.position.z.to_bits() == pos[2].to_bits()
        && result.body.velocity.x.to_bits() == vel[0].to_bits()
        && result.body.velocity.y.to_bits() == vel[1].to_bits()
        && result.body.velocity.z.to_bits() == vel[2].to_bits()
        && result.body.on_ground == physics_body.on_ground
        && result.rotation.map(f32::to_bits) == rotation.map(f32::to_bits)
        && result.impulse.map(f64::to_bits) == view.impulses[slot.0 as usize].map(f64::to_bits)
        && result.air_supply == body.air_supply
        && !result.drowning
}
