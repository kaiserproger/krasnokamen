use super::*;
use crate::boundary::{ConnectionId, PlayerChange};

fn fixture() -> TickState {
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.enable_command_block = true;
    state.day_time = 0;
    state
}
fn place(state: &mut TickState, x: i32, name: &str, command: &str, auto: bool) -> BlockPos {
    let id = mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap();
    let block = state.registry.blocks.block(&id).unwrap();
    let mut properties = state
        .registry
        .blocks
        .by_id(block.default)
        .unwrap()
        .properties
        .clone();
    properties
        .iter_mut()
        .find(|(key, _)| key == "facing")
        .unwrap()
        .1 = "east".into();
    let id = state
        .registry
        .blocks
        .by_name_and_props(&id, &properties)
        .unwrap();
    let pos = BlockPos { x, y: 60, z: 0 };
    state.world.set_block(pos.x, pos.y, pos.z, id.0);
    state.world.fold_overlay();
    save(
        state,
        pos,
        vec![
            ("Command".into(), Tag::String(command.into())),
            ("auto".into(), Tag::Byte(i8::from(auto))),
        ],
    );
    pos
}
fn advance(state: &mut TickState) {
    state.tick += 1;
    crate::redstone::run(state, &mut Vec::new(), &mut Vec::new());
    run(state, &mut ChangeSet::default());
}
#[test]
fn conditional_chain_tracks_predecessor_success() {
    let mut state = fixture();
    let first = place(&mut state, 0, "command_block", "not_a_command", true);
    let second = place(&mut state, 1, "chain_command_block", "time add 3", true);
    let block_id = state.world.block_at(second.x, second.y, second.z).unwrap();
    let block = state.registry.blocks.by_id(BlockStateId(block_id)).unwrap();
    let name = block.block.id.clone();
    let mut properties = block.properties.clone();
    properties
        .iter_mut()
        .find(|(key, _)| key == "conditional")
        .unwrap()
        .1 = "true".into();
    let conditional_id = state
        .registry
        .blocks
        .by_name_and_props(&name, &properties)
        .unwrap();
    state
        .world
        .set_block(second.x, second.y, second.z, conditional_id.0);
    schedule(&mut state, first);
    advance(&mut state);
    assert_eq!(state.day_time, 0);
    let mut fields = read(&state, first).unwrap();
    put(
        &mut fields,
        "Command",
        Tag::String("forceload query 0 0".into()),
    );
    save(&mut state, first, fields);
    schedule(&mut state, first);
    advance(&mut state);
    assert_eq!(state.day_time, 0);
    let mut fields = read(&state, first).unwrap();
    put(
        &mut fields,
        "Command",
        Tag::String("scoreboard objectives add test dummy".into()),
    );
    save(&mut state, first, fields);
    schedule(&mut state, first);
    advance(&mut state);
    assert_eq!(state.day_time, 3);
    let mut fields = read(&state, first).unwrap();
    put(&mut fields, "Command", Tag::String("time add 2".into()));
    save(&mut state, first, fields);
    schedule(&mut state, first);
    advance(&mut state);
    assert_eq!(state.day_time, 8);
}
#[test]
fn conditional_chain_follows_common_command_outcomes() {
    let mut state = fixture();
    let first = place(&mut state, 0, "command_block", "time add 0", true);
    let second = place(&mut state, 1, "chain_command_block", "time add 3", true);
    place(&mut state, 3, "command_block", "time add 0", false);
    let id = state.world.block_at(second.x, second.y, second.z).unwrap();
    let block = state.registry.blocks.by_id(BlockStateId(id)).unwrap();
    let name = block.block.id.clone();
    let mut properties = block.properties.clone();
    properties
        .iter_mut()
        .find(|(key, _)| key == "conditional")
        .unwrap()
        .1 = "true".into();
    let id = state
        .registry
        .blocks
        .by_name_and_props(&name, &properties)
        .unwrap();
    state.world.set_block(second.x, second.y, second.z, id.0);
    for (command, expected) in [
        ("setblock 0 61 0 minecraft:diamond_block", 3),
        ("give @a minecraft:diamond_sword", 6),
        ("enchant @a minecraft:sharpness 1", 9),
        ("enchant @a minecraft:sharpness 1", 9),
        (
            "execute if block 0 61 0 minecraft:diamond_block run say hello",
            12,
        ),
        ("data merge block 3 60 0 {Command:\"time add 4\"}", 15),
    ] {
        let mut fields = read(&state, first).unwrap();
        put(&mut fields, "Command", Tag::String(command.into()));
        save(&mut state, first, fields);
        schedule(&mut state, first);
        advance(&mut state);
        assert_eq!(state.day_time, expected, "{command}");
    }
}

#[test]
fn conditional_chain_executes_at_shared_command_boundary() {
    let mut state = fixture();
    state.dimension_globals.active = true;
    let first = place(&mut state, 0, "command_block", "time add 2", true);
    let second = place(&mut state, 1, "chain_command_block", "time add 3", true);
    let block_id = state.world.block_at(second.x, second.y, second.z).unwrap();
    let block = state.registry.blocks.by_id(BlockStateId(block_id)).unwrap();
    let name = block.block.id.clone();
    let mut properties = block.properties.clone();
    properties
        .iter_mut()
        .find(|(key, _)| key == "conditional")
        .unwrap()
        .1 = "true".into();
    let conditional_id = state
        .registry
        .blocks
        .by_name_and_props(&name, &properties)
        .unwrap();
    state
        .world
        .set_block(second.x, second.y, second.z, conditional_id.0);
    schedule(&mut state, first);
    advance(&mut state);
    let (mut output, mut drains, _) = crate::tick::run_dimension_tick(&mut state, &[]);
    assert_eq!(state.command_context.jobs.len(), 1);
    let before = state.day_time;
    crate::dimensions::travel::finish(&mut state, &mut output, &mut drains);
    assert_eq!(state.day_time, before + 5);
}

#[test]
fn impulse_rising_edge_drives_unconditional_chain_once() {
    let mut state = fixture();
    let impulse = place(&mut state, 0, "command_block", "time add 2", false);
    place(&mut state, 1, "chain_command_block", "time add 3", true);
    let power = state
        .registry
        .blocks
        .block(&mc_data::Identifier::parse("minecraft:redstone_block").unwrap())
        .unwrap()
        .default;
    state.world.set_block(0, 60, -1, power.0);
    changed(&mut state, impulse);
    advance(&mut state);
    assert_eq!(state.day_time, 5);
    changed(&mut state, impulse);
    advance(&mut state);
    assert_eq!(state.day_time, 5);
    state.world.set_block(0, 60, -1, state.registry.states.air);
    changed(&mut state, impulse);
    state.world.set_block(0, 60, -1, power.0);
    changed(&mut state, impulse);
    advance(&mut state);
    assert_eq!(state.day_time, 10);
}

#[test]
fn command_block_level_two_allows_time_but_denies_direct_and_nested_tick() {
    let mut state = fixture();
    for (x, text) in [
        (0, "time set 321"),
        (2, "tick freeze"),
        (4, "execute run tick freeze"),
    ] {
        let pos = place(&mut state, x, "command_block", text, true);
        schedule(&mut state, pos);
        advance(&mut state);
        assert!(
            matches!(mc_data::item_nbt::field(&read(&state, pos).unwrap(), "LastExecution"),
            Some(Tag::Long(tick)) if *tick == state.tick as i64),
            "block actually fired"
        );
        if x == 0 {
            assert_eq!(
                state.day_time, 321,
                "ordinary level2 block command executes"
            );
        }
        assert!(
            !state.tick_control.frozen(),
            "command-block permission: {text}"
        );
        assert!(state.command_context.current.is_none());
    }
}
#[test]
fn repeating_auto_uses_durable_ticks_and_disable_gate() {
    let mut state = fixture();
    let pos = place(&mut state, 0, "repeating_command_block", "time add 1", true);
    changed(&mut state, pos);
    assert!(!state.block_ticks.image().chunks.is_empty());
    advance(&mut state);
    advance(&mut state);
    assert_eq!(state.day_time, 2);
    state.enable_command_block = false;
    advance(&mut state);
    assert_eq!(state.day_time, 2);
}
#[test]
fn setblock_with_auto_runs_the_command_block_but_a_powered_or_chain_one_waits() {
    // Vanilla `CommandBlockEntity.loadAdditional` -> `setAutomatic(true)` -> `scheduleTick()`.
    let mut state = fixture();
    let connection = crate::command_console::CONNECTION;
    for (name, nbt, ran) in [
        (
            "command_block",
            "Command:\"setblock 0 61 0 minecraft:diamond_block\",auto:1b",
            true,
        ),
        (
            "repeating_command_block",
            "Command:\"setblock 0 61 0 minecraft:diamond_block\",auto:1b",
            true,
        ),
        (
            "command_block",
            "Command:\"setblock 0 61 0 minecraft:diamond_block\",auto:0b",
            false,
        ),
        (
            "chain_command_block",
            "Command:\"setblock 0 61 0 minecraft:diamond_block\",auto:1b",
            false,
        ),
    ] {
        crate::command_console::execute(
            &mut state,
            connection,
            "setblock 0 61 0 minecraft:air",
            2,
            &mut ChangeSet::default(),
        );
        crate::tick::run_tick(&mut state, &[]);
        crate::command_console::execute(
            &mut state,
            connection,
            &format!("setblock 0 60 0 minecraft:{name}[facing=east]{{{nbt}}}"),
            2,
            &mut ChangeSet::default(),
        );
        for _ in 0..4 {
            crate::tick::run_tick(&mut state, &[]);
        }
        let id = state.world.block_at(0, 61, 0).unwrap();
        let ran_now = state
            .registry
            .blocks
            .by_id(BlockStateId(id))
            .unwrap()
            .block
            .id
            .path()
            == "diamond_block";
        assert_eq!(ran_now, ran, "{name} {nbt}");
        crate::command_console::execute(
            &mut state,
            connection,
            "setblock 0 60 0 minecraft:air",
            2,
            &mut ChangeSet::default(),
        );
        crate::tick::run_tick(&mut state, &[]);
    }
}
fn first_argument_text(content: &Tag) -> Option<&str> {
    let Tag::Compound(fields) = content else {
        return None;
    };
    let Some(Tag::List(with)) = mc_data::item_nbt::field(fields, "with") else {
        return None;
    };
    let Some(Tag::Compound(first)) = with.elements.first() else {
        return None;
    };
    match mc_data::item_nbt::field(first, "text") {
        Some(Tag::String(text)) => Some(text),
        _ => None,
    }
}
fn lines_of(state: &TickState, connection: ConnectionId, key: &str) -> Vec<Tag> {
    state
        .outbox
        .get(&connection)
        .into_iter()
        .flatten()
        .filter_map(|change| match change {
            PlayerChange::StyledChat {
                content: Tag::Compound(fields),
            } if mc_data::item_nbt::field(fields, "translate")
                == Some(&Tag::String(key.into())) =>
            {
                Some(Tag::Compound(fields.clone()))
            }
            _ => None,
        })
        .collect()
}
#[test]
fn command_block_is_named_at_and_informs_operators_of_its_output() {
    // Vanilla `BaseCommandBlock` names its source `@` and `CommandSourceStack.broadcastToAdmins`
    // sends operators gray italic `chat.type.admin` while `sendCommandFeedback` and
    // `commandBlockOutput` are on.
    let mut state = fixture();
    let operator = ConnectionId(1);
    state.plugin_players.observe_permission(operator, 2);
    let say = place(&mut state, 0, "command_block", "say hi", true);
    let time = place(&mut state, 2, "command_block", "time set 4100", true);
    for pos in [say, time] {
        schedule(&mut state, pos);
    }
    advance(&mut state);
    let announcements = lines_of(&state, operator, "chat.type.announcement");
    assert_eq!(announcements.len(), 1);
    assert_eq!(first_argument_text(&announcements[0]), Some("@"));
    let admin = lines_of(&state, operator, "chat.type.admin");
    assert_eq!(
        admin.len(),
        1,
        "one admin line for `time set`, none for `say`"
    );
    assert_eq!(first_argument_text(&admin[0]), Some("@"));
    let Tag::Compound(fields) = &admin[0] else {
        unreachable!()
    };
    assert_eq!(
        mc_data::item_nbt::field(fields, "color"),
        Some(&Tag::String("gray".into()))
    );
    assert_eq!(
        mc_data::item_nbt::field(fields, "italic"),
        Some(&Tag::Byte(1))
    );
    // sendCommandFeedback off: nothing is broadcast; the console stays "Server".
    state.outbox.clear();
    crate::command_console::execute(
        &mut state,
        crate::command_console::CONNECTION,
        "gamerule send_command_feedback false",
        2,
        &mut ChangeSet::default(),
    );
    state.outbox.clear();
    schedule(&mut state, time);
    state.tick += 2;
    advance(&mut state);
    assert!(lines_of(&state, operator, "chat.type.admin").is_empty());
}
#[test]
fn data_merge_admits_only_supported_fields_and_requires_operator() {
    let mut state = fixture();
    let pos = place(
        &mut state,
        0,
        "repeating_command_block",
        "time add 1",
        false,
    );
    let connection = crate::command_console::CONNECTION;
    let command = "data merge block 0 60 0 {Command:\"time add 7\",auto:1b}";
    crate::command_console::execute(
        &mut state,
        connection,
        command,
        0,
        &mut ChangeSet::default(),
    );
    assert!(!flag(&read(&state, pos).unwrap(), "auto"));
    crate::command_console::execute(
        &mut state,
        connection,
        command,
        2,
        &mut ChangeSet::default(),
    );
    advance(&mut state);
    assert_eq!(state.day_time, 7);
    let before = read(&state, pos).unwrap();
    crate::command_console::execute(
        &mut state,
        connection,
        "data merge block 0 60 0 {Command:\"time set 0\",SuccessCount:12}",
        2,
        &mut ChangeSet::default(),
    );
    assert_eq!(read(&state, pos).unwrap(), before);
}
#[test]
fn commands_use_block_origin_and_opaque_wal() {
    let mut state = fixture();
    let pos = place(
        &mut state,
        0,
        "command_block",
        "setblock ~ ~1 ~ minecraft:diamond_block",
        true,
    );
    schedule(&mut state, pos);
    advance(&mut state);
    let id = state.world.block_at(0, 61, 0).unwrap();
    assert_eq!(
        state
            .registry
            .blocks
            .by_id(BlockStateId(id))
            .unwrap()
            .block
            .id
            .path(),
        "diamond_block"
    );
    assert!(
        state
            .opaque_changes
            .iter()
            .any(|(at, nbt)| *at == pos && nbt.is_some())
    );
    assert!(state.datapacks.origin.is_none());
}

#[test]
fn placement_requires_creative_and_operator_permission() {
    let (mut state, player, _) = crate::hostile_tests::setup(8.0);
    let connection = state.players[&player].connection;
    state.players.get_mut(&player).unwrap().game_mode = crate::commands::GameMode::Creative;
    assert!(!can_place(&state, player));
    state.plugin_players.observe_permission(connection, 2);
    assert!(can_place(&state, player));
    state.players.get_mut(&player).unwrap().game_mode = crate::commands::GameMode::Survival;
    assert!(!can_place(&state, player));
}
