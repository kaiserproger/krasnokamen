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
    for _ in 0..460 {
        run_tick(&mut state, &[]);
        digging |= state
            .entities
            .row(slot(&state, sniffer))
            .appearance
            .extra
            .contains(&(18, EntityDataValue::SnifferState(5)));
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
        let mut targeted = false;
        for _ in 0..40 {
            run_tick(&mut state, &[]);
            let extra = &state.entities.row(slot(&state, frog)).appearance.extra;
            targeted |= extra.contains(&(
                19,
                EntityDataValue::OptionalUnsignedInt(Some(prey.0 as u32)),
            ));
            assert!(
                !extra.iter().any(|(index, value)| *index == 18
                    && matches!(value, EntityDataValue::OptionalUnsignedInt(_))),
                "accessor 18 is the frog variant, never the tongue target"
            );
        }
        assert!(
            targeted,
            "tongue target id was never published at accessor 19"
        );
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
            .contains(&(19, EntityDataValue::Boolean(true),))
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
            .contains(&(19, EntityDataValue::Boolean(true),))
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
        for _ in 0..460 {
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
    let parrot = spawn(&mut state, "parrot", [pos[0] + 2.0, pos[1], pos[2]]);
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
    let rolls = state
        .entities
        .row(slot(&state, parrot))
        .entity_life
        .taming
        .rolls;
    let mut imitated = false;
    for _ in 0..4000 {
        let (_, changes, _) = run_tick(&mut state, &[]);
        imitated |= changes.iter().flat_map(|(_, changes)| changes).any(|change| matches!(change,
            PlayerChange::CommandSound { sound, .. } if sound.as_str() == "minecraft:entity.parrot.imitate.zombie"));
        if imitated {
            break;
        }
    }
    assert!(imitated);
    assert_eq!(
        state
            .entities
            .row(slot(&state, parrot))
            .entity_life
            .taming
            .rolls,
        rolls,
        "the ambient imitation roll must not write persisted taming state"
    );
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
    let dashed = state.entities.row(camel_slot);
    assert!(
        dashed
            .appearance
            .extra
            .contains(&(19, EntityDataValue::Boolean(true))),
        "Camel.DASH is accessor 19"
    );
    assert!(
        !dashed
            .appearance
            .extra
            .iter()
            .any(|(index, _)| *index == 18),
        "accessor 18 is the AbstractHorse flags byte"
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

fn clear_touched(state: &mut TickState) {
    let ids = state.entities.take_touched();
    state.entities.recycle_touched(ids);
}

#[test]
fn sniffer_uses_accessor_18_and_leaves_the_age_lock_accessor_alone() {
    let (mut state, _player, pos) = fixture();
    let pos = [pos[0] + 4.0, pos[1], pos[2]];
    state.world.set_block(
        pos[0] as i32,
        pos[1] as i32 - 1,
        pos[2] as i32,
        block(&state, "minecraft:dirt", &[]),
    );
    state.world.fold_overlay();
    let sniffer = spawn(&mut state, "sniffer", pos);
    let sniffer_slot = slot(&state, sniffer);
    // AgeableMob.DATA_AGE_LOCKED sits at 17.
    crate::special_mobs::metadata(&mut state, sniffer_slot, 17, EntityDataValue::Boolean(true));
    for _ in 0..260 {
        run_tick(&mut state, &[]);
    }
    let extra = &state.entities.row(sniffer_slot).appearance.extra;
    assert!(extra.contains(&(17, EntityDataValue::Boolean(true))));
    assert!(
        extra
            .iter()
            .any(|(index, value)| *index == 18 && matches!(value, EntityDataValue::SnifferState(_)))
    );
    assert!(
        !extra
            .iter()
            .any(|(index, value)| *index == 17 && matches!(value, EntityDataValue::SnifferState(_)))
    );
}

#[test]
fn cooling_sniffer_writes_nothing_and_scans_on_its_own_period() {
    let (mut state, _player, pos) = fixture();
    let pos = [pos[0] + 4.0, pos[1], pos[2]];
    state.world.set_block(
        pos[0] as i32,
        pos[1] as i32 - 1,
        pos[2] as i32,
        block(&state, "minecraft:dirt", &[]),
    );
    state.world.fold_overlay();
    let sniffer = spawn(&mut state, "sniffer", pos);
    let sniffer_slot = slot(&state, sniffer);
    state.entities.set_ground(sniffer_slot, true);
    state.entities.row_mut(sniffer_slot).entity_life.sniffer =
        Some(Box::new(crate::sniffer::Sniffer {
            cooldown_until: 500,
            ..Default::default()
        }));
    clear_touched(&mut state);
    for tick in 0..500 {
        state.tick = tick;
        crate::sniffer::run(&mut state);
    }
    assert!(
        !state.entities.touched_ids().contains(&sniffer),
        "a cooling sniffer was rewritten while it waited"
    );
    let phase = state.entities.row(sniffer_slot).uuid.as_u128() as u64;
    let expected = (500..)
        .find(|tick| (tick + phase).is_multiple_of(20))
        .unwrap();
    let mut found = None;
    for tick in 500..540 {
        state.tick = tick;
        crate::sniffer::run(&mut state);
        if found.is_none()
            && state
                .entities
                .row(sniffer_slot)
                .entity_life
                .sniffer
                .as_ref()
                .is_some_and(|sniffer| sniffer.target.is_some())
        {
            found = Some(tick);
        }
    }
    assert_eq!(found, Some(expected), "ground scans are one per period");
}

#[test]
fn idle_parrot_and_frog_ticks_do_not_dirty_their_rows() {
    let (mut state, _player, pos) = fixture();
    let parrot = spawn(&mut state, "parrot", [pos[0] + 2.0, pos[1], pos[2]]);
    let frog = spawn(&mut state, "frog", [pos[0] - 2.0, pos[1], pos[2]]);
    clear_touched(&mut state);
    for tick in 0..400 {
        state.tick = tick;
        crate::parrot::run(&mut state);
        crate::frog::run(&mut state);
    }
    let touched = state.entities.touched_ids();
    assert!(!touched.contains(&parrot), "parrot row written per tick");
    assert!(!touched.contains(&frog), "idle frog row written per tick");
}

fn sniffer_record(fields: Vec<(String, mc_nbt::Tag)>) -> mc_data::entity_nbt::VanillaEntity {
    mc_data::entity_nbt::VanillaEntity {
        id: "minecraft:sniffer".into(),
        extra_fields: vec![("krasnokamen:Sniffer".into(), mc_nbt::Tag::Compound(fields))],
        ..Default::default()
    }
}

#[test]
fn sniffer_nbt_and_wal_images_share_one_validation() {
    use mc_nbt::{ListTag, Tag};
    let int_array = |x: i32| Tag::IntArray(vec![x, 0, 0]);
    let list = |element_type: u8, elements: Vec<Tag>| {
        Tag::List(ListTag {
            element_type,
            elements,
        })
    };
    // An empty list may carry any element type.
    for element_type in [0, mc_nbt::tag_type::INT_ARRAY, mc_nbt::tag_type::STRING] {
        let sniffer = crate::sniffer::from_record(&sniffer_record(vec![
            ("CooldownUntil".into(), Tag::Long(7)),
            ("Elapsed".into(), Tag::Short(0)),
            ("Explored".into(), list(element_type, Vec::new())),
        ]))
        .unwrap()
        .unwrap();
        assert!(sniffer.explored.is_empty() && sniffer.cooldown_until == 7);
    }
    let too_many = list(
        mc_nbt::tag_type::INT_ARRAY,
        (0..21).map(int_array).collect(),
    );
    for fields in [
        vec![("Explored".into(), too_many)],
        vec![("Elapsed".into(), Tag::Short(5))],
        vec![
            ("Elapsed".into(), Tag::Short(200)),
            ("Target".into(), int_array(1)),
        ],
        vec![("CooldownUntil".into(), Tag::Long(-1))],
        vec![("Cooldown".into(), Tag::Int(9))],
    ] {
        assert!(crate::sniffer::from_record(&sniffer_record(fields)).is_err());
    }
    let image = |patch: serde_json::Value| {
        let mut base = serde_json::json!({
            "cooldown_until": 5, "elapsed": 0, "target": null, "explored": []
        });
        base.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        serde_json::from_value::<crate::sniffer::Sniffer>(base)
    };
    assert!(image(serde_json::json!({})).is_ok());
    assert!(image(serde_json::json!({"target": [1, 2, 3], "elapsed": 9})).is_ok());
    for bad in [
        serde_json::json!({"cooldown": 3}),
        serde_json::json!({"elapsed": 3}),
        serde_json::json!({"elapsed": 200, "target": [1, 2, 3]}),
        serde_json::json!({"explored": (0..21).map(|x| [x, 0, 0]).collect::<Vec<_>>()}),
    ] {
        assert!(image(bad.clone()).is_err(), "{bad}");
    }
}

#[test]
fn obsolete_scalar_rider_fails_closed_in_nbt_and_wal() {
    use mc_nbt::{ListTag, Tag, tag_type};
    let mut life = crate::entity_life::EntityLifeState::default();
    let error = crate::mounts::persistence::from_fields(
        &mut life,
        &[("krasnokamen:Rider".into(), Tag::IntArray(vec![0, 0, 0, 1]))],
    )
    .unwrap_err();
    assert!(
        error.to_string().contains("krasnokamen:Rider is obsolete"),
        "{error}"
    );
    let riders = |ids: &[i32]| {
        Tag::List(ListTag {
            element_type: tag_type::INT_ARRAY,
            elements: ids
                .iter()
                .map(|id| Tag::IntArray(vec![0, 0, 0, *id]))
                .collect(),
        })
    };
    for (ids, reason) in [
        (&[1, 1][..], "duplicate"),
        (&[1, 2, 3, 4, 5][..], "krasnokamen:Riders"),
    ] {
        let error = crate::mounts::persistence::from_fields(
            &mut life,
            &[("krasnokamen:Riders".into(), riders(ids))],
        )
        .unwrap_err();
        assert!(error.to_string().contains(reason), "{error}");
    }
    assert!(
        serde_json::from_value::<crate::mounts::MountState>(serde_json::json!({"rider": null}))
            .is_err()
    );
    assert!(
        serde_json::from_value::<crate::mounts::MountState>(serde_json::json!({"riders": []}))
            .is_ok()
    );
    let mut image = serde_json::to_value(crate::entity_life::EntityLifeState::default()).unwrap();
    image["mount"]["rider"] = serde_json::json!("00000000-0000-0000-0000-000000000001");
    assert!(serde_json::from_value::<crate::entity_life::EntityLifeState>(image).is_err());
}

fn replay_vehicle(
    state: &TickState,
    name: &str,
    uuid: u128,
    riders: &[u128],
) -> crate::journal::ReplayEntity {
    let mut image = crate::journal::EntityStateImage::default();
    image.entity_life.mount.riders = riders.iter().map(|id| uuid::Uuid::from_u128(*id)).collect();
    crate::journal::ReplayEntity {
        uuid: uuid::Uuid::from_u128(uuid).to_string(),
        type_id: state
            .registry
            .entity_types
            .id_of(&Identifier::parse(format!("minecraft:{name}")).unwrap())
            .unwrap(),
        pos: [0.0; 3],
        vel: [0.0; 3],
        yaw: 0.0,
        pitch: 0.0,
        ground: true,
        health: 20.0,
        alive: true,
        age: 0,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: Default::default(),
        active_effects: Default::default(),
        villager: None,
        state: image,
        projectile: None,
        tick: 0,
    }
}

#[test]
fn replay_refuses_overfull_shared_and_single_seat_vehicles() {
    let (state, _, _) = fixture();
    let check = |vehicles: Vec<(&str, u128, Vec<u128>)>| {
        let mut replay = crate::journal::Replay::default();
        for (index, (name, uuid, riders)) in vehicles.iter().enumerate() {
            replay.entities.insert(
                index as i32 + 1,
                replay_vehicle(&state, name, *uuid, riders),
            );
        }
        crate::mounts::persistence::validate_replay(&replay)
    };
    assert!(check(vec![("camel", 10, vec![101, 102])]).is_ok());
    assert!(check(vec![("happy_ghast", 10, vec![101, 102, 103, 104])]).is_ok());
    assert!(check(vec![("camel", 10, vec![101, 102, 103, 104])]).is_err());
    assert!(check(vec![("camel", 10, vec![101, 102, 103])]).is_ok());
    assert!(check(vec![("happy_ghast", 10, vec![101, 102, 103, 104, 105])]).is_err());
    assert!(check(vec![("cow", 10, vec![101, 102])]).is_err());
    assert!(
        check(vec![
            ("camel", 10, vec![101]),
            ("happy_ghast", 11, vec![101]),
        ])
        .is_err(),
        "one rider on two vehicles"
    );
}

// Camel.canAddPassenger is `getPassengers().size() <= 2` (local javap, 26.1.2 and
// 26.2): three seats, the second and third sharing the -0.7 attachment.
#[test]
fn camel_seats_three_riders_and_refuses_a_fourth() {
    let (mut state, first, pos) = fixture();
    let camel = spawn(&mut state, "camel", pos);
    hold(&mut state, first, "saddle", 1);
    run_tick(&mut state, &[use_entity(camel)]);
    run_tick(&mut state, &[use_entity(camel)]);
    let second = join(&mut state, 2);
    interact(&mut state, 2, camel);
    let third = join(&mut state, 3);
    interact(&mut state, 3, camel);
    let fourth = join(&mut state, 4);
    interact(&mut state, 4, camel);
    assert!(crate::mounts::vehicle(&state, second).is_some());
    assert!(crate::mounts::vehicle(&state, third).is_some());
    assert!(crate::mounts::vehicle(&state, fourth).is_none());
    assert_eq!(
        state
            .entities
            .row(slot(&state, camel))
            .appearance
            .passengers
            .len(),
        3
    );
}

fn tamed_cat(
    state: &mut TickState,
    pos: [f64; 3],
    owner: uuid::Uuid,
) -> (EntityId, crate::entities::EntitySlot) {
    let cat = spawn(state, "cat", pos);
    let cat_slot = slot(state, cat);
    let life = &mut state.entities.row_mut(cat_slot).entity_life;
    life.tamed = true;
    life.taming.owner = Some(owner);
    life.sitting = false;
    (cat, cat_slot)
}

#[test]
fn frog_after_a_hunt_or_lost_prey_stays_untouched_while_idle() {
    for lose_prey in [false, true] {
        let (mut state, _player, pos) = fixture();
        let frog = spawn(&mut state, "frog", [pos[0] + 3.0, pos[1], pos[2]]);
        let prey = spawn(&mut state, "slime", [pos[0] + 2.0, pos[1], pos[2]]);
        let prey_slot = slot(&state, prey);
        crate::entity_loot::set_size(&mut state, prey_slot, 1, true);
        state.entities.row_mut(prey_slot).controls.no_ai = true;
        if lose_prey {
            // Close enough to start the tongue, then the prey vanishes mid-attack.
            for _ in 0..3 {
                run_tick(&mut state, &[]);
            }
            assert!(
                state
                    .entities
                    .row(slot(&state, frog))
                    .appearance
                    .extra
                    .contains(&(
                        19,
                        EntityDataValue::OptionalUnsignedInt(Some(prey.0 as u32))
                    )),
                "the tongue never started"
            );
            state.remove_entity(slot(&state, prey));
            run_tick(&mut state, &[]);
        } else {
            for _ in 0..40 {
                run_tick(&mut state, &[]);
            }
            assert!(state.entities.slot_of(prey).is_none(), "prey survived");
        }
        // The tongue accessor now holds an explicit None; it must not keep the row hot.
        assert!(
            state
                .entities
                .row(slot(&state, frog))
                .appearance
                .extra
                .contains(&(19, EntityDataValue::OptionalUnsignedInt(None))),
            "the cleared tongue accessor is the case under test"
        );
        clear_touched(&mut state);
        for tick in 1000..1400 {
            state.tick = tick;
            crate::frog::run(&mut state);
        }
        assert!(
            !state.entities.touched_ids().contains(&frog),
            "frog row written per tick after its prey is gone (lose_prey={lose_prey})"
        );
    }
}

#[test]
fn idle_camel_is_not_written_and_a_running_dash_cooldown_is() {
    let (mut state, _player, pos) = fixture();
    let camel = spawn(&mut state, "camel", pos);
    let camel_slot = slot(&state, camel);
    clear_touched(&mut state);
    for _ in 0..100 {
        crate::mounts::tick(&mut state, &mut ChangeSet::default());
    }
    assert!(
        !state.entities.touched_ids().contains(&camel),
        "idle camel row written per tick"
    );
    state
        .entities
        .row_mut(camel_slot)
        .entity_life
        .mount
        .dash_cooldown = 3;
    clear_touched(&mut state);
    crate::mounts::tick(&mut state, &mut ChangeSet::default());
    assert!(state.entities.touched_ids().contains(&camel));
    assert_eq!(
        state
            .entities
            .row(camel_slot)
            .entity_life
            .mount
            .dash_cooldown,
        2
    );
}

#[test]
fn idle_cats_are_not_written_per_tick() {
    let (mut state, player, pos) = fixture();
    let owner = state.entities.row(slot(&state, player)).uuid;
    let (own, _) = tamed_cat(&mut state, [pos[0] + 2.0, pos[1], pos[2]], owner);
    let (stranger, _) = tamed_cat(
        &mut state,
        [pos[0] - 2.0, pos[1], pos[2]],
        uuid::Uuid::from_u128(0xfeed),
    );
    clear_touched(&mut state);
    for tick in 0..200 {
        state.tick = tick;
        crate::sleep::run(&mut state);
    }
    let touched = state.entities.touched_ids();
    assert!(
        !touched.contains(&own) && !touched.contains(&stranger),
        "idle cats written per tick"
    );
}

#[test]
fn a_wake_touches_only_the_owners_cats() {
    let (mut state, player, pos) = fixture();
    let owner = state.entities.row(slot(&state, player)).uuid;
    tamed_cat(&mut state, [pos[0] + 2.0, pos[1], pos[2]], owner);
    let (stranger, _) = tamed_cat(
        &mut state,
        [pos[0] - 2.0, pos[1], pos[2]],
        uuid::Uuid::from_u128(0xfeed),
    );
    let foot = BlockPos {
        x: pos[0] as i32,
        y: pos[1] as i32,
        z: pos[2] as i32,
    };
    for (dx, part) in [(0, "foot"), (1, "head")] {
        state.world.set_block(
            foot.x + dx,
            foot.y,
            foot.z,
            block(
                &state,
                "minecraft:red_bed",
                &[("facing", "east"), ("part", part)],
            ),
        );
    }
    state.world.fold_overlay();
    state.day_time = 13_000;
    assert!(crate::sleep::use_bed(
        &mut state,
        player,
        ConnectionId(1),
        foot
    ));
    for _ in 0..10 {
        crate::sleep::run(&mut state);
    }
    clear_touched(&mut state);
    crate::sleep::wake(&mut state, player);
    assert!(
        !state.entities.touched_ids().contains(&stranger),
        "a wake wrote a cat that belongs to someone else"
    );
}

#[test]
fn runtime_only_mob_state_is_not_part_of_the_journaled_image() {
    let (mut state, _player, pos) = fixture();
    let frog = spawn(&mut state, "frog", pos);
    let frog_slot = slot(&state, frog);
    let image = crate::journal::EntityStateImage::from_row(
        state.entities.body(frog_slot),
        state.entities.row(frog_slot),
    );
    let target = spawn(&mut state, "slime", [pos[0] + 1.0, pos[1], pos[2]]);
    let row = state.entities.row_mut(frog_slot);
    row.frog_prey = Some((target, [1.0, 2.0, 3.0]));
    row.frog_tongue_ticks = 7;
    row.cat_bed_ticks = 99;
    assert!(
        image.matches(
            state.entities.body(frog_slot),
            state.entities.row(frog_slot)
        ),
        "frog_prey, frog_tongue_ticks or cat_bed_ticks would write an EntityState WAL record"
    );
}

#[test]
fn rider_count_beyond_the_vehicle_seat_capacity_is_refused_by_the_image_reader() {
    use mc_nbt::{ListTag, Tag, tag_type};
    let (state, _player, _pos) = fixture();
    let riders = |count: i32| {
        Tag::List(ListTag {
            element_type: tag_type::INT_ARRAY,
            elements: (1..=count)
                .map(|id| Tag::IntArray(vec![0, 0, 0, id]))
                .collect(),
        })
    };
    let record = |kind: &str, count: i32| mc_data::entity_nbt::VanillaEntity {
        id: format!("minecraft:{kind}"),
        extra_fields: vec![("krasnokamen:Riders".into(), riders(count))],
        ..Default::default()
    };
    let error = crate::journal::EntityStateImage::from_record(&state.registry, &record("cow", 2))
        .unwrap_err();
    assert!(error.to_string().contains("seat capacity 1"), "{error}");
    assert!(
        crate::journal::EntityStateImage::from_record(&state.registry, &record("cow", 1)).is_ok()
    );
    assert!(
        crate::journal::EntityStateImage::from_record(&state.registry, &record("camel", 2)).is_ok()
    );
    assert!(
        crate::journal::EntityStateImage::from_record(&state.registry, &record("camel", 4))
            .is_err()
    );
    assert!(
        crate::journal::EntityStateImage::from_record(&state.registry, &record("happy_ghast", 4))
            .is_ok()
    );
}
