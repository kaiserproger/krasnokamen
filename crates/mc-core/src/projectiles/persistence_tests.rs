use super::*;
use crate::journal::{
    EntityAppearanceImage, JournalEntry, JournalWriter, Replay, ReplayEntity, replay_batches,
};

fn items() -> ItemRegistry {
    mc_data::items::krasnokamen_required_items()
}
fn stack(items: &ItemRegistry, name: &str) -> ItemStack {
    ItemStack::new(
        items
            .id_of(&mc_data::Identifier::parse(name).unwrap())
            .unwrap(),
        1,
    )
}
fn type_id(kind: ProjectileKind) -> u32 {
    mc_data::entity_types::krasnokamen_required_entity_types()
        .id_of(&mc_data::Identifier::parse(kind.name()).unwrap())
        .unwrap()
}
fn live_state() -> crate::tick::TickState {
    let registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let generator: std::sync::Arc<dyn mc_world::ChunkGenerator> = std::sync::Arc::new(
        mc_worldgen::TerrainGenerator::new(42, std::sync::Arc::clone(&registry.blocks)),
    );
    let world = crate::world::ChunkWorld::new(
        42,
        generator,
        &registry,
        mc_world::BlockStateId(registry.states.air),
        [0, 64, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    crate::tick::TickState::new(registry, world, 2, 512)
}
fn image(state: &ProjectileState, items: &ItemRegistry) -> mc_data::entity_nbt::VanillaEntity {
    let mut fields = to_fields(state, items, 0);
    fields.retain(|(key, _)| key != "Item");
    mc_data::entity_nbt::VanillaEntity {
        id: state.kind.name().into(),
        uuid: uuid::Uuid::from_u128(123),
        wire_id: 7,
        pos: [-3.0, 70.0, 6.0],
        motion: [0.25, 0.1, -1.5],
        age: state.age as i32,
        item: (item_key(state.kind) == Some("Item")).then(|| state.stack.clone()),
        extra_fields: fields,
        ..Default::default()
    }
}
fn replay_image(
    entity: &mc_data::entity_nbt::VanillaEntity,
    projectile: ProjectileState,
) -> ReplayEntity {
    ReplayEntity {
        uuid: entity.uuid.to_string(),
        type_id: type_id(projectile.kind),
        pos: entity.pos,
        vel: entity.motion.map(|value| value * 20.0),
        yaw: entity.yaw,
        pitch: entity.pitch,
        ground: entity.on_ground,
        health: entity.health,
        alive: true,
        age: entity.age as u32,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: EntityAppearanceImage::default(),
        active_effects: Default::default(),
        villager: None,
        state: Default::default(),
        projectile: Some(Box::new(projectile)),
        tick: 10,
    }
}
fn entry(state: &ProjectileState, position: [f64; 3], velocity: [f64; 3]) -> JournalEntry {
    JournalEntry::Entity {
        id: 7,
        uuid: uuid::Uuid::from_u128(123).to_string(),
        type_id: type_id(state.kind),
        health: 1.0,
        alive: true,
        age: state.age,
        breed_cooldown: 0,
        pickup_delay: 0,
        item: None,
        appearance: EntityAppearanceImage::default(),
        active_effects: Default::default(),
        villager: None,
        state: Default::default(),
        projectile: Some(Box::new(state.clone())),
        tick: 11,
        pos: position,
        vel: velocity,
        yaw: 25.0,
        pitch: -10.0,
    }
}
fn world(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "krasnokamen-projectiles-{label}-{}",
        uuid::Uuid::new_v4()
    ))
}

#[test]
fn in_flight_checkpoint_and_journal_reopen_preserve_motion_and_single_custody() {
    let items = items();
    let dir = world("flight");
    let mut projectile = ProjectileState::new(
        ProjectileKind::Trident,
        Some(uuid::Uuid::from_u128(9)),
        stack(&items, "minecraft:trident"),
    );
    projectile.pickup = true;
    projectile.stack.damage = Some(38);
    projectile.age = 12;
    let entity = image(&projectile, &items);
    let chunks = std::collections::BTreeMap::from([(
        (-1, 0),
        mc_world::anvil::EntityChunk {
            x: -1,
            z: 0,
            entities: vec![entity],
        },
    )]);
    mc_world::anvil::write_entity_regions(&dir.join("entities"), &items, &chunks, 10).unwrap();
    let checkpoint = mc_world::anvil::read_entity_regions(&dir.join("entities"), &items).unwrap();
    let saved = &checkpoint[&(-1, 0)].entities[0];
    let restored = from_entity(saved, &items).unwrap();
    assert_eq!(restored, projectile);
    let mut base = Replay::default();
    base.entities.insert(7, replay_image(saved, restored));
    projectile.age = 13;
    let pos = [-2.75, 70.1, 4.5];
    let vel = [4.95, 1.0, -29.7];
    {
        let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
        writer
            .append(11, 11, &[entry(&projectile, pos, vel)])
            .unwrap();
        writer.flush().unwrap(); // Abrupt stop: no shutdown/checkpoint update.
    }
    let recovered = replay_batches(
        base,
        &crate::journal::read_complete_batches_after_test(&dir.join("journal"), 10).unwrap(),
    );
    let recovered = &recovered.entities[&7];
    assert_eq!(recovered.pos, pos);
    assert_eq!(recovered.vel, vel);
    assert_eq!(recovered.projectile.as_deref(), Some(&projectile));
    assert_eq!(recovered.projectile.as_ref().unwrap().stack.count, 1);
    let mut live = live_state();
    live.tick = recovered.tick;
    crate::core::restore_entity(&mut live, 7, recovered).unwrap();
    let slot = live.entities.slot_of(crate::boundary::EntityId(7)).unwrap();
    assert_eq!(live.entities.pos(slot), pos);
    assert_eq!(live.entities.vel(slot), vel);
    assert_eq!(
        live.entities.row(slot).projectile.as_deref(),
        Some(&projectile)
    );
    // Pickup/removal is a tombstone, so the checkpoint cannot resurrect ammo.
    {
        let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
        writer
            .append(12, 12, &[JournalEntry::RemoveEntity { id: 7, tick: 12 }])
            .unwrap();
        writer.flush().unwrap();
    }
    let mut base = Replay::default();
    base.entities
        .insert(7, replay_image(saved, from_entity(saved, &items).unwrap()));
    let recovered = replay_batches(
        base,
        &crate::journal::read_complete_batches_after_test(&dir.join("journal"), 10).unwrap(),
    );
    assert!(!recovered.entities.contains_key(&7));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn journal_only_birth_keeps_launch_velocity_and_old_entries_default() {
    let items = items();
    let state = ProjectileState::new(
        ProjectileKind::Arrow,
        None,
        stack(&items, "minecraft:arrow"),
    );
    let entry = entry(&state, [1.0, 2.0, 3.0], [0.0, 5.0, 60.0]);
    let recovered = replay_batches(Replay::default(), &[(11, 11, vec![entry.clone()])]);
    assert_eq!(recovered.entities[&7].vel, [0.0, 5.0, 60.0]);
    let mut old = serde_json::to_value(entry).unwrap();
    assert_eq!(
        old["Entity"]["projectile"],
        serde_json::to_value(&state).unwrap()
    );
    let fields = old["Entity"].as_object_mut().unwrap();
    fields.remove("projectile");
    fields.remove("vel");
    let old: JournalEntry = serde_json::from_value(old).unwrap();
    let recovered = replay_batches(Replay::default(), &[(11, 11, vec![old])]);
    assert!(recovered.entities[&7].projectile.is_none());
    assert_eq!(recovered.entities[&7].vel, [0.0; 3]);
}

#[test]
fn retained_cloud_hit_ledger_and_item_components_roundtrip_actual_nbt() {
    let items = items();
    let potion_fields = vec![
        (
            "id".into(),
            Tag::String("minecraft:lingering_potion".into()),
        ),
        ("count".into(), Tag::Int(1)),
        (
            "components".into(),
            Tag::Compound(vec![
                (
                    "minecraft:potion_contents".into(),
                    Tag::Compound(vec![(
                        "potion".into(),
                        Tag::String("minecraft:poison".into()),
                    )]),
                ),
                (
                    "minecraft:custom_data".into(),
                    Tag::Compound(vec![("retained".into(), Tag::Int(99))]),
                ),
            ]),
        ),
    ];
    let stack = stack_from_fields(&items, &potion_fields).unwrap().unwrap();
    let mut state = ProjectileState::new(
        ProjectileKind::AreaEffectCloud,
        Some(uuid::Uuid::from_u128(999)),
        stack,
    );
    state.age = 32;
    state.hit.push(uuid::Uuid::from_u128(101));
    state.data = ProjectileData::Cloud(super::super::payloads::CloudState {
        radius: 2.75,
        waiting: false,
        dragon: false,
        duration: 600,
        wait_time: 10,
        reapplication_delay: 100,
        radius_on_use: -0.5,
        radius_per_tick: -0.005,
        next_tick: 120,
        expires_tick: 700,
        affected_until: std::collections::BTreeMap::from([(uuid::Uuid::from_u128(101), 150)]),
    });
    let entity = image(&state, &items);
    let fields = mc_data::entity_nbt::entity_to_fields(&items, &entity).unwrap();
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &Tag::Compound(fields)).unwrap();
    let (_, Tag::Compound(fields)) = mc_nbt::read_named(&mut std::io::Cursor::new(bytes)).unwrap()
    else {
        panic!("entity root")
    };
    let entity =
        mc_data::entity_nbt::entity_from_fields(&items, Default::default(), &fields).unwrap();
    assert_eq!(from_entity(&entity, &items).unwrap(), state);
    assert_eq!(
        field(&entity.extra_fields, "Radius"),
        Some(&Tag::Float(2.75))
    );
    assert!(field(&entity.extra_fields, "potion_contents").is_some());
}

#[test]
fn vanilla_arrow_fields_are_authoritative_and_malicious_restore_fails_closed() {
    let items = items();
    let mut state = ProjectileState::new(
        ProjectileKind::Arrow,
        None,
        stack(&items, "minecraft:arrow"),
    );
    state.pickup = true;
    let mut fields = to_fields(&state, &items, 0);
    assert_eq!(from_fields(&fields, &items), Ok(state.clone()));
    fields
        .iter_mut()
        .find(|(key, _)| key == "pickup")
        .unwrap()
        .1 = Tag::Byte(0);
    assert!(!from_fields(&fields, &items).unwrap().pickup);
    fields
        .iter_mut()
        .find(|(key, _)| key == "damage")
        .unwrap()
        .1 = Tag::Double(f64::NAN);
    assert!(
        from_fields(&fields, &items)
            .unwrap_err()
            .starts_with("damage:")
    );
    fields
        .iter_mut()
        .find(|(key, _)| key == "damage")
        .unwrap()
        .1 = Tag::Double(-1.0);
    assert!(
        from_fields(&fields, &items)
            .unwrap_err()
            .starts_with("damage:")
    );
    fields
        .iter_mut()
        .find(|(key, _)| key == "damage")
        .unwrap()
        .1 = Tag::Double(2.0);
    fields.push(("damage".into(), Tag::Double(1.0)));
    assert_eq!(
        from_fields(&fields, &items).unwrap_err(),
        "damage: duplicate field"
    );
    let vanilla = vec![
        ("inGround".into(), Tag::Byte(1)),
        ("pickup".into(), Tag::Byte(0)),
        ("life".into(), Tag::Short(100)),
    ];
    let migrated = from_fields_for(ProjectileKind::Arrow, &vanilla, &items).unwrap();
    assert!(migrated.stuck);
    assert!(
        !migrated.pickup,
        "skeleton arrows never become free ammo after import"
    );
    assert_eq!(migrated.age, 100);
}

#[test]
fn retained_invalid_cloud_and_infinite_motion_are_rejected() {
    let items = items();
    let mut state = ProjectileState::new(
        ProjectileKind::Arrow,
        None,
        stack(&items, "minecraft:arrow"),
    );
    state.damage = f32::INFINITY;
    assert!(validate_state(&state).is_err());
    let mut cloud = super::super::payloads::CloudState::lingering(0);
    cloud.radius = f32::NAN;
    let mut cloud_state = ProjectileState::new(
        ProjectileKind::AreaEffectCloud,
        None,
        stack(&items, "minecraft:lingering_potion"),
    );
    cloud_state.data = ProjectileData::Cloud(cloud);
    assert!(validate_state(&cloud_state).is_err());
    let state = ProjectileState::new(ProjectileKind::Arrow, None, ItemStack::EMPTY);
    let mut fields = to_fields(&state, &items, 0);
    fields.push((
        "Motion".into(),
        Tag::List(mc_nbt::ListTag {
            element_type: mc_nbt::tag_type::DOUBLE,
            elements: vec![
                Tag::Double(f64::INFINITY),
                Tag::Double(0.0),
                Tag::Double(0.0),
            ],
        }),
    ));
    assert!(
        from_fields(&fields, &items)
            .unwrap_err()
            .starts_with("Motion[0]:")
    );
}

fn player_inventory_entry(inventory: Vec<ItemStack>, tick: u64) -> JournalEntry {
    JournalEntry::Player {
        uuid: uuid::Uuid::from_u128(9).to_string(),
        dimension: Default::default(),
        pos: [0.0, 70.0, 0.0],
        yaw: 0.0,
        pitch: 0.0,
        health: 20.0,
        food: 20,
        saturation: 5.0,
        exhaustion: 0.0,
        air: 300,
        experience_level: 0,
        experience_progress: 0.0,
        experience_total: 0,
        game_mode: 0,
        selected: 0,
        inventory,
        carried: ItemStack::EMPTY,
        grid: vec![],
        respawn: None,
        respawn_dimension: Default::default(),
        respawn_forced: false,
        respawn_yaw: 0.0,
        respawn_pitch: 0.0,
        active_effects: Default::default(),
        enchantment_seed: 0,
        enchantment_offer_seed: 0,
        warden_warning: Default::default(),
        current_impulse_impact_pos: None,
        burn_ticks: 0,
        opaque_player_fields: Vec::new(),
        statistics: Default::default(),
        advancement_progress: Some(crate::journal::empty_advancement_progress()),
        unlocked_recipes: Some(Default::default()),
        portal: Default::default(),
        fall_flying: false,
        tick,
    }
}

#[test]
fn ammo_consumption_and_pickup_tombstone_share_restart_custody() {
    let items = items();
    let dir = world("custody");
    let owner = uuid::Uuid::from_u128(9).to_string();
    let ammo = stack(&items, "minecraft:trident");
    let initial = replay_batches(
        Replay::default(),
        &[(10, 10, vec![player_inventory_entry(vec![ammo.clone()], 10)])],
    );
    let config = crate::CoreConfig::new(dir.clone(), 42);
    let checkpoint_player = crate::native_checkpoint::tests::roundtrip_player(
        &config,
        &owner,
        &initial.players[&owner],
        &items,
    )
    .unwrap();
    let mut projectile = ProjectileState::new(
        ProjectileKind::Trident,
        Some(uuid::Uuid::from_u128(9)),
        ammo.clone(),
    );
    projectile.pickup = true;
    {
        let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
        writer
            .append(
                11,
                11,
                &[
                    player_inventory_entry(vec![ItemStack::EMPTY], 11),
                    entry(&projectile, [0.0, 71.0, 1.0], [0.0, 0.0, 50.0]),
                ],
            )
            .unwrap();
        writer.flush().unwrap();
    }
    let mut base = Replay::default();
    base.players
        .insert(owner.clone(), checkpoint_player.clone());
    let recovered = replay_batches(
        base,
        &crate::journal::read_complete_batches_after_test(&dir.join("journal"), 10).unwrap(),
    );
    assert!(
        recovered.players[&owner]
            .inventory
            .iter()
            .all(ItemStack::is_empty)
    );
    assert_eq!(
        recovered.entities[&7].projectile.as_ref().unwrap().stack,
        ammo
    );
    {
        let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
        writer
            .append(
                12,
                12,
                &[
                    player_inventory_entry(vec![ammo.clone()], 12),
                    JournalEntry::RemoveEntity { id: 7, tick: 12 },
                ],
            )
            .unwrap();
        writer.flush().unwrap();
    }
    let mut base = Replay::default();
    base.players.insert(owner.clone(), checkpoint_player);
    let recovered = replay_batches(
        base,
        &crate::journal::read_complete_batches_after_test(&dir.join("journal"), 10).unwrap(),
    );
    assert!(!recovered.entities.contains_key(&7));
    assert_eq!(
        recovered.players[&owner]
            .inventory
            .iter()
            .map(|stack| stack.count)
            .sum::<i32>(),
        1
    );
    assert_eq!(recovered.players[&owner].inventory[0], ammo);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn firework_fuse_and_retained_empty_ammo_roundtrip() {
    let items = items();
    let mut firework = ProjectileState::new(
        ProjectileKind::Firework,
        None,
        stack(&items, "minecraft:firework_rocket"),
    );
    firework.data =
        ProjectileData::Firework(super::super::payloads::FireworkState { lifetime: 43 });
    firework.age = 17;
    firework.shot_at_angle = true;
    let fields = to_fields(&firework, &items, 0);
    assert_eq!(field(&fields, "LifeTime"), Some(&Tag::Int(43)));
    assert_eq!(from_fields(&fields, &items), Ok(firework.clone()));
    firework.attached_to = Some(uuid::Uuid::from_u128(7));
    // FireworkRocketEntity.addAdditionalSaveData does not save its attachment.
    let mut detached = firework.clone();
    detached.attached_to = None;
    assert_eq!(
        from_fields(&to_fields(&firework, &items, 0), &items),
        Ok(detached)
    );
    let mut skeleton = ProjectileState::new(ProjectileKind::Arrow, None, ItemStack::EMPTY);
    skeleton.mob_damage = true;
    skeleton.stuck = true;
    skeleton.age = 44;
    let restored = from_fields(&to_fields(&skeleton, &items, 0), &items).unwrap();
    assert_eq!(restored, skeleton);
    assert!(restored.stack.is_empty());
    assert!(!restored.pickup);
}

#[test]
fn vanilla_cloud_rebases_age_deadlines_and_retained_clock_is_unchanged() {
    let items = items();
    let fields = vec![
        ("Age".into(), Tag::Int(40)),
        ("Duration".into(), Tag::Int(600)),
        ("WaitTime".into(), Tag::Int(10)),
        ("Radius".into(), Tag::Float(2.5)),
    ];
    let mut state = from_fields_for(ProjectileKind::AreaEffectCloud, &fields, &items).unwrap();
    restore_clock(&mut state, 1000);
    let ProjectileData::Cloud(cloud) = &state.data else {
        panic!("cloud")
    };
    assert_eq!(cloud.expires_tick, 1570);
    assert_eq!(cloud.next_tick, 1000);
    let saved = state.clone();
    let mut restored = from_fields(&to_fields(&state, &items, 0), &items).unwrap();
    restore_clock(&mut restored, 1001);
    assert_eq!(
        restored, saved,
        "native retained clocks never restart their duration"
    );
}

#[test]
fn vanilla_acceleration_roundtrips_and_rejects_excessive_power() {
    let items = items();
    let state = ProjectileState::new(ProjectileKind::Fireball, None, ItemStack::EMPTY);
    let mut fields = to_fields(&state, &items, 0);
    assert_eq!(
        field(&fields, "acceleration_power"),
        Some(&Tag::Double(0.1))
    );
    assert_eq!(from_fields(&fields, &items), Ok(state));
    fields
        .iter_mut()
        .find(|(key, _)| key == "acceleration_power")
        .unwrap()
        .1 = Tag::Double(1.01);
    assert!(
        from_fields(&fields, &items)
            .unwrap_err()
            .starts_with("acceleration_power:")
    );
}

#[test]
fn vanilla_infinite_cloud_uses_native_infinite_deadline() {
    let items = items();
    let mut state = from_fields_for(
        ProjectileKind::AreaEffectCloud,
        &[("Duration".into(), Tag::Int(-1))],
        &items,
    )
    .unwrap();
    restore_clock(&mut state, 20);
    let ProjectileData::Cloud(cloud) = &state.data else {
        panic!("cloud")
    };
    assert_eq!(cloud.duration, u32::MAX);
    assert_eq!(cloud.expires_tick, u64::MAX);
    assert_eq!(
        field(&to_fields(&state, &items, 0), "Duration"),
        Some(&Tag::Int(-1))
    );
}

#[test]
fn initial_skeleton_arrow_restore_keeps_stuck_age_and_no_ammo() {
    let items = items();
    let state = ProjectileState::new(ProjectileKind::Arrow, None, ItemStack::EMPTY);
    let entity = image(&state, &items);
    let mut record = replay_image(&entity, state);
    record.projectile = None; // Journal schema before retained projectile state.
    record.age = 733;
    record.appearance.extra.push((
        super::super::ARROW_IN_GROUND_INDEX,
        crate::journal::EntityDataValueImage::Boolean(true),
    ));
    let mut live = live_state();
    crate::core::restore_entity(&mut live, 7, &record).unwrap();
    let slot = live.entities.slot_of(crate::boundary::EntityId(7)).unwrap();
    let restored = live.entities.row(slot).projectile.as_ref().unwrap();
    assert!(restored.stuck);
    assert_eq!(restored.age, 733);
    assert!(restored.stack.is_empty());
    assert!(!restored.pickup);
    record.pos[0] = f64::NAN;
    let error = crate::core::restore_entity(&mut live, 8, &record).unwrap_err();
    assert!(error.to_string().contains("non-finite position"));
    assert!(
        live.entities
            .slot_of(crate::boundary::EntityId(8))
            .is_none()
    );
}

#[test]
fn bad_projectile_numbers_never_publish_a_journal_frame() {
    let items = items();
    let state = ProjectileState::new(
        ProjectileKind::Arrow,
        None,
        stack(&items, "minecraft:arrow"),
    );
    let mut image = serde_json::to_value(entry(&state, [0.0; 3], [0.0; 3])).unwrap();
    image["Entity"]["projectile"]["damage"] = serde_json::json!(-1.0);
    assert!(serde_json::from_value::<JournalEntry>(image).is_err());
    let mut invalid = entry(&state, [0.0; 3], [0.0; 3]);
    let JournalEntry::Entity {
        projectile: Some(projectile),
        ..
    } = &mut invalid
    else {
        unreachable!()
    };
    projectile.damage = f32::NAN;
    let dir = world("bad-journal");
    let mut writer = JournalWriter::open_test(&dir, 1).unwrap();
    assert!(writer.append(11, 11, &[invalid]).is_err());
    assert!(
        crate::journal::read_complete_batches_after_test(&dir.join("journal"), 0,)
            .unwrap()
            .is_empty()
    );
    drop(writer);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_projectile_fields_return_specific_reasons() {
    let items = items();
    for (field, tag, reason) in [
        ("Owner", Tag::IntArray(vec![1, 2, 3]), "Owner:"),
        ("pickup", Tag::Byte(3), "pickup:"),
        ("inGround", Tag::Int(1), "inGround:"),
        ("PierceLevel", Tag::Int(128), "PierceLevel:"),
        ("life", Tag::Int(-1), "life:"),
        ("damage", Tag::Double(-1.0), "damage:"),
    ] {
        let error =
            from_fields_for(ProjectileKind::Arrow, &[(field.into(), tag)], &items).unwrap_err();
        assert!(error.starts_with(reason), "{field}: {error}");
    }
    let mut entity = image(
        &ProjectileState::new(
            ProjectileKind::Arrow,
            None,
            stack(&items, "minecraft:arrow"),
        ),
        &items,
    );
    entity.motion[2] = f64::NAN;
    assert_eq!(
        from_entity(&entity, &items).unwrap_err(),
        "Motion[2]: non-finite number"
    );
}

#[test]
fn crossbow_launch_custody_and_confirmed_kills_round_trip_without_inference() {
    let items = items();
    let mut arrow = ProjectileState::new(
        ProjectileKind::Arrow,
        Some(uuid::Uuid::from_u128(123)),
        stack(&items, "minecraft:arrow"),
    );
    arrow.fired_from_weapon = Some(std::sync::Arc::new(stack(&items, "minecraft:crossbow")));
    arrow
        .crossbow_kills
        .push(serde_json::json!({"type": "minecraft:cow"}));
    let fields = to_fields(&arrow, &items, 0);
    assert_eq!(
        from_fields_for(ProjectileKind::Arrow, &fields, &items).unwrap(),
        arrow
    );

    let vanilla_fields: Vec<_> = fields
        .into_iter()
        .filter(|(key, _)| key != RETAINED)
        .collect();
    let imported = from_fields_for(ProjectileKind::Arrow, &vanilla_fields, &items).unwrap();
    assert!(imported.fired_from_weapon.is_none());
    assert!(imported.crossbow_kills.is_empty());
    arrow.fired_from_weapon = None;
    assert!(validate_state(&arrow).is_err());
    arrow.fired_from_weapon = Some(std::sync::Arc::new(stack(&items, "minecraft:crossbow")));
    arrow
        .crossbow_kills
        .resize(129, serde_json::json!({"type": "minecraft:cow"}));
    assert!(validate_state(&arrow).is_err());
}

fn landed_battle_arrow() -> (crate::tick::TickState, crate::boundary::EntityId, u64) {
    let (mut state, _, centre) = super::super::flight_tests::setup();
    let kind = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:husk").unwrap())
        .unwrap();
    let owner = state.spawn_mob(kind, centre, 20.0);
    let owner_slot = state.entities.slot_of(owner).unwrap();
    state.entities.set_faction(owner_slot, 1);
    state.entities.row_mut(owner_slot).controls.no_ai = true;
    let owner_uuid = state.entities.row(owner_slot).uuid;
    let arrow = super::super::launch(
        &mut state,
        ProjectileState::new(ProjectileKind::Arrow, Some(owner_uuid), ItemStack::EMPTY),
        [centre[0] + 4.0, centre[1] + 0.5, centre[2]],
        [0.0, -1.0, 0.0],
    )
    .unwrap();
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    let record = state
        .entities
        .row(state.entities.slot_of(arrow).unwrap())
        .projectile
        .as_ref()
        .unwrap();
    assert!(record.stuck && record.grounded_at_age.is_some());
    assert_eq!(record.ground_lifetime, 100);
    let landed_at = state.tick;
    (state, arrow, landed_at)
}

#[test]
fn anchored_arrow_checkpoint_requires_native_lifetime_age() {
    let (state, arrow, _) = landed_battle_arrow();
    let entity =
        crate::core::entity_data_record(&state, state.entities.slot_of(arrow).unwrap()).unwrap();
    let fields = mc_data::entity_nbt::entity_to_fields(&state.registry.items, &entity).unwrap();
    let mut valid = Replay {
        tick: state.tick,
        ..Default::default()
    };
    crate::native_checkpoint::hydrate_entity(
        &state.registry,
        &mut valid,
        &Tag::Compound(fields.clone()),
        state.tick,
    )
    .unwrap();
    assert!(
        valid.entities[&arrow.0]
            .projectile
            .as_ref()
            .unwrap()
            .grounded_at_age
            .is_some()
    );
    for replacement in [None, Some(Tag::Int(1))] {
        let mut corrupt = fields.clone();
        corrupt.retain(|(name, _)| name != "krasnokamen:LifetimeAge");
        if let Some(value) = replacement {
            corrupt.push(("krasnokamen:LifetimeAge".into(), value));
        }
        let mut rejected = Replay {
            tick: state.tick,
            ..Default::default()
        };
        let error = crate::native_checkpoint::hydrate_entity(
            &state.registry,
            &mut rejected,
            &Tag::Compound(corrupt),
            state.tick,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("krasnokamen:LifetimeAge"),
            "{error}"
        );
        assert!(
            rejected.entities.is_empty(),
            "invalid image must not publish a body"
        );
    }
}

#[test]
fn battle_arrow_real_checkpoint_wal_restart_matches_uninterrupted_expiry() {
    let (mut live, arrow, landed_at) = landed_battle_arrow();
    for _ in 0..37 {
        let (output, _, _) = crate::tick::run_tick(&mut live, &[]);
        crate::core::journal_entries_for_tick(&mut live, &output).unwrap();
    }
    let checkpoint_tick = live.tick;
    let entity =
        crate::core::entity_data_record(&live, live.entities.slot_of(arrow).unwrap()).unwrap();
    let fields = mc_data::entity_nbt::entity_to_fields(&live.registry.items, &entity).unwrap();
    let mut base = Replay {
        tick: checkpoint_tick,
        ..Default::default()
    };
    crate::native_checkpoint::hydrate_entity(
        &live.registry,
        &mut base,
        &Tag::Compound(fields),
        checkpoint_tick,
    )
    .unwrap();
    base.sim_debt.extend(
        live.world
            .debt_checkpoint(checkpoint_tick)
            .into_iter()
            .map(|image| (image.chunk, image)),
    );
    let saved = &base.entities[&arrow.0];
    assert_eq!(saved.projectile.as_ref().unwrap().ground_age(saved.age), 37);

    let directory = tempfile::tempdir().unwrap();
    let mut writer = JournalWriter::open_test(directory.path(), 1).unwrap();
    for _ in 0..13 {
        let (output, _, _) = crate::tick::run_tick(&mut live, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut live, &output).unwrap();
        writer.append(output.tick, output.tick, &entries).unwrap();
    }
    writer.flush().unwrap();
    drop(writer);
    let batches = crate::journal::read_complete_batches_after_test(
        &directory.path().join("journal"),
        checkpoint_tick,
    )
    .unwrap();
    assert_eq!(batches.len(), 13);
    assert!(
        batches.iter().any(|(_, _, entries)| !entries.is_empty()),
        "exercise actual journal producers, not an invented empty WAL tail"
    );
    let replay = replay_batches(base, &batches);
    assert_eq!(replay.tick, live.tick);
    let saved = &replay.entities[&arrow.0];
    assert_eq!(saved.projectile.as_ref().unwrap().ground_age(saved.age), 50);
    assert_eq!(saved.projectile.as_ref().unwrap().ground_lifetime, 100);
    let (mut restarted, _, _) = super::super::flight_tests::setup();
    restarted.tick = replay.tick;
    restarted.world.restore_sim_debt(&replay, None).unwrap();
    crate::core::restore_entity(&mut restarted, arrow.0, saved).unwrap();
    let mut removed_at = None;
    for _ in 0..100 {
        crate::tick::run_tick(&mut live, &[]);
        crate::tick::run_tick(&mut restarted, &[]);
        assert_eq!(live.tick, restarted.tick);
        let present = live.entities.slot_of(arrow).is_some();
        assert_eq!(
            present,
            restarted.entities.slot_of(arrow).is_some(),
            "restart changed removal at tick {}",
            live.tick
        );
        if !present {
            removed_at = Some(live.tick);
            break;
        }
    }
    assert_eq!(removed_at, Some(landed_at + 100));
}

#[test]
fn battle_arrow_support_loss_clears_anchor_across_checkpoint_and_wal_restart() {
    let (mut live, arrow, landed_at) = landed_battle_arrow();
    for _ in 0..37 {
        let (output, _, _) = crate::tick::run_tick(&mut live, &[]);
        crate::core::journal_entries_for_tick(&mut live, &output).unwrap();
    }
    let checkpoint_tick = live.tick;
    let slot = live.entities.slot_of(arrow).unwrap();
    let pos = live.entities.pos(slot);
    let support = [
        pos[0].floor() as i32,
        (pos[1] - 0.1).floor() as i32,
        pos[2].floor() as i32,
    ];
    let open_support = |state: &mut crate::tick::TickState| {
        state.world.set_block(
            support[0],
            support[1],
            support[2],
            state.registry.states.air,
        );
        state.world.set_block(
            support[0],
            support[1] - 2,
            support[2],
            state.registry.states.stone,
        );
        state.world.fold_overlay();
    };
    open_support(&mut live);
    let entity = crate::core::entity_data_record(&live, slot).unwrap();
    let fields = mc_data::entity_nbt::entity_to_fields(&live.registry.items, &entity).unwrap();
    let mut base = Replay {
        tick: checkpoint_tick,
        ..Default::default()
    };
    crate::native_checkpoint::hydrate_entity(
        &live.registry,
        &mut base,
        &Tag::Compound(fields),
        checkpoint_tick,
    )
    .unwrap();
    base.sim_debt.extend(
        live.world
            .debt_checkpoint(checkpoint_tick)
            .into_iter()
            .map(|image| (image.chunk, image)),
    );
    let directory = tempfile::tempdir().unwrap();
    let mut writer = JournalWriter::open_test(directory.path(), 1).unwrap();
    // The tail starts here (the pre-fall ticks are not journaled), and a compact
    // record is a delta of an image in the same tail, so this state emits the
    // image exactly as it does right after a checkpoint capture.
    crate::journal_touched::clear_field_baselines(&mut live);
    let (output, _, _) = crate::tick::run_tick(&mut live, &[]);
    let entries = crate::core::journal_entries_for_tick(&mut live, &output).unwrap();
    writer.append(output.tick, output.tick, &entries).unwrap();
    writer.flush().unwrap();
    drop(writer);
    let slot = live.entities.slot_of(arrow).unwrap();
    let record = live.entities.row(slot).projectile.as_ref().unwrap();
    assert!(
        !record.stuck,
        "an unsupported arrow must leave the idle fast path"
    );
    assert_eq!(
        record.grounded_at_age, None,
        "falling custody cannot retain a ground clock"
    );
    assert_eq!(record.age, 0);

    // The real post-fall checkpoint image must also be admissible, independently
    // of the pre-fall checkpoint plus the actual journal producer's WAL tail.
    let entity = crate::core::entity_data_record(&live, slot).unwrap();
    let fields = mc_data::entity_nbt::entity_to_fields(&live.registry.items, &entity).unwrap();
    let mut falling = Replay {
        tick: live.tick,
        ..Default::default()
    };
    crate::native_checkpoint::hydrate_entity(
        &live.registry,
        &mut falling,
        &Tag::Compound(fields),
        live.tick,
    )
    .unwrap();
    assert_eq!(
        falling.entities[&arrow.0]
            .projectile
            .as_ref()
            .unwrap()
            .grounded_at_age,
        None
    );
    let batches = crate::journal::read_complete_batches_after_test(
        &directory.path().join("journal"),
        checkpoint_tick,
    )
    .unwrap();
    assert_eq!(batches.len(), 1);
    assert!(!batches[0].2.is_empty());
    let replay = replay_batches(base, &batches);
    let saved = &replay.entities[&arrow.0];
    assert!(!saved.projectile.as_ref().unwrap().stuck);
    assert_eq!(saved.projectile.as_ref().unwrap().grounded_at_age, None);
    let (mut restarted, _, _) = super::super::flight_tests::setup();
    open_support(&mut restarted);
    restarted.tick = replay.tick;
    restarted.world.restore_sim_debt(&replay, None).unwrap();
    crate::core::restore_entity(&mut restarted, arrow.0, saved).unwrap();
    let mut relanded_at = None;
    let mut removed_at = None;
    for _ in 0..140 {
        crate::tick::run_tick(&mut live, &[]);
        crate::tick::run_tick(&mut restarted, &[]);
        assert_eq!(live.tick, restarted.tick);
        let live_slot = live.entities.slot_of(arrow);
        let restarted_slot = restarted.entities.slot_of(arrow);
        assert_eq!(
            live_slot.is_some(),
            restarted_slot.is_some(),
            "tick {}",
            live.tick
        );
        if let (Some(live_slot), Some(restarted_slot)) = (live_slot, restarted_slot) {
            assert_eq!(
                live.entities.pos(live_slot),
                restarted.entities.pos(restarted_slot)
            );
            let record = live.entities.row(live_slot).projectile.as_ref().unwrap();
            let restored = restarted
                .entities
                .row(restarted_slot)
                .projectile
                .as_ref()
                .unwrap();
            assert_eq!(record, restored, "tick {}", live.tick);
            if record.stuck {
                relanded_at.get_or_insert(live.tick);
                assert!(record.grounded_at_age.is_some());
            }
        } else {
            removed_at = Some(live.tick);
            break;
        }
    }
    let relanded_at = relanded_at.expect("the lower landing pad must catch the falling arrow");
    assert!(relanded_at > checkpoint_tick);
    assert_eq!(removed_at, Some(relanded_at + 100));
    assert!(
        removed_at.unwrap() > landed_at + 100,
        "the former ground clock must not expire the new landing"
    );
}

#[test]
fn lazy_ground_clock_survives_native_checkpoint_wal_active_pause_and_age_wrap() {
    let state = live_state();
    let items = &state.registry.items;
    let mut arrow = ProjectileState::new(ProjectileKind::Arrow, None, ItemStack::EMPTY);
    arrow.stuck = true;
    arrow.grounded_at_age = Some(u32::MAX - 10);
    arrow.ground_lifetime = 100;
    let entity_age = 39;
    assert_eq!(
        arrow.ground_age(entity_age),
        50,
        "native age wrap is elapsed admitted time"
    );
    let mut entity = image(&arrow, items);
    entity.age = 50;
    entity.health = 1.0;
    entity.on_ground = true;
    entity.motion = [0.0; 3];
    entity.extra_fields = to_fields(&arrow, items, entity_age);
    entity.extra_fields.push((
        "krasnokamen:LifetimeAge".into(),
        Tag::Long(i64::from(entity_age)),
    ));
    assert_eq!(field(&entity.extra_fields, "life"), Some(&Tag::Short(50)));
    let fields = mc_data::entity_nbt::entity_to_fields(items, &entity).unwrap();
    let mut bytes = Vec::new();
    mc_nbt::write_named(&mut bytes, "", &Tag::Compound(fields)).unwrap();
    let (_, payload) = mc_nbt::read_named(&mut std::io::Cursor::new(bytes)).unwrap();
    for active in [true, false] {
        let mut base = Replay {
            tick: 100,
            ..Default::default()
        };
        crate::native_checkpoint::hydrate_entity(&state.registry, &mut base, &payload, 100)
            .unwrap();
        assert_eq!(base.entities[&7].age, entity_age);
        assert_eq!(base.entities[&7].projectile.as_deref(), Some(&arrow));
        base.sim_debt.insert(
            [-1, 0],
            crate::journal::SimDebtImage {
                chunk: [-1, 0],
                last_sim: 100,
                remaining: 0,
                active,
                tick: 100,
            },
        );
        // No per-arrow Entity entries: only the existing native active clock
        // projects age across this WAL tail, including a frozen chunk.
        let replay = replay_batches(base, &[(150, 150, Vec::new())]);
        let saved = &replay.entities[&7];
        let expected = if active { 100 } else { 50 };
        assert_eq!(
            saved.projectile.as_ref().unwrap().ground_age(saved.age),
            expected
        );
        let mut restored = live_state();
        crate::core::restore_entity(&mut restored, 7, saved).unwrap();
        let row = restored.entities.row(
            restored
                .entities
                .slot_of(crate::boundary::EntityId(7))
                .unwrap(),
        );
        let record = row.projectile.as_deref().unwrap();
        assert_eq!(record.ground_age(row.age), expected);
        assert_eq!(
            record.ground_lifetime, 100,
            "restart cannot extend battle arrow lifetime"
        );
    }
}

#[test]
fn ground_clock_legacy_defaults_and_invalid_native_states() {
    let arrow = ProjectileState::new(ProjectileKind::Arrow, None, ItemStack::EMPTY);
    let mut encoded = serde_json::to_value(&arrow).unwrap();
    encoded.as_object_mut().unwrap().remove("ground_lifetime");
    encoded.as_object_mut().unwrap().remove("grounded_at_age");
    let legacy: ProjectileState = serde_json::from_value(encoded).unwrap();
    assert_eq!(legacy.ground_lifetime, super::super::ARROW_GROUND_LIFETIME);
    assert_eq!(legacy.grounded_at_age, None);
    for lifetime in [0, 1, 500, super::super::ARROW_GROUND_LIFETIME + 1] {
        let mut invalid = arrow.clone();
        invalid.ground_lifetime = lifetime;
        assert!(validate_state(&invalid).is_err());
    }
    for kind in [
        ProjectileKind::Arrow,
        ProjectileKind::SpectralArrow,
        ProjectileKind::Trident,
        ProjectileKind::Snowball,
    ] {
        let mut state = ProjectileState::new(kind, None, ItemStack::EMPTY);
        assert!(validate_state(&state).is_ok());
        state.ground_lifetime = 100;
        assert_eq!(
            validate_state(&state).is_ok(),
            matches!(kind, ProjectileKind::Arrow | ProjectileKind::SpectralArrow)
        );
    }
    let mut invalid = arrow;
    invalid.grounded_at_age = Some(0);
    assert!(validate_state(&invalid).is_err());
    invalid.stuck = true;
    assert!(validate_state(&invalid).is_ok());
    invalid.kind = ProjectileKind::Trident;
    assert!(
        validate_state(&invalid).is_err(),
        "loyalty retains its dynamic flight clock"
    );
}
