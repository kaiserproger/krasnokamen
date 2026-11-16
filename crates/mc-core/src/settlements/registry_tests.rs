use super::*;

fn create(registry: &mut Registry, sequence: &mut u64, name: &str) -> SettlementId {
    registry
        .create(sequence, name.into(), "owner".into(), [0, 64, 0], 48)
        .unwrap()
}

#[test]
fn settlement_ids_never_reuse_deleted_or_replayed_ids() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let first = create(&mut registry, &mut sequence, "first");
    assert!(registry.remove(first));
    let mut reopened = Registry::default();
    for image in registry.take_images() {
        reopened.apply_image(image);
    }
    reopened.set_high_water(registry.high_water());
    assert_eq!(reopened.records().count(), 0);
    let second = create(&mut reopened, &mut sequence, "second");
    assert!(second.0 > first.0);
    assert_eq!(reopened.high_water(), second.0);
}

#[test]
fn touched_record_images_coalesce_without_capturing_unchanged_records() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let first = create(&mut registry, &mut sequence, "first");
    let second = create(&mut registry, &mut sequence, "second");
    assert_eq!(registry.take_images().len(), 2);
    assert!(registry.take_images().is_empty());
    registry.rename(first, "first".into()).unwrap();
    assert!(registry.take_images().is_empty());
    registry.rename(first, "renamed".into()).unwrap();
    registry.rename(first, "final".into()).unwrap();
    let images = registry.take_images();
    assert_eq!(images.len(), 1);
    assert_eq!(images[0].id, first);
    assert_eq!(images[0].value.as_ref().unwrap().name, "final");
    assert_eq!(registry.get(second).unwrap().revision, 1);
    registry.remove(first);
    assert_eq!(
        registry.take_images(),
        vec![RecordImage {
            id: first,
            value: None
        }]
    );
}

#[test]
fn refused_registry_mutations_do_not_spend_identity_or_touch_records() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let first = create(&mut registry, &mut sequence, "first");
    registry.take_images();
    for name in ["", "has spaces", "first", "../escape"] {
        assert!(
            registry
                .create(&mut sequence, name.into(), "owner".into(), [0; 3], 48)
                .is_err()
        );
    }
    assert_eq!(sequence, first.0);
    assert!(registry.take_images().is_empty());
    let second = create(&mut registry, &mut sequence, "second");
    registry.take_images();
    assert!(registry.rename(second, "first".into()).is_err());
    assert!(registry.take_images().is_empty());
    sequence = u64::MAX;
    assert!(
        registry
            .create(
                &mut sequence,
                "exhausted".into(),
                "owner".into(),
                [0; 3],
                48
            )
            .is_err()
    );
    assert_eq!(registry.records().count(), 2);
}

#[test]
fn record_images_replay_idempotently_and_derived_counts_are_not_durable() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let id = create(&mut registry, &mut sequence, "town");
    registry.count_member(id);
    assert_eq!(registry.member_count(id), 1);
    let images = registry.images();
    let mut reopened = Registry::default();
    for image in images.iter().chain(&images) {
        reopened.apply_image(image.clone());
    }
    assert_eq!(reopened.images(), images);
    assert_eq!(reopened.member_count(id), 0);
    assert!(reopened.take_images().is_empty());
}

#[test]
fn autonomous_village_identity_and_claim_survive_record_restart() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let id = registry
        .adopt_village(&mut sequence, "minecraft:overworld:12:65:-8", [12, 65, -8])
        .unwrap();
    assert_eq!(registry.get(id).unwrap().owner, None);
    assert!(!registry.get(id).unwrap().can_manage("alice", true));
    registry.take_images();
    assert!(registry.claim(id, "alice".into(), [500, 65, 0]).is_err());
    assert!(
        registry.take_images().is_empty(),
        "refused claim must not dirty a native row"
    );
    registry.claim(id, "alice".into(), [12, 65, -8]).unwrap();
    assert!(registry.get(id).unwrap().can_manage("alice", false));
    assert!(!registry.get(id).unwrap().can_manage("bob", false));
    assert!(registry.claim(id, "bob".into(), [12, 65, -8]).is_err());
    registry.rename(id, "claimed".into()).unwrap();
    let bytes = serde_json::to_vec(&registry.images()).unwrap();
    let mut restored = Registry::default();
    for image in serde_json::from_slice::<Vec<RecordImage>>(&bytes).unwrap() {
        restored.apply_image(image);
    }
    assert_eq!(
        restored.adopt_village(&mut sequence, "minecraft:overworld:12:65:-8", [12, 65, -8]),
        Ok(id)
    );
    assert_eq!(restored.records().count(), 1);
    assert_eq!(restored.get(id).unwrap().owner.as_deref(), Some("alice"));
    let mut fresh = Registry::default();
    assert_eq!(
        fresh.adopt_village(&mut 0, "minecraft:overworld:12:65:-8", [12, 65, -8]),
        Ok(id)
    );
}

#[test]
fn autonomous_villages_share_settlement_sequence_and_dedupe_saved_sources() {
    let mut registry = Registry::default();
    let mut sequence = 0;
    let owned = create(&mut registry, &mut sequence, "owned");
    let source = "minecraft:overworld:12:65:-8";
    let village = registry
        .adopt_village(&mut sequence, source, [12, 65, -8])
        .unwrap();
    assert_eq!(village.0, owned.0 + 1);
    assert_eq!(sequence, village.0);
    assert_eq!(registry.high_water(), village.0);
    registry
        .claim(village, "alice".into(), [12, 65, -8])
        .unwrap();
    registry.rename(village, "claimed".into()).unwrap();

    let mut restored = Registry::default();
    for image in registry.images() {
        restored.apply_image(image);
    }
    let mut restored_sequence = 0;
    assert_eq!(
        restored.adopt_village(&mut restored_sequence, source, [99, 70, 99]),
        Ok(village)
    );
    assert_eq!(restored_sequence, 0, "dedupe must not allocate another ID");
    assert!(restored.take_images().is_empty());
    assert_eq!(restored.get(village).unwrap().anchor, [12, 65, -8]);
    assert_eq!(
        restored.get(village).unwrap().owner.as_deref(),
        Some("alice")
    );

    let next = restored
        .adopt_village(
            &mut restored_sequence,
            "minecraft:overworld:100:65:100",
            [100, 65, 100],
        )
        .unwrap();
    assert_eq!(next.0, village.0 + 1);
    assert_eq!(restored_sequence, next.0);
    assert_eq!(
        create(&mut restored, &mut restored_sequence, "next_owned").0,
        next.0 + 1
    );
}

#[test]
fn owned_permissions_and_legacy_owner_json_remain_unchanged() {
    let mut registry = Registry::default();
    let id = create(&mut registry, &mut 0, "owned");
    let record = registry.get(id).unwrap();
    assert!(record.can_manage("owner", false));
    assert!(record.can_manage("operator", true));
    assert!(!record.can_manage("stranger", false));
    let value = serde_json::to_value(record).unwrap();
    assert_eq!(value["owner"], "owner");
    let restored: SettlementRecord = serde_json::from_value(value).unwrap();
    assert_eq!(restored.owner.as_deref(), Some("owner"));
    assert!(registry.claim(id, "stranger".into(), [0, 64, 0]).is_err());
}

#[test]
fn living_member_index_tracks_native_join_leave_death_move_and_removal() {
    use crate::settlements::allocator::tests::villager;
    use crate::settlements::construction::tests::fixture;

    fn consistent(state: &crate::tick::TickState) {
        let registry = &state.settlements;
        let mut inverse = BTreeMap::<SettlementId, BTreeSet<EntityId>>::new();
        for (&entity, &id) in &registry.resident_settlement {
            inverse.entry(id).or_default().insert(entity);
        }
        assert_eq!(
            registry.resident_ids, inverse,
            "inverse equals the occupancy projection"
        );
        for record in registry.records() {
            let native: BTreeSet<_> = state
                .entities
                .slots()
                .filter_map(|slot| {
                    let body = state.entities.body(slot);
                    (body.alive
                        && state
                            .entities
                            .row(slot)
                            .villager
                            .as_ref()
                            .is_some_and(|v| v.settlement == Some(record.id)))
                    .then_some(body.id)
                })
                .collect();
            assert_eq!(
                registry.member_ids(record.id).collect::<BTreeSet<_>>(),
                native
            );
            assert_eq!(registry.member_count(record.id), native.len());
        }
    }

    let (mut state, first, builder) = fixture();
    let second = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "second".into(),
            "owner".into(),
            [8, 201, 8],
            48,
        )
        .unwrap();
    consistent(&state);
    let newcomer = villager(&mut state, [2.5, 201.0, 2.5]);
    let entity = state.entities.body(newcomer).id;
    crate::settlements::bind(&mut state, first, entity).unwrap();
    crate::settlements::bind(&mut state, first, entity).unwrap();
    consistent(&state);
    crate::settlements::unbind(&mut state, entity).unwrap();
    consistent(&state);
    crate::settlements::bind(&mut state, first, entity).unwrap();
    // Migration publishes the native membership column, then refreshes its
    // existing occupancy share; no separate roster mutation is permitted.
    state
        .entities
        .row_mut(newcomer)
        .villager
        .as_mut()
        .unwrap()
        .settlement = Some(second);
    crate::settlements::refresh_counts(&mut state, [entity]);
    consistent(&state);
    assert_eq!(
        state.settlements.member_ids(second).collect::<Vec<_>>(),
        vec![entity]
    );
    let dead = state.entities.body(builder).id;
    state.entities.body_mut(builder).alive = false;
    crate::settlements::refresh_counts(&mut state, [dead]);
    consistent(&state);
    assert!(crate::settlements::remove(&mut state, second));
    consistent(&state);
    assert!(state.settlements.member_ids(second).next().is_none());
    crate::settlements::bind(&mut state, first, entity).unwrap();
    let before = state.settlements.member_ids(first).collect::<Vec<_>>();
    state.settlements.clear_counts();
    assert!(state.settlements.resident_ids.is_empty());
    crate::settlements::rebuild_counts(&mut state);
    consistent(&state);
    assert_eq!(
        state.settlements.member_ids(first).collect::<Vec<_>>(),
        before
    );
    state.settlements.apply_image(RecordImage {
        id: first,
        value: None,
    });
    consistent(&state);
    assert!(state.settlements.member_ids(first).next().is_none());
}
