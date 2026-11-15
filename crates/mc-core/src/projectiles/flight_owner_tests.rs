use super::{arrow_stack, setup};
use crate::boundary::{ConnectionId, PlayerInput};
use crate::projectiles::{ProjectileKind, ProjectileState, launch};
use crate::tick::run_tick;

fn retained_owner_pvp(disconnect: bool) {
    for pvp in [false, true] {
        let (mut state, owner, centre) = setup();
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(2),
                uuid: uuid::Uuid::new_v4(),
                name: "victim".into(),
                permission_level: 0,
            }],
        );
        let victim = state.connections.entity_of(ConnectionId(2)).unwrap();
        let victim_slot = state.entities.slot_of(victim).unwrap();
        state
            .entities
            .set_pos(victim_slot, [centre[0] + 6.0, centre[1], centre[2]]);
        state.entities.row_mut(victim_slot).protected_until = 0;
        let health = state.players[&victim].vitals.health;
        let owner_uuid = state
            .entities
            .row(state.entities.slot_of(owner).unwrap())
            .uuid;
        state.gamerules.insert("pvp".into(), pvp.to_string());
        let stack = arrow_stack(&state);
        launch(
            &mut state,
            ProjectileState::new(ProjectileKind::Arrow, Some(owner_uuid), stack),
            [centre[0] + 1.0, centre[1] + 1.0, centre[2]],
            [1.0, 0.0, 0.0],
        )
        .unwrap();
        let input = if disconnect {
            PlayerInput::Leave {
                connection: ConnectionId(1),
            }
        } else {
            PlayerInput::Command {
                connection: ConnectionId(1),
                command: "kill @s".into(),
                permission_level: 2,
            }
        };
        run_tick(&mut state, &[input]);
        assert!(state.entities.slot_of_uuid(owner_uuid).is_none());
        if disconnect {
            assert!(state.offline_players.contains_key(&owner_uuid));
        }
        assert_eq!(
            state.players[&victim].vitals.health, health,
            "owner leaves before impact"
        );
        for _ in 0..8 {
            run_tick(&mut state, &[]);
        }
        if pvp {
            assert!(
                state.players[&victim].vitals.health < health,
                "the same trajectory must actually hit when PvP is allowed"
            );
        } else {
            assert_eq!(
                state.players[&victim].vitals.health, health,
                "retained player ownership must still enforce pvp=false"
            );
        }
    }
}

#[test]
fn dead_player_arrow_owner_respects_pvp_rule() {
    retained_owner_pvp(false);
}

#[test]
fn disconnected_player_arrow_owner_respects_pvp_rule() {
    retained_owner_pvp(true);
}

#[test]
fn player_arrow_respects_pvp_after_owner_travels_and_disconnects() {
    struct Empty(mc_world::BlockStateId, mc_world::ChunkGeometry);
    impl mc_world::ChunkGenerator for Empty {
        fn generate(&self, position: mc_world::ChunkPos) -> mc_world::Chunk {
            mc_world::Chunk::empty_with_geometry(
                position,
                self.0,
                mc_data::Identifier::parse("minecraft:plains").unwrap(),
                self.1,
            )
        }
    }
    let dimension = crate::dimensions::Dimension::Nether;
    for pvp in [false, true] {
        let (mut state, owner, centre) = setup();
        let registry = std::sync::Arc::clone(&state.registry);
        let world = crate::world::ChunkWorld::new(
            42,
            std::sync::Arc::new(Empty(
                mc_world::BlockStateId(registry.states.air),
                dimension.geometry(),
            )),
            &registry,
            mc_world::BlockStateId(registry.states.air),
            [8, 64, 8],
            dimension.geometry(),
        );
        let mut destination = crate::tick::TickState::new(registry, world, 0, 512);
        destination.dimension = dimension;
        state
            .other_dimensions
            .insert(dimension, Box::new(destination));
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(2),
                uuid: uuid::Uuid::new_v4(),
                name: "victim".into(),
                permission_level: 0,
            }],
        );
        let victim = state.connections.entity_of(ConnectionId(2)).unwrap();
        let slot = state.entities.slot_of(victim).unwrap();
        state
            .entities
            .set_pos(slot, [centre[0] + 6.0, centre[1], centre[2]]);
        state.entities.row_mut(slot).protected_until = 0;
        let health = state.players[&victim].vitals.health;
        let owner_uuid = state
            .entities
            .row(state.entities.slot_of(owner).unwrap())
            .uuid;
        state.gamerules.insert("pvp".into(), pvp.to_string());
        let stack = arrow_stack(&state);
        launch(
            &mut state,
            ProjectileState::new(ProjectileKind::Arrow, Some(owner_uuid), stack),
            [centre[0] + 1.0, centre[1] + 1.0, centre[2]],
            [1.0, 0.0, 0.0],
        )
        .unwrap();
        run_tick(
            &mut state,
            &[PlayerInput::Command {
                connection: ConnectionId(1),
                command: "execute in the_nether run tp @s 8 64 8".into(),
                permission_level: 2,
            }],
        );
        assert_eq!(
            crate::dimensions::travel::locate(&state, owner).unwrap().0,
            dimension
        );
        run_tick(
            &mut state,
            &[PlayerInput::Leave {
                connection: ConnectionId(1),
            }],
        );
        run_tick(&mut state, &[]);
        assert!(!crate::chunk_tickets::owner_online(&state, owner_uuid));
        assert!(!state.offline_players.contains_key(&owner_uuid));
        assert!(
            state.other_dimensions[&dimension]
                .offline_players
                .contains_key(&owner_uuid)
        );
        assert_eq!(
            state.players[&victim].vitals.health, health,
            "owner leaves before impact"
        );
        for _ in 0..8 {
            run_tick(&mut state, &[]);
        }
        if pvp {
            assert!(
                state.players[&victim].vitals.health < health,
                "the same trajectory must hit with PvP enabled"
            );
        } else {
            assert_eq!(
                state.players[&victim].vitals.health, health,
                "remote offline ownership must enforce pvp=false"
            );
        }
    }
}
