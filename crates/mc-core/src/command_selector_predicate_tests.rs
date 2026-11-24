use super::*;
use serde_json::{Value, json};

fn install(state: &mut TickState, predicates: &[(&str, Value)]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    let pack = root.path().join("selectors");
    let directory = pack.join("data/test/predicate");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(
        pack.join("pack.mcmeta"),
        r#"{"pack":{"pack_format":101,"description":"Selector predicates"}}"#,
    )
    .unwrap();
    for (name, predicate) in predicates {
        std::fs::write(
            directory.join(format!("{name}.json")),
            predicate.to_string(),
        )
        .unwrap();
    }
    let packs = crate::datapacks::PackSet::load(
        root.path(),
        crate::datapacks::Selection {
            enabled: vec!["vanilla".into(), "file/selectors".into()],
            disabled: Vec::new(),
        },
    )
    .unwrap();
    state
        .datapacks
        .install(packs, root.path().to_owned(), state.registry.clone());
    root
}

fn select(state: &TickState, executor: EntityId, input: &str) -> Vec<EntityId> {
    resolve_target(state, executor, &crate::commands::target_of(input).unwrap()).unwrap()
}

#[test]
fn loaded_predicates_bind_selected_entity_and_origin_before_limit() {
    let (mut state, alice, bob) = fixture();
    let _root = install(
        &mut state,
        &[
            (
                "east",
                json!({"condition":"minecraft:location_check","predicate":{"position":{"x":{"min":5}}}}),
            ),
            (
                "player",
                json!({"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:player","distance":{"absolute":0}}}),
            ),
        ],
    );
    let cow_type = state.registry.cow_type_id;
    state.spawn_mob(cow_type, [6.0, 80.0, 0.0], 10.0);
    assert_eq!(
        select(
            &state,
            alice,
            "@e[predicate=test:east,predicate=test:player,sort=nearest,limit=1]"
        ),
        vec![bob]
    );
    assert_eq!(select(&state, bob, "@a[predicate=!test:east]"), vec![alice]);
    // The command source and explicit selector origin are deliberately west;
    // the loot ORIGIN still belongs to each selected entity.
    assert_eq!(
        select(
            &state,
            alice,
            "@a[x=-100,predicate=test:east,predicate=test:player]"
        ),
        vec![bob]
    );
    assert!(
        select(
            &state,
            alice,
            "@a[predicate=test:east,predicate=!test:east]"
        )
        .is_empty()
    );
}

#[test]
fn loaded_predicates_compose_boolean_conditions_without_losing_identity() {
    let (mut state, alice, bob) = fixture();
    let _root = install(
        &mut state,
        &[(
            "east_player",
            json!([
                {"condition":"minecraft:any_of","terms":[
                    {"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:player"}},
                    {"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:cow"}}
                ]},
                {"condition":"minecraft:all_of","terms":[
                    {"condition":"minecraft:location_check","predicate":{"position":{"x":{"min":5}}}},
                    {"condition":"minecraft:inverted","term":{"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:cow"}}}
                ]}
            ]),
        )],
    );
    let cow_type = state.registry.cow_type_id;
    let cow = state.spawn_mob(cow_type, [6.0, 80.0, 0.0], 10.0);
    assert_eq!(
        select(&state, alice, "@e[predicate=test:east_player]"),
        vec![bob]
    );
    let mut inverse = select(&state, alice, "@e[predicate=!test:east_player]");
    inverse.sort();
    let mut expected = vec![alice, cow];
    expected.sort();
    assert_eq!(inverse, expected);
}

#[test]
fn missing_and_unsupported_predicates_fail_without_mutating_targets() {
    let (mut state, alice, bob) = fixture();
    let root = install(
        &mut state,
        &[(
            "player",
            json!({"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:player"}}),
        )],
    );
    // Vanilla parses the `predicate=` option without touching a registry
    // (`EntitySelectorOptions.lambda$bootStrap$55`: `Identifier.read`,
    // `ResourceKey.create`) and only at evaluation looks the key up
    // (`lambda$bootStrap$56` offsets 32-60: `lookup().get(key)` then
    // `Optional.isEmpty -> return false`) *before* the invert `ixor`
    // (offsets 128-145). A predicate no datapack defines therefore selects
    // nobody, and `!` does not rescue it.
    for input in [
        "@a[predicate=test:missing]",
        "@a[predicate=!test:missing]",
        "@e[type=minecraft:cow,predicate=test:missing]",
    ] {
        let target = crate::commands::target_of(input).unwrap();
        assert!(
            resolve_target(&state, alice, &target).unwrap().is_empty(),
            "{input} selects nobody"
        );
    }
    command(
        &mut state,
        alice,
        "kill @a[predicate=test:player,predicate=!test:missing]",
    );
    assert!(!chat_contains(&state, "Unknown predicate"));
    for entity in [alice, bob] {
        assert!(
            state
                .entities
                .body(state.entities.slot_of(entity).unwrap())
                .alive
        );
    }
    // `/execute if predicate <id>` is the argument form that does resolve at
    // parse time in vanilla (`ResourceOrIdArgument.lootPredicate`,
    // `argument.resource_or_id.no_such_element`); this tree has no such
    // condition at all (`condition()` in command_context.rs covers entity,
    // block, score and blocks), so it can never silently match here either.
    for predicate in [
        json!({"condition":"minecraft:any_of","terms":[{"condition":"minecraft:entity_properties","entity":"this","predicate":{}},{"condition":"test:unknown"}]}),
        json!({"condition":"minecraft:inverted","term":{"condition":"test:unknown"}}),
    ] {
        std::fs::write(
            root.path()
                .join("selectors/data/test/predicate/invalid.json"),
            predicate.to_string(),
        )
        .unwrap();
        let result =
            crate::datapacks::PackSet::load(root.path(), state.datapacks.selection().clone());
        assert!(
            result.is_err(),
            "unsupported conditions must not publish as false predicates"
        );
        assert_eq!(
            select(&state, alice, "@a[name=Bob,predicate=test:player]"),
            vec![bob]
        );
    }
}

#[test]
fn global_selector_uses_root_catalog_with_each_targets_native_dimension() {
    let (mut state, alice, _) = fixture();
    let _root = install(
        &mut state,
        &[(
            "nether",
            json!({"condition":"minecraft:location_check","predicate":{"dimension":"minecraft:the_nether","position":{"y":{"min":140}}}}),
        )],
    );
    let registry = Arc::clone(&state.registry);
    let generator: Arc<dyn mc_world::ChunkGenerator> =
        Arc::new(mc_worldgen::NetherGenerator::try_new(42, Arc::clone(&registry.blocks)).unwrap());
    let world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [8, 150, 8],
        crate::dimensions::Dimension::Nether.geometry(),
    );
    let mut nether = TickState::new(registry, world, 0, 4096);
    nether.entities.set_next_wire_id(1000);
    nether.dimension = crate::dimensions::Dimension::Nether;
    let pig_type = state.registry.entity_type_id("minecraft:pig").unwrap();
    state.spawn_mob(pig_type, [2.0, 150.0, 0.0], 10.0);
    let remote = nether.spawn_mob(pig_type, [8.0, 150.0, 8.0], 10.0);
    state
        .other_dimensions
        .insert(crate::dimensions::Dimension::Nether, Box::new(nether));
    let request = crate::dimensions::travel::Request {
        connection: ConnectionId(1),
        executor: alice,
        dimension: crate::dimensions::Dimension::Overworld,
        command: parse("kill @e[type=minecraft:pig,predicate=test:nether]", true).unwrap(),
    };
    let target = crate::dimensions::commands::target(&request.command).unwrap();
    assert_eq!(
        crate::dimensions::commands::resolve(&state, &request, target, [0.0, 80.0, 0.0]).unwrap(),
        vec![remote]
    );
}

/// One `/execute` chain through the production orchestrator (`native` owns
/// neither `/execute` nor its continuations), returning its success count.
fn chain(state: &mut TickState, executor: EntityId, input: &str) -> i32 {
    state.command_context.current = None;
    let context = crate::command_context::Context::source(
        state,
        crate::command_console::CONNECTION,
        executor,
        2,
    );
    crate::command_context::run(
        state,
        input,
        context,
        &mut crate::boundary::ChangeSet::default(),
    )
}

#[test]
fn execute_if_predicate_binds_the_datapack_predicate_to_the_command_origin() {
    let (mut state, alice, _bob) = fixture();
    let _root = install(
        &mut state,
        &[
            (
                "east",
                json!({"condition":"minecraft:location_check","predicate":{"position":{"x":{"min":5}}}}),
            ),
            (
                "player",
                json!({"condition":"minecraft:entity_properties","entity":"this","predicate":{"type":"minecraft:player"}}),
            ),
        ],
    );
    // alice's body is at x = 0: the predicate matches the executor's own location.
    let start = state.day_time;
    assert_eq!(
        chain(
            &mut state,
            alice,
            "execute if predicate test:player run time add 1"
        ),
        1
    );
    assert_eq!(state.day_time, start + 1);
    // `positioned` moves the loot ORIGIN, not the body: vanilla binds
    // `LootContextParams.ORIGIN` to the command source position.
    assert_eq!(
        chain(
            &mut state,
            alice,
            "execute positioned -10 80 0 if predicate test:east run time add 10"
        ),
        0
    );
    assert_eq!(
        chain(
            &mut state,
            alice,
            "execute positioned 10 80 0 if predicate test:east run time add 2"
        ),
        1
    );
    assert_eq!(state.day_time, start + 3);
    // `unless` inverts the same condition.
    assert_eq!(
        chain(
            &mut state,
            alice,
            "execute unless predicate test:east run time add 4"
        ),
        1
    );
    assert_eq!(state.day_time, start + 7);
    // Vanilla resolves the id while parsing, so an unknown id fails the command
    // with `argument.resource_or_id.no_such_element` instead of selecting nothing.
    assert_eq!(
        chain(
            &mut state,
            alice,
            "execute if predicate test:missing run time add 100"
        ),
        0
    );
    assert_eq!(state.day_time, start + 7);
    let feedback = state
        .outbox
        .get(&crate::command_console::CONNECTION)
        .cloned()
        .unwrap_or_default();
    assert!(
        feedback.iter().any(|change| matches!(
            change,
            crate::boundary::PlayerChange::StyledChat { content }
                if *content
                    == crate::command_feedback::translated(
                        "argument.resource_or_id.no_such_element",
                        Vec::new(),
                    )
        )),
        "{feedback:?}"
    );
}
