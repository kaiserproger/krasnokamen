use super::*;
use crate::boundary::ConnectionId;
use crate::command_execution::CommandResult;

fn fixture() -> (TickState, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("custom");
    std::fs::create_dir_all(path.join("data/test/function")).unwrap();
    std::fs::write(path.join("pack.mcmeta"), r#"{"pack":{"pack_format":101,"description":"Custom pack"}}"#).unwrap();
    std::fs::write(path.join("data/test/function/value.mcfunction"), "scoreboard players set marker oracle 7\n").unwrap();
    let selection = Selection {
        disabled: vec!["file/custom".into()],
        ..Default::default()
    };
    let packs = crate::datapacks::PackSet::load(root.path(), selection).unwrap();
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.plugin_players.observe_permission(ConnectionId(1), 4);
    state.datapacks = crate::datapacks::Runtime::new(packs, root.path().to_owned(), state.registry.clone());
    (state, root)
}

fn run(state: &mut TickState, text: &str) {
    let executor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let context = crate::command_context::Context::source(state, ConnectionId(1), executor, 4);
    crate::command_context::run(state, text, context, &mut crate::boundary::ChangeSet::default());
}

fn score(state: &TickState, holder: &str) -> i32 {
    state.scoreboard.scores[holder]["oracle"].value
}

fn finish_worker(state: &mut TickState) {
    let mut reload = state.datapacks.reload.take().expect("queued worker");
    let prepared = reload.receiver.recv().expect("worker completion");
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    sender.send(prepared).unwrap();
    reload.receiver = receiver;
    state.datapacks.reload = Some(reload);
    crate::datapacks::tick(state, &mut crate::boundary::ChangeSet::default());
}

#[test]
fn enable_first_and_before_vanilla_preserve_requested_precedence() {
    for order in [vec!["first"], vec!["before", "vanilla"]] {
        let mut selection = Selection {
            enabled: vec!["vanilla".into(), "file/custom".into()],
            disabled: Vec::new(),
        };
        move_enabled(&mut selection, "file/custom", &order).unwrap();
        assert_eq!(selection.enabled, ["file/custom", "vanilla"]);
    }
}

#[test]
fn activation_completion_keeps_issuing_connection() {
    let (mut state, _root) = fixture();
    let connection = ConnectionId(1);
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, "execute store result score enabled oracle run datapack enable file/custom");
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("enabled"));
    assert_eq!(state.datapacks.reload.as_ref().unwrap().connection, connection);
    assert_eq!(state.datapacks.selection().enabled, ["vanilla"]);
    finish_worker(&mut state);
    assert_eq!(score(&state, "enabled"), 2);
    assert_eq!(state.datapacks.selection().enabled, ["vanilla", "file/custom"]);
    assert!(state.datapacks.packs().functions.contains_key("test:value"));
}

#[test]
fn enabling_an_enabled_pack_with_an_order_is_an_error() {
    let (mut state, _root) = fixture();
    let mut packs = state.datapacks.packs().clone();
    packs.selection.enabled.push("file/custom".into());
    packs.selection.disabled.clear();
    state.datapacks.install(packs, _root.path().to_owned(), state.registry.clone());
    assert!(matches!(command(&mut state, ConnectionId(1), "datapack enable file/custom first"), CommandResult::Failed));
    assert!(state.datapacks.reload.is_none());
    assert_eq!(state.datapacks.selection().enabled, ["vanilla", "file/custom"]);
}

#[test]
fn list_separates_enabled_and_available_with_vanilla_keys_and_total_result() {
    let (mut state, _root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, "scoreboard players set count oracle -1");
    run(&mut state, "execute store result score count oracle run datapack list");
    assert_eq!(score(&state, "count"), -1);
    finish_worker(&mut state);
    assert_eq!(score(&state, "count"), 2);
    let feedback = format!("{:?}", state.outbox);
    assert!(feedback.contains("commands.datapack.list.enabled.success"), "{feedback}");
    assert!(feedback.contains("commands.datapack.list.available.success"), "{feedback}");
    assert!(feedback.contains("Custom pack"), "{feedback}");
    assert!(feedback.contains("hover_event"), "{feedback}");
    assert!(!feedback.contains("hoverEvent"), "{feedback}");
}

#[test]
fn list_hover_keeps_structured_description_and_current_codec_keys() {
    let (mut state, root) = fixture();
    std::fs::write(
        root.path().join("custom/pack.mcmeta"),
        r#"{"pack":{"pack_format":101,"description":{"text":"Formatted description","color":"gold"}}}"#,
    ).unwrap();
    let packs = crate::datapacks::PackSet::load(root.path(), state.datapacks.selection().clone()).unwrap();
    state.datapacks.install(packs, root.path().to_owned(), state.registry.clone());
    run(&mut state, "datapack list available");
    finish_worker(&mut state);
    let feedback = format!("{:?}", state.outbox);
    assert!(feedback.contains("hover_event"), "{feedback}");
    assert!(feedback.contains("(\"value\", Compound"), "{feedback}");
    assert!(feedback.contains("(\"color\", String(\"gold\"))"), "{feedback}");
    assert!(!feedback.contains("contents"), "{feedback}");
}

#[test]
fn list_discovers_added_removed_and_changed_metadata_without_reloading_functions() {
    let (mut state, root) = fixture();
    let added = root.path().join("added");
    std::fs::create_dir(&added).unwrap();
    std::fs::write(added.join("pack.mcmeta"), r#"{"pack":{"description":"Discovered live","pack_format":121}}"#).unwrap();
    run(&mut state, "datapack list available");
    finish_worker(&mut state);
    assert!(state.datapacks.catalog().contains_key("file/added"));
    assert_eq!(state.datapacks.selection().enabled, ["vanilla"]);
    assert!(state.datapacks.packs().functions.is_empty());
    std::fs::remove_file(added.join("pack.mcmeta")).unwrap();
    std::fs::remove_dir(added).unwrap();
    run(&mut state, "datapack list available");
    finish_worker(&mut state);
    assert!(!state.datapacks.catalog().contains_key("file/added"));
}

#[test]
fn create_writes_description_and_does_not_enable_or_overwrite_pack() {
    let (mut state, root) = fixture();
    let executor = state.connections.entity_of(ConnectionId(1)).unwrap();
    let context = crate::command_context::Context::source(&state, ConnectionId(1), executor, 4);
    let run = |state: &mut TickState, text: &str| {
        crate::command_context::run(state, text, context, &mut crate::boundary::ChangeSet::default());
    };
    run(&mut state, r#"datapack create "New pack" {text:"Created description",color:"gold"}"#);
    finish_worker(&mut state);
    let path = root.path().join("New pack/pack.mcmeta");
    let bytes = std::fs::read(&path).unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(metadata["pack"]["description"]["text"], "Created description");
    assert_eq!(metadata["pack"]["description"]["color"], "gold");
    assert_eq!(metadata["pack"]["min_format"], serde_json::json!([121, 0]));
    assert_eq!(state.datapacks.selection().enabled, ["vanilla"]);
    assert!(state.datapacks.catalog().contains_key("file/New pack"));
    run(&mut state, r#"datapack create "New pack" "overwrite""#);
    finish_worker(&mut state);
    assert_eq!(std::fs::read(path).unwrap(), bytes);
    for invalid in ["../escape", "CON", "trailing.", "a/b", "LPT1"] {
        run(&mut state, &format!("datapack create \"{invalid}\" \"bad\""));
        assert!(state.datapacks.reload.is_none(), "{invalid}");
    }
}

fn install_function(state: &mut TickState, root: &std::path::Path, lines: &[&str]) {
    let mut packs = state.datapacks.packs().clone();
    packs.functions.insert("test:deferred".into(), lines.iter().map(|line| (*line).to_owned()).collect());
    state.datapacks.install(packs, root.to_owned(), state.registry.clone());
}

#[test]
fn create_stores_final_success_and_failure_without_running_following_lines_early() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        r#"execute store result score result oracle store success score success oracle run datapack create created "Created""#,
        "scoreboard players operation observed oracle = result oracle",
        r#"execute store result score failed_result oracle store success score failed_success oracle run datapack create created "Duplicate""#,
        "scoreboard players set finished oracle 1",
    ]);
    run(&mut state, "function test:deferred");
    assert!(!state.scoreboard.scores.contains_key("observed"));
    assert!(!state.scoreboard.scores.contains_key("success"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "result"), 1);
    assert_eq!(score(&state, "success"), 1);
    assert_eq!(score(&state, "observed"), 1);
    assert!(!state.scoreboard.scores.contains_key("finished"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "failed_result"), 0);
    assert_eq!(score(&state, "failed_success"), 0);
    assert_eq!(score(&state, "finished"), 1);
    assert!(!state.command_context.waiting());
}

#[test]
fn nested_execute_list_preserves_context_and_result_through_completion() {
    let (mut state, _root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, "execute positioned 4 5 6 as @s run execute store result score total oracle store success score success oracle run datapack list");
    assert!(!state.scoreboard.scores.contains_key("total"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "total"), 2);
    assert_eq!(score(&state, "success"), 1);
    assert!(state.command_context.current.is_none());
}

#[test]
fn duplicate_completion_cannot_overwrite_first_result_or_complete_later_command() {
    let (mut state, _root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, "execute store result score first oracle run datapack list");
    let id = state.datapacks.reload.as_ref().unwrap().completion.unwrap();
    crate::command_context::complete(&mut state, id, true, 41);
    crate::command_context::complete(&mut state, id, false, 0);
    finish_worker(&mut state);
    assert_eq!(score(&state, "first"), 41);
    run(&mut state, "execute store result score second oracle run datapack list");
    crate::command_context::complete(&mut state, id, true, 99);
    assert!(!state.scoreboard.scores.contains_key("second"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "second"), 2);
}

#[test]
fn function_budget_is_not_replenished_by_worker_resume() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        "datapack list",
        "scoreboard players set within oracle 1",
        "scoreboard players set overflow oracle 1",
    ]);
    run(&mut state, "gamerule minecraft:max_command_sequence_length 3");
    run(&mut state, "function test:deferred");
    finish_worker(&mut state);
    assert_eq!(score(&state, "within"), 1);
    assert!(!state.scoreboard.scores.contains_key("overflow"));
    assert!(format!("{:?}", state.outbox).contains("budget exceeded"));
}

#[test]
fn disconnected_worker_stores_failure_and_resumes_following_function_line() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        "execute store result score result oracle store success score success oracle run datapack list",
        "scoreboard players set resumed oracle 1",
    ]);
    run(&mut state, "function test:deferred");
    let mut reload = state.datapacks.reload.take().unwrap();
    let _ = reload.receiver.recv().unwrap();
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    drop(sender);
    reload.receiver = receiver;
    state.datapacks.reload = Some(reload);
    crate::datapacks::tick(&mut state, &mut crate::boundary::ChangeSet::default());
    assert_eq!(score(&state, "result"), 0);
    assert_eq!(score(&state, "success"), 0);
    assert_eq!(score(&state, "resumed"), 1);
    assert!(!state.command_context.waiting());
}

#[test]
fn replacement_connection_does_not_receive_completion_or_function_continuation() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        "execute store result score stale oracle run datapack list",
        "scoreboard players set stale_next oracle 1",
    ]);
    run(&mut state, "function test:deferred");
    state.connections.remove_by_connection(ConnectionId(1)).unwrap();
    state.connections.insert(ConnectionId(1), crate::boundary::EntityId(i32::MAX - 1),
        uuid::Uuid::new_v4(), "replacement".into());
    state.outbox.clear();
    finish_worker(&mut state);
    assert!(!state.scoreboard.scores.contains_key("stale"));
    assert!(!state.scoreboard.scores.contains_key("stale_next"));
    assert!(state.outbox.get(&ConnectionId(1)).is_none_or(|messages| messages.is_empty()));
    assert!(!state.command_context.waiting());
}

fn place_command_block(state: &mut TickState, x: i32, name: &str, conditional: bool, command: &str) -> crate::boundary::BlockPos {
    let name = mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap();
    let block = state.registry.blocks.block(&name).unwrap();
    let mut properties = state.registry.blocks.by_id(block.default).unwrap().properties.clone();
    for (key, value) in &mut properties {
        if key == "facing" { *value = "east".into(); }
        if key == "conditional" { *value = conditional.to_string(); }
    }
    let block = state.registry.blocks.by_name_and_props(&name, &properties).unwrap();
    let pos = crate::boundary::BlockPos { x, y: 60, z: 0 };
    state.world.set_block(pos.x, pos.y, pos.z, block.0);
    state.world.fold_overlay();
    run(state, &format!("data merge block {x} 60 0 {{Command:{},auto:1b}}",
        serde_json::to_string(command).unwrap()));
    pos
}

#[test]
fn deferred_command_block_finishes_before_conditional_chain_and_preserves_result() {
    let (mut state, _root) = fixture();
    state.enable_command_block = true;
    state.day_time = 0;
    run(&mut state, "scoreboard objectives add oracle dummy");
    let first = place_command_block(&mut state, 0, "command_block", false,
        "execute store result score block_total oracle run datapack list");
    place_command_block(&mut state, 1, "chain_command_block", true, "time add 3");
    state.command_blocks_due.push(first);
    crate::command_blocks::run(&mut state, &mut crate::boundary::ChangeSet::default());
    assert!(state.command_context.waiting());
    assert_eq!(state.day_time, 0);
    assert!(!state.scoreboard.scores.contains_key("block_total"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "block_total"), 2);
    assert_eq!(state.day_time, 3);
    assert!(!state.command_context.waiting());
}

#[test]
fn execute_forks_resume_each_source_once_before_following_function_line() {
    let (mut state, root) = fixture();
    crate::tick::run_tick(&mut state, &[crate::boundary::PlayerInput::Join {
        connection: ConnectionId(2), uuid: uuid::Uuid::new_v4(),
        name: "partner".into(), permission_level: 0,
    }]);
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        "execute as @a run execute store result score @s oracle run datapack list",
        "scoreboard players set finished oracle 1",
    ]);
    run(&mut state, "function test:deferred");
    finish_worker(&mut state);
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("finished"));
    assert_eq!(["target", "partner"].into_iter()
        .filter(|name| state.scoreboard.scores.contains_key(*name)).count(), 1);
    finish_worker(&mut state);
    assert_eq!(score(&state, "target"), 2);
    assert_eq!(score(&state, "partner"), 2);
    assert_eq!(score(&state, "finished"), 1);
    assert!(!state.command_context.waiting());
}

#[test]
fn completion_waits_for_shared_command_authority_before_store_and_resume() {
    let (mut state, _root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    state.dimension_globals.active = true;
    state.command_context.shared = true;
    run(&mut state, "execute store result score shared oracle run datapack list");
    state.command_context.shared = false;
    finish_worker(&mut state);
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("shared"));
    state.command_context.shared = true;
    crate::datapacks::poll_worker(&mut state, &mut crate::boundary::ChangeSet::default());
    crate::command_context::resume(&mut state, &mut crate::boundary::ChangeSet::default());
    assert_eq!(score(&state, "shared"), 2);
    assert!(!state.command_context.waiting());
}

#[test]
fn create_preserves_boolean_component_style_and_catalog_failure_creates_nothing() {
    let (mut state, root) = fixture();
    run(&mut state, r#"datapack create styled {text:"Styled",bold:true}"#);
    finish_worker(&mut state);
    let metadata: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.path().join("styled/pack.mcmeta")).unwrap()).unwrap();
    assert_eq!(metadata["pack"]["description"]["bold"], true);
    std::fs::write(root.path().join("custom/pack.mcmeta"), "not JSON").unwrap();
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, r#"execute store success score failed oracle run datapack create refused "Refused""#);
    finish_worker(&mut state);
    assert_eq!(score(&state, "failed"), 0);
    assert!(!root.path().join("refused").exists());
}

#[test]
fn queued_root_commands_run_after_completion_without_replenishing_awaited_sequence() {
    let (mut state, _root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    run(&mut state, "execute store result score count oracle run datapack list");
    run(&mut state, "scoreboard players operation queued oracle = count oracle");
    assert!(!state.scoreboard.scores.contains_key("queued"));
    assert_eq!(state.command_context.jobs.len(), 1);
    finish_worker(&mut state);
    assert_eq!(score(&state, "queued"), 2);
    assert!(state.command_context.jobs.is_empty());
}

#[test]
fn deferred_jobs_reject_disconnected_reused_and_deopped_sources_in_both_drains() {
    for shared in [false, true] {
        for action in ["disconnect", "replace", "deop"] {
            let (mut state, _root) = fixture();
            let (mut output, _, _) = crate::tick::run_tick(&mut state, &[crate::boundary::PlayerInput::Join {
                connection: ConnectionId(2), uuid: uuid::Uuid::new_v4(),
                name: "queued-source".into(), permission_level: 4,
            }]);
            state.dimension_globals.active = shared;
            state.command_context.shared = shared;
            run(&mut state, "datapack list");
            let actor = state.connections.entity_of(ConnectionId(2)).unwrap();
            let context = crate::command_context::Context::source(&state, ConnectionId(2), actor, 4);
            crate::command_context::run(&mut state, "time set 6000", context, &mut Default::default());
            assert_eq!(state.command_context.jobs.len(), 1);
            if action == "deop" {
                state.plugin_players.observe_permission(ConnectionId(2), 0);
            } else {
                // Hold the worker reply while real Leave/Join inputs replace
                // the session; never fabricate an entity-less connection.
                let mut reload = state.datapacks.reload.take().unwrap();
                let prepared = reload.receiver.recv().unwrap();
                let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                reload.receiver = receiver;
                state.datapacks.reload = Some(reload);
                let mut inputs = vec![crate::boundary::PlayerInput::Leave { connection: ConnectionId(2) }];
                if action == "replace" {
                    inputs.push(crate::boundary::PlayerInput::Join {
                        connection: ConnectionId(2), uuid: uuid::Uuid::new_v4(),
                        name: "replacement".into(), permission_level: 4,
                    });
                }
                (output, _, _) = crate::tick::run_tick(&mut state, &inputs);
                sender.send(prepared).unwrap();
            }
            let time = state.day_time;
            state.command_context.shared = false;
            state.outbox.clear();
            finish_worker(&mut state);
            let mut drains = Vec::new();
            if shared {
                crate::dimensions::travel::finish(&mut state, &mut output, &mut drains);
            }
            assert_eq!(state.day_time, time, "{shared}/{action}");
            assert!(state.command_context.jobs.is_empty(), "{shared}/{action}");
            assert!(state.outbox.get(&ConnectionId(2)).is_none_or(Vec::is_empty));
            assert!(!drains.iter().any(|(connection, messages)| *connection == ConnectionId(2)
                && messages.iter().any(|message| matches!(message, crate::boundary::PlayerChange::StyledChat { .. }))));
        }
    }
}

#[test]
fn deferred_permission_revocation_cancels_stores_and_function_continuation() {
    let (mut state, root) = fixture();
    run(&mut state, "scoreboard objectives add oracle dummy");
    install_function(&mut state, root.path(), &[
        "execute store result score stale oracle run datapack list",
        "scoreboard players set stale_next oracle 1",
    ]);
    run(&mut state, "function test:deferred");
    state.plugin_players.observe_permission(ConnectionId(1), 0);
    state.outbox.clear();
    finish_worker(&mut state);
    assert!(!state.scoreboard.scores.contains_key("stale"));
    assert!(!state.scoreboard.scores.contains_key("stale_next"));
    assert!(state.outbox.get(&ConnectionId(1)).is_none_or(Vec::is_empty));
    assert!(!state.command_context.waiting());
}

#[test]
fn plain_reload_awaits_publication_before_next_function_and_preserves_catalog_on_failure() {
    let (mut state, root) = fixture();
    let functions = root.path().join("custom/data/test/function");
    std::fs::write(functions.join("outer.mcfunction"), "reload\nfunction test:value\n").unwrap();
    run(&mut state, "datapack enable file/custom");
    finish_worker(&mut state);
    run(&mut state, "scoreboard objectives add oracle dummy");
    std::fs::write(functions.join("value.mcfunction"), "scoreboard players set marker oracle 17\n").unwrap();
    run(&mut state, "function test:outer");
    assert!(state.command_context.waiting());
    assert!(!state.scoreboard.scores.contains_key("marker"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "marker"), 17);
    let old = state.datapacks.packs.clone();
    let registry = state.registry.clone();
    std::fs::write(root.path().join("custom/pack.mcmeta"), "malformed").unwrap();
    run(&mut state, "execute store success score failed oracle run reload");
    assert!(!state.scoreboard.scores.contains_key("failed"));
    finish_worker(&mut state);
    assert_eq!(score(&state, "failed"), 0);
    assert!(std::sync::Arc::ptr_eq(&old, &state.datapacks.packs));
    assert!(std::sync::Arc::ptr_eq(&registry, &state.registry));
}
