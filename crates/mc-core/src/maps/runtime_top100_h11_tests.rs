use super::*;
use crate::boundary::{BlockPos, PlayerInput};
use crate::dimensions::Dimension;
use crate::registry::CoreRegistry;
use crate::tick::run_tick;

fn join(state: &mut TickState, connection: u64) -> crate::boundary::EntityId {
    run_tick(
        state,
        &[PlayerInput::Join {
            connection: ConnectionId(connection),
            uuid: uuid::Uuid::from_u128(u128::from(connection)),
            name: format!("mapper{connection}"),
            permission_level: 0,
        }],
    );
    let want = state.desired_chunks();
    state.world.ensure_blocking(&want);
    run_tick(
        state,
        &[PlayerInput::ConfirmTeleport {
            connection: ConnectionId(connection),
            teleport_id: 1,
        }],
    );
    state
        .connections
        .entity_of(ConnectionId(connection))
        .unwrap()
}

#[test]
fn frame_boundary_and_hidden_clone_are_visible_to_the_right_viewer() {
    let registry = Arc::new(CoreRegistry::load(None).expect("registry loads"));
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    let alice = join(&mut state, 1);
    let bob = join(&mut state, 2);
    state.config_view_distance = 1;
    for player in [alice, bob] {
        let slot = state.entities.slot_of(player).unwrap();
        state.entities.set_pos(slot, [0.0, 65.0, 0.0]);
    }
    let id = state
        .maps
        .allocate(SavedMap::new([0, 0], 0, Dimension::Overworld).unwrap())
        .unwrap();
    Arc::make_mut(state.maps.records.get_mut(&id).unwrap())
        .banners
        .push(crate::maps::Banner {
            pos: [0, 65, 0],
            color: "red".to_owned(),
            name: None,
        });
    state.world.set_block(0, 65, 0, state.registry.states.air);
    state.maps.viewers.insert(
        (ConnectionId(99), id),
        (Arc::clone(&state.maps.records[&id]), 0),
    );
    let stack = crate::maps::filled_map(&state, id).unwrap();
    state.players.get_mut(&alice).unwrap().inventory.slots[0] = stack.clone();
    state.players.get_mut(&bob).unwrap().inventory.slots[10] = stack.clone();
    let pumpkin = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:carved_pumpkin").unwrap())
        .unwrap();
    state.players.get_mut(&bob).unwrap().inventory.slots[39] = mc_data::ItemStack::new(pumpkin, 1);
    for x in [7, 9] {
        state
            .world
            .set_block(x + 1, 65, 8, state.registry.states.stone);
        state.world.set_block(x, 65, 8, state.registry.states.air);
    }
    state.world.fold_overlay();
    for (x, target) in [(7, 16.0), (9, 17.0)] {
        let frame = crate::item_frame::spawn(
            &mut state,
            BlockPos { x, y: 65, z: 8 },
            4,
            false,
            stack.clone(),
        )
        .expect("supported frame");
        let slot = state.entities.slot_of(frame).unwrap();
        state.entities.set_pos(slot, [target, 65.0, 0.0]);
    }
    state.outbox.clear();
    state.tick = 10;
    tick(&mut state);
    let update = |connection| {
        state.outbox[&ConnectionId(connection)]
            .iter()
            .find_map(|change| match change {
                PlayerChange::Map { update } if update.id == id => Some(update),
                _ => None,
            })
            .unwrap()
    };
    let alice_update = update(1);
    let bob_update = update(2);
    assert_eq!(
        alice_update.markers.iter().filter(|m| m.kind == 1).count(),
        2,
        "the frame exactly on the boundary must be included and the outside frame excluded"
    );
    assert_eq!(
        alice_update.markers.iter().filter(|m| m.kind == 0).count(),
        1
    );
    assert_eq!(bob_update.markers.iter().filter(|m| m.kind == 0).count(), 2);
    assert_eq!(alice_update.rectangle, [0, 0, 128, 128]);
    assert_eq!(alice_update.colors, state.maps.records[&id].colors);
    assert_eq!(bob_update.colors, alice_update.colors);
    assert_eq!(state.maps.records[&id].frame_markers, vec![(16, 0, 90)]);
    assert!(
        state.maps.records[&id].banners.is_empty(),
        "destroyed loaded banner remains"
    );
    state.dimension = Dimension::Nether;
    state.outbox.clear();
    state.tick = 25;
    tick(&mut state);
    assert!(!state.maps.viewers.contains_key(&(ConnectionId(99), id)));
    let cross_dimension = state.outbox[&ConnectionId(1)]
        .iter()
        .find_map(|change| match change {
            PlayerChange::Map { update } if update.id == id => Some(update),
            _ => None,
        })
        .unwrap();
    assert!(cross_dimension.markers.is_empty());
    assert_eq!(cross_dimension.rectangle, [0; 4]);
}

#[test]
fn spatial_candidates_match_dense_frames_and_bound_work() {
    use crate::blueprint::MovementClass;
    use crate::boundary::{EntityAppearance, EntityId};
    use crate::entities::{ColdFields, EntityKind, EntityStore};

    for count in [0, 10_000, 100_000] {
        let mut entities = EntityStore::default();
        for i in 0..count {
            let pos = match i {
                0 => [16.0, 65.0, 0.0],
                1 => [-16.0, 65.0, 0.0],
                2 => [16.01, 65.0, 0.0],
                _ => [1024.0 + f64::from(i) * 16.0, 65.0, 0.0],
            };
            entities.spawn(
                EntityId(i + 1),
                uuid::Uuid::from_u128((i + 1) as u128),
                EntityKind::Mob {
                    type_id: if i % 2 == 0 { 7 } else { 8 },
                },
                ColdFields {
                    pos,
                    vel: [0.0; 3],
                    yaw: 0.0,
                    pitch: 0.0,
                    ground: true,
                    health: 20.0,
                    breed_cooldown: 0,
                    pickup_delay: 0,
                    item: None,
                    appearance: Arc::new(EntityAppearance::default()),
                    movement: MovementClass::Walker,
                    half_width: 0.3,
                    height: 1.8,
                    speed: 0.1,
                    attack_damage: 1.0,
                    sight_range: 16.0,
                    attack_reach: 1.0,
                },
            );
        }
        for viewers in [1, 100] {
            FRAME_CANDIDATE_VISITS.with(|visits| visits.set(0));
            let mut baseline = 0;
            for _ in 0..viewers {
                let expected: Vec<_> = [7, 8]
                    .into_iter()
                    .flat_map(|kind| entities.mob_slots(kind))
                    .inspect(|_| baseline += 1)
                    .filter(|&slot| {
                        let p = entities.pos(slot);
                        p[0].abs() <= 16.0 && p[2].abs() <= 16.0
                    })
                    .collect();
                let actual: Vec<_> =
                    nearby_frames(&entities, [0.0, 65.0, 0.0], 16.0, [Some(7), Some(8)])
                        .into_iter()
                        .filter(|&slot| {
                            let p = entities.pos(slot);
                            p[0].abs() <= 16.0 && p[2].abs() <= 16.0
                        })
                        .collect();
                assert_eq!(actual, expected);
            }
            let visited = FRAME_CANDIDATE_VISITS.with(|visits| visits.get());
            eprintln!("frames={count} viewers={viewers} dense={baseline} indexed={visited}");
            if count > 0 {
                assert!(visited * 100 < baseline, "spatial enumeration regressed");
            } else {
                assert_eq!(visited, 0);
            }
        }
    }
}
