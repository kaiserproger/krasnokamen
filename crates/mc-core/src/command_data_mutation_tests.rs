use super::*;
use crate::command_execution::CommandResult::{Failed, Success};

fn mutate(state: &mut TickState, context: Context, text: &str) -> Result<i32, String> {
    match data(state, context, text) {
        -1 => Err("data command failed".into()),
        value => Ok(value),
    }
}

fn prepare(
    state: &TickState,
    context: Context,
    original: &Tag,
    action: &str,
    mut arguments: &str,
) -> Result<(i32, Tag), String> {
    let mut candidate = original.clone();
    let count = crate::command_storage::mutate(
        state,
        context,
        action,
        &mut candidate,
        &mut arguments,
        |_| Ok(()),
    )?;
    Ok((count, candidate))
}

#[test]
fn data_mutation_sources_read_live_entity_and_block_owners_atomically() {
    let (mut state, id, context) = fixture();
    state.players.get_mut(&id).unwrap().vitals.health = 13.5;
    run(&mut state, context, "setblock 4 60 4 minecraft:furnace");
    let stack = item(&state, "minecraft:iron_ingot", 3);
    state
        .block_entities
        .entry((4, 60, 4))
        .or_insert_with(|| crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 3],
            furnace: Some(Default::default()),
        })
        .slots[2] = stack;
    let mut changes = ChangeSet::default();
    let mut command = |state: &mut TickState, text: &str| {
        crate::command_context::native(state, text, context, &mut changes, true).unwrap()
    };
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:copy health set from entity @s Health"
        ),
        Success(1)
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:copy count set from block 4 60 4 Items[{Slot:2b}].count"
        ),
        Success(1)
    );
    let root = state.command_storage["test:copy"].clone();
    assert_eq!(*get(&root, "health").unwrap(), Tag::Float(13.5));
    assert_eq!(*get(&root, "count").unwrap(), Tag::Int(3));
    state.command_storage_dirty = false;
    for text in [
        "data modify storage test:copy health set from entity @s Absent",
        "data modify storage test:copy health set from entity @a Health",
        "data modify storage test:copy health set from block 4 60 4 Items[{Slot:9b}].count",
        "data modify storage test:copy health set from entity @s Health trailing",
    ] {
        assert_eq!(command(&mut state, text), Failed, "{text}");
        assert_eq!(state.command_storage["test:copy"], root, "{text}");
        assert!(!state.command_storage_dirty, "{text}");
    }
    assert_eq!(state.players[&id].vitals.health, 13.5);
    assert_eq!(
        state.block_entities.get(&(4, 60, 4)).unwrap().slots[2].count,
        3
    );
}

#[test]
fn data_mutation_preparation_does_not_commit_partial_nbt_edits() {
    let (state, _, context) = fixture();
    let original = crate::command_snbt::parse("{values:[1,2],nested:{keep:3}}").unwrap();
    let (count, candidate) = prepare(
        &state,
        context,
        &original,
        "modify",
        "nested merge value {new:4}",
    )
    .unwrap();
    assert_eq!(count, 1);
    assert_eq!(*get(&candidate, "nested.new").unwrap(), Tag::Int(4));
    assert!(get(&original, "nested.new").is_err());
    assert!(state.command_storage.is_empty());
    assert!(!state.command_storage_dirty);
    for arguments in [
        "values append value 'wrong-type'",
        "nested.keep set value 4 trailing",
        "nested.keep set from storage test:missing value",
    ] {
        assert!(
            prepare(&state, context, &original, "modify", arguments,).is_err(),
            "{arguments}"
        );
        assert_eq!(*get(&original, "nested.keep").unwrap(), Tag::Int(3));
        assert_eq!(
            *get(&original, "values").unwrap(),
            crate::command_snbt::parse("[1,2]").unwrap()
        );
    }
}

#[test]
fn data_mutation_entity_native_delta_preserves_identity_and_unrelated_runtime() {
    let (mut state, _, context) = fixture();
    let mut changes = ChangeSet::default();
    assert_eq!(
        crate::command_context::native(
            &mut state,
            "summon minecraft:cow 4 60 4 {CustomName:'before'}",
            context,
            &mut changes,
            true,
        ),
        Ok(Success(1))
    );
    let id = changes.entities_spawned[0];
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.body_mut(slot).attack_cooldown = 17;
    state.entities.row_mut(slot).breed_cooldown = 35;
    state.entities.row_mut(slot).entity_life.taming.rolls = 123;
    let uuid = state.entities.row(slot).uuid;
    assert_eq!(mutate(&mut state, context,
        "data merge entity @e[type=minecraft:cow,limit=1] {Health:7.5f,NoAI:1b,Invulnerable:1b,CustomName:'after',Tags:['native'],UUID:[I;1,2,3,4],Motion:[0.25d,0.0d,0.0d]}"
    ).unwrap(), 1);
    assert_eq!(state.entities.slot_of(id), Some(slot));
    let row = state.entities.row(slot);
    assert_eq!(row.uuid, uuid);
    assert_eq!(row.health, 7.5);
    assert_eq!(row.appearance.health, 7.5);
    assert_eq!(row.appearance.custom_name.as_deref(), Some("after"));
    assert!(row.controls.no_ai && row.controls.invulnerable);
    assert_eq!(row.breed_cooldown, 35);
    assert_eq!(row.entity_life.taming.rolls, 123);
    assert_eq!(state.entities.body(slot).attack_cooldown, 17);
    assert_eq!(state.entities.vel(slot), [5.0, 0.0, 0.0]);
    assert!(state.appearance_changed.contains(&id));
    let after = entity(&state, id).unwrap();
    assert_eq!(*get(&after, "Health").unwrap(), Tag::Float(7.5));
    assert_eq!(
        *get(&after, "Tags[0]").unwrap(),
        Tag::String("native".into())
    );
}

#[test]
fn data_mutation_entity_preflight_is_atomic_and_players_are_rejected() {
    let (mut state, player, context) = fixture();
    let mut changes = ChangeSet::default();
    assert_eq!(
        crate::command_context::native(
            &mut state,
            "summon minecraft:cow 4 60 4",
            context,
            &mut changes,
            true,
        ),
        Ok(Success(1))
    );
    let id = changes.entities_spawned[0];
    let slot = state.entities.slot_of(id).unwrap();
    let row = state.entities.row(slot).clone();
    let body = *state.entities.body(slot);
    assert!(mutate(
        &mut state,
        context,
        "data merge entity @e[type=minecraft:cow,limit=1] {Health:7.0f,equipment:{not_a_slot:{id:'minecraft:stone'}}}",
    ).is_err());
    assert_eq!(state.entities.row(slot), &row);
    assert_eq!(state.entities.body(slot), &body);
    let before = entity(&state, player).unwrap();
    assert!(mutate(&mut state, context, "data merge entity @s {Health:1.0f}").is_err());
    assert_eq!(entity(&state, player).unwrap(), before);
}

#[test]
fn data_mutation_block_commits_live_inventory_without_resetting_furnace() {
    let (mut state, _, context) = fixture();
    let pos = BlockPos { x: 4, y: 60, z: 4 };
    run(&mut state, context, "setblock 4 60 4 minecraft:furnace");
    let stack = item(&state, "minecraft:iron_ingot", 3);
    let furnace = mc_inventory::furnace::FurnaceState {
        lit_remaining: 87,
        lit_total: 1600,
        cook_progress: 31,
        cook_total: 200,
        ..Default::default()
    };
    state.block_entities.insert(
        (4, 60, 4),
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY, ItemStack::EMPTY, stack],
            furnace: Some(furnace),
        },
    );
    assert_eq!(
        mutate(
            &mut state,
            context,
            "data modify block 4 60 4 Items[0].count set value 5"
        )
        .unwrap(),
        1
    );
    let live = state.block_entities.get(&(4, 60, 4)).unwrap();
    assert_eq!(live.slots[2].count, 5);
    let furnace = live.furnace.as_ref().unwrap();
    assert_eq!(
        (
            furnace.lit_remaining,
            furnace.lit_total,
            furnace.cook_progress,
            furnace.cook_total
        ),
        (87, 1600, 31, 200)
    );
    assert_eq!(
        *get(&block(&state, pos).unwrap(), "Items[0].count").unwrap(),
        Tag::Int(5)
    );
    assert!(
        state
            .opaque_changes
            .iter()
            .any(|(changed, bytes)| *changed == pos && bytes.is_some())
    );

    let before = block(&state, pos).unwrap();
    let queued = state.opaque_changes.len();
    assert!(mutate(&mut state, context,
        "data merge block 4 60 4 {cooking_time_spent:99s,Items:[{Slot:2b,id:'test:missing_item',count:1}]}"
    ).is_err());
    assert_eq!(block(&state, pos).unwrap(), before);
    assert_eq!(state.opaque_changes.len(), queued);
}

#[test]
fn data_mutation_block_preserves_campfire_progress_on_metadata_edits() {
    let (mut state, _, context) = fixture();
    let pos = BlockPos { x: 4, y: 60, z: 4 };
    run(&mut state, context, "setblock 4 60 4 minecraft:campfire");
    let Tag::Compound(fields) = crate::command_snbt::parse(
        "{Items:[{Slot:0b,id:'minecraft:beef',count:1}],CookingTimes:[I;13,0,0,0],CookingTotalTimes:[I;600,0,0,0]}"
    ).unwrap() else { panic!("compound"); };
    let campfire =
        crate::campfire::CampfireState::from_fields(&fields, &state.registry.items).unwrap();
    state.campfires.insert((4, 60, 4), campfire.clone());
    assert_eq!(
        mutate(
            &mut state,
            context,
            "data merge block 4 60 4 {CustomName:'kept'}"
        )
        .unwrap(),
        1
    );
    assert_eq!(state.campfires[&(4, 60, 4)], campfire);
    assert_eq!(
        *get(&block(&state, pos).unwrap(), "CookingTimes").unwrap(),
        Tag::IntArray(vec![13, 0, 0, 0])
    );
    assert!(
        mutate(
            &mut state,
            context,
            "data merge block 4 60 4 {CustomName:'invalid',CookingTotalTimes:[I;1]}"
        )
        .is_err()
    );
    assert_eq!(state.campfires[&(4, 60, 4)], campfire);
    assert_eq!(
        *get(&block(&state, pos).unwrap(), "CustomName").unwrap(),
        Tag::String("kept".into())
    );
}

#[test]
fn data_mutation_entity_sources_and_failure_results_use_normal_command_dispatch() {
    let (mut state, _, context) = fixture();
    run(&mut state, context, "summon minecraft:cow 4 60 4");
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    run(
        &mut state,
        context,
        "data merge storage test:source {health:6.0f}",
    );
    run(
        &mut state,
        context,
        "execute store result score count oracle store success score ok oracle run data modify entity @e[type=minecraft:cow,limit=1] Health set from storage test:source health",
    );
    assert_eq!(score(&state, "count"), 1);
    assert_eq!(score(&state, "ok"), 1);
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @e[type=minecraft:cow,limit=1] Health"
        )
        .unwrap(),
        6
    );
    run(
        &mut state,
        context,
        "execute store success score ok oracle run data merge entity @s {Health:1.0f}",
    );
    assert_eq!(score(&state, "ok"), 0);
}

#[test]
fn data_mutation_existing_entity_and_item_survive_checkpoint_then_real_wal_restart() {
    use crate::boundary::PlayerInput;
    use crate::native_checkpoint::{encode_entities, record, refresh_metadata};
    let (mut state, player, context) = fixture();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    let mut changes = ChangeSet::default();
    for command in [
        "summon minecraft:cow 10 64 10 {PersistenceRequired:1b,NoAI:1b,CustomName:'before'}",
        "summon minecraft:item 12 64 12 {Item:{id:'minecraft:diamond',count:9},PickupDelay:100s}",
    ] {
        assert_eq!(
            crate::command_context::native(&mut state, command, context, &mut changes, true),
            Ok(Success(1))
        );
    }
    let cow = changes.entities_spawned[0];
    let item_id = changes.entities_spawned[1];
    let cow_uuid = state
        .entities
        .row(state.entities.slot_of(cow).unwrap())
        .uuid
        .to_string();
    let drop_uuid = state
        .entities
        .row(state.entities.slot_of(item_id).unwrap())
        .uuid
        .to_string();
    let directory = tempfile::tempdir().unwrap();
    let config = crate::CoreConfig::new(directory.path().to_owned(), 42);
    let store = crate::native_checkpoint::tests::fixture_store(&config).unwrap();
    state.tick = 1;
    let (_, captured, _) = crate::core::checkpoint_data_for(&state, state.tick);
    let entities = encode_entities(
        state.dimension,
        &captured,
        &state.registry.items,
        state.tick,
    )
    .unwrap();
    let mut checkpoint = store.read_checkpoint().unwrap();
    let mut manifest: crate::journal::CheckpointManifest =
        serde_json::from_slice(&checkpoint.globals["checkpoint"].payload).unwrap();
    manifest.tick = state.tick;
    for child in manifest.dimensions.values_mut() {
        child.tick = state.tick;
    }
    checkpoint.globals.insert(
        "checkpoint".into(),
        record(state.tick, serde_json::to_vec(&manifest).unwrap()),
    );
    refresh_metadata(&mut checkpoint.globals, &manifest).unwrap();
    for global in checkpoint.globals.values_mut() {
        global.captured_tick = state.tick;
    }
    store
        .commit_checkpoint(
            mc_world::native::NativeCheckpoint {
                tick: state.tick,
                previous_tick: checkpoint.tick,
                chunks: &[],
                entities: &entities,
                players: &[],
                globals: &checkpoint.globals,
            },
            &state.registry.blocks,
            &state.registry.items,
        )
        .unwrap();
    drop(store);

    let connection = state.players[&player].connection;
    let inputs = [
        "data merge entity @e[type=minecraft:cow,limit=1] {Health:6.0f,CustomName:'after',Tags:['durable']}",
        "data modify entity @e[type=minecraft:item,limit=1] Item.count set value 4",
    ].map(|command| PlayerInput::Command {
        connection, command: command.into(), permission_level: 2,
    });
    let (output, _, _) = crate::tick::run_tick(&mut state, &inputs);
    assert!(
        output.changes.entities_spawned.is_empty(),
        "the action edits existing entities"
    );
    assert_eq!(
        state
            .entities
            .row(state.entities.slot_of(cow).unwrap())
            .health,
        6.0
    );
    assert_eq!(
        state
            .entities
            .row(state.entities.slot_of(item_id).unwrap())
            .item
            .as_ref()
            .unwrap()
            .count,
        4
    );
    let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    let mut writer = crate::journal::JournalWriter::open_test(directory.path(), 1).unwrap();
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    drop(state);

    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let replay = &recovered.dimensions[&crate::dimensions::Dimension::Overworld];
    let cow = &replay.entities[&cow.0];
    let item = &replay.entities[&item_id.0];
    assert_eq!(cow.uuid, cow_uuid);
    assert_eq!(item.uuid, drop_uuid);
    assert_eq!(cow.health, 6.0);
    assert_eq!(cow.appearance.custom_name.as_deref(), Some("after"));
    assert_eq!(item.item.as_ref().unwrap().count, 4);
    let (_, tags) = mc_nbt::read_named(&mut cow.state.opaque_entity_fields.as_slice()).unwrap();
    assert_eq!(
        *get(&tags, "Tags[0]").unwrap(),
        Tag::String("durable".into())
    );
}

#[test]
fn data_mutation_preserves_live_hopper_and_gateway_clocks() {
    let (mut state, _, context) = fixture();
    let pos = BlockPos { x: 4, y: 60, z: 4 };
    run(&mut state, context, "setblock 4 60 4 minecraft:hopper");
    state.tick = 20;
    state
        .redstone_containers
        .apply_data_cooldown(pos, 7, state.tick);
    state.tick = 23;
    mutate(
        &mut state,
        context,
        "data merge block 4 60 4 {CustomName:'clock'}",
    )
    .unwrap();
    assert_eq!(
        state.redstone_containers.data_cooldown(pos, state.tick),
        Some(4)
    );
    mutate(
        &mut state,
        context,
        "data modify block 4 60 4 TransferCooldown set value 1",
    )
    .unwrap();
    assert_eq!(
        state.redstone_containers.data_cooldown(pos, state.tick),
        Some(1)
    );
    run(&mut state, context, "setblock 4 60 4 minecraft:end_gateway");
    mutate(&mut state, context, "data merge block 4 60 4 {Age:20L}").unwrap();
    state.gateway_ages.get_mut(&pos).unwrap().age = 42;
    mutate(
        &mut state,
        context,
        "data merge block 4 60 4 {ExactTeleport:1b}",
    )
    .unwrap();
    let gateway = crate::end_portals::gateway(&state, pos).unwrap();
    assert_eq!(gateway.age, 42);
    assert!(gateway.exact);
    let before = block(&state, pos).unwrap();
    assert!(
        mutate(
            &mut state,
            context,
            "data merge block 4 60 4 {Age:0L,exit_portal:[I;1,2]}"
        )
        .is_err()
    );
    assert_eq!(block(&state, pos).unwrap(), before);
    assert_eq!(crate::end_portals::gateway(&state, pos).unwrap(), gateway);
}

#[test]
fn data_mutation_vibration_owners_preserve_live_deadlines_and_reject_partial_updates() {
    let (mut state, _, context) = fixture();
    for (index, (name, scalar, deadline)) in [
        (
            "sculk_sensor",
            "last_vibration_frequency",
            "krasnokamen:phase_until",
        ),
        (
            "calibrated_sculk_sensor",
            "last_vibration_frequency",
            "krasnokamen:phase_until",
        ),
        (
            "sculk_shrieker",
            "warning_level",
            "krasnokamen:shriek_until",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let pos = BlockPos {
            x: 4 + index as i32 * 2,
            y: 60,
            z: 4,
        };
        let target = format!("block {} {} {}", pos.x, pos.y, pos.z);
        run(
            &mut state,
            context,
            &format!("setblock {} {} {} minecraft:{name}", pos.x, pos.y, pos.z),
        );
        state.tick = 50;
        let Tag::Compound(mut fields) = crate::command_snbt::parse(&format!(
            "{{id:'minecraft:{name}',{scalar}:1,'{deadline}':90L,listener:{{selector:{{tick:-1L}},event_delay:10,event:{{game_event:'minecraft:step',distance:4.0f,pos:[4.0d,60.0d,4.0d]}}}}}}"
        )).unwrap() else { panic!("compound"); };
        // Seed an admitted resident image without updating the stale disk image:
        // subsequent /data reads must consult the actual live owner.
        if name == "sculk_shrieker" {
            crate::shrieker::removed(&mut state, pos);
            let prepared = crate::shrieker::prepare_data(&state, pos, &mut fields, true).unwrap();
            crate::shrieker::apply_data(&mut state, pos, prepared);
        } else {
            crate::sculk::removed(&mut state, pos);
            let prepared = crate::sculk::prepare_data(&state, pos, &mut fields, true).unwrap();
            crate::sculk::apply_data(&mut state, pos, prepared);
        }
        let pending = crate::vibration::pending(&state, pos).cloned().unwrap();
        state.tick = 53;
        assert_eq!(
            query(&mut state, context, &format!("data get {target} {scalar}")).unwrap(),
            1
        );
        assert_eq!(
            query(
                &mut state,
                context,
                &format!("data get {target} listener.event_delay")
            )
            .unwrap(),
            7
        );
        mutate(
            &mut state,
            context,
            &format!("data modify {target} {scalar} set value 2"),
        )
        .unwrap();
        assert_eq!(
            query(&mut state, context, &format!("data get {target} {scalar}")).unwrap(),
            2
        );
        assert_eq!(crate::vibration::pending(&state, pos), Some(&pending));
        let before = block(&state, pos).unwrap();
        let writes = state.opaque_changes.len();
        assert!(
            mutate(
                &mut state,
                context,
                &format!("data merge {target} {{{scalar}:3,listener:{{event_delay:-1}}}}")
            )
            .is_err()
        );
        assert_eq!(block(&state, pos).unwrap(), before);
        assert_eq!(crate::vibration::pending(&state, pos), Some(&pending));
        assert_eq!(state.opaque_changes.len(), writes);
        assert!(
            mutate(
                &mut state,
                context,
                &format!("data modify {target} '{deadline}' set value 0L")
            )
            .is_err()
        );

        mutate(
            &mut state,
            context,
            &format!("data modify {target} listener.event_delay set value 5"),
        )
        .unwrap();
        assert_eq!(crate::vibration::pending(&state, pos).unwrap().arrival, 58);
        let native = if name == "sculk_shrieker" {
            crate::shrieker::data_fields(&state, pos).unwrap()
        } else {
            crate::sculk::data_fields(&state, pos).unwrap()
        };
        assert_eq!(
            mc_data::item_nbt::field(&native, deadline),
            Some(&Tag::Long(90))
        );
        mutate(
            &mut state,
            context,
            &format!("data remove {target} listener"),
        )
        .unwrap();
        assert!(crate::vibration::pending(&state, pos).is_none());
    }
}

#[test]
fn data_mutation_catalyst_updates_live_cursors_without_resetting_bloom() {
    let (mut state, _, context) = fixture();
    let pos = BlockPos { x: 4, y: 60, z: 4 };
    run(
        &mut state,
        context,
        "setblock 4 60 4 minecraft:sculk_catalyst",
    );
    crate::sculk::removed(&mut state, pos);
    let Tag::Compound(mut fields) = crate::command_snbt::parse(
        "{id:'minecraft:sculk_catalyst','krasnokamen:bloom_until':90L,cursors:[{pos:[I;4,60,4],charge:12,decay_delay:1,update_delay:3}]}"
    ).unwrap() else { panic!("compound"); };
    let prepared = crate::sculk::prepare_data(&state, pos, &mut fields, false).unwrap();
    crate::sculk::apply_data(&mut state, pos, prepared);
    assert_eq!(
        query(
            &mut state,
            context,
            "data get block 4 60 4 cursors[0].charge"
        )
        .unwrap(),
        12
    );
    mutate(
        &mut state,
        context,
        "data modify block 4 60 4 cursors[0].charge set value 8",
    )
    .unwrap();
    assert_eq!(
        query(
            &mut state,
            context,
            "data get block 4 60 4 cursors[0].charge"
        )
        .unwrap(),
        8
    );
    let before = block(&state, pos).unwrap();
    assert!(
        mutate(
            &mut state,
            context,
            "data modify block 4 60 4 cursors[0] merge value {charge:5,pos:[I;1,2]}"
        )
        .is_err()
    );
    assert_eq!(block(&state, pos).unwrap(), before);
    let native = crate::sculk::data_fields(&state, pos).unwrap();
    assert_eq!(
        mc_data::item_nbt::field(&native, "krasnokamen:bloom_until"),
        Some(&Tag::Long(90))
    );
}

/// Vanilla's `data merge entity {Health:0}` kills a living mob: the merge is accepted (the
/// summon path refuses zero health because a summon must be alive), the row keeps health 0
/// and the mob enters the corpse clock rather than lingering alive at zero health.
#[test]
fn data_merge_health_zero_kills_a_living_mob_like_vanilla() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:cow 4 60 4 {Health:9f}",
    );
    let cow = "@e[type=minecraft:cow,limit=1]";
    let slot = state
        .entities
        .slots()
        .find(|slot| matches!(state.entities.body(*slot).kind, EntityKind::Mob { .. }))
        .expect("the summoned cow");
    // A summon must be alive: a zero Health is refused with Invalid Health, and nothing spawns.
    let slots_before = state.entities.slots().count();
    assert_eq!(
        crate::command_context::run(
            &mut state,
            "summon minecraft:cow 4 60 4 {Health:0f}",
            context,
            &mut ChangeSet::default()
        ),
        0,
        "no branch of the zero-health summon succeeds"
    );
    assert_eq!(
        state.entities.slots().count(),
        slots_before,
        "the refused summon spawns nothing"
    );
    assert!(
        said(&mut state).contains("Invalid Health"),
        "the refusal says Invalid Health: {}",
        said(&mut state)
    );
    // Deliberate difference from vanilla (recorded in the row): vanilla clamps a negative
    // Health to 0 and the mob dies; here the same normaliser refuses it as Invalid Health.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Health:-5f}}")
        ),
        -1,
        "a negative Health is refused"
    );
    assert!(
        state.entities.body(slot).alive,
        "the refused merge leaves the cow alive"
    );
    assert_eq!(state.entities.row(slot).health, 9.0);
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Health:0f}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    let id = state.entities.body(slot).id;
    assert_eq!(state.entities.row(slot).health, 0.0);
    assert!(!state.entities.body(slot).alive, "zero health is a death");
    assert!(
        state.entities.is_dying(id),
        "the mob enters the corpse clock"
    );
    // A dead mob is not a data target any more: the same merge on the corpse is refused.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Health:5f}}")
        ),
        -1
    );
    assert!(reported(&mut state));
}
