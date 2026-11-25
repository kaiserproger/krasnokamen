use crate::boundary::{BlockPos, ConnectionId, DigAction, EntityId, Hand, PlayerInput};
use crate::farming_tests::{Y, block, put, setup};
use crate::tick::{TickState, run_tick};
use mc_data::{Identifier, ItemStack};

const POS: BlockPos = BlockPos {
    x: 1,
    y: Y + 1,
    z: 0,
};
fn give(state: &mut TickState, actor: EntityId, name: &str, count: i32) {
    let id = state
        .registry
        .items
        .id_of(&Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap();
    state.players.get_mut(&actor).unwrap().inventory.slots[0] = ItemStack::new(id, count);
}
fn use_on(state: &mut TickState, pos: BlockPos, face: u8, cursor: [f32; 3]) {
    run_tick(
        state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: pos,
            face,
            cursor,
            sequence: 1,
        }],
    );
}
fn install(state: &mut TickState, name: &str, props: &[(&str, &str)]) -> u32 {
    let id = block(state, name, props);
    put(state, POS.x, POS.y, POS.z, id);
    id
}
#[test]
fn interact_parity_axes_preserve_properties_and_wear() {
    let (mut state, actor) = setup();
    give(&mut state, actor, "iron_axe", 1);
    for (source, result, props) in [
        ("oak_log", "stripped_oak_log", &[("axis", "x")][..]),
        ("oxidized_copper", "weathered_copper", &[][..]),
        ("waxed_weathered_copper", "weathered_copper", &[][..]),
    ] {
        install(&mut state, source, props);
        use_on(&mut state, POS, 1, [0.5; 3]);
        assert_eq!(
            state.world.block_at(POS.x, POS.y, POS.z),
            Some(block(&state, result, props)),
            "{source}"
        );
    }
    assert_eq!(state.players[&actor].inventory.slots[0].damage, Some(3));
}
#[test]
fn interact_parity_sponge_absorbs_adjacent_water() {
    let (mut state, actor) = setup();
    let water = state.registry.states.water_levels[0];
    put(&mut state, 2, Y + 1, 0, water);
    give(&mut state, actor, "sponge", 1);
    use_on(&mut state, BlockPos { y: Y, ..POS }, 1, [0.5, 1.0, 0.5]);
    assert_eq!(
        state.world.block_at(1, Y + 1, 0),
        Some(block(&state, "wet_sponge", &[]))
    );
    assert_eq!(
        state.world.block_at(2, Y + 1, 0),
        Some(state.registry.states.air)
    );
    assert!(state.players[&actor].inventory.slots[0].is_empty());
}
#[test]
fn interact_parity_dragon_egg_use_teleports() {
    let (mut state, _) = setup();
    let egg = install(&mut state, "dragon_egg", &[]);
    use_on(&mut state, POS, 1, [0.5; 3]);
    assert_ne!(state.world.block_at(POS.x, POS.y, POS.z), Some(egg));
    // Falling is scheduled after teleport; custody is either a block or the
    // native falling entity, never a dropped egg or a duplicate.
    assert!(
        state.players.values().all(|player| player
            .inventory
            .slots
            .iter()
            .all(|stack| stack.is_empty()))
    );
}
#[test]
fn interact_parity_decorated_pot_stores_one_item() {
    let (mut state, actor) = setup();
    install(
        &mut state,
        "decorated_pot",
        &[
            ("facing", "north"),
            ("waterlogged", "false"),
            ("cracked", "false"),
        ],
    );
    give(&mut state, actor, "diamond", 3);
    use_on(&mut state, POS, 2, [0.5; 3]);
    assert_eq!(state.players[&actor].inventory.slots[0].count, 2);
    assert_eq!(
        state.block_entities[&(POS.x, POS.y, POS.z)].slots[0].count,
        1
    );
}
#[test]
fn interact_parity_eternal_fire_magma_and_end_bedrock() {
    let (mut state, _) = setup();
    for (dimension, support) in [
        (crate::dimensions::Dimension::Overworld, "netherrack"),
        (crate::dimensions::Dimension::Overworld, "magma_block"),
        (crate::dimensions::Dimension::End, "bedrock"),
    ] {
        state.dimension = dimension;
        let support = block(&state, support, &[]);
        put(&mut state, POS.x, Y, POS.z, support);
        install(
            &mut state,
            "fire",
            &[
                ("age", "15"),
                ("east", "false"),
                ("north", "false"),
                ("south", "false"),
                ("up", "false"),
                ("west", "false"),
            ],
        );
        crate::fire::scheduled_tick(&mut state, POS, &mut Vec::new(), &mut Vec::new());
        assert_ne!(
            state.world.block_at(POS.x, POS.y, POS.z),
            Some(state.registry.states.air)
        );
    }
}
#[test]
fn interact_parity_painting_places_on_wall() {
    let (mut state, actor) = setup();
    install(&mut state, "stone", &[]);
    give(&mut state, actor, "painting", 1);
    use_on(&mut state, POS, 2, [0.5; 3]);
    assert!(state.players[&actor].inventory.slots[0].is_empty());
    let painting = state.registry.entity_type_id("minecraft:painting").unwrap();
    assert_eq!(state.entities.mob_slots(painting).count(), 1);
    let air = state.registry.states.air;
    put(&mut state, POS.x, POS.y, POS.z, air);
    let mut drops = Vec::new();
    crate::painting::blocks_changed(&mut state, &[(POS, air)], &mut drops);
    assert_eq!(state.entities.mob_slots(painting).count(), 0);
    assert_eq!(drops.len(), 1);
    crate::painting::blocks_changed(&mut state, &[(POS, air)], &mut drops);
    assert_eq!(drops.len(), 1);
}

/// Drive one player command through the shared tick boundary the way a live
/// multi-dimension server does: queued during the tick while
/// dimension_globals is active, executed with command_context.shared, and
/// settled by dimensions::commands::commit instead of the tick block phase.
fn shared_command(
    state: &mut TickState,
    actor: EntityId,
    text: &str,
) -> crate::boundary::ChangeSet {
    state.dimension_globals.active = true;
    let mut changes = crate::boundary::ChangeSet::default();
    let context = crate::command_context::Context::source(state, ConnectionId(1), actor, 4);
    crate::command_context::submit(state, text, context, &mut changes);
    let job = state.command_context.jobs.pop().expect("command queued");
    state.command_context.shared = true;
    crate::command_context::run(state, &job.text, job.context, &mut changes);
    state.command_context.shared = false;
    state.dimension_globals.active = false;
    changes
}

#[test]
fn painting_drops_when_shared_command_commit_removes_support() {
    let (mut state, actor) = setup();
    install(&mut state, "stone", &[]);
    give(&mut state, actor, "painting", 1);
    use_on(&mut state, POS, 2, [0.5; 3]);
    let painting = state.registry.entity_type_id("minecraft:painting").unwrap();
    let id = state
        .entities
        .mob_slots(painting)
        .map(|slot| state.entities.body(slot).id)
        .next()
        .expect("painting placed");
    let changes = shared_command(
        &mut state,
        actor,
        &format!("setblock {} {} {} minecraft:air", POS.x, POS.y, POS.z),
    );
    assert!(
        state.entities.slot_of(id).is_none(),
        "painting remains after the shared command commit removed its support"
    );
    assert!(
        state.hostile.removed.contains(&id),
        "removal staged for the next tick's publish"
    );
    assert_eq!(
        changes.entities_spawned.len(),
        1,
        "painting item drop missing from the commit changes"
    );
}

#[test]
fn item_frame_drops_when_shared_command_commit_removes_support() {
    let (mut state, actor) = setup();
    install(&mut state, "stone", &[]);
    give(&mut state, actor, "item_frame", 1);
    let (output, _, _) = run_tick(
        &mut state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: POS,
            face: 2,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    );
    let id = *output
        .changes
        .entities_spawned
        .first()
        .expect("frame placed");
    let changes = shared_command(
        &mut state,
        actor,
        &format!("setblock {} {} {} minecraft:air", POS.x, POS.y, POS.z),
    );
    assert!(
        state.entities.slot_of(id).is_none(),
        "item frame remains after the shared command commit removed its support"
    );
    assert!(
        state.hostile.removed.contains(&id),
        "frame removal staged for the next tick's publish"
    );
    assert_eq!(
        changes.entities_spawned.len(),
        1,
        "frame item drop missing from the commit changes"
    );
}

#[test]
fn staged_item_components_commit_through_shared_command_boundary() {
    let (mut state, actor) = setup();
    let pos = BlockPos {
        x: 3,
        y: Y + 1,
        z: 0,
    };
    let pot = state
        .registry
        .blocks
        .block(&Identifier::parse("minecraft:decorated_pot").unwrap())
        .expect("decorated pot block")
        .default
        .0;
    // An accepted placement stages the block-entity image carrying the item's
    // components; the shared commit must install it like the tick does.
    let sherds = mc_nbt::Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::STRING,
        elements: vec![mc_nbt::Tag::String("minecraft:angler_pottery_sherd".into()); 4],
    });
    let image = mc_nbt::Tag::Compound(vec![
        (
            "id".into(),
            mc_nbt::Tag::String("minecraft:decorated_pot".into()),
        ),
        ("x".into(), mc_nbt::Tag::Int(pos.x)),
        ("y".into(), mc_nbt::Tag::Int(pos.y)),
        ("z".into(), mc_nbt::Tag::Int(pos.z)),
        ("sherds".into(), sherds),
    ]);
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &image).expect("staged image encodes");
    state
        .merge_item_component_placements
        .push((pos, pot, bytes));
    shared_command(
        &mut state,
        actor,
        &format!(
            "setblock {} {} {} minecraft:decorated_pot",
            pos.x, pos.y, pos.z
        ),
    );
    let chunk = state
        .world
        .chunk(crate::boundary::ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        })
        .expect("chunk materialized");
    let bytes = &chunk.block_entities[&pos];
    let mc_nbt::Tag::Compound(fields) = mc_nbt::read_network(&mut bytes.as_slice()).unwrap() else {
        panic!("block entity image decodes to a compound")
    };
    assert!(
        fields.iter().any(|(key, _)| key == "sherds"),
        "staged item components were not committed at the shared boundary"
    );
}

#[test]
fn monster_spawner_index_updates_through_shared_command_commit() {
    let (mut state, _actor) = setup();
    let pos = BlockPos {
        x: 5,
        y: Y + 1,
        z: 0,
    };
    // World hydration left spawner NBT in the chunk alongside the block.
    let spawner = block(&state, "spawner", &[]);
    put(&mut state, pos.x, pos.y, pos.z, spawner);
    let fields = vec![
        (
            "id".into(),
            mc_nbt::Tag::String("minecraft:mob_spawner".into()),
        ),
        ("x".into(), mc_nbt::Tag::Int(pos.x)),
        ("y".into(), mc_nbt::Tag::Int(pos.y)),
        ("z".into(), mc_nbt::Tag::Int(pos.z)),
        ("Delay".into(), mc_nbt::Tag::Short(0)),
        (
            "SpawnData".into(),
            mc_nbt::Tag::Compound(vec![(
                "entity".into(),
                mc_nbt::Tag::Compound(vec![(
                    "id".into(),
                    mc_nbt::Tag::String("minecraft:zombie".into()),
                )]),
            )]),
        ),
    ];
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &mc_nbt::Tag::Compound(fields)).expect("spawner encodes");
    state.world.set_opaque_block_entity(pos, Some(bytes));
    // A command edit at the shared boundary stages the write and settles
    // through dimensions::commands::commit; the spawner index must follow.
    state.merge_block_writes.push((pos, spawner));
    let mut changes = crate::boundary::ChangeSet::default();
    crate::dimensions::commands::commit(&mut state, &mut changes);
    assert!(
        state.monster_spawners.contains(pos),
        "spawner was not admitted to the runtime index at the shared boundary"
    );
}

#[test]
fn interact_parity_brushing_completes_once() {
    for (source, result) in [("suspicious_sand", "sand"), ("suspicious_gravel", "gravel")] {
        let (mut state, actor) = setup();
        let table_id = Identifier::parse("minecraft:archaeology/desert_pyramid").unwrap();
        let expected = state
            .registry
            .chest_loot
            .get(&table_id)
            .unwrap()
            .roll(
                &mut mc_data::loot::chest_26_1_2::ChestRng::new(42),
                &state.registry.items,
                0.0,
            )
            .unwrap()
            .into_iter()
            .next()
            .expect("seed yields an archaeology item");
        install(&mut state, source, &[("dusted", "0")]);
        let nbt = mc_nbt::Tag::Compound(vec![
            (
                "id".into(),
                mc_nbt::Tag::String("minecraft:brushable_block".into()),
            ),
            (
                "LootTable".into(),
                mc_nbt::Tag::String("minecraft:archaeology/desert_pyramid".into()),
            ),
            ("LootTableSeed".into(), mc_nbt::Tag::Long(42)),
            (
                "private_marker".into(),
                mc_nbt::Tag::String("not client data".into()),
            ),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &nbt).unwrap();
        state.world.set_opaque_block_entity(POS, Some(bytes));
        give(&mut state, actor, "brush", 1);
        let slot = state.entities.slot_of(actor).unwrap();
        let [x, y, z] = state.entities.pos(slot);
        let dx = f64::from(POS.x) + 0.5 - x;
        let dz = f64::from(POS.z) + 0.5 - z;
        let dy = f64::from(POS.y) + 0.5 - y - 1.62;
        #[allow(clippy::disallowed_methods)]
        let (yaw, pitch) = {
            let yaw = (-dx).atan2(dz).to_degrees() as f32;
            let pitch = (-dy).atan2(dx.hypot(dz)).to_degrees() as f32;
            (yaw, pitch)
        };
        state.entities.set_rot(slot, yaw, pitch);
        use_on(&mut state, POS, 1, [0.5; 3]);
        let mut dusted = [false; 4];
        dusted[0] = true;
        let mut saw_reveal = false;
        for _ in 0..200 {
            let (_, drains, _) = run_tick(&mut state, &[]);
            for (_, changes) in drains {
                for change in changes {
                    if let crate::boundary::PlayerChange::BlockEntityData {
                        position,
                        block_entity_type,
                        nbt,
                        ..
                    } = change
                        && position == POS
                        && block_entity_type
                            == mc_data::block_entity_kinds::BlockEntityKind::BrushableBlock
                                .protocol_id()
                    {
                        let mc_nbt::Tag::Compound(fields) = nbt else {
                            panic!("brush update must be a compound")
                        };
                        assert!(
                            fields.iter().any(|(name, value)| name == "item"
                                && matches!(value, mc_nbt::Tag::Compound(_))),
                            "revealed item is rendered"
                        );
                        assert!(
                            fields.iter().any(|(name, _)| name == "hit_direction"),
                            "render orientation is present"
                        );
                        assert!(
                            fields
                                .iter()
                                .all(|(name, _)| matches!(name.as_str(), "item" | "hit_direction")),
                            "private saved fields must not reach viewers"
                        );
                        saw_reveal = true;
                    }
                }
            }
            for (n, seen) in dusted.iter_mut().enumerate().skip(1) {
                if state.world.block_at(POS.x, POS.y, POS.z)
                    == Some(block(&state, source, &[("dusted", &n.to_string())]))
                {
                    *seen = true;
                }
            }
            if state.world.block_at(POS.x, POS.y, POS.z) == Some(block(&state, result, &[])) {
                break;
            }
        }
        assert!(saw_reveal, "brushing must send a visible item update");
        assert!(
            dusted.into_iter().all(|seen| seen),
            "all dust stages appear"
        );
        let expected_id = expected.item_id;
        assert!(state.entities.item_slots().any(|slot| {
            state
                .entities
                .row(slot)
                .item
                .as_ref()
                .is_some_and(|item| item.item_id == expected_id)
        }));
        assert_eq!(
            state.world.block_at(POS.x, POS.y, POS.z),
            Some(block(&state, result, &[]))
        );
        assert_eq!(state.players[&actor].inventory.slots[0].damage, Some(1));
    }
}

#[test]
fn interact_parity_spawn_protection_with_operator() {
    let (mut state, _) = setup();
    run_tick(
        &mut state,
        &[PlayerInput::SpawnProtectionOperators {
            operators: vec![uuid::Uuid::new_v4()],
        }],
    );
    let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
    // PLACE-001: a non-operator inside the radius is protected, so the dig
    // path refuses and the block survives.
    assert!(
        crate::spawn_protection::protected(&state, actor, POS),
        "a non-operator inside the spawn radius is protected"
    );
    let stone = state.registry.states.stone;
    put(&mut state, POS.x, POS.y, POS.z, stone);
    let dig = PlayerInput::Dig {
        connection: ConnectionId(1),
        position: POS,
        action: DigAction::Finish,
        sequence: 1,
    };
    assert!(crate::tick::client_build_refused(&state, actor, &dig));
    run_tick(&mut state, &[dig]);
    assert_eq!(
        state.world.block_at(POS.x, POS.y, POS.z),
        Some(stone),
        "spawn protection refuses the break"
    );
    assert!(crate::tick::client_build_refused(
        &state,
        actor,
        &PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: POS,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1
        }
    ));
    assert!(!crate::spawn_protection::protected(
        &state,
        actor,
        BlockPos { x: 17, ..POS }
    ));
    state.dimension = crate::dimensions::Dimension::Nether;
    assert!(!crate::spawn_protection::protected(&state, actor, POS));
    state.dimension = crate::dimensions::Dimension::Overworld;
    let slot = state.entities.slot_of(actor).unwrap();
    state.spawn_operators.insert(state.entities.row(slot).uuid);
    assert!(!crate::spawn_protection::protected(&state, actor, POS));
    state.spawn_operators.clear();
    assert!(!crate::spawn_protection::protected(&state, actor, POS));
}

#[test]
fn interact_parity_sponge_capacity_and_radius() {
    let (mut state, _) = setup();
    let water = state.registry.states.water_levels[0];
    for x in -4..=4 {
        for y in Y + 1..=Y + 9 {
            for z in -4..=4 {
                put(&mut state, x, y, z, water);
            }
        }
    }
    let origin = BlockPos {
        x: 0,
        y: Y + 5,
        z: 0,
    };
    let dry = block(&state, "sponge", &[]);
    put(&mut state, origin.x, origin.y, origin.z, dry);
    let mut writes = vec![(origin, dry)];
    crate::sponge::after_blocks(&mut state, &mut writes, &mut Vec::new());
    assert_eq!(
        writes
            .iter()
            .filter(|(_, id)| *id == state.registry.states.air)
            .count(),
        64
    );
    assert_eq!(
        state.world.block_at(origin.x, origin.y, origin.z),
        Some(block(&state, "wet_sponge", &[]))
    );

    let (mut state, _) = setup();
    for x in 2..=8 {
        put(&mut state, x, POS.y, 0, water);
    }
    install(&mut state, "sponge", &[]);
    let mut writes = vec![(POS, dry)];
    crate::sponge::after_blocks(&mut state, &mut writes, &mut Vec::new());
    assert_eq!(
        state.world.block_at(7, POS.y, 0),
        Some(state.registry.states.air)
    );
    assert_eq!(state.world.block_at(8, POS.y, 0), Some(water));
}

#[test]
fn interact_parity_dragon_egg_conserves_block_before_falling() {
    let (mut state, actor) = setup();
    let egg = install(&mut state, "dragon_egg", &[]);
    assert!(crate::vanilla_interact::dragon_egg(&mut state, actor, POS));
    assert_ne!(state.world.block_at(POS.x, POS.y, POS.z), Some(egg));
    let count = (-14..=16)
        .flat_map(|x| (Y - 6..=Y + 8).flat_map(move |y| (-15..=15).map(move |z| (x, y, z))))
        .filter(|&(x, y, z)| state.world.block_at(x, y, z) == Some(egg))
        .count();
    assert_eq!(count, 1);
}

#[test]
fn interact_parity_dragon_egg_requests_a_cold_target_through_the_demand_path() {
    let (mut state, actor) = setup();
    // Away from the join chunks, so only the egg's own column is resident.
    let pos = BlockPos {
        x: 64,
        y: Y + 1,
        z: 64,
    };
    state.world.ensure_blocking(
        &[(
            crate::boundary::ChunkPos {
                x: pos.x.div_euclid(16),
                z: pos.z.div_euclid(16),
            },
            0,
        )]
        .into_iter()
        .collect(),
    );
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(
        slot,
        [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 2.5,
        ],
    );
    state.entities.set_ground(slot, true);
    let egg = block(&state, "dragon_egg", &[]);
    put(&mut state, pos.x, pos.y, pos.z, egg);
    let stone = block(&state, "stone", &[]);
    // Resident cells inside the sample box are solid; the cold chunks are left
    // exactly as the world holds them (not resident), so the target a real server
    // would load is unknown here — which is the point of the deferred record.
    for dx in -15..=15 {
        for dz in -15..=15 {
            let x = pos.x + dx;
            let z = pos.z + dz;
            if !state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)) {
                continue;
            }
            for dy in -7..=7 {
                let y = pos.y + dy;
                if (x, y, z) != (pos.x, pos.y, pos.z) {
                    state.world.set_block(x, y, z, stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    let own_chunk = crate::boundary::ChunkPos {
        x: pos.x.div_euclid(16),
        z: pos.z.div_euclid(16),
    };
    // Through the real tick: the click records the request and the same tick's
    // chunk phase demands the target chunk at the highest priority.
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: pos,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    );
    // The player's own chunk is the only priority-0 demand a pre-image state ever has: the
    // egg adds a second one, the chunk of the sample it could not judge.
    let after = state.desired_chunks();
    let requested: Vec<_> = after
        .iter()
        .filter(|(chunk, priority)| **priority == 0 && **chunk != own_chunk)
        .map(|(chunk, _)| *chunk)
        .collect();
    assert_eq!(
        requested.len(),
        1,
        "exactly one new highest-priority demand is the egg's sampled target chunk: {requested:?}"
    );
    assert_eq!(
        state.world.block_at(pos.x, pos.y, pos.z),
        Some(egg),
        "the egg waits for its target chunk instead of being skipped or moved into it cold"
    );
    // The admission the request asked for: the next tick lands the egg inside the
    // sampled box and empties the origin.
    state
        .world
        .ensure_blocking(&[(requested[0], 0)].into_iter().collect());
    crate::tick::run_tick(&mut state, &[]);
    let air_state = state.registry.states.air;
    assert_eq!(
        state.world.block_at(pos.x, pos.y, pos.z),
        Some(air_state),
        "the origin is emptied by the tick that admits the chunk"
    );
    let mut custody = 0_usize;
    for dx in -15..=15 {
        for dz in -15..=15 {
            for dy in -7..=7 {
                if state.world.block_at(pos.x + dx, pos.y + dy, pos.z + dz) == Some(egg) {
                    custody += 1;
                }
            }
        }
    }
    let falling = state
        .entities
        .slots()
        .filter(|slot| {
            state
                .entities
                .row(*slot)
                .falling
                .is_some_and(|state| state.block_state == egg)
        })
        .count();
    assert_eq!(
        custody + falling,
        1,
        "the egg moved to its target chunk (blocks {custody}, falling {falling})"
    );
}

#[test]
fn interact_parity_dragon_egg_drops_an_expired_request() {
    let (mut state, actor) = setup();
    let pos = BlockPos {
        x: 64,
        y: Y + 1,
        z: 64,
    };
    state.world.ensure_blocking(
        &[(
            crate::boundary::ChunkPos {
                x: pos.x.div_euclid(16),
                z: pos.z.div_euclid(16),
            },
            0,
        )]
        .into_iter()
        .collect(),
    );
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(
        slot,
        [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 2.5,
        ],
    );
    state.entities.set_ground(slot, true);
    let egg = block(&state, "dragon_egg", &[]);
    put(&mut state, pos.x, pos.y, pos.z, egg);
    let stone = block(&state, "stone", &[]);
    // Resident cells are solid, so the only sample the click can keep is a cold one.
    for dx in -15..=15 {
        for dz in -15..=15 {
            let x = pos.x + dx;
            let z = pos.z + dz;
            if !state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)) {
                continue;
            }
            for dy in -7..=7 {
                let y = pos.y + dy;
                if (x, y, z) != (pos.x, pos.y, pos.z) {
                    state.world.set_block(x, y, z, stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    crate::tick::run_tick(
        &mut state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: pos,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    );
    let own_chunk = crate::boundary::ChunkPos {
        x: pos.x.div_euclid(16),
        z: pos.z.div_euclid(16),
    };
    let egg_demand = |state: &TickState| {
        state
            .desired_chunks()
            .into_iter()
            .any(|(chunk, priority)| priority == 0 && chunk != own_chunk)
    };
    assert!(egg_demand(&state), "the click demanded the sampled chunk");
    // The chunk never arrives (no loader worker completes it here): past the budget the
    // record is dropped, the demand goes with it and the egg stays where it was.
    // One tick past the documented 40-tick budget (the constant is not part of the public
    // surface a reversed tree still exports).
    for _ in 0..41 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert!(
        !egg_demand(&state),
        "an expired request no longer demands its chunk"
    );
    assert_eq!(state.world.block_at(pos.x, pos.y, pos.z), Some(egg));
}

#[test]
fn interact_parity_dragon_egg_never_teleports_into_a_cold_overlay() {
    let (mut state, actor) = setup();
    // Away from the join chunks, so only the egg's own column is resident.
    let pos = BlockPos {
        x: 64,
        y: Y + 1,
        z: 64,
    };
    state.world.ensure_blocking(
        &[(
            crate::boundary::ChunkPos {
                x: pos.x.div_euclid(16),
                z: pos.z.div_euclid(16),
            },
            0,
        )]
        .into_iter()
        .collect(),
    );
    let slot = state.entities.slot_of(actor).unwrap();
    state.entities.set_pos(
        slot,
        [
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 2.5,
        ],
    );
    state.entities.set_ground(slot, true);
    let egg = block(&state, "dragon_egg", &[]);
    put(&mut state, pos.x, pos.y, pos.z, egg);
    let stone = block(&state, "stone", &[]);
    let air = state.registry.states.air;
    // The vanilla target search samples a +-15 x +-7 x +-15 box. Make every
    // resident cell in it solid and leave air only in non-resident chunks, so
    // the only accepted target a search can find is a cold overlay cell.
    let mut cold_air = 0_usize;
    for dx in -15..=15 {
        for dz in -15..=15 {
            let x = pos.x + dx;
            let z = pos.z + dz;
            let resident = state.world.has_chunk(x.div_euclid(16), z.div_euclid(16));
            for dy in -7..=7 {
                let y = pos.y + dy;
                if (x, y, z) == (pos.x, pos.y, pos.z) {
                    continue;
                }
                if resident {
                    state.world.set_block(x, y, z, stone);
                } else {
                    state.world.set_block(x, y, z, air);
                    cold_air += 1;
                }
            }
        }
    }
    assert!(cold_air > 0, "the fixture needs non-resident cells");
    state.world.fold_overlay();
    assert_eq!(state.world.block_at(pos.x, pos.y, pos.z), Some(egg));
    run_tick(
        &mut state,
        &[PlayerInput::UseItemOn {
            connection: ConnectionId(1),
            hand: Hand::Main,
            position: pos,
            face: 1,
            cursor: [0.5; 3],
            sequence: 1,
        }],
    );
    assert_eq!(
        state.world.block_at(pos.x, pos.y, pos.z),
        Some(egg),
        "the egg must not move into a non-resident chunk's cold overlay"
    );
    assert!(
        state.merge_block_writes.is_empty(),
        "no block write is staged for a cold teleport target"
    );
}
