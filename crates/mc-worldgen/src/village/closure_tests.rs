use super::*;

#[test]
fn absent_jigsaw_template_has_vanilla_empty_semantics() {
    let root = tempfile::tempdir().unwrap();
    let registry =
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap();
    let id = Identifier::parse("minecraft:missing_piece").unwrap();
    let template = load_piece(root.path(), &id, &id, &registry).unwrap();
    assert_eq!(template.size(), [0; 3]);
    assert!(template.blocks().is_empty());
    assert!(template.jigsaws().is_empty());
    assert!(template.entities().is_empty());
    assert!(template.block_entities().is_empty());
    assert!(template.chests().is_empty());
}

#[test]
fn malformed_jigsaw_template_still_fails_closed() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("data/minecraft/structure");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("malformed.nbt"), b"invalid NBT").unwrap();
    let registry =
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap();
    let id = Identifier::parse("minecraft:malformed").unwrap();
    assert!(matches!(
        load_piece(root.path(), &id, &id, &registry),
        Err(ClosureError::Template(
            crate::structures::StructureError::Nbt { .. }
        ))
    ));
}

#[test]
fn jigsaw_template_resolves_its_namespace() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("data/example/structure");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("piece.nbt"), b"invalid NBT").unwrap();
    let registry =
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap();
    let id = Identifier::parse("example:piece").unwrap();
    assert!(matches!(
        load_piece(root.path(), &id, &id, &registry),
        Err(ClosureError::Template(crate::structures::StructureError::Nbt { .. }))
    ));
    let other = Identifier::parse("other:piece").unwrap();
    assert_eq!(load_piece(root.path(), &other, &other, &registry).unwrap().size(), [0; 3]);
}

#[test]
fn jigsaw_template_loads_valid_nbt_from_its_namespace() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("data/example/structure");
    std::fs::create_dir_all(&directory).unwrap();
    let empty_compounds = || mc_nbt::Tag::List(mc_nbt::ListTag {
        element_type: mc_nbt::tag_type::COMPOUND,
        elements: Vec::new(),
    });
    let template = mc_nbt::Tag::Compound(vec![
        ("size".into(), mc_nbt::Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::INT,
            elements: vec![mc_nbt::Tag::Int(1), mc_nbt::Tag::Int(2), mc_nbt::Tag::Int(3)],
        })),
        ("palette".into(), empty_compounds()),
        ("blocks".into(), empty_compounds()),
    ]);
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &template).unwrap();
    std::fs::write(directory.join("piece.nbt"), bytes).unwrap();
    let registry =
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap();
    let id = Identifier::parse("example:piece").unwrap();
    assert_eq!(load_piece(root.path(), &id, &id, &registry).unwrap().size(), [1, 2, 3]);
}

#[test]
fn unsupported_placement_rules_are_refused_before_closure_loading() {
    use mc_data::village_data::{FrequencyReductionMethod, RandomSpreadPlacement, StructureSetSpec};
    let root = tempfile::tempdir().unwrap();
    let blocks =
        BlockRegistry::from_report(&mc_data::blocks::krasnokamen_required_blocks_report()).unwrap();
    let id = Identifier::parse("example:test").unwrap();
    let placement = RandomSpreadPlacement {
        spacing: 32,
        separation: 8,
        salt: 42,
        spread_type: RandomSpreadType::Linear,
        locate_offset: [0; 3],
        frequency_reduction_method: FrequencyReductionMethod::Default,
        frequency: 0.5,
        exclusion_zone: None,
        min_distance_from_world_origin: None,
    };
    let set = StructureSetSpec {
        id: id.clone(),
        placement: PlacementSpec::RandomSpread(placement),
        structures: Vec::new(),
    };
    assert!(matches!(
        load_jigsaw_closure(root.path(), &id, set, &blocks),
        Err(ClosureError::UnsupportedPlacement { field: "frequency", .. })
    ));
}
