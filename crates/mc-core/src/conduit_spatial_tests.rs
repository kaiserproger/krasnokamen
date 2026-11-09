use super::*;
use std::cell::Cell;

thread_local! {
    pub(super) static VISITS: Cell<usize> = const { Cell::new(0) };
    pub(super) static PLAYER_VISITS: Cell<usize> = const { Cell::new(0) };
}

fn block(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse(name).unwrap())
        .unwrap()
        .default
        .0
}

fn spawn(state: &mut TickState, pos: [f64; 3]) -> EntitySlot {
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:zombie").unwrap())
        .unwrap();
    let id = state.spawn_mob(kind, pos, 20.0);
    state.entities.slot_of(id).unwrap()
}

fn soak(state: &mut TickState, slot: EntitySlot, at: [f64; 3]) {
    state.entities.set_pos(slot, at);
    let [x, y, z] = at.map(|v| v.floor() as i32);
    let water = block(state, "minecraft:water");
    state.world.set_block(x, y, z, water);
    state.world.set_block(x, y + 1, z, water);
}

fn exhaustive(state: &TickState, pos: BlockPos) -> Option<EntitySlot> {
    state.entities.slots().filter(|&slot| {
        VISITS.set(VISITS.get()+1);
        let body = state.entities.body(slot);
        body.alive && distance(pos,state.entities.pos(slot)) < 64.0
            && wet(state,state.entities.pos(slot))
            && matches!(body.kind,EntityKind::Mob { type_id } if state.registry.blueprint(type_id).is_some_and(|b| b.category == crate::blueprint::MobCategory::Monster))
    }).min_by_key(|&slot| state.entities.body(slot).id)
}

#[test]
fn conduit_oracle_floor_distance_seams_and_lowest_id() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let first = spawn(&mut state, [1000.0, 80.0, 1000.0]);
    let second = spawn(&mut state, [1000.0, 80.0, 1000.0]);
    for pos in [
        BlockPos { x: 0, y: 80, z: 0 },
        BlockPos {
            x: 16,
            y: 80,
            z: 16,
        },
        BlockPos {
            x: -8,
            y: 80,
            z: -8,
        },
    ] {
        let at = [pos.x as f64, pos.y as f64, pos.z as f64];
        soak(&mut state, first, [at[0] + 7.999, at[1], at[2]]);
        soak(&mut state, second, [at[0] + 1.0, at[1], at[2]]);
        assert_eq!(attack_target(&state, pos), exhaustive(&state, pos));
        assert_eq!(attack_target(&state, pos), Some(first));
        state.entities.body_mut(first).alive = false;
        assert_eq!(attack_target(&state, pos), Some(second));
        state.entities.body_mut(first).alive = true;
        state.entities.set_pos(second, [1000.0, 80.0, 1000.0]);
        for offset in [-8.001, -8.0, -7.999, -7.0, 7.999, 8.0] {
            soak(&mut state, first, [at[0] + offset, at[1], at[2]]);
            assert_eq!(attack_target(&state, pos), exhaustive(&state, pos));
            assert_eq!(
                attack_target(&state, pos).is_some(),
                offset > -7.999 && offset < 8.0
            );
        }
    }
}

#[test]
fn conduit_queries_exclude_remote_population_and_deliver_power() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let actor = crate::mob_lanes::tests::join(&mut state);
    let player = state.entities.slot_of(actor).unwrap();
    let pos = BlockPos { x: 0, y: 82, z: 0 };
    let water = block(&state, "minecraft:water");
    let prismarine = block(&state, "minecraft:prismarine");
    for x in -2..=2 {
        for y in -2..=2 {
            for z in -2..=2 {
                if frame_cell(x, y, z) {
                    state
                        .world
                        .set_block(pos.x + x, pos.y + y, pos.z + z, prismarine);
                }
                if (-1..=1).contains(&x) && (-1..=1).contains(&y) && (-1..=1).contains(&z) {
                    state
                        .world
                        .set_block(pos.x + x, pos.y + y, pos.z + z, water);
                }
            }
        }
    }
    let conduit = block(&state, "minecraft:conduit");
    state.world.set_block(pos.x, pos.y, pos.z, conduit);
    after_blocks(&mut state, &[(pos, conduit)]);
    soak(&mut state, player, [-0.5, 82.0, -0.5]);
    for i in 0..4096 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    state.tick = 40;
    VISITS.set(0);
    PLAYER_VISITS.set(0);
    run(&mut state, &mut Vec::new());
    assert_eq!(PLAYER_VISITS.get(), 1);
    assert!(VISITS.get() <= 1, "visited {}", VISITS.get());
    assert!(
        state.players[&actor]
            .active_effects
            .has(EffectId::ConduitPower)
    );
}

#[test]
#[ignore = "focused A/B cost measurement; run in mem-run -x"]
fn conduit_focused_cost() {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    crate::mob_lanes::tests::join(&mut state);
    for i in 0..50_000 {
        state.spawn_mob(
            state.registry.cow_type_id,
            [1000.0 + i as f64, 80.0, 1000.0],
            10.0,
        );
    }
    let pos = BlockPos { x: 0, y: 80, z: 0 };
    for round in 0..4 {
        for old in [true, false] {
            VISITS.set(0);
            let started = std::time::Instant::now();
            for _ in 0..32 {
                std::hint::black_box(if old {
                    exhaustive(&state, pos)
                } else {
                    attack_target(&state, pos)
                });
                let players: Vec<_> = if old {
                    state
                        .entities
                        .slots()
                        .filter(|&slot| {
                            matches!(state.entities.body(slot).kind, EntityKind::Player { .. })
                        })
                        .collect()
                } else {
                    state.entities.player_slots().collect()
                };
                std::hint::black_box(players);
            }
            eprintln!(
                "S08 N=50001 C=1 P=1 round={round} exhaustive={old} ns/event={} attack_candidates/event={}",
                started.elapsed().as_nanos() / 32,
                VISITS.get() / 32
            );
        }
    }
}
