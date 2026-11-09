use super::*;
use crate::mounts::tests::{fixture, slot, spawn};

#[test]
fn fence_click_matches_dense_oracle_and_detaches_only_its_knot() {
    let (mut state, actor, pos) = fixture();
    let fence_pos = BlockPos {
        x: pos[0] as i32 + 2,
        y: pos[1] as i32,
        z: pos[2] as i32,
    };
    let fence = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:oak_fence").unwrap())
        .unwrap()
        .default
        .0;
    state
        .world
        .set_block(fence_pos.x, fence_pos.y, fence_pos.z, fence);
    let actor_uuid = state.entities.row(slot(&state, actor)).uuid;
    let mut animals = Vec::new();
    for (offset, holder) in [
        (0.0, Some(Holder::Entity(actor_uuid))),
        (4.0, Some(Holder::Entity(actor_uuid))),
        (20.0, Some(Holder::Entity(actor_uuid))),
        (1.0, Some(Holder::Entity(Uuid::from_u128(7)))),
        (2.0, None),
    ] {
        let cow = spawn(&mut state, "cow", [pos[0] + offset, pos[1], pos[2]]);
        let s = slot(&state, cow);
        state.entities.set_leash(s, holder);
        animals.push(cow);
    }
    let expected: Vec<_> = state
        .entities
        .slots()
        .filter(|&s| {
            let at = state.entities.pos(s);
            state.entities.row(s).entity_life.leash == Some(Holder::Entity(actor_uuid))
                && (at[0] - f64::from(fence_pos.x)).powi(2)
                    + (at[1] - f64::from(fence_pos.y)).powi(2)
                    + (at[2] - f64::from(fence_pos.z)).powi(2)
                    <= 144.0
        })
        .map(|s| state.entities.body(s).id)
        .collect();
    assert_eq!(
        would_use_fence(&state, actor, fence_pos),
        !expected.is_empty()
    );
    assert!(use_fence(&mut state, actor, fence_pos));
    let fence_holder = Some(Holder::Fence([fence_pos.x, fence_pos.y, fence_pos.z]));
    for cow in &animals {
        assert_eq!(
            state.entities.row(slot(&state, *cow)).entity_life.leash == fence_holder,
            expected.contains(cow)
        );
    }
    let mut changes = ChangeSet::default();
    tick(&mut state, &mut changes);
    let knot = changes.entities_spawned[0];
    let knot_slot = slot(&state, knot);
    assert!(knot_interact(&mut state, knot_slot));
    for cow in expected {
        assert_eq!(
            state.entities.row(slot(&state, cow)).entity_life.leash,
            None
        );
    }
}

#[test]
#[ignore = "measurement workload; run in the exclusive lane"]
fn measure_large_population_fence_preview() {
    use std::hint::black_box;
    use std::time::Instant;
    let (mut state, actor, pos) = fixture();
    let p = BlockPos {
        x: pos[0] as i32 + 2,
        y: pos[1] as i32,
        z: pos[2] as i32,
    };
    let fence = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:oak_fence").unwrap())
        .unwrap()
        .default
        .0;
    state.world.set_block(p.x, p.y, p.z, fence);
    for _ in 0..10_000 {
        spawn(&mut state, "cow", pos);
    }
    let uuid = state.entities.row(slot(&state, actor)).uuid;
    let start = Instant::now();
    for _ in 0..100 {
        black_box(
            state
                .entities
                .slots()
                .any(|s| state.entities.row(s).entity_life.leash == Some(Holder::Entity(uuid))),
        );
    }
    let dense = start.elapsed();
    let start = Instant::now();
    for _ in 0..100 {
        black_box(would_use_fence(&state, actor, p));
    }
    eprintln!(
        "entities=10000 leashed=0 iterations=100 dense={dense:?} indexed={:?}",
        start.elapsed()
    );
}
