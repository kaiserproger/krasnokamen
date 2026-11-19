//! Compare recovered running-tail entity/debt/climate images with the live cut.
use super::*;

#[test]
fn game_clock_running_nonzero_held_prefix_nonempty_tail_matches_live_checkpoint() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    let item = drill
        .state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:stone").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        &mut drill.state,
        [34.5, 200.0, 40.5],
        mc_data::ItemStack::new(item, 2),
    );
    let slot = drill.state.entities.slot_of(id).unwrap();
    drill.state.entities.row_mut(slot).age = 40;
    drill.state.tick_control.set_frozen(true);
    for _ in 0..3 {
        drill.run(&[]).unwrap();
    }
    drill.checkpoint();
    drill.state.tick_control.set_frozen(false);
    for _ in 0..2 {
        drill.run(&[]).unwrap();
    }
    drill.writer.as_mut().unwrap().flush().unwrap();
    let batches = crate::journal::read_complete_batches_after(
        &drill.config.world_dir.join("journal"),
        drill.manifest.tick,
    )
    .unwrap();
    assert_eq!(batches.len(), 2, "the running tail must be nonempty");
    assert!(batches.iter().all(|(_, _, entries)| entries.iter().any(|entry|
        matches!(entry, JournalEntry::Climate { value } if value.game_clock == crate::weather::GameClockImage::Running { held_ticks: 3 })
    )), "every running output carries Climate at H=3");
    let store = fixture_store(&drill.config).unwrap();
    let recovered = crate::native_checkpoint::recover_with_reader(
        &drill.config,
        Arc::clone(&drill.state.registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    assert!(
        recovered.dimensions[&crate::dimensions::Dimension::Overworld].tick
            > recovered.manifest.tick
    );
    let encoded = crate::native_checkpoint::encode_checkpoint(&recovered).unwrap();
    let encoded_manifest: CheckpointManifest =
        serde_json::from_slice(&encoded.globals["checkpoint"].payload).unwrap();
    assert!(encoded_manifest.climate.unwrap().advance_time);
    assert!(encoded_manifest.debt.iter().any(|debt| debt.active));
    drop(recovered);
    drill.checkpoint();
    let live = store.read_checkpoint().unwrap();
    let live_manifest: CheckpointManifest =
        serde_json::from_slice(&live.globals["checkpoint"].payload).unwrap();
    assert_eq!(
        serde_json::to_vec(&encoded_manifest.debt).unwrap(),
        serde_json::to_vec(&live_manifest.debt).unwrap()
    );
    assert_eq!(
        serde_json::to_vec(&encoded_manifest.climate).unwrap(),
        serde_json::to_vec(&live_manifest.climate).unwrap()
    );
    let payloads = |image: &mc_world::native::NativeCheckpointImage| {
        image
            .entities
            .iter()
            .filter_map(|entity| {
                entity
                    .payload
                    .as_ref()
                    .map(|payload| (entity.identity, payload.clone()))
            })
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(payloads(&encoded), payloads(&live));
}
