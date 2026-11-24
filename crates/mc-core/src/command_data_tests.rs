use super::*;
use crate::boundary::ChangeSet;
use crate::entities::EntityKind;
use mc_data::{Identifier, ItemStack};

#[path = "command_data_mutation_tests.rs"]
mod mutation_tests;

fn fixture() -> (TickState, EntityId, Context) {
    let (mut state, id, _) = crate::hostile_tests::setup(8.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, [1.75, 60.0, 2.25]);
    let context = Context::source(&state, crate::command_console::CONNECTION, id, 2);
    (state, id, context)
}
fn run(state: &mut TickState, context: Context, command: &str) {
    crate::command_context::run(state, command, context, &mut ChangeSet::default());
}
fn score(state: &TickState, holder: &str) -> i32 {
    state
        .scoreboard
        .scores
        .get(holder)
        .and_then(|scores| scores.get("oracle"))
        .unwrap_or_else(|| {
            panic!(
                "missing {holder} score; objectives={:?}; feedback={:?}",
                state.scoreboard.objectives, state.outbox
            )
        })
        .value
}
fn item(state: &TickState, name: &str, count: i32) -> ItemStack {
    ItemStack::new(
        state
            .registry
            .items
            .id_of(&Identifier::parse(name).unwrap())
            .unwrap(),
        count,
    )
}
#[test]
fn data_paths_cover_filters_quoted_keys_arrays_and_result_types() {
    let root = crate::command_snbt::parse("{Inventory:[{Slot:0b,id:'minecraft:stone',count:3},{Slot:1b,id:'minecraft:dirt',count:4}],Pos:[-1.25d,60.0d,2.0d],UUID:[I;1,2,3,4],foo:{'a.b':{text:'a😀'}}}").unwrap();
    assert_eq!(
        *get(&root, "Inventory[{Slot:1b}].count").unwrap(),
        Tag::Int(4)
    );
    assert_eq!(*get(&root, "Pos[-1]").unwrap(), Tag::Double(2.0));
    assert_eq!(*get(&root, "UUID[0]").unwrap(), Tag::Int(1));
    assert_eq!(result(&get(&root, "Pos[0]").unwrap()), -2);
    assert_eq!(result(&get(&root, "Inventory").unwrap()), 2);
    assert_eq!(result(&get(&root, "foo.\"a.b\".text").unwrap()), 3);
    // A path that selects several values has no single result.
    assert!(get(&root, "Inventory[].count").is_err());
    assert!(get(&root, "Inventory[{Slot:9b}]").is_err());
    assert!(get(&root, "Pos[100]").is_err());
    let mut input = " Inventory[{id: 'minecraft:stone'}].count 2";
    assert_eq!(
        argument(&mut input).unwrap(),
        "Inventory[{id: 'minecraft:stone'}].count"
    );
    assert_eq!(argument(&mut input).unwrap(), "2");
    assert!(argument(&mut input).is_err());
    assert_eq!(snbt(&root), "{Inventory: [{Slot: 0b, count: 3, id: \"minecraft:stone\"}, {Slot: 1b, count: 4, id: \"minecraft:dirt\"}], Pos: [-1.25d, 60.0d, 2.0d], UUID: [I; 1, 2, 3, 4], foo: {\"a.b\": {text: \"a😀\"}}}".replace("\"a.b\":", "a.b:"));
}
#[test]
fn data_player_queries_read_live_inventory_vitals_and_absent_paths() {
    let (mut state, id, context) = fixture();
    let stack = item(&state, "minecraft:stone", 7);
    let player = state.players.get_mut(&id).unwrap();
    player.inventory.slots[0] = stack;
    player.inventory.selected = 0;
    player.vitals.health = 13.5;
    player.vitals.food = 9;
    assert_eq!(
        query(&mut state, context, "data get entity @s Health").unwrap(),
        13
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s Health 10").unwrap(),
        135
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s foodLevel").unwrap(),
        9
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s SelectedItem.count").unwrap(),
        7
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @s Inventory[{Slot:0b}].count"
        )
        .unwrap(),
        7
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s Pos[0] -10").unwrap(),
        -18
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s EnderItems").unwrap(),
        0
    );
    assert_eq!(query(&mut state, context, "data get entity @s").unwrap(), 1);
    let tag = entity(&state, id).unwrap();
    assert_eq!(
        *get(&tag, "Dimension").unwrap(),
        Tag::String("minecraft:overworld".into())
    );
    assert!(get(&tag, "KrasnokamenStatistics").is_err());
    assert!(query(&mut state, context, "data get entity @s Pos 2").is_err());
    assert!(query(&mut state, context, "data get entity @s Missing").is_err());
    assert!(query(&mut state, context, "data get entity @a Health").is_err());
    assert!(
        query(
            &mut state,
            Context {
                permission: 1,
                ..context
            },
            "data get entity @s Health"
        )
        .is_err()
    );
    assert_eq!(state.players[&id].inventory.slots[0].count, 7);
}

#[test]
fn data_mob_faction_is_public_only_while_affiliated() {
    let (mut state, _, context) = fixture();
    let kind = state.registry.entity_type_id("minecraft:cow").unwrap();
    let id = state.spawn_mob(kind, [2.0, 60.0, 2.0], 20.0);
    let slot = state.entities.slot_of(id).unwrap();
    let selector =
        crate::commands::target_of("@e[type=minecraft:cow,nbt={\"krasnokamen:Faction\":7}]")
            .unwrap();
    let path = format!(
        "data get entity {} \"krasnokamen:Faction\"",
        state.entities.row(slot).uuid
    );
    assert!(get(&entity(&state, id).unwrap(), "\"krasnokamen:Faction\"").is_err());
    assert!(query(&mut state, context, &path).is_err());
    state.entities.set_faction(slot, 7);
    assert_eq!(
        *get(&entity(&state, id).unwrap(), "\"krasnokamen:Faction\"").unwrap(),
        Tag::Int(7)
    );
    assert_eq!(query(&mut state, context, &path).unwrap(), 7);
    assert_eq!(
        crate::command_execution::resolve_target(&state, id, &selector).unwrap(),
        vec![id]
    );
    state.entities.set_faction(slot, 0);
    assert!(get(&entity(&state, id).unwrap(), "\"krasnokamen:Faction\"").is_err());
    assert!(query(&mut state, context, &path).is_err());
    assert!(
        crate::command_execution::resolve_target(&state, id, &selector)
            .unwrap()
            .is_empty()
    );
}
#[test]
fn data_block_accepts_every_registered_block_entity_kind() {
    let (mut state, _, context) = fixture();
    for kind in mc_data::block_entity_kinds::BlockEntityKind::ALL {
        let path = kind.block_paths()[0];
        run(
            &mut state,
            context,
            &format!("setblock 4 60 4 minecraft:{path}"),
        );
        let tag =
            block(&state, BlockPos { x: 4, y: 60, z: 4 }).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert_eq!(
            *get(&tag, "id").unwrap(),
            Tag::String(kind.vanilla_id().into()),
            "{path}"
        );
        assert_eq!(
            query(&mut state, context, "data get block 4 60 4 x").unwrap(),
            4
        );
    }
    assert!(query(&mut state, context, "data get block 1 100 1").is_err());
    assert!(query(&mut state, context, "data get block 10000 60 10000").is_err());
}
#[test]
fn data_block_inventory_uses_live_owner_and_furnace_clock() {
    let (mut state, _, context) = fixture();
    run(&mut state, context, "setblock 4 60 4 minecraft:furnace");
    let stack = item(&state, "minecraft:iron_ingot", 3);
    let live =
        state
            .block_entities
            .entry((4, 60, 4))
            .or_insert_with(|| crate::tick::BlockEntityState {
                slots: vec![ItemStack::EMPTY; 3],
                furnace: Some(Default::default()),
            });
    live.slots[2] = stack;
    live.furnace.as_mut().unwrap().cook_progress = 37;
    assert_eq!(
        query(
            &mut state,
            context,
            "data get block 4 60 4 Items[{Slot:2b}].count"
        )
        .unwrap(),
        3
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get block 4 60 4 cooking_time_spent"
        )
        .unwrap(),
        37
    );
    assert_eq!(state.block_entities[&(4, 60, 4)].slots[2].count, 3);
}
#[test]
fn data_mob_query_projects_runtime_health_and_summoned_metadata() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {Age:-123,VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:none\",level:1}}",
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @e[type=minecraft:villager,limit=1] Age"
        )
        .unwrap(),
        -123
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @e[type=minecraft:villager,limit=1] VillagerData.level"
        )
        .unwrap(),
        1
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @e[type=minecraft:villager,limit=1] Health"
        )
        .unwrap(),
        20
    );
}
#[test]
fn execute_store_chains_results_success_and_failure_without_inventing_success() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    run(
        &mut state,
        context,
        "execute store result score hp oracle store success score ok oracle run data get entity @s Health 10",
    );
    assert_eq!(score(&state, "hp"), 200);
    assert_eq!(score(&state, "ok"), 1);
    run(
        &mut state,
        context,
        "execute store result score hp oracle store success score ok oracle run data get entity @s absent",
    );
    assert_eq!(score(&state, "hp"), 0);
    assert_eq!(score(&state, "ok"), 0);
    run(
        &mut state,
        context,
        "execute store success score ok oracle run data get entity @s EnderItems",
    );
    assert_eq!(
        score(&state, "ok"),
        1,
        "successful zero result is not failure"
    );
    let before = state.day_time;
    run(
        &mut state,
        context,
        "execute store success score ok oracle run time add 123",
    );
    assert_eq!(
        state.day_time, before,
        "unsupported mutating producer must not execute"
    );
    let health = query(&mut state, context, "data get entity @s Health").unwrap();
    run(
        &mut state,
        context,
        "execute store success score ok oracle run damage @s 1 minecraft:generic_kill",
    );
    assert_eq!(
        query(&mut state, context, "data get entity @s Health").unwrap(),
        health,
        "damage without a typed result must be rejected before native admission"
    );
    assert_eq!(score(&state, "ok"), 0);
    assert!(state.scoreboard_dirty);
    assert!(state.command_context.current.is_none());
}
#[test]
fn execute_store_handles_query_producers_and_terminal_conditions() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    state.day_time = 12345;
    for (command, expected) in [
        ("time query time", 12345),
        ("gamerule minecraft:advance_time", 1),
        ("worldborder get", 59999968),
    ] {
        run(
            &mut state,
            context,
            &format!("execute store result score value oracle run {command}"),
        );
        assert_eq!(score(&state, "value"), expected, "{command}");
    }
    run(
        &mut state,
        context,
        "execute store result score copy oracle run scoreboard players get value oracle",
    );
    assert_eq!(score(&state, "copy"), 59999968);
    run(
        &mut state,
        context,
        "execute store result score count oracle if entity @a",
    );
    assert_eq!(score(&state, "count"), 1);
    run(
        &mut state,
        context,
        "execute store result score count oracle unless entity @e[type=minecraft:villager]",
    );
    assert_eq!(score(&state, "count"), 1);
    run(
        &mut state,
        context,
        "execute store result score count oracle if entity @e[type=minecraft:villager]",
    );
    assert_eq!(score(&state, "count"), 0);
}
#[test]
fn execute_store_forks_bind_holders_at_modifier_and_preserve_cross_world_context() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    run(
        &mut state,
        context,
        "execute as @a store result score @s oracle run data get entity @s Pos[0] 100",
    );
    assert_eq!(
        state
            .scoreboard
            .scores
            .values()
            .filter_map(|v| v.get("oracle"))
            .next()
            .unwrap()
            .value,
        175
    );
    let (mut nether, _, _) = fixture();
    nether.dimension = crate::dimensions::Dimension::Nether;
    nether.day_time = 678;
    state
        .other_dimensions
        .insert(nether.dimension, Box::new(nether));
    // Cross-world native dispatch runs at the shared tick boundary.
    state.command_context.shared = true;
    run(
        &mut state,
        context,
        "execute store result score nether oracle in minecraft:the_nether run time query time",
    );
    assert_eq!(score(&state, "nether"), 678);
    run(
        &mut state,
        context,
        "execute store success score missing oracle as @e[type=minecraft:villager] run data get entity @s Health",
    );
    assert_eq!(score(&state, "missing"), 0);
}

fn stored(state: &mut TickState, context: Context, command: &str, expected: Option<i32>) {
    run(
        state,
        context,
        &format!(
            "execute store result score result oracle store success score success oracle run {command}"
        ),
    );
    assert_eq!(score(state, "result"), expected.unwrap_or(0), "{command}");
    assert_eq!(
        score(state, "success"),
        i32::from(expected.is_some()),
        "{command}"
    );
}

#[test]
fn execute_store_give_and_clear_execute_once_and_return_real_counts() {
    let (mut state, id, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    stored(&mut state, context, "give @s minecraft:stone 7", Some(1));
    let stone = item(&state, "minecraft:stone", 1).item_id;
    let count = |state: &TickState| {
        state.players[&id]
            .inventory
            .slots
            .iter()
            .filter(|stack| stack.item_id == stone)
            .map(|stack| stack.count)
            .sum::<i32>()
    };
    assert_eq!(count(&state), 7);
    stored(&mut state, context, "clear @s minecraft:stone 0", Some(7));
    assert_eq!(count(&state), 7);
    stored(&mut state, context, "clear @s minecraft:stone 3", Some(3));
    assert_eq!(count(&state), 4);
    stored(&mut state, context, "clear @s minecraft:stone", Some(4));
    assert_eq!(count(&state), 0);
    stored(&mut state, context, "clear @s minecraft:stone", None);
    stored(&mut state, context, "give @s minecraft:stone 6401", None);
    assert_eq!(count(&state), 0);
}

#[test]
fn execute_store_score_mutations_preserve_signed_zero_and_failed_outcomes() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    for (command, expected) in [
        ("scoreboard players set value oracle -7", Some(-7)),
        ("scoreboard players add value oracle 7", Some(0)),
        ("scoreboard players remove value oracle 3", Some(-3)),
        ("scoreboard players set source oracle 4", Some(4)),
        (
            "scoreboard players operation value oracle *= source oracle",
            Some(-12),
        ),
        ("scoreboard players set source oracle 0", Some(0)),
        (
            "scoreboard players operation value oracle /= source oracle",
            None,
        ),
    ] {
        stored(&mut state, context, command, expected);
    }
    assert_eq!(
        score(&state, "value"),
        -12,
        "failed arithmetic must not mutate scores"
    );
    stored(
        &mut state,
        context,
        "scoreboard players reset value oracle",
        Some(1),
    );
    assert!(!state.scoreboard.scores.contains_key("value"));
}

#[test]
fn execute_store_entity_mutations_report_affected_entities_and_failures() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    stored(&mut state, context, "summon minecraft:pig 4 60 4", Some(1));
    stored(&mut state, context, "summon minecraft:pig 5 60 4", Some(1));
    stored(
        &mut state,
        context,
        "effect give @e[type=minecraft:pig] minecraft:speed 30",
        Some(2),
    );
    stored(
        &mut state,
        context,
        "effect clear @e[type=minecraft:pig]",
        Some(2),
    );
    stored(
        &mut state,
        context,
        "effect clear @e[type=minecraft:pig]",
        None,
    );
    stored(
        &mut state,
        context,
        "tp @e[type=minecraft:pig] 9 61 9",
        Some(2),
    );
    assert_eq!(
        query(
            &mut state,
            context,
            "data get entity @e[type=minecraft:pig,limit=1] Pos[0]"
        )
        .unwrap(),
        9,
    );
    let queued = state.merge_damage.len();
    stored(&mut state, context, "kill @e[type=minecraft:pig]", Some(2));
    assert_eq!(state.merge_damage.len(), queued + 2);
    stored(
        &mut state,
        context,
        "kill @e[type=minecraft:villager]",
        None,
    );
    assert_eq!(state.merge_damage.len(), queued + 2);
    stored(
        &mut state,
        context,
        "summon minecraft:pig 30000000 60 4",
        None,
    );
}

#[test]
fn execute_store_experience_uses_target_counts_not_experience_values() {
    let (mut state, id, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    stored(&mut state, context, "experience add @s 7 levels", Some(1));
    assert_eq!(state.players[&id].vitals.experience.level, 7);
    stored(&mut state, context, "experience set @s 0 levels", Some(1));
    assert_eq!(state.players[&id].vitals.experience.level, 0);
    stored(&mut state, context, "experience set @s 999 points", None);
    assert_eq!(state.players[&id].vitals.experience.level, 0);
}

#[test]
fn execute_store_block_mutations_report_counts_and_unchanged_data_failure() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "scoreboard objectives add oracle dummy",
    );
    stored(
        &mut state,
        context,
        "setblock 4 60 4 minecraft:stone",
        Some(1),
    );
    stored(&mut state, context, "setblock 4 60 4 minecraft:stone", None);
    stored(
        &mut state,
        context,
        "fill 4 60 4 6 60 4 minecraft:dirt",
        Some(3),
    );
    stored(
        &mut state,
        context,
        "setblock 4 60 4 minecraft:command_block",
        Some(1),
    );
    stored(
        &mut state,
        context,
        "data merge block 4 60 4 {Command:'say stored'}",
        Some(1),
    );
    stored(
        &mut state,
        context,
        "data merge block 4 60 4 {Command:'say stored'}",
        None,
    );
    assert_eq!(
        query(&mut state, context, "data get block 4 60 4 Command").unwrap(),
        10,
    );
}

#[test]
fn tag_commands_update_selector_data_and_report_counts() {
    use crate::command_execution::CommandResult::{Failed, Success};
    let (mut state, _, context) = fixture();
    let mut changes = ChangeSet::default();
    let mut command = |state: &mut TickState, text| {
        crate::command_context::native(state, text, context, &mut changes, true).unwrap()
    };
    assert_eq!(command(&mut state, "tag @s add map_test"), Success(1));
    assert_eq!(command(&mut state, "tag @s add map_test"), Failed);
    assert_eq!(command(&mut state, "tag @s list"), Success(1));
    assert_eq!(
        command(&mut state, "tag @s[tag=map_test] remove map_test"),
        Success(1)
    );
    assert_eq!(command(&mut state, "tag @s remove map_test"), Failed);
    assert_eq!(command(&mut state, "tag @s list"), Success(0));
    state.spawn_mob(state.registry.cow_type_id, [2.0, 60.0, 2.0], 10.0);
    assert_eq!(
        command(&mut state, "tag @e[type=minecraft:cow] add herd"),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "tag @e[type=minecraft:cow,tag=herd] list"),
        Success(1)
    );
    assert_eq!(
        command(
            &mut state,
            "tag @e[type=minecraft:cow,tag=herd] remove herd"
        ),
        Success(1)
    );
}

#[test]
fn map_sound_and_particle_commands_report_delivery() {
    use crate::command_execution::CommandResult::{Failed, Success};
    let (mut state, _, context) = fixture();
    let mut changes = ChangeSet::default();
    for command in [
        "playsound minecraft:block.bell.use master @s ~ ~ ~ 1 1 0",
        "particle minecraft:sweep_attack ~ ~ ~ 0 0 0 0 1 normal @s",
    ] {
        assert_eq!(
            crate::command_context::native(&mut state, command, context, &mut changes, true)
                .unwrap(),
            Success(1),
            "{command}"
        );
    }
    assert!(state.outbox[&crate::boundary::ConnectionId(1)].iter().any(|change| matches!(change, crate::boundary::PlayerChange::CommandSound { sound, .. } if sound.as_str() == "minecraft:block.bell.use")));
    assert!(
        state.outbox[&crate::boundary::ConnectionId(1)]
            .iter()
            .any(|change| matches!(
                change,
                crate::boundary::PlayerChange::CommandParticle {
                    particle_id: 77,
                    count: 1,
                    ..
                }
            ))
    );
    assert_eq!(
        crate::command_context::native(
            &mut state,
            "particle minecraft:unsupported ~ ~ ~",
            context,
            &mut changes,
            true
        )
        .unwrap(),
        Failed
    );
}

#[test]
fn data_storage_modify_reports_results_and_survives_reads() {
    use crate::command_execution::CommandResult::{Failed, Success};
    let (mut state, _, context) = fixture();
    let mut changes = ChangeSet::default();
    let mut command = |state: &mut TickState, text| {
        crate::command_context::native(state, text, context, &mut changes, true).unwrap()
    };
    assert_eq!(
        command(
            &mut state,
            "data merge storage test:map {list:[1,2],value:3}"
        ),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data modify storage test:map value set value 7"),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data get storage test:map value"),
        Success(7)
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:map list append value 4"
        ),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data get storage test:map list"),
        Success(3)
    );
    assert_eq!(
        command(&mut state, "data modify storage test:map value set value 7"),
        Failed
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:map list prepend value 0"
        ),
        Success(1)
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:map list insert 1 value 9"
        ),
        Success(1)
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:map copied set from storage test:map value"
        ),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data get storage test:map copied"),
        Success(7)
    );
    assert_eq!(
        command(
            &mut state,
            "data modify storage test:map compound merge value {key:1}"
        ),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data remove storage test:map copied"),
        Success(1)
    );
    assert_eq!(
        command(&mut state, "data get storage test:map copied"),
        Failed
    );
    let image = crate::command_storage::encode(&state.command_storage);
    let replay = crate::journal::replay_batches(
        crate::journal::Replay::default(),
        &[(
            1,
            1,
            vec![crate::journal::JournalEntry::CommandStorage { value: image }],
        )],
    );
    assert_eq!(
        crate::command_storage::decode(replay.command_storage.as_ref().unwrap()).unwrap(),
        state.command_storage
    );
}

fn said(state: &mut TickState) -> String {
    format!(
        "{:?}",
        state
            .outbox
            .remove(&crate::command_console::CONNECTION)
            .unwrap_or_default()
    )
}
/// Whether the command told the player anything: a refusal is never silent.
/// The wording is the command's own and not pinned here.
fn reported(state: &mut TickState) -> bool {
    state
        .outbox
        .remove(&crate::command_console::CONNECTION)
        .is_some_and(|changes| !changes.is_empty())
}
/// The result of one `data` command; -1 when it failed.
fn data(state: &mut TickState, context: Context, line: &str) -> i32 {
    state.command_context.current = Some(context);
    let result = command(state, context.connection, context.executor, line);
    state.command_context.current = None;
    match result {
        crate::command_execution::CommandResult::Success(value) => value,
        crate::command_execution::CommandResult::Completed => 0,
        _ => -1,
    }
}
fn cow_uuid_and_pos(state: &TickState) -> (uuid::Uuid, [f64; 3]) {
    let slot = state
        .entities
        .slots()
        .find(|slot| {
            crate::command_execution::entity_type_name(state, state.entities.body(*slot).id)
                == Some("minecraft:cow")
        })
        .unwrap();
    (state.entities.row(slot).uuid, state.entities.pos(slot))
}

#[test]
fn data_tags_and_armor_stand_fields_commit_together() {
    let (mut state, _, context) = fixture();
    run(&mut state, context, "summon minecraft:armor_stand 4 60 4");
    let target = "@e[type=minecraft:armor_stand,limit=1]";
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {target} {{Small:1b,Tags:['guard']}}")
        ),
        1
    );
    assert_eq!(
        query(
            &mut state,
            context,
            &format!("data get entity {target} Small")
        )
        .unwrap(),
        1
    );
    assert_eq!(
        query(
            &mut state,
            context,
            &format!("data get entity {target} Tags[0]")
        )
        .unwrap(),
        5
    );
}

#[test]
fn data_entity_edit_preserves_unrelated_runtime_state() {
    let (mut state, _, context) = fixture();
    run(&mut state, context, "summon minecraft:cow 4 60 4");
    let (uuid, _) = cow_uuid_and_pos(&state);
    let slot = state.entities.slot_of_uuid(uuid).unwrap();
    state.entities.body_mut(slot).attack_cooldown = 17;
    state.entities.row_mut(slot).breed_cooldown = 35;
    state.entities.row_mut(slot).entity_life.taming.rolls = 123;
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge entity @e[type=minecraft:cow,limit=1] {Health:7.5f,NoAI:1b}"
        ),
        1
    );
    assert_eq!(state.entities.body(slot).attack_cooldown, 17);
    assert_eq!(state.entities.row(slot).breed_cooldown, 35);
    assert_eq!(state.entities.row(slot).entity_life.taming.rolls, 123);
}

#[test]
fn data_merge_modify_and_remove_edit_a_mobs_data_and_keep_its_identity() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:cow 4 60 4 {Health:9f}",
    );
    let (uuid, pos) = cow_uuid_and_pos(&state);
    let cow = "@e[type=minecraft:cow,limit=1]";

    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Health:5f,CustomName:'\"Bessie\"',NoAI:1b}}")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} Health")
        ),
        5
    );
    assert_eq!(
        data(&mut state, context, &format!("data get entity {cow} NoAI")),
        1
    );
    assert!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} CustomName")
        ) >= 1
    );
    assert_eq!(
        cow_uuid_and_pos(&state),
        (uuid, pos),
        "identity and place stay"
    );

    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {cow} Health set value 3f")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} Health")
        ),
        3
    );
    // Copying from another target: the cow takes the player's health value.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {cow} Health set from entity @s Health")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} Health")
        ),
        20
    );

    // A removed field loads as its default.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data remove entity {cow} NoAI")
        ),
        1
    );
    assert_eq!(
        data(&mut state, context, &format!("data get entity {cow} NoAI")),
        0
    );
    assert_eq!(cow_uuid_and_pos(&state), (uuid, pos));
    said(&mut state);

    // Nothing changed is a failure, not a success.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Health:20f}}")
        ),
        -1
    );
    assert!(reported(&mut state));
}

#[test]
fn data_entity_edits_refuse_players_unsupported_fields_and_position_without_changing_anything() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:cow 4 60 4 {Health:9f}",
    );
    let cow = "@e[type=minecraft:cow,limit=1]";
    said(&mut state);

    assert_eq!(
        data(&mut state, context, "data merge entity @s {Health:1f}"),
        -1
    );
    assert!(reported(&mut state));
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Air:12s}}")
        ),
        -1
    );
    assert!(reported(&mut state));
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {cow} Pos[0] set value 99.0d")
        ),
        -1
    );
    assert!(reported(&mut state));
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} Health")
        ),
        9
    );
    // A block target that is not a block entity is refused, not edited.
    assert_eq!(
        data(
            &mut state,
            context,
            "data modify block 0 60 0 x set value 1b"
        ),
        -1
    );
    assert!(reported(&mut state));
}

#[test]
fn data_entity_edit_of_a_villager_keeps_its_profession_level_and_age() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {Age:-123,VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:none\",level:3}}",
    );
    let villager = "@e[type=minecraft:villager,limit=1]";
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{CustomName:'\"Vic\"'}}")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {villager} VillagerData.level")
        ),
        3
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {villager} Age")
        ),
        -123
    );
}
/// A villager's runtime state is the row's own persistence image, not summon NBT:
/// `data merge|modify entity` writes `Inventory`, `Brain`, `Offers`, `Gossips`, `Xp`
/// and `krasnokamen:VillagerRuntime` through `villager::persistence`, which validates
/// them, while the settlement keeps its membership whoever the edit names.
#[test]
fn data_entity_merge_lands_villager_runtime_state_and_keeps_membership() {
    use crate::settlements::SettlementId;
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:2}}",
    );
    let villager = "@e[type=minecraft:villager,limit=1]";
    let slot = state
        .entities
        .slots()
        .find(|slot| state.entities.row(*slot).villager.is_some())
        .expect("the summoned villager");
    // Two wheat stacks, seeded through the command the test is about.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {villager} {{Inventory:[{{Slot:0b,id:\"minecraft:wheat\",count:5}},{{Slot:1b,id:\"minecraft:wheat\",count:1}}]}}"
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    {
        let row = state.entities.row_mut(slot).villager.as_mut().unwrap();
        row.settlement = Some(SettlementId(7));
        row.household = Some("Baker".into());
    }

    // Inventory: the merged list replaces the stacks, as vanilla does — the old ones are
    // gone and nothing is created in their place.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {villager} {{Inventory:[{{Slot:0b,id:\"minecraft:stone\",count:2}}]}}"
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    let merged = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(merged.inventory[0].as_ref().unwrap().count, 2);
    assert_eq!(merged.inventory[1], None, "the replaced stack is not kept");
    // `data get` of a non-numeric path prints the SNBT and reports its length; the printed id
    // is the one the merge wrote.
    query(
        &mut state,
        context,
        &format!("data get entity {villager} Inventory[0].id"),
    )
    .unwrap();
    assert!(said(&mut state).contains("minecraft:stone"));

    // Brain memories, xp and the runtime clock land through the same parser.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {villager} {{Brain:{{memories:{{\"minecraft:last_slept\":{{value:100L}}}}}}}}"
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{Xp:7}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        query(
            &mut state,
            context,
            &format!("data get entity {villager} Xp")
        )
        .unwrap(),
        7
    );
    let edited = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(edited.last_slept, Some(100));
    assert_eq!(edited.xp, 7);

    // Membership and lineage are the settlement's: the merge may move the clock, never
    // the villager between settlements or households.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {villager} {{\"krasnokamen:VillagerRuntime\":{{last_gossip:5L,settlement:99L,household:\"Other\"}}}}"
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    let after = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(after.last_gossip, 5, "the runtime clock is editable");
    assert_eq!(after.settlement, Some(SettlementId(7)));
    assert_eq!(after.household.as_deref(), Some("Baker"));
}

/// What a merge still refuses: a field neither summon NBT nor the villager row owns,
/// the engine's own private bookkeeping, and identity or place.
#[test]
fn data_entity_merge_refuses_foreign_and_private_fields_by_name() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:2}}",
    );
    let villager = "@e[type=minecraft:villager,limit=1]";
    let slot = state
        .entities
        .slots()
        .find(|slot| state.entities.row(*slot).villager.is_some())
        .expect("the summoned villager");
    let before = state.entities.row(slot).villager.clone().unwrap();
    for (field, key) in [
        ("Air", "Air"),
        ("krasnokamen:WireId", "\"krasnokamen:WireId\""),
        ("DataVersion", "DataVersion"),
    ] {
        assert_eq!(
            data(
                &mut state,
                context,
                &format!("data merge entity {villager} {{{key}:1}}")
            ),
            -1,
            "{field} must be refused"
        );
        let said = said(&mut state);
        assert!(
            said.contains(&format!(
                "Editing {field} of minecraft:villager is not supported"
            )),
            "{field} refusal must name the field: {said}"
        );
        assert_eq!(
            state.entities.row(slot).villager.as_ref().unwrap(),
            &before,
            "{field} must leave the villager unchanged"
        );
    }
    // A field the parser cannot accept is refused by the parser's own message, and the
    // row is untouched.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{Offers:{{Recipes:[{{buy:1b}}]}}}}")
        ),
        -1
    );
    assert!(reported(&mut state));
    assert_eq!(state.entities.row(slot).villager.as_ref().unwrap(), &before);

    // Place and identity: `Pos` and `id` are refused by name with `/tp` as the remedy.
    for (field, value) in [("Pos", "[0.5d,61.0d,0.5d]"), ("id", "\"minecraft:cow\"")] {
        assert_eq!(
            data(
                &mut state,
                context,
                &format!("data merge entity {villager} {{{field}:{value}}}")
            ),
            -1,
            "{field} must be refused"
        );
        let said = said(&mut state);
        assert!(
            said.contains(&format!("Editing {field} is not supported; use /tp")),
            "{field} refusal must name the field and the remedy: {said}"
        );
        assert_eq!(state.entities.row(slot).villager.as_ref().unwrap(), &before);
    }
    // `UUID` is silently kept, as vanilla keeps it: a merge naming another UUID
    // changes nothing, and removing it is refused.
    let uuid = state.entities.row(slot).uuid;
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{UUID:[I;9,9,9,9]}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        state.entities.row(slot).uuid,
        uuid,
        "the UUID is kept, never rewritten by a merge"
    );
    assert_eq!(state.entities.row(slot).villager.as_ref().unwrap(), &before);
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data remove entity {villager} UUID")
        ),
        -1
    );
    assert!(
        said(&mut state).contains("Removing UUID is not supported"),
        "removing the UUID is refused"
    );
}

/// A repeated identical edit of a villager-owned field is a no-op: the merge compares
/// the edited image with the entity's own native image, so the same merge says
/// "Nothing changed" instead of re-authoring the row — including an engine-private key
/// that `data get` never prints.
#[test]
fn a_repeated_identical_owned_merge_reports_nothing_changed() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\",level:2}}",
    );
    let villager = "@e[type=minecraft:villager,limit=1]";
    let slot = state
        .entities
        .slots()
        .find(|slot| {
            crate::command_execution::entity_type_name(&state, state.entities.body(*slot).id)
                == Some("minecraft:villager")
        })
        .expect("the summoned villager");
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{Xp:7}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    let merged = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(merged.xp, 7);
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {villager} {{Xp:7}}")
        ),
        -1,
        "the identical second merge must not modify the row"
    );
    let message = said(&mut state);
    assert!(
        message.contains("Nothing changed. The specified properties already have these values"),
        "the identical merge says nothing changed: {message}"
    );
    assert_eq!(state.entities.row(slot).villager.as_ref().unwrap(), &merged);

    // The runtime image is the case the merge base has to get right: `data get` does not
    // print engine-private fields, so a merge that added the key to the printed compound
    // would look new on every repeat and would replace the rest of the runtime image with
    // only the authored fields.
    let row = state.entities.row_mut(slot).villager.as_mut().unwrap();
    row.last_gossip = 11;
    let line = format!(
        "data merge entity {villager} {{\"krasnokamen:VillagerRuntime\":{{profession:\"miner\"}}}}"
    );
    assert_eq!(data(&mut state, context, &line), 1, "{}", said(&mut state));
    let merged = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(
        merged.data.profession,
        crate::villager::VillagerProfession::Miner,
        "the runtime role lands"
    );
    assert_eq!(
        merged.last_gossip, 11,
        "a partial runtime merge keeps the native fields it did not name"
    );
    assert_eq!(
        data(&mut state, context, &line),
        -1,
        "the identical second merge of the runtime key must not modify the row"
    );
    let message = said(&mut state);
    assert!(
        message.contains("Nothing changed. The specified properties already have these values"),
        "the identical runtime merge says nothing changed: {message}"
    );
    assert_eq!(state.entities.row(slot).villager.as_ref().unwrap(), &merged);
}

/// A `Brain` edit is a POI move: the claim maps follow the live row in the same
/// frame, so no stale claim keeps another resident out of the old bed or desk.
#[test]
fn data_merge_of_a_brain_moves_the_poi_claims() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:villager 4 60 4 {VillagerData:{type:\"minecraft:plains\",profession:\"minecraft:farmer\"}}",
    );
    let villager = "@e[type=minecraft:villager,limit=1]";
    let slot = state
        .entities
        .slots()
        .find(|slot| {
            crate::command_execution::entity_type_name(&state, state.entities.body(*slot).id)
                == Some("minecraft:villager")
        })
        .expect("the summoned villager");
    let id = state.entities.body(slot).id;
    let mut changes = ChangeSet::default();
    crate::villager::brain::tick(&mut state, &mut changes);
    let home = BlockPos { x: 6, y: 61, z: 6 };
    let job = BlockPos { x: 8, y: 61, z: 4 };
    assert!(state.villager_pois.homes.get(&home).is_none());
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {villager} {{Brain:{{memories:{{\
                 \"minecraft:home\":{{value:{{pos:[I;{},{},{}],dimension:\"minecraft:overworld\"}}}},\
                 \"minecraft:job_site\":{{value:{{pos:[I;{},{},{}],dimension:\"minecraft:overworld\"}}}}}}}}}}",
                home.x, home.y, home.z, job.x, job.y, job.z
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    let row = state.entities.row(slot).villager.clone().unwrap();
    assert_eq!(row.home, Some(home), "the home memory lands in the row");
    assert_eq!(
        row.job_site,
        Some(job),
        "the job-site memory lands in the row"
    );
    crate::villager::brain::tick(&mut state, &mut changes);
    assert!(
        state
            .villager_pois
            .homes
            .get(&home)
            .is_some_and(|owners| owners.contains(&id)),
        "the merged home is claimed in the same frame"
    );
    assert!(
        state
            .villager_pois
            .jobs
            .get(&job)
            .is_some_and(|owners| owners.contains(&id)),
        "the merged job site is claimed in the same frame"
    );
}

fn item_entity(state: &TickState) -> crate::entities::EntitySlot {
    state
        .entities
        .slots()
        .find(|slot| matches!(state.entities.body(*slot).kind, EntityKind::Item { .. }))
        .unwrap()
}

#[test]
fn data_edits_an_item_entitys_stack_delay_age_and_gravity() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:item 4 60 4 {Item:{id:'minecraft:stone',count:3},PickupDelay:20s,Age:100s}",
    );
    let slot = item_entity(&state);
    let id = state.entities.body(slot).id;
    let dirt = item(&state, "minecraft:dirt", 5);
    state.appearance_changed.clear();
    let drops = "@e[type=minecraft:item,limit=1]";
    assert_eq!(
        data(
            &mut state,
            context,
            &format!(
                "data merge entity {drops} {{Item:{{id:'minecraft:dirt',count:5}},PickupDelay:7s,NoGravity:1b}}"
            )
        ),
        1,
        "{}",
        said(&mut state)
    );
    let row = state.entities.row(slot);
    assert_eq!(row.item.as_ref().unwrap().item_id, dirt.item_id);
    assert_eq!(row.item.as_ref().unwrap().count, 5);
    assert_eq!(row.appearance.item.as_ref().unwrap().item_id, dirt.item_id);
    assert_eq!(state.entities.body(slot).pickup_delay, 7);
    assert!(state.entities.row(slot).controls.no_gravity);
    assert_eq!(
        state.entities.row(slot).age,
        100,
        "untouched fields keep their value"
    );
    assert!(
        state.appearance_changed.contains(&id),
        "the edited stack must reach the WAL"
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {drops} Item.count set value 9")
        ),
        1
    );
    assert_eq!(state.entities.row(slot).item.as_ref().unwrap().count, 9);
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {drops} Item.count")
        ),
        9
    );
    // A removed field loads as its default, like a compound without it does.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data remove entity {drops} NoGravity")
        ),
        1
    );
    assert!(!state.entities.row(slot).controls.no_gravity);
    // An unknown item is refused before anything changes.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {drops} Item.id set value 'minecraft:nope'")
        ),
        -1
    );
    assert_eq!(
        state.entities.row(slot).item.as_ref().unwrap().item_id,
        dirt.item_id
    );
    // Removing the stack is refused: an item entity always carries one.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data remove entity {drops} Item")
        ),
        -1
    );
    assert_eq!(state.entities.row(slot).item.as_ref().unwrap().count, 9);
}

fn place(state: &mut TickState, context: Context, block: &str) {
    run(
        state,
        context,
        &format!("setblock 4 60 4 minecraft:{block}"),
    );
}

#[test]
fn data_modify_and_merge_edit_any_block_entity_through_its_native_owner() {
    let (mut state, _, context) = fixture();
    // A container: the live slots follow the edited image.
    place(&mut state, context, "chest");
    assert_eq!(
        data(
            &mut state,
            context,
            "data modify block 4 60 4 Items set value [{Slot:3b,id:'minecraft:stone',count:5}]"
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(state.block_entities[&(4, 60, 4)].slots[3].count, 5);
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 Items[0].count"),
        5
    );
    assert_eq!(
        data(&mut state, context, "data remove block 4 60 4 Items[0]"),
        1
    );
    assert!(
        state.block_entities[&(4, 60, 4)]
            .slots
            .iter()
            .all(ItemStack::is_empty)
    );
    // A furnace: the clock lives in its own owner state.
    place(&mut state, context, "furnace");
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {cooking_time_spent:11s,lit_time_remaining:40s}"
        ),
        1,
        "{}",
        said(&mut state)
    );
    let furnace = state.block_entities[&(4, 60, 4)].furnace.as_ref().unwrap();
    assert_eq!((furnace.cook_progress, furnace.lit_remaining), (11, 40));
    // A sign: a plain native image with no live owner.
    place(&mut state, context, "oak_sign");
    assert_eq!(
        data(&mut state, context, "data merge block 4 60 4 {is_waxed:1b}"),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 is_waxed"),
        1
    );
    // A brewing stand: the timer is rebuilt from the installed image.
    place(&mut state, context, "brewing_stand");
    assert_eq!(
        data(
            &mut state,
            context,
            "data modify block 4 60 4 BrewTime set value 50s"
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 BrewTime"),
        50
    );
}

#[test]
fn data_block_reads_live_campfire_progress_before_metadata_edit() {
    let (mut state, _, context) = fixture();
    place(&mut state, context, "campfire");
    let fields = vec![
        ("CookingTimes".into(), Tag::IntArray(vec![17, 0, 0, 0])),
        (
            "CookingTotalTimes".into(),
            Tag::IntArray(vec![200, 0, 0, 0]),
        ),
    ];
    let campfire =
        crate::campfire::CampfireState::from_fields(&fields, &state.registry.items).unwrap();
    state.campfires.insert((4, 60, 4), campfire);
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 CookingTimes[0]"),
        17
    );
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {CustomName:'test'}"
        ),
        1
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 CookingTimes[0]"),
        17
    );
}

#[test]
fn data_block_metadata_edit_preserves_live_brewing_owner() {
    let (mut state, _, context) = fixture();
    place(&mut state, context, "brewing_stand");
    let mut brewing = crate::brewing::BrewingState::default();
    brewing.brew_time = 123;
    brewing.fuel = 8;
    state.brewing.insert((4, 60, 4), brewing.clone());

    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {CustomName:'test'}"
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(state.brewing.get(&(4, 60, 4)), Some(&brewing));
}

#[test]
fn data_block_edits_are_refused_without_changing_anything() {
    let (mut state, _, context) = fixture();
    place(&mut state, context, "chest");
    data(
        &mut state,
        context,
        "data modify block 4 60 4 Items set value [{Slot:0b,id:'minecraft:stone',count:2}]",
    );
    let before = block(&state, BlockPos { x: 4, y: 60, z: 4 }).unwrap();
    for line in [
        "data merge block 4 60 4 {Items:[{Slot:0b,id:'minecraft:nope',count:1}]}",
        "data merge block 4 60 4 {Items:[{Slot:99b,id:'minecraft:stone',count:1}]}",
        "data modify block 4 60 4 Items[0].count set value 2",
        "data merge block 4 61 4 {a:1b}",
        "data merge block 10000 60 10000 {a:1b}",
        "data remove block 4 60 4 Missing",
    ] {
        assert_eq!(data(&mut state, context, line), -1, "{line}");
        assert!(reported(&mut state), "{line}");
        assert_eq!(
            block(&state, BlockPos { x: 4, y: 60, z: 4 }).unwrap(),
            before,
            "{line}"
        );
    }
    // Identity and position are the block's own: an edit cannot move or retype it.
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {x:99,y:1,z:1,id:'minecraft:furnace',Lock:'k'}"
        ),
        1
    );
    assert_eq!(data(&mut state, context, "data get block 4 60 4 x"), 4);
    assert_eq!(
        *get(
            &block(&state, BlockPos { x: 4, y: 60, z: 4 }).unwrap(),
            "id"
        )
        .unwrap(),
        Tag::String("minecraft:chest".into())
    );
    // Level 1 may not edit data at all.
    let weak = Context::source(
        &state,
        crate::command_console::CONNECTION,
        context.executor,
        1,
    );
    assert_eq!(
        data(&mut state, weak, "data merge block 4 60 4 {Lock:'z'}"),
        -1
    );
}

#[test]
fn data_edits_a_command_block_through_the_same_command_path() {
    let (mut state, _, context) = fixture();
    place(&mut state, context, "command_block");
    assert_eq!(
        data(
            &mut state,
            context,
            "data modify block 4 60 4 Command set value 'say hi'"
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 Command"),
        6
    );
    // Vanilla BaseCommandBlock.load restores SuccessCount as well as Command.
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {SuccessCount:9}"
        ),
        1
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 SuccessCount"),
        9
    );
    // Invalid native field types refuse the whole edit, including Command.
    assert_eq!(
        data(
            &mut state,
            context,
            "data merge block 4 60 4 {Command:'say no',SuccessCount:'invalid'}"
        ),
        -1
    );
    assert!(reported(&mut state));
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 Command"),
        6
    );
    assert_eq!(
        data(&mut state, context, "data get block 4 60 4 SuccessCount"),
        9
    );
    assert_eq!(
        data(&mut state, context, "data remove block 4 60 4 Command"),
        1
    );
}

fn only_slot(state: &TickState, name: &str) -> crate::entities::EntitySlot {
    let mut found = state.entities.slots().filter(|slot| {
        crate::command_execution::entity_type_name(state, state.entities.body(*slot).id)
            == Some(name)
    });
    let slot = found.next().unwrap_or_else(|| panic!("no {name}"));
    assert!(found.next().is_none(), "more than one {name}");
    slot
}

#[test]
fn data_edit_of_a_baby_keeps_its_hitbox_and_growing_up_restores_it() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:cow 4 60 4 {Age:-24000}",
    );
    let slot = only_slot(&state, "minecraft:cow");
    let type_id = match state.entities.body(slot).kind {
        EntityKind::Mob { type_id } => type_id,
        _ => unreachable!(),
    };
    let blueprint = state.registry.blueprint(type_id).unwrap();
    let (half_width, height) = (blueprint.half_width, blueprint.height);
    let cow = "@e[type=minecraft:cow,limit=1]";
    assert!(state.entities.row(slot).appearance.baby);
    assert_eq!(state.entities.body(slot).height, height * 0.5);
    assert_eq!(state.entities.body(slot).half_width, half_width * 0.5);
    // An edit that leaves the age alone does not halve the hitbox a second time.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{CustomName:'\"Calf\"'}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(state.entities.body(slot).height, height * 0.5);
    assert_eq!(state.entities.body(slot).half_width, half_width * 0.5);
    // Growing up through an edit restores the adult hitbox.
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {cow} {{Age:0}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert!(!state.entities.row(slot).appearance.baby);
    assert_eq!(state.entities.body(slot).height, height);
    assert_eq!(state.entities.body(slot).half_width, half_width);
}

#[test]
fn data_edit_of_an_experience_orb_changes_its_value_and_count() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:experience_orb 4 60 4 {Value:3s}",
    );
    let slot = only_slot(&state, "minecraft:experience_orb");
    let orb = "@e[type=minecraft:experience_orb,limit=1]";
    let before = state.entities.row(slot).appearance.experience_orb.unwrap();
    assert_eq!(before.0, 3);
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {orb} {{Value:9s,Count:2}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        state.entities.row(slot).appearance.experience_orb,
        Some((9, 2)),
        "the reported success is real"
    );
}

#[test]
fn data_edit_keeps_a_zombie_villagers_cure_and_a_name_removal_clears_it() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:zombie_villager 4 60 4",
    );
    let slot = only_slot(&state, "minecraft:zombie_villager");
    state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .unwrap()
        .conversion_time = Some(120);
    let target = "@e[type=minecraft:zombie_villager,limit=1]";
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data merge entity {target} {{CustomName:'\"Z\"'}}")
        ),
        1,
        "{}",
        said(&mut state)
    );
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .conversion_time,
        Some(120),
        "an unrelated edit does not cancel the cure"
    );
    assert_eq!(
        state.entities.row(slot).appearance.custom_name.as_deref(),
        Some("\"Z\"")
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data remove entity {target} CustomName")
        ),
        1
    );
    assert_eq!(state.entities.row(slot).appearance.custom_name, None);
}

#[test]
fn data_edit_of_one_field_keeps_love_and_persistence_the_entity_already_had() {
    let (mut state, _, context) = fixture();
    run(
        &mut state,
        context,
        "summon minecraft:cow 4 60 4 {Health:9f,InLove:300,PersistenceRequired:1b}",
    );
    let cow = "@e[type=minecraft:cow,limit=1]";
    for field in ["InLove", "PersistenceRequired"] {
        assert_ne!(
            data(
                &mut state,
                context,
                &format!("data get entity {cow} {field}")
            ),
            0,
            "{field} is set before the edit"
        );
    }
    let love = data(
        &mut state,
        context,
        &format!("data get entity {cow} InLove"),
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data modify entity {cow} Health set value 4f")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} InLove")
        ),
        love
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} PersistenceRequired")
        ),
        1
    );
    assert_eq!(
        data(
            &mut state,
            context,
            &format!("data get entity {cow} Health")
        ),
        4
    );
}
