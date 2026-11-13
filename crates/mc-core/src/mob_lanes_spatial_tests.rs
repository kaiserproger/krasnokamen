use super::*;
use std::cell::Cell;

thread_local! { pub(super) static VISITS: Cell<usize> = const { Cell::new(0) }; }

fn spawn(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntitySlot {
    let kind = state
        .registry
        .entity_types
        .id_of(&Identifier::parse(name).unwrap())
        .unwrap();
    let id = state.spawn_mob(kind, pos, 20.0);
    state.entities.slot_of(id).unwrap()
}

fn exhaustive(
    state: &TickState,
    at: [f64; 3],
    muzzle: [f64; 3],
) -> Option<(EntitySlot, [f64; 3], f64)> {
    state
        .entities
        .slots()
        .filter_map(|slot| snowball_candidate(state, at, muzzle, slot))
        .min_by(|a, b| a.2.total_cmp(&b.2).then_with(|| a.0.cmp(&b.0)))
}

#[test]
fn snowball_oracle_seams_ties_occlusion_and_compaction() {
    let mut state = tests::fixture("minecraft:plains");
    // The negative seam target crosses into chunk -2; unloaded space correctly
    // blocks sight, so load the whole oracle scene before expecting a hit.
    state.world.ensure_blocking(
        &(-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| (crate::boundary::ChunkPos { x, z }, 0)))
            .collect(),
    );
    let first = spawn(&mut state, "minecraft:zombie", [1000.0, 80.0, 1000.0]);
    let second = spawn(&mut state, "minecraft:zombie", [1000.0, 80.0, 1000.0]);
    for at in [[-0.5, 80.0, -0.5], [15.5, 80.0, 15.5], [-15.5, 80.0, -15.5]] {
        let muzzle = [at[0], at[1] + 1.5, at[2]];
        state.entities.set_pos(first, [at[0] + 4.0, at[1], at[2]]);
        state.entities.set_pos(second, [at[0] - 4.0, at[1], at[2]]);
        assert_eq!(
            snowball_target(&state, at, muzzle),
            exhaustive(&state, at, muzzle)
        );
        assert_eq!(snowball_target(&state, at, muzzle).unwrap().0, first);
        state.entities.body_mut(first).alive = false;
        assert_eq!(snowball_target(&state, at, muzzle).unwrap().0, second);
        state.entities.body_mut(first).alive = true;
        for offset in [10.0, 10.001] {
            state
                .entities
                .set_pos(first, [at[0] + offset, at[1], at[2]]);
            state.entities.set_pos(second, [1000.0, 80.0, 1000.0]);
            assert_eq!(
                snowball_target(&state, at, muzzle),
                exhaustive(&state, at, muzzle)
            );
            assert_eq!(
                snowball_target(&state, at, muzzle).is_some(),
                offset == 10.0
            );
        }
    }
    let at = [0.5, 80.0, 0.5];
    let muzzle = [0.5, 81.5, 0.5];
    state.entities.set_pos(first, [4.5, 80.0, 0.5]);
    let stone = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:stone").unwrap())
        .unwrap()
        .default
        .0;
    for y in 80..=83 {
        state.world.set_block(2, y, 0, stone);
    }
    assert_eq!(
        snowball_target(&state, at, muzzle),
        exhaustive(&state, at, muzzle)
    );
    assert!(snowball_target(&state, at, muzzle).is_none());
    state.remove_entity(first);
    state.entities.set_pos(EntitySlot(0), [-3.5, 80.0, 0.5]);
    assert_eq!(
        snowball_target(&state, at, muzzle).unwrap().0,
        EntitySlot(0)
    );
}

#[test]
fn snowball_query_does_not_visit_remote_population() {
    let mut state = tests::fixture("minecraft:plains");
    for i in 0..4096 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    VISITS.set(0);
    assert!(snowball_target(&state, [-0.5, 80.0, -16.0], [-0.5, 81.5, -16.0]).is_none());
    assert_eq!(VISITS.get(), 0);
}

#[test]
#[ignore = "focused A/B cost measurement; run in mem-run -x"]
fn snowball_and_trader_focused_cost() {
    let mut state = tests::fixture("minecraft:plains");
    for i in 0..50_000 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    let at = [-0.5, 80.0, -16.0];
    let muzzle = [-0.5, 81.5, -16.0];
    for round in 0..4 {
        for old in [true, false] {
            VISITS.set(0);
            let started = std::time::Instant::now();
            for _ in 0..32 {
                std::hint::black_box(if old {
                    exhaustive(&state, at, muzzle)
                } else {
                    snowball_target(&state, at, muzzle)
                });
            }
            eprintln!(
                "S06 N=50000 G=1 round={round} exhaustive={old} ns/event={} candidates/event={}",
                started.elapsed().as_nanos() / 32,
                VISITS.get() / 32
            );
        }
        let started = std::time::Instant::now();
        for _ in 0..32 {
            std::hint::black_box(due_traders(&state));
        }
        eprintln!(
            "S05 N=50000 T=0 round={round} current ns/tick={}",
            started.elapsed().as_nanos() / 32
        );
    }
}
