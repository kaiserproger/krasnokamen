use crate::boundary::BlockPos;

#[test]
fn common_attachment_support_removal_matrix() {
    use crate::crafter::tests::{block, fixture};
    use mc_nbt::Tag;

    let (mut state, _) = fixture(None);
    let vanilla = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(
        crate::registry::CoreRegistry::load(Some(&vanilla)).expect("full vanilla block loot"),
    );
    let pos = BlockPos { x: 3, y: 201, z: 3 };
    type AttachmentCase<'a> = (
        &'a str,
        &'a [(&'a str, &'a str)],
        [i32; 3],
        &'a str,
        Option<&'a str>,
    );
    let cases: &[AttachmentCase<'_>] = &[
        ("oak_sign", &[], [0, -1, 0], "oak_sign", Some("sign")),
        (
            "oak_wall_sign",
            &[("facing", "east")],
            [-1, 0, 0],
            "oak_sign",
            Some("sign"),
        ),
        (
            "oak_hanging_sign",
            &[],
            [0, 1, 0],
            "oak_hanging_sign",
            Some("hanging_sign"),
        ),
        (
            "oak_wall_hanging_sign",
            &[("facing", "north")],
            [1, 0, 0],
            "oak_hanging_sign",
            Some("hanging_sign"),
        ),
        ("torch", &[], [0, -1, 0], "torch", None),
        (
            "wall_torch",
            &[("facing", "east")],
            [-1, 0, 0],
            "torch",
            None,
        ),
        ("soul_torch", &[], [0, -1, 0], "soul_torch", None),
        (
            "soul_wall_torch",
            &[("facing", "east")],
            [-1, 0, 0],
            "soul_torch",
            None,
        ),
        ("redstone_torch", &[], [0, -1, 0], "redstone_torch", None),
        (
            "redstone_wall_torch",
            &[("facing", "east")],
            [-1, 0, 0],
            "redstone_torch",
            None,
        ),
        ("copper_torch", &[], [0, -1, 0], "copper_torch", None),
        (
            "copper_wall_torch",
            &[("facing", "east")],
            [-1, 0, 0],
            "copper_torch",
            None,
        ),
        (
            "white_banner",
            &[],
            [0, -1, 0],
            "white_banner",
            Some("banner"),
        ),
        (
            "white_wall_banner",
            &[("facing", "east")],
            [-1, 0, 0],
            "white_banner",
            Some("banner"),
        ),
        ("dandelion", &[], [0, -1, 0], "dandelion", None),
        ("poppy", &[], [0, -1, 0], "poppy", None),
        (
            "golden_dandelion",
            &[],
            [0, -1, 0],
            "golden_dandelion",
            None,
        ),
        ("rail", &[], [0, -1, 0], "rail", None),
        ("powered_rail", &[], [0, -1, 0], "powered_rail", None),
        ("detector_rail", &[], [0, -1, 0], "detector_rail", None),
        ("activator_rail", &[], [0, -1, 0], "activator_rail", None),
    ];
    let mut failures = Vec::new();
    for &(name, properties, delta, item, block_entity) in cases {
        let support = BlockPos {
            x: pos.x + delta[0],
            y: pos.y + delta[1],
            z: pos.z + delta[2],
        };
        let supporting = if matches!(name, "dandelion" | "poppy" | "golden_dandelion") {
            state.registry.states.dirt
        } else {
            state.registry.states.stone
        };
        let current = block(&state, name, properties);
        state
            .world
            .set_block(support.x, support.y, support.z, supporting);
        state.world.set_block(pos.x, pos.y, pos.z, current);
        state.world.fold_overlay();
        if let Some(id) = block_entity {
            let mut nbt = Vec::new();
            mc_nbt::write_network(
                &mut nbt,
                &Tag::Compound(vec![
                    ("id".into(), Tag::String(format!("minecraft:{id}"))),
                    ("x".into(), Tag::Int(pos.x)),
                    ("y".into(), Tag::Int(pos.y)),
                    ("z".into(), Tag::Int(pos.z)),
                ]),
            )
            .unwrap();
            state.world.set_opaque_block_entity(pos, Some(nbt));
        }
        assert!(
            mc_placement::supported(
                &state.registry.blocks,
                &state.registry.tags,
                pos,
                current,
                &|cell| state.world.block_at(cell.x, cell.y, cell.z)
            ),
            "{name}: valid attachment"
        );
        let air = state.registry.states.air;
        let unrelated = BlockPos {
            z: pos.z + 1,
            ..pos
        };
        let mut unchanged = Vec::new();
        crate::tick::pop_unsupported(&mut state, &mut vec![(unrelated, air)], &mut unchanged);
        assert_eq!(
            state.world.block_at(pos.x, pos.y, pos.z),
            Some(current),
            "{name}: unrelated update"
        );
        assert!(
            unchanged.is_empty(),
            "{name}: unrelated update must not drop an item"
        );
        state.world.set_block(support.x, support.y, support.z, air);
        let mut writes = vec![(support, air)];
        let mut spawned = Vec::new();
        crate::tick::pop_unsupported(&mut state, &mut writes, &mut spawned);
        if state.world.block_at(pos.x, pos.y, pos.z) != Some(air) || !writes.contains(&(pos, air)) {
            failures.push(format!("{name}: did not pop on support removal"));
        }
        let wanted = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(format!("minecraft:{item}")).unwrap())
            .unwrap();
        let drops: Vec<_> = spawned
            .iter()
            .filter_map(|id| state.entities.slot_of(*id))
            .filter_map(|slot| state.entities.row(slot).item.as_ref())
            .map(|stack| (stack.item_id, stack.count))
            .collect();
        if drops != vec![(wanted, 1)] {
            failures.push(format!("{name}: expected one {item} item, got {drops:?}"));
        }
        if block_entity.is_some()
            && state
                .world
                .chunk(crate::boundary::ChunkPos { x: 0, z: 0 })
                .unwrap()
                .block_entities
                .contains_key(&pos)
        {
            failures.push(format!("{name}: retained removed block entity"));
        }
        state.world.set_block(pos.x, pos.y, pos.z, air);
        state.world.fold_overlay();
    }
    assert!(
        failures.is_empty(),
        "support-removal failures: {failures:#?}"
    );
}
/// Ilya (swarm): a valid oak door survived placement, then popped on its own
/// with only an adjacent interior y67 clear nearby. The door's support is its
/// floor cell; an adjacent same-level edit must not pop either half.
#[test]
fn oak_door_survives_adjacent_interior_clear() {
    use crate::crafter::tests::{block, fixture};

    let (mut state, _) = fixture(None);
    // Full vanilla loot so the support-loss drop asserts exactly one item.
    let vanilla = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(
        crate::registry::CoreRegistry::load(Some(&vanilla)).expect("full vanilla block loot"),
    );
    let air = state.registry.states.air;
    let stone = state.registry.states.stone;
    let lower_pos = BlockPos { x: 3, y: 201, z: 3 };
    let upper_pos = BlockPos {
        y: lower_pos.y + 1,
        ..lower_pos
    };
    let lower = block(
        &state,
        "oak_door",
        &[("facing", "west"), ("half", "lower"), ("hinge", "left")],
    );
    let upper = block(
        &state,
        "oak_door",
        &[("facing", "west"), ("half", "upper"), ("hinge", "left")],
    );
    state
        .world
        .set_block(lower_pos.x, lower_pos.y - 1, lower_pos.z, stone);
    state
        .world
        .set_block(lower_pos.x, lower_pos.y, lower_pos.z, lower);
    state
        .world
        .set_block(upper_pos.x, upper_pos.y, upper_pos.z, upper);
    // Adjacent interior cell with stray fill, as in the Ilya hut.
    let interior = BlockPos {
        x: lower_pos.x + 1,
        ..lower_pos
    };
    state
        .world
        .set_block(interior.x, interior.y, interior.z, stone);
    // Clearing that adjacent interior cell must not disturb the door.
    // Convention (matrix above): apply the source edit to the world first,
    // then hand the same write to pop_unsupported.
    state
        .world
        .set_block(interior.x, interior.y, interior.z, air);
    state.world.fold_overlay();
    let mut writes = vec![(interior, air)];
    let mut spawned = Vec::new();
    crate::tick::pop_unsupported(&mut state, &mut writes, &mut spawned);
    assert_eq!(
        state.world.block_at(lower_pos.x, lower_pos.y, lower_pos.z),
        Some(lower),
        "adjacent interior clear must not pop the door lower half"
    );
    assert_eq!(
        state.world.block_at(upper_pos.x, upper_pos.y, upper_pos.z),
        Some(upper),
        "adjacent interior clear must not pop the door upper half"
    );
    assert!(
        spawned.is_empty(),
        "adjacent interior clear must not drop a door item"
    );
    // Removing the floor under the door pops both halves whole with one item.
    let floor = BlockPos {
        y: lower_pos.y - 1,
        ..lower_pos
    };
    state.world.set_block(floor.x, floor.y, floor.z, air);
    state.world.fold_overlay();
    let mut writes = vec![(floor, air)];
    let mut spawned = Vec::new();
    crate::tick::pop_unsupported(&mut state, &mut writes, &mut spawned);
    assert_eq!(
        state.world.block_at(lower_pos.x, lower_pos.y, lower_pos.z),
        Some(air),
        "floor removal pops the door lower half"
    );
    assert_eq!(
        state.world.block_at(upper_pos.x, upper_pos.y, upper_pos.z),
        Some(air),
        "floor removal pops the door upper half with its pair"
    );
    let wanted = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_door").unwrap())
        .unwrap();
    let drops: Vec<_> = spawned
        .iter()
        .filter_map(|id| state.entities.slot_of(*id))
        .filter_map(|slot| state.entities.row(slot).item.as_ref())
        .map(|stack| (stack.item_id, stack.count))
        .collect();
    assert_eq!(
        drops,
        vec![(wanted, 1)],
        "legitimate support loss must drop exactly one oak_door item"
    );
}
