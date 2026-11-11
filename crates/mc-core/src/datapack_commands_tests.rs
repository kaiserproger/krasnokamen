//! `/datapack` list, enable, disable and create, ZIP packs and the selector
//! `predicate=` option, through the real command path and the datapack worker.
use super::*;
use crate::command_execution::CommandResult::{self, Completed, Failed, Success};
use std::io::Write;

fn pack(root: &Path, name: &str, description: &str, resources: &[(&str, &str)]) {
    let pack = root.join(name);
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        format!(r#"{{"pack":{{"pack_format":101,"description":{description}}}}}"#),
    )
    .unwrap();
    for (path, text) in resources {
        let path = pack.join("data").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
}

fn fixture(root: &Path) -> TickState {
    let (mut state, _, _) = crate::hostile_tests::setup(8.0);
    state.datapacks = Runtime::new(
        PackSet::load(root, Selection::default()).unwrap(),
        root.to_path_buf(),
    );
    state
}

/// Run one command as the console at `permission`, returning its result.
fn execute(state: &mut TickState, line: &str, permission: u8) -> CommandResult {
    state.command_context.current = None;
    let context = crate::command_context::Context::source(
        state,
        crate::command_console::CONNECTION,
        crate::command_console::EXECUTOR,
        permission,
    );
    crate::command_context::begin_sequence(state);
    state.command_context.current = Some(context);
    crate::command_context::native(state, line, context, &mut ChangeSet::default(), false)
        .unwrap_or(Failed)
}

fn run(state: &mut TickState, line: &str) -> CommandResult {
    execute(state, line, 4)
}

/// Everything the console was told since the last call, as components.
fn messages(state: &mut TickState) -> Vec<Tag> {
    state
        .outbox
        .remove(&crate::command_console::CONNECTION)
        .unwrap_or_default()
        .into_iter()
        .flat_map(|change| match change {
            PlayerChange::StyledChat { content } => vec![content],
            PlayerChange::Chat { message } => vec![text(message)],
            PlayerChange::TextBurst { lines } => lines
                .into_iter()
                .filter_map(|line| match line {
                    PlayerChange::StyledChat { content } => Some(content),
                    PlayerChange::Chat { message } => Some(text(message)),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        })
        .collect()
}

fn field<'a>(tag: &'a Tag, name: &str) -> &'a Tag {
    let Tag::Compound(fields) = tag else {
        panic!("{tag:?} is not a compound");
    };
    fields
        .iter()
        .find_map(|(key, value)| (key == name).then_some(value))
        .unwrap_or_else(|| panic!("{tag:?} has no {name}"))
}

fn string(tag: &Tag) -> &str {
    let Tag::String(value) = tag else {
        panic!("{tag:?} is not a string");
    };
    value
}

fn items(tag: &Tag) -> &[Tag] {
    let Tag::List(list) = tag else {
        panic!("{tag:?} is not a list");
    };
    &list.elements
}

fn key(tag: &Tag) -> &str {
    string(field(tag, "translate"))
}

/// The pack links inside a `datapack list` message: its count and entries.
fn listed(message: &Tag) -> (String, Vec<&Tag>) {
    let with = items(field(message, "with"));
    let count = string(field(&with[0], "text")).to_owned();
    let links = match &with[1] {
        list if matches!(list, Tag::Compound(f) if f.iter().any(|(k, _)| k == "extra")) => items(
            field(list, "extra"),
        )
        .iter()
        .filter(|part| matches!(part, Tag::Compound(f) if f.iter().any(|(k, _)| k == "translate")))
        .collect(),
        single => vec![single],
    };
    (count, links)
}

fn link_id(link: &Tag) -> String {
    // The insertion is the quoted id; strip the quotes the `/` forces.
    string(field(link, "insertion"))
        .trim_matches('"')
        .to_owned()
}

#[test]
fn list_entries_are_chat_links_with_title_and_description_hover() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "alpha", r#""Alpha pack""#, &[]);
    pack(&root, "beta", r#"{"text":"Beta pack","color":"gold"}"#, &[]);
    let mut state = fixture(&root);
    run(&mut state, "datapack disable file/beta");
    settle(&mut state);
    messages(&mut state);

    assert_eq!(run(&mut state, "datapack list"), Success(3));
    settle(&mut state);
    let shown = messages(&mut state);
    assert_eq!(
        shown.len(),
        2,
        "enabled group, then available group: {shown:?}"
    );

    assert_eq!(key(&shown[0]), "commands.datapack.list.enabled.success");
    let (count, enabled) = listed(&shown[0]);
    assert_eq!(count, "2");
    assert_eq!(
        enabled.iter().map(|link| link_id(link)).collect::<Vec<_>>(),
        ["vanilla", "file/alpha"]
    );
    let alpha = enabled[1];
    assert_eq!(key(alpha), "chat.square_brackets");
    assert_eq!(string(field(alpha, "color")), "green");
    // Hover: title, a line break, then the pack.mcmeta description.
    let hover = field(alpha, "hover_event");
    assert_eq!(string(field(hover, "action")), "show_text");
    assert_eq!(
        items(field(field(hover, "value"), "extra")),
        [text("alpha"), text("\n"), text("Alpha pack")]
    );
    // The vanilla pack's title and description are translatable built-ins.
    let vanilla_hover = field(enabled[0], "hover_event");
    let parts = items(field(field(vanilla_hover, "value"), "extra"));
    assert_eq!(key(&parts[0]), "dataPack.vanilla.name");
    assert_eq!(key(&parts[2]), "dataPack.vanilla.description");

    assert_eq!(key(&shown[1]), "commands.datapack.list.available.success");
    let (count, available) = listed(&shown[1]);
    assert_eq!(count, "1");
    let beta = available[0];
    assert_eq!(link_id(beta), "file/beta");
    assert_eq!(string(field(beta, "color")), "red");
    let hover = field(field(field(beta, "hover_event"), "value"), "extra");
    assert_eq!(string(field(&items(hover)[2], "text")), "Beta pack");
    assert_eq!(string(field(&items(hover)[2], "color")), "gold");

    // A scope with nothing in it says so with the vanilla key.
    run(&mut state, "datapack enable file/beta");
    settle(&mut state);
    messages(&mut state);
    assert_eq!(run(&mut state, "datapack list available"), Success(0));
    settle(&mut state);
    let shown = messages(&mut state);
    assert_eq!(key(&shown[0]), "commands.datapack.list.available.none");
}

#[test]
fn list_sees_packs_added_and_removed_on_disk_without_a_reload() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "first", r#""First""#, &[]);
    let mut state = fixture(&root);

    // Added after startup: listed as available, not enabled behind the
    // operator's back, and the selection authority is unchanged.
    pack(&root, "added", r#""Added""#, &[]);
    run(&mut state, "datapack list available");
    settle(&mut state);
    let shown = messages(&mut state);
    let (count, links) = listed(&shown[0]);
    assert_eq!(count, "1");
    assert_eq!(link_id(links[0]), "file/added");
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/first"]
    );

    // Removed after startup: gone from the enabled list at once.
    std::fs::remove_dir_all(root.join("first")).unwrap();
    run(&mut state, "datapack list enabled");
    settle(&mut state);
    let shown = messages(&mut state);
    let (count, links) = listed(&shown[0]);
    assert_eq!(count, "1");
    assert_eq!(
        links.iter().map(|l| link_id(l)).collect::<Vec<_>>(),
        ["vanilla"]
    );

    // An archive dropped into the folder is found the same way.
    write_zip(
        &root.join("zipped.zip"),
        &[("pack.mcmeta", r#"{"pack":{"description":"Zipped"}}"#)],
    );
    run(&mut state, "datapack list available");
    settle(&mut state);
    let shown = messages(&mut state);
    let (count, links) = listed(&shown[0]);
    assert_eq!(count, "2");
    assert_eq!(
        links.iter().map(|l| link_id(l)).collect::<Vec<_>>(),
        ["file/added", "file/zipped.zip"]
    );
}

#[test]
fn enable_and_disable_follow_vanilla_ordering_errors_and_results() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    for name in ["a", "b", "c"] {
        pack(
            &root,
            name,
            "\"p\"",
            &[(&format!("test/function/{name}.mcfunction"), "time add 1")],
        );
    }
    let mut state = fixture(&root);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/a", "file/b", "file/c"]
    );

    // Disable: the result is the number of packs left loaded; the functions
    // of the pack disappear once the worker's recompile is live.
    assert_eq!(run(&mut state, "datapack disable file/b"), Success(3));
    assert!(
        state.datapacks.packs().functions.contains_key("test:b"),
        "old packs stay authoritative until the worker answers"
    );
    settle(&mut state);
    assert!(!state.datapacks.packs().functions.contains_key("test:b"));
    let shown = messages(&mut state);
    assert_eq!(key(&shown[0]), "commands.datapack.modify.disable");
    assert_eq!(link_id(&items(field(&shown[0], "with"))[0]), "file/b");
    assert_eq!(state.datapacks.selection().disabled, ["file/b"]);

    // Vanilla refusals.
    for (line, expected) in [
        (
            "datapack disable file/b",
            "commands.datapack.disable.failed",
        ),
        ("datapack disable nosuch", "commands.datapack.unknown"),
        (
            "datapack disable vanilla",
            "commands.datapack.disable.failed.feature",
        ),
        ("datapack enable file/a", "commands.datapack.enable.failed"),
        (
            "datapack enable file/a first",
            "commands.datapack.enable.failed",
        ),
        (
            "datapack enable file/b before nosuch",
            "commands.datapack.unknown",
        ),
        (
            "datapack enable file/b after file/b",
            "commands.datapack.unknown",
        ),
    ] {
        assert_eq!(run(&mut state, line), Failed, "{line}");
        let shown = messages(&mut state);
        assert_eq!(key(&shown[0]), expected, "{line}");
    }
    assert!(
        state.datapacks.reload.is_none(),
        "a refused command starts no worker"
    );
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/a", "file/c"]
    );

    // Enabling places the pack: `first` stays behind the built-in pack.
    assert_eq!(run(&mut state, "datapack enable file/b first"), Success(4));
    settle(&mut state);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/b", "file/a", "file/c"]
    );
    assert!(state.datapacks.packs().functions.contains_key("test:b"));
    let shown = messages(&mut state);
    assert_eq!(key(&shown[0]), "commands.datapack.modify.enable");

    run(&mut state, "datapack disable file/c");
    settle(&mut state);
    run(&mut state, "datapack enable file/c after file/b");
    settle(&mut state);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/b", "file/c", "file/a"]
    );
    run(&mut state, "datapack disable file/b");
    settle(&mut state);
    run(&mut state, "datapack enable file/b before file/a");
    settle(&mut state);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/c", "file/b", "file/a"]
    );
    run(&mut state, "datapack disable file/c");
    settle(&mut state);
    run(&mut state, "datapack enable file/c last");
    settle(&mut state);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/b", "file/a", "file/c"]
    );

    // A pack the folder does not hold is refused by the worker; nothing changes.
    messages(&mut state);
    let before = state.datapacks.selection().clone();
    assert_eq!(run(&mut state, "datapack enable file/ghost"), Success(5));
    settle(&mut state);
    let shown = messages(&mut state);
    assert_eq!(key(&shown[0]), "commands.datapack.unknown");
    assert_eq!(*state.datapacks.selection(), before);

    // Quoted names work, and a second job while one runs is refused.
    pack(&root, "odd name", "\"p\"", &[]);
    run(&mut state, "datapack list");
    assert_eq!(run(&mut state, "datapack disable file/b"), Failed);
    assert_eq!(
        string(field(&messages(&mut state)[0], "text")),
        "Datapack reload already in progress"
    );
    settle(&mut state);
    messages(&mut state);
    assert_eq!(
        run(&mut state, "datapack enable \"file/odd name\""),
        Success(5)
    );
    settle(&mut state);
    assert!(
        state
            .datapacks
            .selection()
            .enabled
            .contains(&"file/odd name".to_owned())
    );
}

#[test]
fn create_writes_an_empty_directory_pack_with_a_description() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    let mut state = fixture(&root);

    // Permission 4 only: level 2 may manage packs but not create them.
    assert_eq!(
        execute(&mut state, "datapack create made \"Made here\"", 2),
        Failed
    );
    assert!(!root.join("made").exists());
    messages(&mut state);

    assert_eq!(
        run(&mut state, "datapack create made \"Made here\""),
        Completed
    );
    assert!(
        !root.join("made").exists(),
        "the worker writes, not the tick"
    );
    settle(&mut state);
    let shown = messages(&mut state);
    assert_eq!(key(&shown[0]), "commands.datapack.create.success");
    assert_eq!(
        string(field(&items(field(&shown[0], "with"))[0], "text")),
        "made"
    );
    assert!(root.join("made/data").is_dir());
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("made/pack.mcmeta")).unwrap())
            .unwrap();
    assert_eq!(meta["pack"]["description"], "Made here");
    assert_eq!(meta["pack"]["min_format"], DATA_PACK_FORMAT);

    // Created, not enabled: available, and a component description is kept.
    assert_eq!(
        run(
            &mut state,
            r##"datapack create fancy {"text":"Fancy","color":"red"}"##
        ),
        Completed
    );
    settle(&mut state);
    messages(&mut state);
    let meta: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(root.join("fancy/pack.mcmeta")).unwrap())
            .unwrap();
    assert_eq!(meta["pack"]["description"]["color"], "red");
    assert_eq!(state.datapacks.selection().enabled, ["vanilla"]);
    run(&mut state, "datapack list available");
    settle(&mut state);
    let shown = messages(&mut state);
    let (count, links) = listed(&shown[0]);
    assert_eq!(count, "2");
    assert_eq!(
        links.iter().map(|l| link_id(l)).collect::<Vec<_>>(),
        ["file/fancy", "file/made"]
    );
    // ... and it loads once enabled.
    run(&mut state, "datapack enable file/made");
    settle(&mut state);
    assert_eq!(
        state.datapacks.selection().enabled,
        ["vanilla", "file/made"]
    );
    messages(&mut state);

    // Vanilla refusals.
    for (line, expected) in [
        (
            "datapack create made x",
            "commands.datapack.create.already_exists",
        ),
        (
            "datapack create \"Bad Name\" x",
            "commands.datapack.create.invalid_name",
        ),
        (
            "datapack create .. x",
            "commands.datapack.create.invalid_name",
        ),
        (
            "datapack create con x",
            "commands.datapack.create.invalid_full_name",
        ),
        (
            "datapack create lpt1.txt x",
            "commands.datapack.create.invalid_full_name",
        ),
        (
            "datapack create trailing. x",
            "commands.datapack.create.invalid_full_name",
        ),
    ] {
        let result = run(&mut state, line);
        settle(&mut state);
        let shown = messages(&mut state);
        assert_eq!(
            result.clone(),
            if expected.contains("already") {
                Completed
            } else {
                Failed
            },
            "{line}"
        );
        assert_eq!(key(&shown[0]), expected, "{line}");
    }
    assert!(!root.join("con").exists());
}

fn write_zip(path: &Path, files: &[(&str, &str)]) {
    let mut writer = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    for (name, body) in files {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(body.as_bytes()).unwrap();
    }
    writer.finish().unwrap();
}

#[test]
fn zip_packs_load_functions_tags_recipes_and_refuse_unsafe_entries() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    std::fs::create_dir_all(&root).unwrap();
    write_zip(
        &root.join("bundle.zip"),
        &[
            (
                "pack.mcmeta",
                r#"{"pack":{"pack_format":101,"description":"Zip"}}"#,
            ),
            ("data/test/function/zipped.mcfunction", "time add 7"),
            (
                "data/test/tags/function/all.json",
                r#"{"values":["test:zipped"]}"#,
            ),
            ("README.txt", "outside data/ is ignored"),
        ],
    );
    let packs = PackSet::load(&root, Selection::default()).unwrap();
    assert_eq!(
        packs.selection.enabled,
        ["vanilla", "file/bundle.zip"],
        "an archive is a pack named by its file name"
    );
    assert!(packs.functions.contains_key("test:zipped"));
    assert_eq!(&*packs.tags["test:all"], ["test:zipped"]);
    let mut state = fixture(&root);
    state.day_time = 0;
    run(&mut state, "function #test:all");
    assert_eq!(state.day_time, 7);

    // Disabling an archive pack and enabling it again goes through the worker.
    run(&mut state, "datapack disable file/bundle.zip");
    settle(&mut state);
    assert!(
        !state
            .datapacks
            .packs()
            .functions
            .contains_key("test:zipped")
    );
    run(&mut state, "datapack enable file/bundle.zip");
    settle(&mut state);
    assert!(
        state
            .datapacks
            .packs()
            .functions
            .contains_key("test:zipped")
    );

    // A path that leaves `data/` is refused for the whole archive.
    write_zip(
        &root.join("bundle.zip"),
        &[
            ("pack.mcmeta", r#"{"pack":{"description":"Zip"}}"#),
            ("data/test/../../evil.mcfunction", "time add 1"),
        ],
    );
    let error = PackSet::load(&root, Selection::default()).err().unwrap();
    assert!(error.contains("Unsafe datapack archive path"), "{error}");

    // So is an archive with no metadata, and unsupported resource kinds.
    write_zip(
        &root.join("bundle.zip"),
        &[("data/test/function/x.mcfunction", "time add 1")],
    );
    assert!(PackSet::load(&root, Selection::default()).is_err());
    write_zip(
        &root.join("bundle.zip"),
        &[
            ("pack.mcmeta", r#"{"pack":{"description":"Zip"}}"#),
            ("data/test/worldgen/biome/x.json", "{}"),
        ],
    );
    let loaded = PackSet::load(&root, Selection::default()).unwrap();
    assert_eq!(
        loaded.ignored["file/bundle.zip"],
        ["test/worldgen/biome/x.json"],
        "a ZIP pack's worldgen resources are ignored like a directory pack's"
    );
}

#[test]
fn an_archive_with_more_entries_than_the_resource_limit_is_refused() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    std::fs::create_dir_all(&root).unwrap();
    // Entries outside data/ are ignored by the loader but still cost memory
    // when the archive opens.
    let names: Vec<String> = (0..=MAX_RESOURCES)
        .map(|index| format!("outside/{index}.txt"))
        .collect();
    let mut files = vec![("pack.mcmeta", r#"{"pack":{"description":"Zip"}}"#)];
    files.extend(names.iter().map(|name| (name.as_str(), "")));
    write_zip(&root.join("bundle.zip"), &files);
    let error = PackSet::load(&root, Selection::default()).err().unwrap();
    assert!(error.contains("too many entries"), "{error}");
}

fn summon(state: &mut TickState, kind: &str) {
    run(state, &format!("summon minecraft:{kind} ~ ~ ~"));
}

fn install_predicates(state: &mut TickState, packs: &PackSet) {
    packs
        .install_content(Arc::make_mut(&mut state.registry))
        .unwrap();
}

#[test]
fn selector_predicate_selects_and_negates_datapack_predicates() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(
        &root,
        "p",
        "\"p\"",
        &[
            (
                "test/predicate/zombie.json",
                r#"{"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:zombie"}}"#,
            ),
            (
                "test/predicate/not_zombie.json",
                r#"[{"condition":"minecraft:inverted","term":{"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:zombie"}}}]"#,
            ),
            (
                "test/predicate/scored.json",
                r#"{"condition":"minecraft:entity_scores","entity":"this","scores":{"hits":{"min":2}}}"#,
            ),
            (
                "test/predicate/either.json",
                r#"{"condition":"minecraft:any_of","terms":[{"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:zombie"}},{"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:cow"}}]}"#,
            ),
        ],
    );
    let packs = PackSet::load(&root, Selection::default()).unwrap();
    let mut state = fixture(&root);
    install_predicates(&mut state, &packs);
    run(&mut state, "kill @e[type=!minecraft:player]");
    summon(&mut state, "zombie");
    summon(&mut state, "zombie");
    summon(&mut state, "cow");

    // How many mobs a selector picks, without touching them.
    let count = |state: &mut TickState, selector: &str| {
        state.command_context.current = None;
        let context = crate::command_context::Context::source(
            state,
            crate::command_console::CONNECTION,
            crate::command_console::EXECUTOR,
            4,
        );
        let target = crate::commands::target_of(&format!(
            "@e[type=!minecraft:player,type=!minecraft:item,type=!minecraft:experience_orb,{selector}]"
        ))
        .unwrap();
        crate::dimensions::travel::context_targets(state, context, &target)
            .unwrap()
            .len()
    };
    assert_eq!(count(&mut state, "predicate=test:zombie"), 2);
    assert_eq!(
        count(&mut state, "predicate=!test:zombie"),
        1,
        "negated: only the cow"
    );
    assert_eq!(
        count(&mut state, "predicate=test:not_zombie"),
        1,
        "inverted condition list"
    );
    assert_eq!(
        count(&mut state, "predicate=test:either"),
        3,
        "any_of terms"
    );
    assert_eq!(
        count(
            &mut state,
            "predicate=test:zombie,predicate=test:not_zombie"
        ),
        0,
        "all must match"
    );
    // A predicate no datapack defines matches nothing.
    assert_eq!(count(&mut state, "predicate=test:nosuch"), 0);

    // entity_scores reads the candidate's own scoreboard value.
    run(&mut state, "scoreboard objectives add hits dummy");
    let zombie = state
        .entities
        .slots()
        .find(|slot| {
            crate::command_execution::entity_type_name(&state, state.entities.body(*slot).id)
                == Some("minecraft:zombie")
        })
        .map(|slot| state.entities.row(slot).uuid.hyphenated().to_string())
        .unwrap();
    run(
        &mut state,
        &format!("scoreboard players set {zombie} hits 3"),
    );
    assert_eq!(count(&mut state, "predicate=test:scored"), 1);
}

#[test]
fn predicates_with_conditions_the_selector_cannot_evaluate_are_refused_at_load() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(
        &root,
        "bad",
        "\"p\"",
        &[(
            "test/predicate/bad.json",
            r#"{"condition":"minecraft:value_check","value":1,"range":1}"#,
        )],
    );
    let error = PackSet::load(&root, Selection::default()).err().unwrap();
    assert!(
        error.contains("Unsupported predicate test:bad") && error.contains("value_check"),
        "{error}"
    );
    // A malformed selector option is a parse error, not an unsupported option.
    assert!(crate::commands::parse("kill @e[predicate=Bad Id]", true).is_err());
    assert!(crate::commands::parse("kill @e[predicate=test:ok]", true).is_ok());
}

const GEM_RECIPES: [(&str, &str); 3] = [
    (
        "minecraft/recipe/bone_meal.json",
        r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:bone"],"result":{"id":"minecraft:diamond","count":2}}"#,
    ),
    (
        "test/recipe/new_gem.json",
        r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:bone"],"result":{"id":"minecraft:emerald","count":1}}"#,
    ),
    (
        "minecraft/loot_table/blocks/stone.json",
        r#"{"type":"minecraft:block","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:diamond"}]}]}"#,
    ),
];

fn stone_drops(registry: &crate::registry::CoreRegistry) -> Vec<String> {
    let block = mc_data::Identifier::parse("minecraft:stone").unwrap();
    let tool = mc_data::loot::LootContextItem::empty();
    // The vanilla table names its random sequence, a datapack table need not.
    [
        None,
        Some(mc_data::Identifier::parse("minecraft:blocks/stone").unwrap()),
    ]
    .into_iter()
    .find_map(|sequence| {
        let context = mc_data::loot::BlockLootContext::try_new(
            &block,
            &[],
            &tool,
            mc_data::loot::LootRandomBinding::new(sequence, 1),
        )
        .unwrap();
        registry.loot.evaluate_block(&context).ok().flatten()
    })
    .expect("stone has a loot table")
    .iter()
    .map(|drop| drop.item.as_str().to_owned())
    .collect()
}

fn recipe_result(registry: &crate::registry::CoreRegistry, id: &str) -> Option<String> {
    registry
        .recipes
        .iter()
        .find(|recipe| recipe.id.as_str() == id)
        .map(|recipe| recipe.result.item.as_str().to_owned())
}

#[test]
fn enabling_and_disabling_a_pack_installs_its_recipes_and_loot_and_resyncs_clients() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "gems", "\"Gems\"", &GEM_RECIPES);
    let mut state = fixture(&root);
    // The world starts with the pack off.
    state.datapacks = Runtime::new(
        PackSet::load(
            &root,
            Selection {
                enabled: vec!["vanilla".into()],
                disabled: vec!["file/gems".into()],
            },
        )
        .unwrap(),
        root.clone(),
    );
    let recipes_before = state.registry.recipes.len();
    let vanilla_bone_meal = recipe_result(&state.registry, "minecraft:bone_meal");
    let vanilla_drops = stone_drops(&state.registry);
    assert_ne!(vanilla_bone_meal.as_deref(), Some("minecraft:diamond"));
    assert_ne!(vanilla_drops, ["minecraft:diamond"]);
    state.outbox.clear();
    // The cell a joining player's recipes are read from (a core with nobody
    // online still follows the reload).
    let published = Arc::new(std::sync::RwLock::new(Arc::clone(&state.registry)));
    state.published_registry = Some(Arc::clone(&published));

    run(&mut state, "datapack enable file/gems");
    settle(&mut state);
    assert!(
        Arc::ptr_eq(&published.read().unwrap(), &state.registry),
        "the registry joiners see is the reloaded one"
    );
    assert_eq!(
        recipe_result(&state.registry, "test:new_gem").as_deref(),
        Some("minecraft:emerald"),
        "a recipe the pack adds"
    );
    assert_eq!(
        recipe_result(&state.registry, "minecraft:bone_meal").as_deref(),
        Some("minecraft:diamond"),
        "a recipe the pack replaces"
    );
    assert_eq!(stone_drops(&state.registry), ["minecraft:diamond"]);
    assert_eq!(state.registry.recipes.len(), recipes_before + 1);
    let player = crate::boundary::ConnectionId(1);
    let sent = state.outbox.get(&player).cloned().unwrap_or_default();
    let announced = sent
        .iter()
        .position(|change| matches!(change, PlayerChange::RecipesReloaded { .. }))
        .expect("the client is told the recipes changed");
    assert!(
        sent[announced..]
            .iter()
            .any(|change| matches!(change, PlayerChange::RecipesUnlocked { replace: true, .. })),
        "and its recipe book is sent again: {sent:?}"
    );
    messages(&mut state);

    run(&mut state, "datapack disable file/gems");
    settle(&mut state);
    assert_eq!(recipe_result(&state.registry, "test:new_gem"), None);
    assert!(Arc::ptr_eq(&published.read().unwrap(), &state.registry));
    assert_eq!(
        recipe_result(&state.registry, "minecraft:bone_meal"),
        vanilla_bone_meal,
        "a replaced vanilla recipe comes back"
    );
    assert_eq!(stone_drops(&state.registry), vanilla_drops);
    assert_eq!(state.registry.recipes.len(), recipes_before);
}

/// What a reload costs the tick thread: the registry clone and the packs'
/// recipes and loot, with the default registry.
#[test]
#[ignore = "microbenchmark; prints the median reload install time"]
fn datapack_reload_install_microbench() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "gems", "\"Gems\"", &GEM_RECIPES);
    let mut state = fixture(&root);
    let packs = PackSet::load(&root, Selection::default()).unwrap();
    let mut samples = Vec::new();
    for _ in 0..7 {
        let start = std::time::Instant::now();
        install_registry(&mut state, &packs).unwrap();
        samples.push(start.elapsed().as_micros());
    }
    samples.sort_unstable();
    eprintln!(
        "reload install us: median={} samples={samples:?}",
        samples[3]
    );
}

#[test]
fn a_reload_with_an_unusable_recipe_keeps_the_old_registry_and_selection() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(
        &root,
        "broken",
        "\"Broken\"",
        &[(
            "test/recipe/nothing.json",
            r#"{"type":"minecraft:crafting_shapeless","ingredients":["minecraft:bone"],"result":{"id":"minecraft:no_such_item","count":1}}"#,
        )],
    );
    let mut state = fixture(&root);
    state.datapacks = Runtime::new(
        PackSet::load(
            &root,
            Selection {
                enabled: vec!["vanilla".into()],
                disabled: vec!["file/broken".into()],
            },
        )
        .unwrap(),
        root.clone(),
    );
    let recipes = state.registry.recipes.len();
    run(&mut state, "datapack enable file/broken");
    settle(&mut state);
    let shown = messages(&mut state);
    assert!(
        crate::command_debug::plain(&shown[0]).contains("Unknown recipe output"),
        "{shown:?}"
    );
    assert_eq!(state.registry.recipes.len(), recipes);
    assert_eq!(state.datapacks.selection().enabled, ["vanilla"]);
}

/// Run one console command on a live core and wait for its datapack feedback.
fn console(core: &crate::core::Core, line: &str, feedback_key: &str) {
    let mut ticks = core.subscribe();
    let (command, _completed) = crate::command_console::ConsoleCommand::new(line.into());
    assert!(core.submit(crate::boundary::PlayerInput::ConsoleCommand(command)));
    for _ in 0..400 {
        if matches!(
            ticks.blocking_recv(),
            Err(tokio::sync::broadcast::error::RecvError::Closed)
        ) {
            panic!("the tick thread stopped");
        }
        for (connection, queue) in core.try_drain_outbox() {
            if connection == crate::command_console::CONNECTION
                && queue
                    .iter()
                    .any(|change| format!("{change:?}").contains(feedback_key))
            {
                return;
            }
        }
    }
    panic!("no {feedback_key} after {line}");
}

#[test]
fn the_saved_pack_selection_decides_which_recipes_load_after_a_restart() {
    let dir =
        std::env::temp_dir().join(format!("mc-core-datapack-restart-{}", uuid::Uuid::new_v4()));
    pack(&dir.join("datapacks"), "p", "\"P\"", &GEM_RECIPES);
    let config = crate::config::CoreConfig::new(dir.clone(), 42);
    let has_gem =
        |core: &crate::core::Core| recipe_result(core.registry(), "test:new_gem").is_some();

    let core = crate::core::Core::open(config.clone()).unwrap();
    assert!(has_gem(&core), "a pack found on disk starts enabled");
    console(
        &core,
        "datapack disable file/p",
        "commands.datapack.modify.disable",
    );
    core.shutdown();

    let core = crate::core::Core::open(config.clone()).unwrap();
    assert!(
        !has_gem(&core),
        "disabled before the restart, still disabled after it"
    );
    assert_eq!(
        recipe_result(core.registry(), "minecraft:bone_meal"),
        recipe_result(
            &crate::registry::CoreRegistry::load(None).unwrap(),
            "minecraft:bone_meal"
        )
    );
    console(
        &core,
        "datapack enable file/p",
        "commands.datapack.modify.enable",
    );
    core.shutdown();

    let core = crate::core::Core::open(config).unwrap();
    assert!(
        has_gem(&core),
        "enabled before the restart, still enabled after it"
    );
    core.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

const WORLDGEN_PACK: [(&str, &str); 5] = [
    ("test/function/hello.mcfunction", "say hello"),
    ("test/worldgen/biome/odd.json", "{}"),
    ("minecraft/dimension/the_nether.json", "{}"),
    ("minecraft/dimension_type/overworld.json", "{}"),
    ("minecraft/tags/worldgen/biome/odd.json", r#"{"values":[]}"#),
];

#[test]
fn a_pack_with_worldgen_resources_loads_its_other_content_and_warns_once() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "wg", "\"WG\"", &WORLDGEN_PACK);
    let loaded = PackSet::load(&root, Selection::default()).unwrap();
    assert!(
        loaded.functions.contains_key("test:hello"),
        "the pack's other content loads"
    );
    assert_eq!(
        loaded.ignored["file/wg"],
        [
            "minecraft/dimension/the_nether.json",
            "minecraft/dimension_type/overworld.json",
            "minecraft/tags/worldgen/biome/odd.json",
            "test/worldgen/biome/odd.json",
        ],
        "every ignored resource path is named"
    );
    let warnings = loaded.warnings(None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(
        warnings[0].contains("file/wg")
            && warnings[0].contains("test/worldgen/biome/odd.json")
            && warnings[0].contains("ignored"),
        "{warnings:?}"
    );
    assert!(
        loaded.warnings(Some(&loaded)).is_empty(),
        "an unchanged pack is not warned about again"
    );

    // Enabling it from the console tells the operator once, then reloads stay quiet.
    let mut state = fixture(&root);
    state.datapacks = Runtime::new(
        PackSet::load(
            &root,
            Selection {
                enabled: vec!["vanilla".into()],
                disabled: vec!["file/wg".into()],
            },
        )
        .unwrap(),
        root.clone(),
    );
    messages(&mut state);
    run(&mut state, "datapack enable file/wg");
    settle(&mut state);
    let shown: Vec<String> = messages(&mut state)
        .iter()
        .map(crate::command_debug::plain)
        .collect();
    assert!(
        shown
            .iter()
            .any(|line| line.contains("commands.datapack.modify.enable")),
        "the enable still succeeds: {shown:?}"
    );
    assert_eq!(
        shown
            .iter()
            .filter(|line| line.contains("test/worldgen/biome/odd.json"))
            .count(),
        1,
        "{shown:?}"
    );
    assert!(state.datapacks.packs().functions.contains_key("test:hello"));
    run(&mut state, "reload");
    settle(&mut state);
    let shown: Vec<String> = messages(&mut state)
        .iter()
        .map(crate::command_debug::plain)
        .collect();
    assert!(
        !shown.iter().any(|line| line.contains("worldgen")),
        "a reload of the same pack does not warn again: {shown:?}"
    );
}

#[test]
fn a_selected_pack_gone_from_disk_is_dropped_with_a_warning() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "kept", "\"K\"", &[]);
    let loaded = PackSet::load(
        &root,
        Selection {
            enabled: vec!["vanilla".into(), "file/gone".into(), "file/kept".into()],
            disabled: Vec::new(),
        },
    )
    .unwrap();
    assert_eq!(loaded.selection.enabled, ["vanilla", "file/kept"]);
    assert_eq!(loaded.dropped, ["file/gone"]);
    let warnings = loaded.warnings(None);
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("file/gone") && warnings[0].contains("dropped"));
    assert!(loaded.warnings(Some(&loaded)).is_empty());
}

#[test]
fn a_pack_removed_while_the_server_is_down_opens_the_world_without_it() {
    let dir = std::env::temp_dir().join(format!("mc-core-datapack-gone-{}", uuid::Uuid::new_v4()));
    pack(&dir.join("datapacks"), "p", "\"P\"", &GEM_RECIPES);
    let config = crate::config::CoreConfig::new(dir.clone(), 42);
    let core = crate::core::Core::open(config.clone()).unwrap();
    assert!(recipe_result(core.registry(), "test:new_gem").is_some());
    core.shutdown();

    std::fs::remove_dir_all(dir.join("datapacks").join("p")).unwrap();
    let core = crate::core::Core::open(config.clone()).unwrap();
    assert!(
        recipe_result(core.registry(), "test:new_gem").is_none(),
        "the missing pack's content is gone, and the world still opens"
    );
    core.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn a_corrupt_saved_selection_fails_closed() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    let bad = |enabled: &[&str], disabled: &[&str]| Selection {
        enabled: enabled.iter().map(|id| (*id).to_owned()).collect(),
        disabled: disabled.iter().map(|id| (*id).to_owned()).collect(),
    };
    for selection in [
        bad(&[], &[]),
        bad(&["file/a"], &[]),
        bad(&["vanilla", "vanilla"], &[]),
        bad(&["vanilla", "file/a"], &["file/a"]),
        bad(&["vanilla", ""], &[]),
        bad(&["vanilla", "file/"], &[]),
        bad(&["vanilla", "file/.."], &[]),
        bad(&["vanilla", "file/a/b"], &[]),
        bad(&["vanilla", "file/a\\b"], &[]),
        bad(&["vanilla", "../escape"], &[]),
        bad(&["vanilla", "datapack/a"], &[]),
        bad(&["vanilla"], &["bogus"]),
    ] {
        let mut runtime = Runtime::new(PackSet::default(), root.clone());
        assert!(
            runtime.restore_selection(selection.clone()).is_err(),
            "{selection:?}"
        );
        assert_eq!(
            runtime.selection(),
            &Selection::default(),
            "a refused selection changes nothing: {selection:?}"
        );
        assert!(
            PackSet::load(&root, selection.clone()).is_err(),
            "{selection:?}"
        );
    }
    // An absent saved selection is the default one; a good one restores.
    let mut runtime = Runtime::new(PackSet::default(), root.clone());
    runtime
        .restore_selection(bad(&["vanilla", "file/a"], &["file/b"]))
        .unwrap();
    assert_eq!(runtime.selection().enabled, ["vanilla", "file/a"]);
    assert_eq!(
        Selection::default(),
        bad(&["vanilla"], &[]),
        "a manifest without the field opens with the default selection"
    );
}

const BAD_RECIPE: &str = r#"{"type": "minecraft:crafting_shapeless", "#;

fn base_registry() -> crate::registry::CoreRegistry {
    let (state, _, _) = crate::hostile_tests::setup(8.0);
    (*state.registry).clone()
}

#[test]
fn a_new_pack_that_cannot_load_is_skipped_at_start_and_the_error_names_pack_and_file() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "good", "\"Good\"", &GEM_RECIPES);
    pack(
        &root,
        "truncated",
        "\"T\"",
        &[("test/recipe/cut.json", BAD_RECIPE)],
    );
    pack(
        &root,
        "tagged",
        "\"T\"",
        &[("tagged/tags/item/gems.json", r#"{"values":[]}"#)],
    );
    let base = base_registry();
    // The same selection a world that never saw these packs has.
    let (packs, skipped) = PackSet::open(&root, Selection::default(), &base).unwrap();
    assert_eq!(packs.selection.enabled, ["vanilla", "file/good"]);
    assert_eq!(skipped.len(), 2, "{skipped:?}");
    let named = |pack: &str, file: &str| {
        skipped
            .iter()
            .any(|line| line.contains(pack) && line.contains(file))
    };
    assert!(
        named("file/truncated", "test/recipe/cut.json"),
        "{skipped:?}"
    );
    assert!(
        named("file/tagged", "tagged/tags/item/gems.json"),
        "{skipped:?}"
    );

    // A pack the saved selection already holds that no longer loads is not
    // skipped: the world was saved with it enabled.
    let saved = Selection {
        enabled: vec!["vanilla".into(), "file/truncated".into()],
        disabled: Vec::new(),
    };
    let error = PackSet::open(&root, saved, &base)
        .err()
        .expect("fails closed");
    assert!(
        error.contains("file/truncated") && error.contains("test/recipe/cut.json"),
        "{error}"
    );
}

#[test]
fn a_pack_the_live_reload_refused_does_not_stop_the_next_start() {
    let dir =
        std::env::temp_dir().join(format!("mc-core-datapack-refused-{}", uuid::Uuid::new_v4()));
    let packs = dir.join("datapacks");
    pack(&packs, "keep", "\"K\"", &GEM_RECIPES);
    let config = crate::config::CoreConfig::new(dir.clone(), 42);
    let core = crate::core::Core::open(config.clone()).unwrap();
    // The operator drops a broken pack in and enables it: refused, naming it.
    pack(
        &packs,
        "broken",
        "\"B\"",
        &[("test/recipe/cut.json", BAD_RECIPE)],
    );
    console(&core, "datapack enable file/broken", "file/broken");
    core.shutdown();

    let core = crate::core::Core::open(config).unwrap();
    assert!(
        recipe_result(core.registry(), "test:new_gem").is_some(),
        "the same selection as before the refusal"
    );
    core.shutdown();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn the_live_refusal_of_a_pack_names_the_pack_and_the_file() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(
        &root,
        "broken",
        "\"B\"",
        &[("test/recipe/cut.json", BAD_RECIPE)],
    );
    let mut state = fixture(&root);
    state.datapacks = Runtime::new(
        PackSet::load(
            &root,
            Selection {
                enabled: vec!["vanilla".into()],
                disabled: vec!["file/broken".into()],
            },
        )
        .unwrap(),
        root.clone(),
    );
    run(&mut state, "datapack enable file/broken");
    settle(&mut state);
    let shown = messages(&mut state);
    let text = crate::command_debug::plain(&shown[0]);
    assert!(
        text.contains("file/broken")
            && text.contains("test/recipe/cut.json")
            && text.contains("old packs and selection retained"),
        "{text}"
    );
}

fn filter_pack(root: &Path, name: &str, filter: &str, resources: &[(&str, &str)]) {
    pack(root, name, "\"F\"", resources);
    std::fs::write(
        root.join(name).join("pack.mcmeta"),
        format!(r#"{{"pack":{{"pack_format":101,"description":"F"}},"filter":{filter}}}"#),
    )
    .unwrap();
}

#[test]
fn a_pack_filter_hides_the_files_of_the_packs_below_it() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "gems", "\"Gems\"", &GEM_RECIPES);
    filter_pack(
        &root,
        "nobone",
        r#"{"block":[{"namespace":"minecraft","path":"recipe/bone_meal\\..*"},{"path":"loot_table/blocks/stone.json"}]}"#,
        &[("test/recipe/own.json", GEM_RECIPES[1].1)],
    );
    let base = base_registry();
    assert!(recipe_result(&base, "minecraft:bone_meal").is_some());
    let installed = |enabled: &[&str]| {
        let selection = Selection {
            enabled: enabled.iter().map(|id| (*id).to_owned()).collect(),
            disabled: Vec::new(),
        };
        let packs = PackSet::load_known(&root, selection).unwrap();
        let mut registry = base.clone();
        packs.install_content(&mut registry).unwrap();
        registry
    };

    // On top of the pack that replaces bone meal: both the pack's and the
    // builtin recipe are hidden; the filtering pack's own files stay.
    let registry = installed(&["vanilla", "file/gems", "file/nobone"]);
    assert_eq!(recipe_result(&registry, "minecraft:bone_meal"), None);
    assert_eq!(
        recipe_result(&registry, "test:new_gem").as_deref(),
        Some("minecraft:emerald"),
        "another namespace is not matched"
    );
    assert_eq!(
        recipe_result(&registry, "test:own").as_deref(),
        Some("minecraft:emerald")
    );
    let stone = mc_data::Identifier::parse("minecraft:stone").unwrap();
    assert!(
        registry.loot.block_loot(&stone).is_none() && registry.loot.block_drop(&stone).is_none(),
        "the filter's loot pattern hides every stone table below it"
    );
    // Below the pack that replaces it, the filter hides nothing of that pack.
    let registry = installed(&["vanilla", "file/nobone", "file/gems"]);
    assert_eq!(
        recipe_result(&registry, "minecraft:bone_meal").as_deref(),
        Some("minecraft:diamond")
    );
    assert_eq!(stone_drops(&registry), ["minecraft:diamond"]);
}

#[test]
fn a_malformed_pack_filter_is_refused_naming_the_pack() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    filter_pack(&root, "bad", r#"{"block":[{"path":"("}]}"#, &[]);
    let error = PackSet::load(&root, Selection::default())
        .err()
        .expect("refused");
    assert!(
        error.contains("file/bad") && error.contains("filter"),
        "{error}"
    );
}

#[test]
fn a_pack_command_while_another_is_still_compiling_says_so_instead_of_misreporting_the_selection() {
    let world = tempfile::tempdir().unwrap();
    let root = world.path().join("datapacks");
    pack(&root, "gems", "\"Gems\"", &GEM_RECIPES);
    let mut state = fixture(&root);
    state.datapacks = Runtime::new(
        PackSet::load(
            &root,
            Selection {
                enabled: vec!["vanilla".into()],
                disabled: vec!["file/gems".into()],
            },
        )
        .unwrap(),
        root.clone(),
    );
    run(&mut state, "datapack enable file/gems");
    run(&mut state, "datapack disable file/gems");
    settle(&mut state);
    let shown = messages(&mut state)
        .iter()
        .map(crate::command_debug::plain)
        .collect::<Vec<_>>();
    assert!(
        shown
            .iter()
            .any(|line| line.contains("already in progress")),
        "{shown:?}"
    );
    assert!(
        !shown.iter().any(|line| line.contains("not enabled")),
        "{shown:?}"
    );
    assert!(
        state
            .datapacks
            .selection()
            .enabled
            .contains(&"file/gems".to_owned())
    );
}
