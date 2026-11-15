use super::*;
use crate::boundary::{ChangeSet, ConnectionId, PlayerChange, PlayerInput};

#[path = "command_summon_scoreboard_tags_disk_tests.rs"]
mod scoreboard_tags_disk_tests;
#[path = "command_summon_scoreboard_tags_tests.rs"]
mod scoreboard_tags_tests;
#[path = "command_summon_variant_disk_tests.rs"]
mod variant_disk_tests;
#[path = "command_summon_variant_refusal_tests.rs"]
mod variant_refusal_tests;

fn summon(kind: &str, nbt: &str) -> (TickState, EntityId) {
    let (mut state, actor, _) = crate::hostile_tests::setup(8.0);
    let parsed = crate::commands::parse(&format!("summon {kind} ~ ~ ~ {nbt}"), true).unwrap();
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(&mut state, ConnectionId(1), actor, &parsed, &mut changes);
    assert_eq!(
        changes.entities_spawned.len(),
        1,
        "{kind} {nbt}: {:?}",
        state.outbox
    );
    let id = changes.entities_spawned[0];
    (state, id)
}

#[test]
fn summon_faction_is_native() {
    let (mut state, id) = summon("zombie", "{\"krasnokamen:Faction\":65535}");
    let slot = state.entities.slot_of(id).unwrap();
    assert_eq!(state.entities.body(slot).faction, u16::MAX);
    let uuid = state.entities.row(slot).uuid;
    let Tag::Compound(public) = crate::command_data::entity(&state, id).unwrap() else {
        panic!("expected public entity compound");
    };
    assert_eq!(
        mc_data::item_nbt::field(&public, "krasnokamen:Faction"),
        Some(&Tag::Int(65535))
    );
    assert_eq!(
        crate::command_data::command(
            &mut state,
            ConnectionId(1),
            id,
            &format!("data get entity {uuid} \"krasnokamen:Faction\""),
        ),
        crate::command_execution::CommandResult::Success(65535)
    );
    assert_eq!(
        crate::command_data::command(
            &mut state,
            ConnectionId(1),
            id,
            &format!("data merge entity {uuid} {{\"krasnokamen:Faction\":8}}"),
        ),
        crate::command_execution::CommandResult::Failed
    );
    assert_eq!(state.entities.body(slot).faction, u16::MAX);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == state.entities.row(slot).uuid)
        .unwrap();
    let fields = mc_data::entity_nbt::entity_to_fields(&state.registry.items, native).unwrap();
    assert_eq!(
        mc_data::item_nbt::field(&fields, "krasnokamen:Faction"),
        Some(&Tag::Int(65535))
    );
    let image = crate::journal::EntityStateImage::from_row(
        state.entities.body(slot),
        state.entities.row(slot),
    );
    let json = serde_json::to_value(&image).unwrap();
    assert_eq!(json["faction"], 65535);
    for invalid in [
        serde_json::json!(-1),
        serde_json::json!(65536),
        serde_json::json!("7"),
        serde_json::Value::Null,
    ] {
        let mut corrupt = json.clone();
        corrupt["faction"] = invalid;
        assert!(serde_json::from_value::<crate::journal::EntityStateImage>(corrupt).is_err());
    }
    for invalid in [
        Tag::Int(-1),
        Tag::Int(65536),
        Tag::Short(7),
        Tag::String("7".into()),
    ] {
        let mut corrupt = fields.clone();
        *corrupt
            .iter_mut()
            .find(|(key, _)| key == "krasnokamen:Faction")
            .unwrap() = ("krasnokamen:Faction".into(), invalid.clone());
        let mut replay = crate::journal::Replay::default();
        assert!(
            crate::native_checkpoint::hydrate_entity(
                &state.registry,
                &mut replay,
                &Tag::Compound(corrupt),
                state.tick,
            )
            .is_err(),
            "persisted {invalid:?} must fail closed"
        );
        assert!(replay.entities.is_empty());
        let nbt = Tag::Compound(vec![("krasnokamen:Faction".into(), invalid)]);
        assert!(
            prepare(
                &state,
                state.registry.entity_type_id("minecraft:zombie").unwrap(),
                "minecraft:zombie",
                [0.0, 80.0, 0.0],
                &nbt,
            )
            .is_err()
        );
    }
    for nbt in ["{}", "{\"krasnokamen:Faction\":0}"] {
        let (state, id) = summon("zombie", nbt);
        let slot = state.entities.slot_of(id).unwrap();
        let image = crate::journal::EntityStateImage::from_row(
            state.entities.body(slot),
            state.entities.row(slot),
        );
        assert_eq!(image.faction, 0);
        assert!(image.opaque_entity_fields.is_empty());
        let json = serde_json::to_value(&image).unwrap();
        assert!(json.get("faction").is_none(), "zero adds no WAL bytes");
        // Blueprint-derived loot fields (base armor) are serde(skip) and rebuilt at
        // restore, so compare the serialized JSON image, not the in-memory image.
        let decoded =
            serde_json::from_value::<crate::journal::EntityStateImage>(json.clone()).unwrap();
        assert_eq!(decoded.faction, 0);
        assert_eq!(serde_json::to_value(&decoded).unwrap(), json);
        assert!(
            image
                .to_fields(&state.registry.blocks)
                .iter()
                .all(|(key, _)| key != "krasnokamen:Faction"),
            "zero adds no checkpoint bytes"
        );
    }
}

#[test]
fn experience_orb_summon_preserves_native_value_count_kind_and_checkpoint() {
    for (nbt, expected) in [
        ("{Value:5s,Count:4,Motion:[0d,0d,0d]}", (5, 4)),
        ("{Value:7s}", (7, 1)),
        ("", (0, 1)),
        ("{}", (0, 1)),
        ("{Value:-1s}", (-1, 1)),
        ("{Value:32767s}", (32767, 1)),
        ("{Value:-32768s}", (-32768, 1)),
    ] {
        let (state, id) = summon("experience_orb", nbt);
        let slot = state.entities.slot_of(id).unwrap();
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        assert_eq!(
            state.entities.experience_orbs().collect::<Vec<_>>(),
            vec![id]
        );
        assert!(
            state
                .entities
                .bodies()
                .iter()
                .all(|body| !matches!(body.kind, crate::entities::EntityKind::Mob { .. })),
            "{nbt}: not a mob row"
        );
        assert!(matches!(
            body.kind,
            crate::entities::EntityKind::ExperienceOrb { .. }
        ));
        assert_eq!(row.appearance.experience_orb, Some(expected));
        assert_eq!(body.movement, crate::blueprint::MovementClass::Item);
        assert_eq!((body.half_width, body.height), (0.25, 0.5));
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        assert_eq!(native.experience_orb, Some(expected));
        let fields = mc_data::entity_nbt::entity_to_fields(&state.registry.items, native).unwrap();
        assert!(fields.contains(&("Value".into(), Tag::Short(expected.0 as i16))));
        assert!(fields.contains(&("Count".into(), Tag::Int(expected.1))));
        let root = std::env::temp_dir().join(format!(
            "krasnokamen-summon-signed-orb-{}",
            uuid::Uuid::new_v4()
        ));
        mc_world::anvil::write_entity_regions(&root, &state.registry.items, &chunks, 0).unwrap();
        let reopened = mc_world::anvil::read_entity_regions(&root, &state.registry.items).unwrap();
        let restored = reopened
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        assert_eq!(
            restored.experience_orb,
            Some(expected),
            "{nbt}: disk round-trip"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

#[test]
fn experience_orb_summon_enters_native_pickup_processing() {
    let (mut state, id) = summon("experience_orb", "{Value:5s,Count:4}");
    let player = *state.players.keys().next().unwrap();
    crate::experience_orbs::run(&mut state, &mut ChangeSet::default(), &mut Vec::new());
    assert_eq!(state.players[&player].vitals.experience.total, 5);
    let slot = state.entities.slot_of(id).expect("three pickups remain");
    assert_eq!(
        state.entities.row(slot).appearance.experience_orb,
        Some((5, 3))
    );
}

/// Entry-free rule (`scoreboard::require_no_team`): a summon with an explicit
/// `UUID` and a battle faction refuses an entry already in a `/team` team (the
/// edge's faction team would move it out, and a later `/team leave` would
/// disconnect its viewers). Without the team the same summon spawns it.
#[test]
fn summon_with_uuid_and_faction_refuses_an_entry_in_a_team() {
    let (mut state, actor, _) = crate::hostile_tests::setup(8.0);
    let entry = uuid::Uuid::from_u128(7).to_string();
    let team = |state: &mut TickState, args: &[&str]| {
        let command = crate::scoreboard::ScoreboardCommand::parse("team", args).unwrap();
        crate::scoreboard::execute(state, actor, &command);
    };
    team(&mut state, &["add", "allies"]);
    team(&mut state, &["join", "allies", &entry]);
    let summon = |state: &mut TickState| {
        let parsed = crate::commands::parse(
            "summon zombie ~ ~ ~ {UUID:[I;0,0,0,7],\"krasnokamen:Faction\":1}",
            true,
        )
        .unwrap();
        let mut changes = ChangeSet::default();
        state.outbox.clear();
        crate::command_execution::execute(state, ConnectionId(1), actor, &parsed, &mut changes);
        changes.entities_spawned
    };
    assert!(summon(&mut state).is_empty());
    let text = format!(
        "{entry} is in team allies; an entry in a team cannot join a battle faction (team leave {entry} first)"
    );
    assert!(
        state.outbox.values().flatten().any(
            |change| matches!(change, PlayerChange::Chat { message } if message.contains(&text))
        ),
        "{:?}",
        state.outbox
    );
    team(&mut state, &["leave", &entry]);
    let spawned = summon(&mut state);
    assert_eq!(spawned.len(), 1);
    let slot = state.entities.slot_of(spawned[0]).unwrap();
    assert_eq!(state.entities.row(slot).uuid.to_string(), entry);
    assert_eq!(state.entities.body(slot).faction, 1);
}

#[test]
fn negative_experience_orb_pickup_preserves_xp_and_native_mending_damage() {
    for (nbt, expected_damage) in [
        ("{Value:-1s}", None),
        ("{Value:-1s}", Some(20)),
        ("{Value:-32768s}", Some(20)),
    ] {
        let (mut state, id) = summon("experience_orb", nbt);
        let player = *state.players.keys().next().unwrap();
        let actor = state.players.get_mut(&player).unwrap();
        actor.vitals.experience.add_points(10);
        let before = actor.vitals.experience;
        if expected_damage.is_some() {
            actor.inventory.slots[0] = crate::command_items::ItemArgument::parse(
                r#"minecraft:diamond_pickaxe[damage=20,enchantments={"minecraft:mending":1}]"#,
            )
            .unwrap()
            .stack(&state.registry.items)
            .unwrap();
        }
        let mut changes = ChangeSet::default();
        crate::experience_orbs::run(&mut state, &mut changes, &mut Vec::new());
        assert_eq!(state.players[&player].vitals.experience, before, "{nbt}");
        assert_eq!(
            state.players[&player].inventory.slots[0].damage,
            expected_damage
        );
        assert!(state.entities.slot_of(id).is_none());
        assert_eq!(changes.entities_removed, vec![id]);
    }
}

#[test]
fn experience_orb_summon_rejects_invalid_or_misapplied_fields_before_allocation() {
    let mut state = rejected_tag_fixture();
    for value in ["0", "-1", "1.5f", "'bad'"] {
        rejected_tag_in_state(&mut state, "experience_orb", "Count", value);
    }
    rejected_tag_in_state(&mut state, "experience_orb", "Value", "'bad'");
    rejected_tag_in_state(&mut state, "cow", "Value", "5s");
    rejected_tag_in_state(&mut state, "cow", "Count", "4");
}

#[test]
fn persistence_required_iron_golem_template_and_checkpoint_are_native() {
    let (state, id) = summon("iron_golem", "{NoAI:1b,PersistenceRequired:1b}");
    let slot = state.entities.slot_of(id).unwrap();
    let body = state.entities.body(slot);
    let row = state.entities.row(slot);
    assert!(row.persistent && row.controls.no_ai);
    let image = EntityStateImage::from_row(body, row);
    let wal: EntityStateImage =
        serde_json::from_slice(&serde_json::to_vec(&image).unwrap()).unwrap();
    assert!(wal.persistent && wal.controls.no_ai);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    let restored = EntityStateImage::from_record(&state.registry, native).unwrap();
    assert!(restored.persistent && restored.controls.no_ai);
    assert!(
        native
            .extra_fields
            .contains(&("PersistenceRequired".into(), Tag::Byte(1)))
    );
}

#[test]
fn snow_golem_pumpkin_summon_sets_authoritative_metadata() {
    for (nbt, expected) in [("{Pumpkin:0b}", 0), ("{Pumpkin:1b}", 0x10), ("{}", 0x10)] {
        let (state, id) = summon("snow_golem", nbt);
        let slot = state.entities.slot_of(id).unwrap();
        assert_eq!(
            crate::entity_life::snow_golem_pumpkin_flags(&state.entities.row(slot).appearance)
                & 0x10,
            expected,
        );
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let record = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|record| record.uuid == state.entities.row(slot).uuid)
            .unwrap();
        assert!(
            record
                .extra_fields
                .contains(&("Pumpkin".into(), Tag::Byte(i8::from(expected != 0))))
        );
    }
}

#[test]
fn no_ai_is_applied() {
    let (state, id) = summon("cow", "{NoAI:1b}");
    assert!(
        state
            .entities
            .row(state.entities.slot_of(id).unwrap())
            .controls
            .no_ai
    );
}

#[test]
fn invulnerable_is_applied() {
    let (mut state, id) = summon("cow", "{Invulnerable:1b}");
    let slot = state.entities.slot_of(id).unwrap();
    let row = state.entities.row_mut(slot);
    assert!(row.controls.invulnerable);
    assert_eq!(
        state
            .entities
            .land_hit(slot, 1000, 4.0, crate::survival::DamageKind::Melee),
        0.0
    );
}

#[test]
fn summon_no_gravity_and_invulnerable_are_effective_and_persistent() {
    let (mut state, id) = summon("cow", "{NoGravity:1b,Invulnerable:1b,NoAI:1b}");
    let slot = state.entities.slot_of(id).unwrap();
    assert!(state.entities.row(slot).controls.no_gravity);
    assert!(state.entities.row(slot).opaque_entity_fields.is_empty());
    let target = crate::commands::target_of("@e[type=minecraft:cow,nbt={NoGravity:1b}]").unwrap();
    assert!(
        crate::command_execution::resolve_target(&state, id, &target)
            .unwrap()
            .contains(&id)
    );
    let metadata = |state: &TickState| {
        state
            .entities
            .row(slot)
            .appearance
            .extra
            .iter()
            .find(|(index, _)| *index == 5)
            .cloned()
    };
    assert_eq!(
        metadata(&state),
        Some((5, crate::boundary::EntityDataValue::Boolean(true)))
    );
    state.appearance_changed.clear();
    crate::entity_controls::set_no_gravity(&mut state, slot, false);
    assert_eq!(
        metadata(&state),
        Some((5, crate::boundary::EntityDataValue::Boolean(false)))
    );
    assert!(state.appearance_changed.contains(&id));
    crate::entity_controls::set_no_gravity(&mut state, slot, true);
    let mut pos = state.entities.pos(slot);
    pos[1] += 6.0;
    state.entities.set_pos(slot, pos);
    state.entities.set_ground(slot, false);
    let y = pos[1];
    assert_eq!(
        state
            .entities
            .land_hit(slot, 1000, 4.0, crate::survival::DamageKind::Melee),
        0.0,
    );
    for _ in 0..20 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert_eq!(state.entities.pos(slot)[1], y);
    let row = state.entities.row(slot);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    assert!(
        native
            .extra_fields
            .contains(&("NoGravity".into(), Tag::Byte(1)))
    );
    assert!(
        native
            .extra_fields
            .contains(&("Invulnerable".into(), Tag::Byte(1)))
    );
    let restored = EntityStateImage::from_record(&state.registry, native).unwrap();
    assert!(restored.controls.no_gravity && restored.controls.invulnerable);
    let wal: EntityStateImage =
        serde_json::from_slice(&serde_json::to_vec(&restored).unwrap()).unwrap();
    assert!(wal.controls.no_gravity && wal.controls.invulnerable);
}

#[test]
fn summon_no_gravity_floats_in_multiple_mob_motion_lanes() {
    for kind in ["cow", "villager", "pillager", "copper_golem"] {
        for no_gravity in [false, true] {
            let (mut state, id) = summon(
                kind,
                if no_gravity {
                    "{NoGravity:1b}"
                } else {
                    "{NoGravity:0b}"
                },
            );
            let slot = state.entities.slot_of(id).unwrap();
            let mut pos = state.entities.pos(slot);
            pos[1] += 6.0;
            state.entities.set_pos(slot, pos);
            state.entities.set_vel(slot, [0.0; 3]);
            state.entities.set_ground(slot, false);
            for _ in 0..10 {
                crate::tick::run_tick(&mut state, &[]);
            }
            let y = state.entities.pos(slot)[1];
            if no_gravity {
                assert_eq!(y, pos[1], "{kind} must hover");
            } else {
                assert!(y < pos[1], "{kind} must fall");
            }
        }
    }
}

#[test]
fn health_is_applied() {
    let (state, id) = summon("cow", "{Health:7f}");
    assert_eq!(
        state
            .entities
            .row(state.entities.slot_of(id).unwrap())
            .health,
        7.0
    );
}

#[test]
fn motion_is_applied_in_core_units() {
    let (state, id) = summon("cow", "{Motion:[0.1d,0.2d,-0.3d]}");
    assert_eq!(
        state.entities.vel(state.entities.slot_of(id).unwrap()),
        [2.0, 4.0, -6.0]
    );
}

#[test]
fn rotation_is_applied() {
    let (state, id) = summon("cow", "{Rotation:[90f,7f]}");
    assert_eq!(
        state.entities.rot(state.entities.slot_of(id).unwrap())[..2],
        [90.0, 7.0]
    );
}

#[test]
fn age_is_applied_to_an_ageable_species() {
    let (state, id) = summon("cow", "{Age:-1234}");
    let row = state.entities.row(state.entities.slot_of(id).unwrap());
    assert_eq!(row.entity_life.child_ticks, 1234);
    assert!(row.appearance.baby);
}

#[test]
fn custom_name_literal_component_is_applied() {
    for nbt in [
        "{CustomName:'Template Cow'}",
        "{CustomName:{text:'Template Cow'}}",
    ] {
        let (state, id) = summon("cow", nbt);
        let row = state.entities.row(state.entities.slot_of(id).unwrap());
        assert_eq!(row.appearance.custom_name.as_deref(), Some("Template Cow"));
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        assert!(
            native
                .extra_fields
                .contains(&("CustomName".into(), Tag::String("Template Cow".into())))
        );
    }
}

fn rejected_tag_fixture() -> TickState {
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(2),
            uuid: uuid::Uuid::new_v4(),
            name: "Other".into(),
            permission_level: 0,
        }],
    );
    state
}

fn rejected_tag_reaches_sender(kind: &str, key: &str, value: &str) {
    rejected_tag_in_state(&mut rejected_tag_fixture(), kind, key, value);
}

fn rejected_tag_in_state(state: &mut TickState, kind: &str, key: &str, value: &str) {
    state.outbox.clear();
    let before = state.entities.len();
    let (_, drains, _) = crate::tick::run_tick(
        state,
        &[PlayerInput::Command {
            connection: ConnectionId(1),
            command: format!("summon {kind} ~ ~ ~ {{{key}:{value}}}"),
            permission_level: 2,
        }],
    );
    assert_eq!(
        state.entities.len(),
        before,
        "rejected {key} must not allocate an entity"
    );
    let has_error = |connection| {
        drains.iter().any(|(recipient, queue)| {
            *recipient == connection
                && queue.iter().any(|change| {
                    matches!(
                        change, PlayerChange::Chat { message } if message.contains(key)
                    )
                })
        })
    };
    assert!(
        has_error(ConnectionId(1)),
        "{key} error did not reach sender chat: {:?}",
        drains
    );
    assert!(
        !has_error(ConnectionId(2)),
        "{key} error leaked to another player"
    );
}

macro_rules! unsupported_tag {
    ($test:ident, $key:literal, $value:literal) => {
        #[test]
        fn $test() {
            rejected_tag_reaches_sender("cow", $key, $value);
        }
    };
}
#[test]
fn custom_name_visible_is_authoritative_and_can_clear() {
    for visible in [false, true] {
        let (mut state, id) = summon(
            "cow",
            &format!(
                "{{CustomName:'Visible Cow',CustomNameVisible:{}b}}",
                i8::from(visible)
            ),
        );
        let slot = state.entities.slot_of(id).unwrap();
        let row = state.entities.row(slot);
        assert_eq!(row.appearance.custom_name_visible, visible);
        assert!(row.opaque_entity_fields.is_empty());
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        assert_eq!(
            native
                .extra_fields
                .iter()
                .filter(|(key, _)| key == "CustomNameVisible")
                .count(),
            1
        );
        assert!(
            native
                .extra_fields
                .contains(&("CustomNameVisible".into(), Tag::Byte(i8::from(visible))))
        );
        assert_eq!(
            crate::core::appearance_from_vanilla_entity(native).custom_name_visible,
            visible
        );
        let image = EntityStateImage::from_record(&state.registry, native).unwrap();
        assert!(image.opaque_entity_fields.is_empty());
        Arc::make_mut(&mut state.entities.row_mut(slot).appearance).custom_name_visible = false;
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.wire_id == id.0)
            .unwrap();
        assert!(
            native
                .extra_fields
                .contains(&("CustomNameVisible".into(), Tag::Byte(0)))
        );
        let mut invalid = native.clone();
        invalid
            .extra_fields
            .iter_mut()
            .find(|(key, _)| key == "CustomNameVisible")
            .unwrap()
            .1 = Tag::String("true".into());
        assert!(EntityStateImage::from_record(&state.registry, &invalid).is_err());
    }
}
unsupported_tag!(silent_is_explicitly_unsupported, "Silent", "1b");
unsupported_tag!(glowing_is_explicitly_unsupported, "Glowing", "1b");
#[test]
fn summon_tags_match_selectors_and_survive_checkpoint() {
    for (kind, nbt) in [
        ("cow", "{Tags:['x','second']}"),
        ("armor_stand", "{Tags:['x','second']}"),
        (
            "item",
            "{Tags:['x','second'],Item:{id:'minecraft:stone'},PickupDelay:32767}",
        ),
        ("experience_orb", "{Tags:['x','second'],Value:1}"),
    ] {
        let (mut state, id) = summon(kind, nbt);
        let slot = state.entities.slot_of(id).unwrap();
        let retained = state.entities.row(slot).opaque_entity_fields.clone();
        assert!(!retained.is_empty(), "{kind}: no retained tags");
        let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
        let context = crate::command_context::Context::source(&state, ConnectionId(1), actor, 2);
        let before = state.day_time;
        crate::command_context::run(
            &mut state,
            "execute if entity @e[tag=x,tag=second,tag=!absent] run time add 7",
            context,
            &mut ChangeSet::default(),
        );
        assert_eq!(state.day_time, before + 7, "{kind}: selector missed tags");
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.wire_id == id.0)
            .unwrap();
        let restored = EntityStateImage::from_record(&state.registry, native).unwrap();
        assert_eq!(
            restored.opaque_entity_fields, retained,
            "{kind}: checkpoint lost tags"
        );
    }
}

#[test]
fn summon_tags_reject_invalid_types_before_spawn() {
    for value in ["'x'", "1", "[1,2]"] {
        let state = rejected_tag_fixture();
        let nbt = crate::command_snbt::parse(&format!("{{Tags:{value}}}")).unwrap();
        let error = prepare(
            &state,
            state.registry.cow_type_id,
            "minecraft:cow",
            [0.; 3],
            &nbt,
        )
        .err()
        .expect("invalid Tags must be refused");
        assert!(error.message().contains("Tags"), "{}", error.message());
        assert!(error.message().contains("string"), "{}", error.message());
        rejected_tag_reaches_sender("cow", "Tags", value);
    }
}
unsupported_tag!(left_handed_is_explicitly_unsupported, "LeftHanded", "1b");
unsupported_tag!(unknown_tag_names_reach_chat, "UnsupportedExample", "1b");

#[test]
fn is_baby_is_rejected_when_species_has_no_retained_baby_model() {
    rejected_tag_reaches_sender("iron_golem", "IsBaby", "1b");
}

#[test]
fn age_is_rejected_when_species_is_not_ageable() {
    rejected_tag_reaches_sender("iron_golem", "Age", "-100");
}

#[test]
fn unsupported_styled_custom_name_is_not_silently_flattened() {
    rejected_tag_reaches_sender("cow", "CustomName", "{text:'Cow',color:'red'}");
}

#[test]
fn villager_age_uses_existing_villager_authority_and_checkpoint() {
    let (state, id) = summon("villager", "{Age:-321}");
    let row = state.entities.row(state.entities.slot_of(id).unwrap());
    assert_eq!(row.villager.as_ref().unwrap().age, -321);
    assert!(row.appearance.baby);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    let restored = crate::villager::persistence::villager_from_fields(
        &state.registry.items,
        &native.extra_fields,
        state.tick,
    )
    .unwrap();
    assert_eq!(restored.age, -321);
}

#[test]
fn zombie_villager_is_baby_uses_existing_conversion_authority() {
    for baby in [false, true] {
        let (state, id) = summon(
            "zombie_villager",
            &format!("{{IsBaby:{}b}}", i8::from(baby)),
        );
        let row = state.entities.row(state.entities.slot_of(id).unwrap());
        let villager = row.villager.as_ref().unwrap();
        assert_eq!(villager.age < 0, baby);
        assert_eq!(villager.conversion_time, Some(-1));
        assert_eq!(row.appearance.baby, baby);
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|chunk| &chunk.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        let restored = crate::villager::persistence::villager_from_fields(
            &state.registry.items,
            &native.extra_fields,
            state.tick,
        )
        .unwrap();
        assert_eq!(restored.age < 0, baby);
    }
}

#[test]
fn tadpole_age_uses_existing_native_growth_clock() {
    let (state, id) = summon("tadpole", "{Age:1234}");
    let row = state.entities.row(state.entities.slot_of(id).unwrap());
    assert_eq!(row.entity_life.age_lock.tadpole_age, 1234);
    assert_eq!(row.age, 1234);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    assert_eq!(native.age, 1234);
}

#[test]
fn invalid_supported_tags_reach_sender_chat() {
    for (key, value) in [
        ("PersistenceRequired", "'bad'"),
        ("NoAI", "'bad'"),
        ("NoGravity", "'bad'"),
        ("Invulnerable", "'bad'"),
        ("Health", "-1f"),
        ("Rotation", "[90f]"),
        ("Motion", "[0d,0d]"),
        ("Age", "'bad'"),
    ] {
        rejected_tag_reaches_sender("cow", key, value);
    }
    rejected_tag_reaches_sender("zombie_villager", "IsBaby", "'bad'");
    rejected_tag_reaches_sender("tadpole", "Age", "-1");
}

#[test]
fn summon_common_state_survives_real_wal_replay_and_native_region_restart() {
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    let (output, _, _) = crate::tick::run_tick(&mut state, &[PlayerInput::Command {
        connection: ConnectionId(1),
        command: "summon sheep ~ ~ ~ {PersistenceRequired:1b,NoAI:1b,Invulnerable:1b,Age:-1234,Health:7f,CustomName:{text:'Durable Sheep'},CustomNameVisible:1b,CanPickUpLoot:1b,equipment:{mainhand:{id:'minecraft:iron_sword',components:{'minecraft:damage':4}},body:{id:'minecraft:leather_horse_armor'}},drop_chances:{mainhand:2f,body:0.5f}}".into(),
        permission_level: 2,
    }]);
    assert_eq!(output.changes.entities_spawned.len(), 1);
    let id = output.changes.entities_spawned[0];
    let slot = state.entities.slot_of(id).unwrap();
    let uuid = state.entities.row(slot).uuid;
    let expected_equipment = state.entities.row(slot).appearance.equipment.clone();
    let expected_body = state.entities.row(slot).loot.body_armor.clone();
    let root = std::env::temp_dir().join(format!(
        "krasnokamen-summon-common-{}",
        uuid::Uuid::new_v4()
    ));
    let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    let mut writer = crate::journal::JournalWriter::open(&root, 1).unwrap();
    writer.append(1, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let replay = crate::journal::replay_batches(
        crate::journal::Replay::default(),
        &crate::journal::read_complete_batches_after(&root.join("journal"), 0).unwrap(),
    );
    assert_eq!(replay.entities.len(), 1);
    let record = &replay.entities[&id.0];
    assert_eq!(record.appearance.equipment, expected_equipment);
    assert_eq!(record.state.loot.body_armor, expected_body);
    assert_eq!(record.state.loot.drop_chances[0], 2.0);
    assert_eq!(record.state.loot.extra_drop_chances[0], 0.5);
    assert!(record.state.loot.can_pick_up_loot);
    assert!(
        record.state.persistent
            && record.state.controls.no_ai
            && record.state.controls.invulnerable
    );
    assert_eq!(
        record.appearance.custom_name.as_deref(),
        Some("Durable Sheep")
    );
    assert!(record.appearance.custom_name_visible);
    state.remove_entity(slot);
    crate::core::restore_entity(&mut state, id.0, record).unwrap();
    let slot = state.entities.slot_of(id).unwrap();
    let row = state.entities.row(slot);
    assert!(row.persistent && row.controls.no_ai && row.controls.invulnerable);
    assert_eq!(
        row.entity_life.child_ticks,
        record.state.entity_life.child_ticks
    );
    assert_eq!(row.appearance.custom_name.as_deref(), Some("Durable Sheep"));
    assert!(row.appearance.custom_name_visible);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let entities_dir = root.join("entities");
    mc_world::anvil::write_entity_regions(&entities_dir, &state.registry.items, &chunks, 0)
        .unwrap();
    let reopened =
        mc_world::anvil::read_entity_regions(&entities_dir, &state.registry.items).unwrap();
    let native = reopened
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == uuid)
        .unwrap();
    let image = EntityStateImage::from_record(&state.registry, native).unwrap();
    assert!(image.persistent && image.controls.no_ai && image.controls.invulnerable);
    assert_eq!(image.entity_life.child_ticks, row.entity_life.child_ticks);
    assert_eq!(native.wire_id, id.0);
    assert!(
        native
            .extra_fields
            .contains(&("CustomName".into(), Tag::String("Durable Sheep".into())))
    );
    assert!(
        native
            .extra_fields
            .contains(&("CustomNameVisible".into(), Tag::Byte(1)))
    );
    assert!(crate::core::appearance_from_vanilla_entity(native).custom_name_visible);
    assert_eq!(
        crate::entity_loot::equipment_from_fields(&state.registry.items, &native.extra_fields)
            .unwrap(),
        expected_equipment
    );
    assert_eq!(image.loot.body_armor, expected_body);
    assert_eq!(image.loot.drop_chances[0], 2.0);
    assert_eq!(image.loot.extra_drop_chances[0], 0.5);
    assert!(image.loot.can_pick_up_loot);
    assert!(image.opaque_entity_fields.is_empty());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn slime_size_is_applied_and_clamped() {
    for (kind, nbt, size) in [
        ("slime", "{Size:1}", 2),
        ("magma_cube", "{Size:3}", 4),
        ("slime", "{Size:900}", 127),
    ] {
        let (state, id) = summon(kind, nbt);
        let row = state.entities.row(state.entities.slot_of(id).unwrap());
        assert_eq!(row.loot.slime_size, Some(size), "{kind} {nbt}");
    }
    let (mut state, actor, _) = crate::hostile_tests::setup(8.0);
    let parsed = crate::commands::parse("summon cow ~ ~ ~ {Size:1}", true).unwrap();
    let mut changes = ChangeSet::default();
    crate::command_execution::execute(&mut state, ConnectionId(1), actor, &parsed, &mut changes);
    assert!(
        changes.entities_spawned.is_empty(),
        "Size is slime/magma-only"
    );
}

#[test]
fn summon_equipment_all_living_slots_and_saddle_reach_authoritative_state() {
    for kind in [
        "cow",
        "zombie",
        "skeleton",
        "villager",
        "wolf",
        "armor_stand",
    ] {
        let (state, id) = summon(
            kind,
            "{equipment:{mainhand:{id:'minecraft:stick',count:2},offhand:{id:'minecraft:shield'},feet:{id:'minecraft:iron_boots'},legs:{id:'minecraft:iron_leggings'},chest:{id:'minecraft:iron_chestplate'},head:{id:'minecraft:iron_helmet'},body:{id:'minecraft:leather_horse_armor'}}}",
        );
        let row = state.entities.row(state.entities.slot_of(id).unwrap());
        for (stack, name) in row.appearance.equipment.iter().zip([
            "stick",
            "shield",
            "iron_boots",
            "iron_leggings",
            "iron_chestplate",
            "iron_helmet",
        ]) {
            let stack = stack.as_ref().unwrap();
            assert_eq!(
                state
                    .registry
                    .items
                    .name_of(stack.item_id)
                    .unwrap()
                    .as_str(),
                format!("minecraft:{name}"),
                "{kind}"
            );
        }
        assert_eq!(row.appearance.equipment[0].as_ref().unwrap().count, 2);
        assert!(row.loot.body_armor.is_some());
        assert_eq!(
            row.appearance.body_armor.as_ref(),
            row.loot.body_armor.as_deref()
        );
    }
    for kind in ["horse", "pig", "strider"] {
        let (state, id) = summon(kind, "{equipment:{saddle:{id:'minecraft:saddle'}}}");
        let row = state.entities.row(state.entities.slot_of(id).unwrap());
        assert!(row.loot.saddle.is_some(), "{kind}");
        assert_eq!(row.appearance.saddle.as_ref(), row.loot.saddle.as_deref());
    }
}

#[test]
fn summon_preserves_spawn_defaults_unless_equipment_is_authored() {
    let (mut state, id) = summon("skeleton", "{}");
    let slot = state.entities.slot_of(id).unwrap();
    let crate::entities::EntityKind::Mob { type_id } = state.entities.body(slot).kind else {
        panic!("expected mob");
    };
    // Distinct initialized values prove omission preserves, rather than reconstructs, defaults.
    let stack = mc_data::item_nbt::stack_from_component_patch(
        &state.registry.items,
        &mc_data::Identifier::parse("minecraft:bow").unwrap(),
        &[],
    )
    .unwrap();
    {
        let row = state.entities.row_mut(slot);
        Arc::make_mut(&mut row.appearance).equipment[0] = Some(stack.clone());
        row.loot.body_armor = Some(Box::new(stack.clone()));
        row.loot.saddle = Some(Box::new(stack.clone()));
        row.loot.drop_chances = [0.3; 6];
        row.loot.extra_drop_chances = [0.4; 2];
        row.loot.can_pick_up_loot = true;
    }
    let image = prepare(
        &state,
        type_id,
        "minecraft:skeleton",
        [0.0; 3],
        &Tag::Compound(vec![("NoAI".into(), Tag::Byte(1))]),
    )
    .unwrap();
    apply(&mut state, id, image);
    let row = state.entities.row(slot);
    assert_eq!(row.appearance.equipment[0], Some(stack.clone()));
    assert_eq!(row.loot.body_armor.as_deref(), Some(&stack));
    assert_eq!(row.loot.saddle.as_deref(), Some(&stack));
    assert_eq!(row.loot.drop_chances, [0.3; 6]);
    assert_eq!(row.loot.extra_drop_chances, [0.4; 2]);
    assert!(row.loot.can_pick_up_loot);
    let image = prepare(
        &state,
        type_id,
        "minecraft:skeleton",
        [0.0; 3],
        &Tag::Compound(vec![
            ("equipment".into(), Tag::Compound(vec![])),
            (
                "drop_chances".into(),
                Tag::Compound(vec![("head".into(), Tag::Int(2))]),
            ),
            ("CanPickUpLoot".into(), Tag::Byte(0)),
        ]),
    )
    .unwrap();
    apply(&mut state, id, image);
    let row = state.entities.row(slot);
    assert!(row.appearance.equipment.iter().all(Option::is_none));
    assert!(row.loot.body_armor.is_none() && row.loot.saddle.is_none());
    assert_eq!(row.loot.drop_chances, [0.3, 0.3, 0.3, 0.3, 0.3, 2.0]);
    assert_eq!(row.loot.extra_drop_chances, [0.4; 2]);
    assert!(!row.loot.can_pick_up_loot);
}

#[test]
fn summon_equipment_rejects_invalid_stacks_components_and_chances_before_allocation() {
    let mut state = rejected_tag_fixture();
    for value in [
        "1b",
        "{unknown:{}}",
        "{mainhand:1b}",
        "{mainhand:{id:'minecraft:not_an_item'}}",
        "{mainhand:{id:1b}}",
        "{mainhand:{count:1}}",
        "{mainhand:{id:'minecraft:stick',count:'bad'}}",
        "{mainhand:{id:'minecraft:stick',count:-1}}",
        "{mainhand:{id:'minecraft:stick',unknown:1}}",
        "{mainhand:{id:'minecraft:stick',components:1b}}",
        "{mainhand:{id:'minecraft:stick',components:{'minecraft:damage':'bad'}}}",
        "{mainhand:{id:'minecraft:stick',components:{'minecraft:enchantments':{'minecraft:not_an_enchantment':1}}}}",
        "{mainhand:{id:'minecraft:stick',components:{'minecraft:not_a_component':1}}}",
        "{saddle:{id:'minecraft:saddle'}}",
    ] {
        rejected_tag_in_state(&mut state, "cow", "equipment", value);
    }
    for value in ["1b", "{unknown:1f}", "{head:-1f}", "{head:'bad'}"] {
        rejected_tag_in_state(&mut state, "cow", "drop_chances", value);
    }
    rejected_tag_in_state(&mut state, "cow", "CanPickUpLoot", "'bad'");
    for key in ["equipment", "drop_chances", "CanPickUpLoot"] {
        rejected_tag_in_state(&mut state, "oak_boat", key, "{}");
    }
}

#[test]
fn summon_battle_equipment_retains_custom_model_and_asset_in_appearance() {
    let (mut state, id) = summon(
        "zombie",
        "{equipment:{mainhand:{id:'minecraft:iron_sword',components:{'minecraft:item_model':'krasnokamen_battle:iron_longsword'}},chest:{id:'minecraft:iron_chestplate',components:{'minecraft:equippable':{slot:'chest',asset_id:'krasnokamen_battle:holy'}}}}}",
    );
    let row = state.entities.row(state.entities.slot_of(id).unwrap());
    let held = row.appearance.equipment[0].as_ref().unwrap();
    assert_eq!(
        held.item_model.as_ref().unwrap().as_str(),
        "krasnokamen_battle:iron_longsword"
    );
    let chest = row.appearance.equipment[4].as_ref().unwrap();
    let fields = mc_data::item_nbt::stack_to_fields(&state.registry.items, chest).unwrap();
    let Some(Tag::Compound(components)) = mc_data::item_nbt::field(&fields, "components") else {
        panic!("missing components")
    };
    let Some(Tag::Compound(equippable)) =
        mc_data::item_nbt::field(components, "minecraft:equippable")
    else {
        panic!("missing equippable")
    };
    assert_eq!(
        mc_data::item_nbt::field(equippable, "asset_id"),
        Some(&Tag::String("krasnokamen_battle:holy".into()))
    );
    let expected = row.appearance.equipment.clone();
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let native = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    let payload = Tag::Compound(
        mc_data::entity_nbt::entity_to_fields(&state.registry.items, native).unwrap(),
    );
    let mut replay = crate::journal::Replay::default();
    crate::native_checkpoint::hydrate_entity(&state.registry, &mut replay, &payload, state.tick)
        .unwrap();
    state.remove_entity(state.entities.slot_of(id).unwrap());
    crate::core::restore_entity(&mut state, id.0, &replay.entities[&id.0]).unwrap();
    let restored = state.entities.row(state.entities.slot_of(id).unwrap());
    assert_eq!(restored.appearance.equipment, expected);
}

#[test]
fn summon_equipment_components_match_give_and_survive_native_anvil() {
    let (state, id) = summon(
        "cow",
        "{CanPickUpLoot:1b,equipment:{mainhand:{id:'minecraft:diamond_sword',components:{'minecraft:damage':7,'minecraft:custom_name':'Summoned blade','minecraft:enchantments':{'minecraft:sharpness':3},'minecraft:custom_data':{source:'summon'}}},body:{id:'minecraft:leather_horse_armor',components:{'minecraft:custom_name':'Summoned armor'}}},drop_chances:{mainhand:2,offhand:0,feet:0.1f,legs:0.2f,chest:0.3f,head:0.4f,body:0.5f,saddle:0.6f}}",
    );
    let row = state.entities.row(state.entities.slot_of(id).unwrap());
    let held = row.appearance.equipment[0].as_ref().unwrap();
    let given = crate::command_items::ItemArgument::parse(
        "minecraft:diamond_sword[minecraft:damage=7,minecraft:custom_name='Summoned blade',minecraft:enchantments={'minecraft:sharpness':3},minecraft:custom_data={source:'summon'}]",
    )
    .unwrap()
    .stack(&state.registry.items)
    .unwrap();
    assert_eq!(held, &given);
    assert_eq!(held.damage, Some(7));
    assert_eq!(held.custom_name.as_deref(), Some("Summoned blade"));
    assert_eq!(held.enchantments[0].level, 3);
    assert!(row.loot.can_pick_up_loot);
    assert_eq!(row.loot.drop_chances, [2.0, 0.0, 0.1, 0.2, 0.3, 0.4]);
    assert_eq!(row.loot.extra_drop_chances, [0.5, 0.6]);
    let wal = crate::journal::EntityAppearanceImage::from(&*row.appearance);
    let wal: crate::journal::EntityAppearanceImage =
        serde_json::from_slice(&serde_json::to_vec(&wal).unwrap()).unwrap();
    assert_eq!(wal.equipment, row.appearance.equipment);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let root = std::env::temp_dir().join(format!(
        "krasnokamen-summon-equipment-{}",
        uuid::Uuid::new_v4()
    ));
    mc_world::anvil::write_entity_regions(&root, &state.registry.items, &chunks, 0).unwrap();
    let reopened = mc_world::anvil::read_entity_regions(&root, &state.registry.items).unwrap();
    let native = reopened
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    let restored = EntityStateImage::from_record(&state.registry, native).unwrap();
    let equipment =
        crate::entity_loot::equipment_from_fields(&state.registry.items, &native.extra_fields)
            .unwrap();
    assert_eq!(equipment, row.appearance.equipment);
    assert_eq!(restored.loot.body_armor, row.loot.body_armor);
    assert_eq!(restored.loot.drop_chances, row.loot.drop_chances);
    assert_eq!(
        restored.loot.extra_drop_chances,
        row.loot.extra_drop_chances
    );
    assert!(restored.loot.can_pick_up_loot);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn summon_equipment_rejects_the_same_unsupported_components_as_give() {
    assert!(
        crate::command_items::ItemArgument::parse(
            "minecraft:diamond_sword[minecraft:attack_range={}]"
        )
        .is_err()
    );
    rejected_tag_reaches_sender(
        "cow",
        "equipment",
        "{mainhand:{id:'minecraft:diamond_sword',components:{'minecraft:attack_range':{}}}}",
    );
}

#[test]
fn item_summon_with_age_minus_32768_sets_the_never_clock() {
    let (state, id) = summon(
        "item",
        "{Item:{id:'minecraft:carrot',count:1},Age:-32768s,PickupDelay:32767s}",
    );
    let slot = state.entities.slot_of(id).unwrap();
    assert_eq!(
        state.entities.row(slot).age,
        crate::entities::ITEM_AGE_NEVER
    );
    assert_eq!(state.entities.body(slot).pickup_delay, 32767);
}

/// The checkpoint (Anvil) record of an item whose clock is the never value holds
/// the vanilla short, and the pickup delay 32767.
fn plain_item_record(
    state: &TickState,
    id: crate::boundary::EntityId,
) -> mc_data::entity_nbt::VanillaEntity {
    let uuid = state.entities.row(state.entities.slot_of(id).unwrap()).uuid;
    let (_, chunks, _) = crate::core::checkpoint_data_for(state, state.tick);
    chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == uuid)
        .unwrap()
        .clone()
}

#[test]
fn item_never_clock_exports_as_minus_32768_in_the_checkpoint_record() {
    let (mut state, id) = summon("item", "{Item:{id:'minecraft:carrot',count:1}}");
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.row_mut(slot).age = crate::entities::ITEM_AGE_NEVER;
    state.entities.body_mut(slot).pickup_delay = crate::entities::PICKUP_DELAY_NEVER;
    let record = plain_item_record(&state, id);
    assert_eq!(record.age, i32::from(i16::MIN));
    assert_eq!(record.pickup_delay, 32767);
}

#[test]
fn vanilla_item_age_minus_32768_imports_as_the_never_clock() {
    let (state, id) = summon("item", "{Item:{id:'minecraft:carrot',count:1}}");
    let mut record = plain_item_record(&state, id);
    record.age = i32::from(i16::MIN);
    record
        .extra_fields
        .retain(|(key, _)| key != "krasnokamen:LifetimeAge");
    let restored = EntityStateImage::from_record(&state.registry, &record).unwrap();
    assert_eq!(restored.age, crate::entities::ITEM_AGE_NEVER);
}

#[test]
fn item_entity_summon_carries_stack_pickup_delay_age_and_motion() {
    let (state, id) = summon(
        "item",
        "{Item:{id:\"minecraft:carrot\",count:3},PickupDelay:25,Age:100,Motion:[0.0,0.5,0.0]}",
    );
    let slot = state.entities.slot_of(id).unwrap();
    let row = state.entities.row(slot);
    let stack = row.item.as_ref().expect("item entity stack");
    assert_eq!(stack.count, 3);
    assert_eq!(
        state
            .registry
            .items
            .name_of(stack.item_id)
            .unwrap()
            .to_string(),
        "minecraft:carrot"
    );
    assert_eq!(state.entities.body(slot).pickup_delay, 25);
    assert_eq!(row.age, 100);
    assert!((state.entities.vel(slot)[1] - 10.0).abs() < 1e-9);
}

#[test]
fn item_summon_no_gravity_and_invulnerability_survive_checkpoint() {
    let (mut state, id) = summon(
        "item",
        "{Item:{id:'minecraft:carrot',count:1},PickupDelay:32767,NoGravity:1b,Invulnerable:1b}",
    );
    let slot = state.entities.slot_of(id).unwrap();
    let mut pos = state.entities.pos(slot);
    pos[1] += 6.0;
    state.entities.set_pos(slot, pos);
    state.entities.set_vel(slot, [0.0; 3]);
    state.entities.set_ground(slot, false);
    for _ in 0..10 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert_eq!(state.entities.pos(slot)[1], pos[1]);
    let row = state.entities.row(slot);
    let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let record = chunks
        .values()
        .flat_map(|chunk| &chunk.entities)
        .find(|entity| entity.uuid == row.uuid)
        .unwrap();
    let restored = EntityStateImage::from_record(&state.registry, record).unwrap();
    assert!(restored.controls.no_gravity && restored.controls.invulnerable);
}

#[test]
fn item_entity_summon_rejects_missing_item_and_unknown_fields() {
    let (mut state, actor, _) = crate::hostile_tests::setup(8.0);
    for nbt in [
        "{}",
        "{Item:{id:\"minecraft:carrot\",count:0}}",
        "{Item:{id:\"minecraft:carrot\"},Owner:\"x\"}",
    ] {
        let parsed = crate::commands::parse(&format!("summon item ~ ~ ~ {nbt}"), true).unwrap();
        let mut changes = ChangeSet::default();
        crate::command_execution::execute(
            &mut state,
            ConnectionId(1),
            actor,
            &parsed,
            &mut changes,
        );
        assert!(changes.entities_spawned.is_empty(), "{nbt} must be refused");
    }
}

#[test]
fn villager_summon_keeps_its_authored_profession_and_rejects_bad_data() {
    let (state, id) = summon(
        "villager",
        "{VillagerData:{profession:'minecraft:farmer',type:'minecraft:plains',level:2}}",
    );
    let slot = state.entities.slot_of(id).unwrap();
    let data = &state.entities.row(slot).villager.as_ref().unwrap().data;
    assert_eq!(data.profession, crate::villager::VillagerProfession::Farmer);
    assert_eq!(data.level, 2);
    rejected_tag_reaches_sender("villager", "VillagerData", "{profession:'minecraft:bogus'}");
    rejected_tag_reaches_sender("cow", "VillagerData", "{profession:'minecraft:farmer'}");
}

#[test]
fn climate_variant_summons_publish_and_checkpoint_typed_variants() {
    for (kind, registry, index, variants) in [
        (
            "chicken",
            "chicken_variant",
            18,
            &["temperate", "cold", "warm"][..],
        ),
        ("cow", "cow_variant", 18, &["temperate", "cold", "warm"][..]),
        ("pig", "pig_variant", 19, &["temperate", "cold", "warm"][..]),
        (
            "zombie_nautilus",
            "zombie_nautilus_variant",
            21,
            &["temperate", "warm"][..],
        ),
    ] {
        for variant in variants {
            let name = format!("minecraft:{variant}");
            let variant_id = mc_data::required_registry_entry_id(
                registry,
                &mc_data::Identifier::parse(name.as_str()).unwrap(),
            )
            .unwrap();
            let expected = match kind {
                "chicken" => EntityDataValue::ChickenVariant(variant_id),
                "cow" => EntityDataValue::CowVariant(variant_id),
                "pig" => EntityDataValue::PigVariant(variant_id),
                _ => EntityDataValue::ZombieNautilusVariant(variant_id),
            };
            let ages: &[i32] = if kind == "zombie_nautilus" {
                &[0]
            } else {
                &[0, -24000]
            };
            for age in ages {
                let (state, id) = summon(kind, &format!("{{variant:'{name}',Age:{age}}}"));
                let slot = state.entities.slot_of(id).unwrap();
                let row = state.entities.row(slot);
                assert!(
                    row.appearance.extra.contains(&(index, expected.clone())),
                    "{kind}/{variant}"
                );
                assert_eq!(row.appearance.baby, *age < 0, "{kind}/{variant}");
                assert!(state.appearance_changed.contains(&id));
                let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
                let native = chunks
                    .values()
                    .flat_map(|chunk| &chunk.entities)
                    .find(|record| record.uuid == row.uuid)
                    .unwrap();
                assert!(
                    native
                        .extra_fields
                        .contains(&("variant".into(), Tag::String(name.clone())))
                );
                let mut imported = crate::core::appearance_from_vanilla_entity(native);
                crate::core::native_variant(native, &mut imported).unwrap();
                let image: crate::journal::EntityAppearanceImage =
                    serde_json::from_slice(&serde_json::to_vec(&imported).unwrap()).unwrap();
                assert!(
                    image.extra.contains(&(index, (&expected).into())),
                    "{kind}/{variant}"
                );
            }
        }
    }
}

#[test]
fn climate_variant_summons_reject_wrong_registry_and_type_before_allocation() {
    let mut state = rejected_tag_fixture();
    for kind in ["cow", "pig", "chicken", "zombie_nautilus"] {
        for value in ["1", "'minecraft:not_a_variant'", "'bad:UPPERCASE'"] {
            rejected_tag_in_state(&mut state, kind, "variant", value);
        }
    }
    rejected_tag_in_state(&mut state, "zombie_nautilus", "variant", "'minecraft:cold'");
    rejected_tag_in_state(&mut state, "sheep", "variant", "'minecraft:warm'");
}
#[test]
fn summon_tnt_accepts_fuse_and_explosion_power_and_applies_them() {
    for (nbt, fuse, power) in [
        ("{fuse:7s}", 7, 4.0_f32),
        ("{fuse:10}", 10, 4.0_f32),
        ("{fuse:7s,explosion_power:2.5f}", 7, 2.5_f32),
        ("{}", 80, 4.0_f32),
    ] {
        let (mut state, id) = summon("tnt", nbt);
        let slot = state.entities.slot_of(id).unwrap();
        let row = state.entities.row(slot);
        let record = row.tnt.as_ref().expect("primed tnt state");
        assert_eq!(record.fuse, fuse, "{nbt}");
        assert_eq!(record.explosion_power, power, "{nbt}");
        assert_eq!(
            row.appearance
                .extra
                .iter()
                .find(|(index, _)| *index == crate::tnt::FUSE_INDEX),
            Some(&(crate::tnt::FUSE_INDEX, EntityDataValue::VarInt(fuse as i32))),
            "{nbt}"
        );
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|c| &c.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        let image = EntityStateImage::from_record(&state.registry, native).unwrap();
        let record = image.tnt.expect("tnt checkpoint image");
        assert_eq!(record.fuse, fuse, "{nbt}");
        assert_eq!(record.explosion_power, power, "{nbt}");
        state.remove_entity(slot);
    }
}

#[test]
fn summon_falling_block_accepts_block_state_and_time_and_defaults_to_sand() {
    let (probe, ..) = crate::hostile_tests::setup(8.0);
    let sand = probe.registry.states.sand;
    let gravel = probe
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:gravel").unwrap())
        .unwrap()
        .default
        .0;
    for (nbt, block_state, age) in [
        ("{BlockState:{Name:\"minecraft:sand\"}}", sand, 0),
        ("{BlockState:{Name:\"minecraft:gravel\"},Time:9}", gravel, 9),
        ("{}", sand, 0),
    ] {
        let (mut state, id) = summon("falling_block", nbt);
        let slot = state.entities.slot_of(id).unwrap();
        let row = state.entities.row(slot);
        let falling = row.falling.as_ref().expect("falling block state");
        assert_eq!(falling.block_state, block_state, "{nbt}");
        assert_eq!(falling.age, age, "{nbt}");
        assert_eq!(row.appearance.spawn_data, block_state as i32, "{nbt}");
        let (_, chunks, _) = crate::core::checkpoint_data_for(&state, state.tick);
        let native = chunks
            .values()
            .flat_map(|c| &c.entities)
            .find(|entity| entity.uuid == row.uuid)
            .unwrap();
        let image = EntityStateImage::from_record(&state.registry, native).unwrap();
        let falling = image.falling.expect("falling checkpoint image");
        assert_eq!(falling.block_state, block_state, "{nbt}");
        assert_eq!(falling.age, age, "{nbt}");
        state.remove_entity(slot);
    }
}

#[test]
fn summon_rejects_tnt_and_falling_block_fields_on_wrong_entities_and_bad_values() {
    let mut state = rejected_tag_fixture();
    // fuse belongs to primed TNT only.
    rejected_tag_in_state(&mut state, "zombie", "fuse", "5s");
    rejected_tag_in_state(&mut state, "falling_block", "fuse", "5s");
    // BlockState belongs to falling blocks only.
    rejected_tag_in_state(&mut state, "tnt", "BlockState", "{Name:\"minecraft:sand\"}");
    // A non-gravity block is not a valid falling block state.
    rejected_tag_in_state(
        &mut state,
        "falling_block",
        "BlockState",
        "{Name:\"minecraft:stone\"}",
    );
    rejected_tag_in_state(
        &mut state,
        "falling_block",
        "BlockState",
        "{Name:\"minecraft:not_a_block\"}",
    );
    // Negative Time is invalid.
    rejected_tag_in_state(&mut state, "falling_block", "Time", "-1");
}
