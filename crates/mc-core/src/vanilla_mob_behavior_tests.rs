use crate::boundary::{
    BlockPos, ChangeSet, ConnectionId, EntityDataValue, EntityId, Hand, PlayerChange, PlayerInput,
};
use crate::mounts::tests::{hold, slot, spawn, use_entity};
use crate::tick::{TickState, run_tick};
use mc_data::Identifier;

fn fixture() -> (TickState, EntityId, [f64; 3]) {
    let mut state = crate::mob_lanes::tests::fixture("minecraft:plains");
    let player = join(&mut state, 1);
    (state, player, [8.5, 80.0, 8.5])
}

fn join(state: &mut TickState, connection: u64) -> EntityId {
    run_tick(
        state,
        &[PlayerInput::Join {
            connection: ConnectionId(connection),
            uuid: uuid::Uuid::from_u128(connection as u128),
            name: format!("rider{connection}"),
            permission_level: 0,
        }],
    );
    state.world.ensure_blocking(&state.desired_chunks());
    let player = state
        .connections
        .entity_of(ConnectionId(connection))
        .expect("joined player");
    let player_slot = slot(state, player);
    state.entities.set_pos(player_slot, [8.5, 80.0, 8.5]);
    state.entities.set_vel(player_slot, [0.0; 3]);
    state.entities.set_ground(player_slot, true);
    player
}

fn interact(state: &mut TickState, connection: u64, target: EntityId) {
    run_tick(
        state,
        &[PlayerInput::InteractEntity {
            connection: ConnectionId(connection),
            target,
            hand: Hand::Main,
            location: [0.0; 3],
        }],
    );
}

fn item_names(state: &TickState) -> Vec<String> {
    state
        .entities
        .slots()
        .filter_map(|slot| state.entities.row(slot).item.as_ref())
        .map(|stack| {
            state
                .registry
                .items
                .name_of(stack.item_id)
                .expect("registered dropped item")
                .to_string()
        })
        .collect()
}

fn block(state: &TickState, name: &str, props: &[(&str, &str)]) -> u32 {
    let id = Identifier::parse(name).unwrap();
    let default = state.registry.blocks.block(&id).unwrap().default;
    let mut full = state
        .registry
        .blocks
        .by_id(default)
        .unwrap()
        .properties
        .clone();
    for (key, value) in props {
        if let Some(entry) = full.iter_mut().find(|(name, _)| name == key) {
            entry.1 = (*value).to_owned();
        }
    }
    state
        .registry
        .blocks
        .by_name_and_props(&id, &full)
        .unwrap()
        .0
}

#[test]
fn spiders_avoid_only_unrolled_armadillos() {
    for name in ["spider", "cave_spider"] {
        for rolled in [false, true] {
            let (mut state, _player, pos) = fixture();
            state.day_time = 14_000;
            let armadillo = spawn(&mut state, "armadillo", [pos[0] + 4.0, pos[1], pos[2]]);
            let armadillo_slot = slot(&state, armadillo);
            state
                .entities
                .row_mut(armadillo_slot)
                .entity_life
                .armadillo
                .get_or_insert_with(crate::armadillo::ArmadilloState::default)
                .rolled = rolled;
            let spider = spawn(&mut state, name, [pos[0] + 8.0, pos[1], pos[2]]);
            let spider_slot = slot(&state, spider);
            let before = state.entities.pos(spider_slot);
            for _ in 0..20 {
                run_tick(&mut state, &[]);
            }
            let after = state.entities.pos(spider_slot);
            if !rolled {
                assert!(
                    after[0] > before[0] + 0.1,
                    "{name} did not flee unrolled armadillo: {before:?} -> {after:?}"
                );
            } else {
                assert!(
                    after[0] < before[0] - 0.1,
                    "{name} avoided a rolled armadillo: {before:?} -> {after:?}"
                );
            }
        }
    }
}

#[test]
fn adult_sniffer_digs_one_seed_on_suitable_ground_then_cools_down() {
    let (mut state, _player, pos) = fixture();
    let pos = [pos[0] + 4.0, pos[1], pos[2]];
    let dirt = block(&state, "minecraft:dirt", &[]);
    state
        .world
        .set_block(pos[0] as i32, pos[1] as i32 - 1, pos[2] as i32, dirt);
    state.world.fold_overlay();
    let sniffer = spawn(&mut state, "sniffer", pos);
    assert!(state.entities.slot_of(sniffer).is_some());
    let mut digging = false;
    for _ in 0..240 {
        run_tick(&mut state, &[]);
        digging |= state
            .entities
            .row(slot(&state, sniffer))
            .appearance
            .extra
            .contains(&(17, EntityDataValue::SnifferState(5)));
    }
    assert!(digging, "sniffer never published its actual digging state");
    let seeds = item_names(&state)
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "minecraft:torchflower_seeds" | "minecraft:pitcher_pod"
            )
        })
        .count();
    assert_eq!(seeds, 1, "sniffer must dig exactly one seed");
    let (_, regions, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let saved = regions
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.id == "minecraft:sniffer")
        .unwrap();
    let restored = crate::journal::EntityStateImage::from_record(&state.registry, saved).unwrap();
    let sniffer_slot = slot(&state, sniffer);
    assert_eq!(
        restored.entity_life.sniffer,
        state.entities.row(sniffer_slot).entity_life.sniffer
    );
    state
        .entities
        .set_entity_life(sniffer_slot, restored.entity_life);
    for _ in 0..20 {
        run_tick(&mut state, &[]);
    }
    let after = item_names(&state)
        .into_iter()
        .filter(|name| {
            matches!(
                name.as_str(),
                "minecraft:torchflower_seeds" | "minecraft:pitcher_pod"
            )
        })
        .count();
    assert_eq!(after, 1, "sniffer dropped again during its dig cooldown");
}

#[test]
fn frog_tongue_consumes_only_small_prey_and_drops_vanilla_loot() {
    for (prey_kind, drop) in [("slime", "slime_ball"), ("magma_cube", "ochre_froglight")] {
        let (mut state, _player, pos) = fixture();
        let frog = spawn(&mut state, "frog", [pos[0] + 3.0, pos[1], pos[2]]);
        assert!(state.entities.slot_of(frog).is_some());
        let prey = spawn(&mut state, prey_kind, [pos[0] + 1.0, pos[1], pos[2]]);
        let prey_slot = slot(&state, prey);
        crate::entity_loot::set_size(&mut state, prey_slot, 1, true);
        state.entities.row_mut(prey_slot).controls.no_ai = true;
        let cow = spawn(&mut state, "cow", [pos[0] + 2.0, pos[1], pos[2] + 2.0]);
        for _ in 0..40 {
            run_tick(&mut state, &[]);
        }
        assert!(
            state.entities.slot_of(prey).is_none(),
            "frog left small {prey_kind} alive"
        );
        assert!(
            state.entities.slot_of(cow).is_some(),
            "frog ate ineligible prey"
        );
        assert!(
            item_names(&state).contains(&format!("minecraft:{drop}"))
                || state
                    .players
                    .values()
                    .flat_map(|player| &player.inventory.slots)
                    .any(|stack| {
                        !stack.is_empty()
                            && state
                                .registry
                                .items
                                .name_of(stack.item_id)
                                .is_some_and(|name| name.as_str() == format!("minecraft:{drop}"))
                    }),
            "missing {drop}"
        );
    }
}

#[test]
fn strider_cold_state_slows_and_lava_restores_it() {
    let (mut state, _player, pos) = fixture();
    let strider = spawn(&mut state, "strider", pos);
    let strider_slot = slot(&state, strider);
    let warm_speed = state.entities.row(strider_slot).speed;
    run_tick(&mut state, &[]);
    assert!(
        state.entities.row(strider_slot).speed < warm_speed,
        "cold strider kept full speed"
    );
    assert!(
        state
            .entities
            .row(strider_slot)
            .appearance
            .extra
            .contains(&(18, EntityDataValue::Boolean(true),))
    );
    let lava = block(&state, "minecraft:lava", &[]);
    state
        .world
        .set_block(pos[0] as i32, pos[1] as i32, pos[2] as i32, lava);
    state.world.fold_overlay();
    run_tick(&mut state, &[]);
    assert_eq!(state.entities.row(strider_slot).speed, warm_speed);
    assert!(
        !state
            .entities
            .row(strider_slot)
            .appearance
            .extra
            .contains(&(18, EntityDataValue::Boolean(true),))
    );
}

#[test]
fn cold_strider_rain_damage_obeys_roof_and_clear_weather() {
    for (rain, roof) in [(true, false), (true, true), (false, false)] {
        let (mut state, _, pos) = fixture();
        let strider = spawn(&mut state, "strider", [pos[0] + 4.0, pos[1], pos[2]]);
        let strider_slot = slot(&state, strider);
        let at = state.entities.pos(strider_slot);
        state.entities.row_mut(strider_slot).controls.no_ai = true;
        state.weather = (
            if rain {
                crate::commands::Weather::Rain
            } else {
                crate::commands::Weather::Clear
            },
            1000,
        );
        if roof {
            state.world.set_block(
                at[0] as i32,
                at[1] as i32 + 3,
                at[2] as i32,
                state.registry.states.stone,
            );
            state.world.fold_overlay();
        }
        assert_eq!(crate::conduit::rain_exposed(&state, at), rain && !roof);
        let health = state.entities.row(strider_slot).health;
        run_tick(&mut state, &[]);
        assert_eq!(
            state.entities.row(strider_slot).health < health,
            rain && !roof
        );
    }
}

#[test]
fn sniffer_requires_adulthood_ai_and_diggable_ground() {
    for case in ["baby", "no_ai", "stone"] {
        let (mut state, _, pos) = fixture();
        let pos = [pos[0] + 4.0, pos[1], pos[2]];
        if case != "stone" {
            state.world.set_block(
                pos[0] as i32,
                pos[1] as i32 - 1,
                pos[2] as i32,
                block(&state, "minecraft:dirt", &[]),
            );
            state.world.fold_overlay();
        }
        let sniffer = spawn(&mut state, "sniffer", pos);
        if case == "baby" {
            crate::entity_life::make_child(&mut state, sniffer);
        }
        if case == "no_ai" {
            let slot = slot(&state, sniffer);
            state.entities.row_mut(slot).controls.no_ai = true;
        }
        for _ in 0..240 {
            run_tick(&mut state, &[]);
        }
        assert!(
            !item_names(&state).iter().any(|name| matches!(
                name.as_str(),
                "minecraft:torchflower_seeds" | "minecraft:pitcher_pod"
            )),
            "{case} sniffer dug seeds"
        );
    }
}

#[test]
fn parrot_record_events_and_nearby_hostile_imitation() {
    let (mut state, player, pos) = fixture();
    let _parrot = spawn(&mut state, "parrot", [pos[0] + 2.0, pos[1], pos[2]]);
    let jukebox = BlockPos {
        x: pos[0] as i32,
        y: pos[1] as i32,
        z: pos[2] as i32,
    };
    state.world.set_block(
        jukebox.x,
        jukebox.y,
        jukebox.z,
        block(&state, "minecraft:jukebox", &[]),
    );
    state.world.fold_overlay();
    hold(&mut state, player, "music_disc_cat", 1);
    assert!(crate::jukebox::use_on(
        &mut state,
        player,
        Hand::Main,
        jukebox,
        &mut ChangeSet::default(),
    ));
    let (_, changes, _) = run_tick(&mut state, &[]);
    assert!(
        changes
            .iter()
            .flat_map(|(_, changes)| changes)
            .any(|change| matches!(
                change,
                PlayerChange::ProjectileLevelEvent { event: 1010, .. }
            ))
    );
    crate::jukebox::use_on(
        &mut state,
        player,
        Hand::Main,
        jukebox,
        &mut ChangeSet::default(),
    );
    let (_, changes, _) = run_tick(&mut state, &[]);
    assert!(
        changes
            .iter()
            .flat_map(|(_, changes)| changes)
            .any(|change| matches!(
                change,
                PlayerChange::ProjectileLevelEvent { event: 1011, .. }
            ))
    );
    let zombie = spawn(&mut state, "zombie", [pos[0] + 1.0, pos[1], pos[2]]);
    assert!(state.entities.slot_of(zombie).is_some());
    let mut imitated = false;
    for _ in 0..401 {
        let (_, changes, _) = run_tick(&mut state, &[]);
        imitated |= changes.iter().flat_map(|(_, changes)| changes).any(|change| matches!(change,
            PlayerChange::CommandSound { sound, .. } if sound.as_str() == "minecraft:entity.parrot.imitate.zombie"));
    }
    assert!(imitated);
}

#[test]
fn tamed_cat_gives_one_morning_gift_not_an_arbitrary_wake_gift() {
    let (mut state, player, pos) = fixture();
    let cat = spawn(&mut state, "cat", [pos[0] + 2.0, pos[1], pos[2]]);
    let owner = state.entities.row(slot(&state, player)).uuid;
    let life = &mut state.entities.row_mut(slot(&state, cat)).entity_life;
    life.tamed = true;
    life.taming.owner = Some(owner);
    life.sitting = false;
    let foot = BlockPos {
        x: pos[0] as i32,
        y: pos[1] as i32,
        z: pos[2] as i32,
    };
    let head = BlockPos {
        x: foot.x + 1,
        ..foot
    };
    state.world.set_block(
        foot.x,
        foot.y,
        foot.z,
        block(
            &state,
            "minecraft:red_bed",
            &[("facing", "east"), ("part", "foot")],
        ),
    );
    state.world.set_block(
        head.x,
        head.y,
        head.z,
        block(
            &state,
            "minecraft:red_bed",
            &[("facing", "east"), ("part", "head")],
        ),
    );
    state.world.fold_overlay();
    state.day_time = 13_000;
    assert!(crate::sleep::use_bed(
        &mut state,
        player,
        ConnectionId(1),
        foot
    ));
    for _ in 0..101 {
        run_tick(&mut state, &[]);
    }
    let gifts = [
        "rabbit_foot",
        "rabbit_hide",
        "string",
        "rotten_flesh",
        "feather",
        "chicken",
        "phantom_membrane",
    ];
    let count = |state: &TickState| {
        item_names(state)
            .into_iter()
            .filter(|name| {
                gifts
                    .iter()
                    .any(|gift| name == &format!("minecraft:{gift}"))
            })
            .count()
            + state.players[&player]
                .inventory
                .slots
                .iter()
                .filter(|stack| {
                    !stack.is_empty()
                        && state
                            .registry
                            .items
                            .name_of(stack.item_id)
                            .is_some_and(|name| {
                                gifts
                                    .iter()
                                    .any(|gift| name.as_str() == format!("minecraft:{gift}"))
                            })
                })
                .count()
    };
    assert_eq!(count(&state), 1, "eligible cat gave no morning gift");
    state.day_time = 13_000;
    assert!(crate::sleep::use_bed(
        &mut state,
        player,
        ConnectionId(1),
        foot
    ));
    for _ in 0..10 {
        run_tick(&mut state, &[]);
    }
    crate::sleep::wake(&mut state, player);
    run_tick(&mut state, &[]);
    assert_eq!(count(&state), 1, "arbitrary wake duplicated the gift");
}

#[test]
fn saddled_camel_admits_two_riders_and_dash_has_cooldown() {
    let (mut state, first, pos) = fixture();
    let camel = spawn(&mut state, "camel", pos);
    hold(&mut state, first, "saddle", 1);
    run_tick(&mut state, &[use_entity(camel)]);
    run_tick(&mut state, &[use_entity(camel)]);
    let second = join(&mut state, 2);
    interact(&mut state, 2, camel);
    assert_eq!(
        state
            .entities
            .row(slot(&state, camel))
            .appearance
            .passengers
            .len(),
        2
    );
    assert!(crate::mounts::vehicle(&state, first).is_some());
    assert!(crate::mounts::vehicle(&state, second).is_some());
    let camel_slot = slot(&state, camel);
    state.entities.set_rot(camel_slot, 0.0, 0.0);
    let before_dash = state.entities.pos(camel_slot);
    run_tick(
        &mut state,
        &[PlayerInput::VehicleInput {
            connection: ConnectionId(1),
            flags: 16,
        }],
    );
    assert!(
        state
            .entities
            .row(slot(&state, camel))
            .entity_life
            .mount
            .dash_cooldown
            > 0
    );
    for _ in 0..5 {
        run_tick(&mut state, &[]);
    }
    let after_dash = state.entities.pos(camel_slot);
    assert!(
        after_dash[2] > before_dash[2] + 1.0,
        "dash did not advance: {before_dash:?} -> {after_dash:?}"
    );
    assert!(
        after_dash[1] > before_dash[1],
        "dash did not jump: {before_dash:?} -> {after_dash:?}"
    );
    let cooling = state
        .entities
        .row(slot(&state, camel))
        .entity_life
        .mount
        .dash_cooldown;
    run_tick(
        &mut state,
        &[
            PlayerInput::VehicleInput {
                connection: ConnectionId(1),
                flags: 0,
            },
            PlayerInput::VehicleInput {
                connection: ConnectionId(1),
                flags: 16,
            },
        ],
    );
    assert!(
        state
            .entities
            .row(slot(&state, camel))
            .entity_life
            .mount
            .dash_cooldown
            < cooling
    );
    assert_ne!(
        state
            .entities
            .row(slot(&state, camel))
            .entity_life
            .mount
            .dash_cooldown,
        55
    );
}

#[test]
fn happy_ghast_harness_admits_four_riders_and_survives_rider_codec() {
    let (mut state, first, pos) = fixture();
    let ghast = spawn(&mut state, "happy_ghast", pos);
    hold(&mut state, first, "red_harness", 1);
    run_tick(&mut state, &[use_entity(ghast)]);
    run_tick(&mut state, &[use_entity(ghast)]);
    for connection in 2..=4 {
        join(&mut state, connection);
        interact(&mut state, connection, ghast);
    }
    assert_eq!(
        state
            .entities
            .row(slot(&state, ghast))
            .appearance
            .passengers
            .len(),
        4
    );
    let fifth = join(&mut state, 5);
    interact(&mut state, 5, ghast);
    assert!(crate::mounts::vehicle(&state, fifth).is_none());
    run_tick(
        &mut state,
        &[PlayerInput::Sneak {
            connection: ConnectionId(4),
            sneaking: true,
        }],
    );
    assert_eq!(
        state
            .entities
            .row(slot(&state, ghast))
            .appearance
            .passengers
            .len(),
        3
    );
    assert!(crate::mounts::vehicle(&state, first).is_some());
    let (_, regions, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let directory = tempfile::tempdir().unwrap();
    mc_world::anvil::entity_regions::write_entity_regions(
        directory.path(),
        &state.registry.items,
        &regions,
        state.tick as u32,
    )
    .unwrap();
    let saved = mc_world::anvil::entity_regions::read_entity_regions(
        directory.path(),
        &state.registry.items,
    )
    .unwrap();
    let record = saved
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.id == "minecraft:happy_ghast")
        .unwrap();
    let restored = crate::journal::EntityStateImage::from_record(&state.registry, record).unwrap();
    let ghast_slot = slot(&state, ghast);
    assert_eq!(
        restored.entity_life.mount.riders,
        state.entities.row(ghast_slot).entity_life.mount.riders
    );
    assert_eq!(
        restored.loot.body_armor,
        state.entities.row(ghast_slot).loot.body_armor
    );
    state
        .entities
        .set_entity_life(ghast_slot, restored.entity_life);
    crate::mounts::tick(&mut state, &mut ChangeSet::default());
    assert_eq!(
        state
            .entities
            .row(slot(&state, ghast))
            .appearance
            .passengers
            .len(),
        3
    );
}
