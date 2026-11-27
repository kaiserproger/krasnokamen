//! Data-defined blueprints and capabilities (ADR 0016): a custom mob composes
//! native capabilities, and every out-of-range parameter is a load error.

use std::sync::Arc;

use crate::blueprint::{CUSTOM_TYPE_FLAG, MobBlueprintDef, MovementClass, wire_type_id};
use crate::hostile::CREEPER_SWELL_DIR_INDEX;
use crate::hostile_tests::setup;
use crate::registry::CoreRegistry;
use crate::tick::{TickState, run_tick};
use mc_data::entity_data_value::EntityDataValue;
use mc_domain::EntityId;

#[test]
fn slime_and_magma_cube_blueprints_hop_with_the_head_on_the_body() {
    let registry = CoreRegistry::load(None).expect("registry loads");
    for name in ["minecraft:slime", "minecraft:magma_cube"] {
        let id = registry
            .entity_types
            .id_of(&mc_data::Identifier::parse(name).expect("id"))
            .expect("vanilla type");
        let blueprint = registry.blueprint(id).expect("vanilla blueprint");
        assert!(blueprint.capabilities.hops, "{name} must hop");
        assert!(
            blueprint.capabilities.head_follows_body,
            "{name} keeps its head on the body (SlimeMoveControl)"
        );
    }
}

/// A creeper that flies and is hurt by water: `Flyer` + `Swell` + capability
/// flags, shown to clients as a creeper. Fuse, power and griefing differ from
/// the vanilla creeper so the test proves the blueprint values are read.
fn sky_creeper() -> serde_json::Value {
    serde_json::from_str(include_str!(
        "../../../sdk/rust/examples/sky-creeper/src/sky-creeper.json"
    ))
    .unwrap()
}

fn definition(value: &serde_json::Value) -> MobBlueprintDef {
    MobBlueprintDef::from_json(&value.to_string()).expect("definition parses")
}

fn register(state: &mut TickState, value: &serde_json::Value) -> u32 {
    Arc::get_mut(&mut state.registry)
        .expect("fixture owns its registry")
        .register_blueprint(definition(value))
        .expect("sky creeper admitted")
}

fn vanilla(state: &TickState, name: &str) -> u32 {
    state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(name).expect("id"))
        .expect("vanilla type")
}

fn health(state: &TickState, id: EntityId) -> f32 {
    let slot = state.entities.slot_of(id).expect("alive");
    state.entities.row(slot).health
}

#[test]
fn sky_creeper_flies_and_rain_hurts_it_but_not_a_creeper() {
    let (mut state, player, centre) = setup(12.0);
    let key = register(&mut state, &sky_creeper());
    assert_ne!(key & CUSTOM_TYPE_FLAG, 0);
    let creeper_type = vanilla(&state, "minecraft:creeper");
    assert_eq!(wire_type_id(key), creeper_type, "clients see a creeper");
    state.weather.0 = crate::commands::Weather::Rain;
    let above = [centre[0], centre[1] + 6.0, centre[2]];
    let sky = state.spawn_mob(key, above, 20.0);
    let creeper = state.spawn_mob(creeper_type, [above[0], above[1], above[2] + 3.0], 20.0);
    let sky_slot = state.entities.slot_of(sky).expect("sky slot");
    assert_eq!(state.entities.body(sky_slot).movement, MovementClass::Flyer);
    assert_eq!(
        state.entities.views().get(sky).map(|view| view.type_id),
        Some(creeper_type),
        "the wire view carries the presentation type"
    );
    let distance = |state: &TickState| {
        let at = state.entities.pos(state.entities.slot_of(sky).unwrap());
        let to = state.entities.pos(state.entities.slot_of(player).unwrap());
        ((at[0] - to[0]).powi(2) + (at[2] - to[2]).powi(2)).sqrt()
    };
    let start = distance(&state);
    for _ in 0..10 {
        run_tick(&mut state, &[]);
    }
    let sky_y = state.entities.pos(state.entities.slot_of(sky).unwrap())[1];
    let creeper_y = state.entities.pos(state.entities.slot_of(creeper).unwrap())[1];
    assert!(
        sky_y > centre[1] + 3.0,
        "no gravity: {sky_y} stays aloft over {}",
        centre[1]
    );
    assert!(
        creeper_y < above[1] - 3.0 && creeper_y < sky_y - 1.0,
        "a walker falls under gravity: {creeper_y} vs flyer {sky_y}"
    );
    assert!(distance(&state) < start, "flies toward the player");
    assert!(health(&state, sky) < 20.0, "rain hurts the capability");
    assert_eq!(
        health(&state, creeper),
        20.0,
        "rain does not hurt a creeper"
    );
}

#[test]
fn strider_takes_water_and_exposed_rain_damage_but_not_dry_or_sheltered() {
    for (rain, water, roof, expected) in [
        (false, false, false, 20.0),
        (true, false, true, 20.0),
        (true, false, false, 19.0),
        (false, true, false, 19.0),
    ] {
        let (mut state, _, centre) = setup(12.0);
        let pos = [centre[0], centre[1] + 6.0, centre[2]];
        let kind = vanilla(&state, "minecraft:strider");
        let strider = state.spawn_mob(kind, pos, 20.0);
        if rain {
            state.weather.0 = crate::commands::Weather::Rain;
        }
        if water {
            state.world.set_block(
                pos[0] as i32,
                pos[1] as i32,
                pos[2] as i32,
                state.registry.states.water,
            );
        }
        if roof {
            state.world.set_block(
                pos[0] as i32,
                pos[1] as i32 + 3,
                pos[2] as i32,
                state.registry.states.stone,
            );
        }
        run_tick(&mut state, &[]);
        assert_eq!(
            health(&state, strider),
            expected,
            "rain={rain}, water={water}, roof={roof}"
        );
    }
}

#[test]
fn sky_creeper_swells_with_its_own_fuse_and_non_griefing_blast() {
    let (mut state, player, centre) = setup(1.0);
    let key = register(&mut state, &sky_creeper());
    let sky = state.spawn_mob(key, [centre[0], centre[1] + 1.0, centre[2]], 20.0);
    let mut blasts = Vec::new();
    for tick in 1..=20 {
        let (output, _, _) = run_tick(&mut state, &[]);
        blasts.extend(output.changes.explosions.iter().cloned());
        if tick == 19 {
            assert!(state.entities.slot_of(sky).is_some(), "fuse still burning");
            let slot = state.entities.slot_of(sky).unwrap();
            assert!(
                state
                    .entities
                    .row(slot)
                    .appearance
                    .extra
                    .contains(&(CREEPER_SWELL_DIR_INDEX, EntityDataValue::VarInt(1))),
                "creeper presentation animates the fuse"
            );
        }
    }
    assert!(
        state.entities.slot_of_any(sky).is_none(),
        "20-tick fuse, not 30"
    );
    assert_eq!(blasts.len(), 1);
    assert_eq!(blasts[0].radius, 2.0, "blueprint power");
    assert_eq!(blasts[0].blocks, 0, "griefing off breaks no blocks");
    assert_eq!(
        state
            .world
            .block_at(centre[0] as i32, 200, centre[2] as i32),
        Some(state.registry.states.stone)
    );
    assert!(
        state.players[&player].vitals.health < 20.0,
        "the blast still hurts"
    );
}

#[test]
fn out_of_range_and_unsupported_definitions_are_load_errors() {
    let mut registry = CoreRegistry::load(None).expect("registry loads");
    let before = registry.blueprints.len();
    let cases: [(&str, serde_json::Value, &str); 16] = [
        ("max_health", serde_json::json!(0.5), "max_health"),
        ("max_health", serde_json::json!(4096.0), "max_health"),
        ("speed", serde_json::json!(16.5), "speed"),
        ("sight_range", serde_json::json!(65.0), "sight_range"),
        (
            "attack_reach",
            serde_json::json!(20.0),
            "attack_reach exceeds",
        ),
        ("attack_damage", serde_json::json!(-1.0), "attack_damage"),
        ("armor", serde_json::json!(31.0), "armor"),
        (
            "knockback_resistance",
            serde_json::json!(1.5),
            "knockback_resistance",
        ),
        (
            "capabilities",
            serde_json::json!({"explodes": {"power": 7.0, "fuse_ticks": 30, "griefing": true}}),
            "explosion power",
        ),
        (
            "capabilities",
            serde_json::json!({"explodes": {"power": 3.0, "fuse_ticks": 5, "griefing": true}}),
            "fuse_ticks",
        ),
        ("capabilities", serde_json::json!({}), "swell goal"),
        (
            "goals",
            serde_json::json!([{"priority": 0, "kind": "breed"}]),
            "per-type state",
        ),
        (
            "presents_as",
            serde_json::json!("minecraft:arrow"),
            "living",
        ),
        (
            "presents_as",
            serde_json::json!("minecraft:ender_dragon"),
            "multipart wire ids",
        ),
        (
            "presents_as",
            serde_json::json!("minecraft:giant"),
            "body height",
        ),
        ("id", serde_json::json!("minecraft:sky_creeper"), "reserved"),
    ];
    for (field, value, expected) in cases {
        let mut definition_json = sky_creeper();
        definition_json[field] = value;
        let error = registry
            .register_blueprint(definition(&definition_json))
            .expect_err(field)
            .to_string();
        assert!(error.contains(expected), "{field}: {error}");
        assert_eq!(
            registry.blueprints.len(),
            before,
            "{field}: nothing admitted"
        );
    }
    let mut unknown = sky_creeper();
    unknown["capabilities"]["fire_immune"] = serde_json::json!(true);
    assert!(MobBlueprintDef::from_json(&unknown.to_string()).is_err());
    registry
        .register_blueprint(definition(&sky_creeper()))
        .expect("valid definition admitted");
    let duplicate = registry
        .register_blueprint(definition(&sky_creeper()))
        .expect_err("duplicate id");
    assert!(duplicate.to_string().contains("already registered"));
    for ordinal in 1..crate::blueprint::limits::MAX_CUSTOM_BLUEPRINTS {
        let mut next = sky_creeper();
        next["id"] = serde_json::json!(format!("example:sky_creeper_{ordinal}"));
        registry
            .register_blueprint(definition(&next))
            .expect("within the bound");
    }
    let mut over = sky_creeper();
    over["id"] = serde_json::json!("example:one_too_many");
    let error = registry
        .register_blueprint(definition(&over))
        .expect_err("registry bound");
    assert!(
        error.to_string().contains("data-defined blueprints"),
        "{error}"
    );
}

/// Every vanilla type named in a capability table owns a blueprint, so the
/// table, not a missing-blueprint default, decides its behaviour.
#[test]
fn vanilla_capability_tables_name_registered_blueprints() {
    let registry = CoreRegistry::load(None).expect("registry loads");
    let capable = |name: &str| {
        let key = registry
            .entity_types
            .id_of(&mc_data::Identifier::parse(name).expect("id"))
            .expect("vanilla type");
        registry
            .blueprint(key)
            .unwrap_or_else(|| panic!("{name} has a blueprint"))
            .capabilities
    };
    assert!(capable("minecraft:enderman").hurt_by_water);
    assert!(capable("minecraft:spider").climbs_walls);
    assert!(capable("minecraft:cave_spider").dark_aggro_only);
    assert!(capable("minecraft:phantom").burns_in_daylight);
    assert!(capable("minecraft:skeleton_horse").inverted_healing);
    assert_eq!(
        capable("minecraft:creeper")
            .explodes
            .map(|blast| blast.fuse_ticks),
        Some(crate::blueprint::FUSE_TICKS)
    );
    for name in [
        "minecraft:cod",
        "minecraft:salmon",
        "minecraft:tropical_fish",
        "minecraft:pufferfish",
        "minecraft:tadpole",
    ] {
        assert_eq!(capable(name).aquatic, crate::species::AquaticControl::Fish);
    }
    for name in ["minecraft:squid", "minecraft:glow_squid"] {
        assert_eq!(capable(name).aquatic, crate::species::AquaticControl::Squid);
    }
    assert_eq!(
        capable("minecraft:dolphin").aquatic,
        crate::species::AquaticControl::Dolphin
    );
    for name in [
        "minecraft:copper_golem",
        "minecraft:iron_golem",
        "minecraft:snow_golem",
        "minecraft:shulker",
        "minecraft:allay",
        "minecraft:bat",
        "minecraft:bee",
        "minecraft:blaze",
        "minecraft:cat",
        "minecraft:chicken",
        "minecraft:ghast",
        "minecraft:happy_ghast",
        "minecraft:phantom",
        "minecraft:magma_cube",
        "minecraft:ocelot",
        "minecraft:parrot",
        "minecraft:wither",
        "minecraft:breeze",
    ] {
        assert!(capable(name).fall_damage_immune, "{name}");
    }
    // Capabilities are read through the blueprint, where the former name and
    // contract checks were not: every living type must own one.
    for type_id in 0..mc_data::entity_types::ENTITY_TYPE_COUNT as u32 {
        let contract =
            mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id).unwrap();
        if contract.behavior.archetype == mc_data::entity_types::EntityArchetype::Living
            && contract.name != "minecraft:player"
        {
            assert!(registry.blueprint(type_id).is_some(), "{}", contract.name);
        }
    }
}

/// R6 G1: every simulated body size comes from the vanilla f32 contract, so a
/// 0.6-wide body is `f64::from(0.6f32) / 2` and a clip can never leave it a
/// rounding step inside a block. The hand-written `0.3` / `1.95` literals are
/// gone from the blueprint table and from the player, glider, crouch, dismount
/// and falling-block bodies.
/// R6 G1 on the live path: a summoned villager keeps the f32 contract body
/// through `sync_appearance`, for an adult and a baby (exactly half). The refresh
/// used to overwrite it with a hand-written 0.3 / 1.95 / 0.15 / 0.975 table,
/// which a comparison of registry rows could not see.
#[test]
fn a_summoned_villager_keeps_the_contract_body_through_the_appearance_refresh() {
    let (mut state, _player, centre) = setup(0.0);
    let villager_type = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:villager").unwrap())
        .unwrap();
    let contract = crate::blueprint::contract_dimensions("minecraft:villager");
    for age in [0_i32, -200] {
        let villager = state.spawn_mob(villager_type, centre, 20.0);
        let slot = state.entities.slot_of(villager).unwrap();
        state.entities.row_mut(slot).villager = Some(Box::new(crate::villager::VillagerState {
            age,
            ..Default::default()
        }));
        crate::villager::sync_appearance(&mut state, slot);
        let body = state.entities.body(slot);
        let scale = if age < 0 { 0.5 } else { 1.0 };
        assert_eq!(
            body.half_width,
            contract.half_width() * scale,
            "age {age}: the refresh must carry the contract half width"
        );
        assert_eq!(
            body.height,
            contract.height * scale,
            "age {age}: the refresh must carry the contract height"
        );
    }
}

#[test]
fn body_sizes_come_from_the_vanilla_entity_contract() {
    let registry = CoreRegistry::load(None).expect("registry");
    for name in [
        "minecraft:zombie",
        "minecraft:skeleton",
        "minecraft:cow",
        "minecraft:spider",
        "minecraft:villager",
        "minecraft:iron_golem",
        "minecraft:chicken",
    ] {
        let id = registry
            .entity_types
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .expect("vanilla type");
        let blueprint = registry.blueprint(id).expect("blueprint");
        let dimensions = crate::blueprint::contract_dimensions(name);
        assert_eq!(blueprint.half_width, dimensions.half_width(), "{name}");
        assert_eq!(blueprint.height, dimensions.height, "{name}");
        // Independent oracle: the literal f32 contract value, not a second call.
        if name == "minecraft:zombie" {
            assert_eq!(blueprint.half_width, 0.6_f32 as f64 / 2.0, "{name}");
            assert_eq!(blueprint.height, 1.95_f32 as f64, "{name}");
        }
    }
    let zombie = crate::blueprint::contract_dimensions("minecraft:zombie");
    assert_eq!(zombie.half_width(), f64::from(0.6_f32) / 2.0);
    assert_ne!(
        zombie.half_width(),
        0.3,
        "the f64 literal is not the contract width"
    );
    assert_eq!(
        crate::blueprint::PLAYER_BODY.half_width(),
        f64::from(0.6_f32) / 2.0
    );
    assert_eq!(crate::blueprint::PLAYER_BODY.height, f64::from(1.8_f32));
    assert_eq!(
        crate::blueprint::PLAYER_FALL_FLYING_HEIGHT,
        f64::from(0.6_f32)
    );
    assert_eq!(
        crate::blueprint::PLAYER_CROUCHING_HEIGHT,
        f64::from(1.5_f32)
    );
    assert_eq!(
        crate::blueprint::FALLING_BLOCK_BODY.half_width(),
        f64::from(0.98_f32) / 2.0
    );
    assert_eq!(
        crate::blueprint::FALLING_BLOCK_BODY.height,
        f64::from(0.98_f32)
    );
}
