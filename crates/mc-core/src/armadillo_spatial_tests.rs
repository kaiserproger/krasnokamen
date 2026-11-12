use super::*;
use std::cell::Cell;

thread_local! { pub(super) static VISITS: Cell<usize> = const { Cell::new(0) }; }

fn spawn(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntitySlot {
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(name).unwrap())
        .unwrap();
    let id = state.spawn_mob(kind, pos, 20.0);
    state.entities.slot_of(id).unwrap()
}

fn exhaustive(state: &TickState, own: EntitySlot, at: [f64; 3]) -> bool {
    let hurt = state.entities.row(own).last_player_hit.map(|(id, _)| id);
    state
        .entities
        .slots()
        .any(|slot| scares(state, own, at, hurt, slot))
}

#[test]
fn threat_oracle_seams_negative_coordinates_and_state_changes() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let own = spawn(&mut state, "minecraft:armadillo", [-16.0, 80.0, -0.5]);
    let threat = spawn(&mut state, "minecraft:zombie", [100.0, 80.0, 100.0]);
    for center in [[-16.0, 80.0, -0.5], [15.9, 80.0, 16.0], [-0.1, 80.0, -16.0]] {
        state.entities.set_pos(own, center);
        for offset in [
            [7.0, 2.0, -7.0],
            [-7.0, -2.0, 7.0],
            [7.001, 0.0, 0.0],
            [0.0, 2.001, 0.0],
        ] {
            state
                .entities
                .set_pos(threat, std::array::from_fn(|i| center[i] + offset[i]));
            assert_eq!(
                threat_near(&state, own, center),
                exhaustive(&state, own, center)
            );
            assert_eq!(
                threat_near(&state, own, center),
                offset[0].abs() <= 7.0 && offset[1].abs() <= 2.0
            );
        }
    }
    let at = state.entities.pos(own);
    state.entities.set_pos(threat, at);
    state.entities.body_mut(threat).alive = false;
    assert!(!threat_near(&state, own, at));
    let cow = spawn(&mut state, "minecraft:cow", at);
    state.entities.row_mut(own).last_player_hit = Some((state.entities.body(cow).id, state.tick));
    assert!(threat_near(&state, own, at));
    state.entities.row_mut(own).last_player_hit = None;
    let player = crate::mob_lanes::tests::join(&mut state);
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(slot, at);
    state.players.get_mut(&player).unwrap().sprinting = true;
    assert!(threat_near(&state, own, at));
    assert_eq!(threat_near(&state, own, at), exhaustive(&state, own, at));
    state.players.get_mut(&player).unwrap().sprinting = false;
    assert!(!threat_near(&state, own, at));
}

#[test]
fn threat_query_does_not_visit_remote_population() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let at = [-0.5, 80.0, -16.0];
    let own = spawn(&mut state, "minecraft:armadillo", at);
    for i in 0..4096 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    VISITS.set(0);
    assert!(!threat_near(&state, own, at));
    assert!(VISITS.get() <= 1, "visited {} candidates", VISITS.get());
}

#[test]
#[ignore = "focused A/B cost measurement; run in mem-run -x"]
fn threat_focused_cost() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let at = [-0.5, 80.0, -16.0];
    let own = spawn(&mut state, "minecraft:armadillo", at);
    for i in 0..50_000 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    for round in 0..4 {
        for old in [true, false] {
            VISITS.set(0);
            let started = std::time::Instant::now();
            for _ in 0..32 {
                std::hint::black_box(if old {
                    exhaustive(&state, own, at)
                } else {
                    threat_near(&state, own, at)
                });
            }
            eprintln!(
                "S04 N=50001 A=1 round={round} exhaustive={old} ns/event={} candidates/event={}",
                started.elapsed().as_nanos() / 32,
                VISITS.get() / 32
            );
        }
    }
}
