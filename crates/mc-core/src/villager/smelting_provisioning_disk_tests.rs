use super::*;

#[test]
fn provisioned_workstation_custody_survives_checkpoint_and_rejects_removed_block() {
    let directory = tempfile::tempdir().unwrap();
    let config = CoreConfig::new(directory.path().to_path_buf(), 42);
    let (mut state, worker, _) = disk_setup(&config);
    state
        .world
        .set_block(FURNACE.0, FURNACE.1, FURNACE.2, state.registry.states.air);
    state.block_entities.remove(&FURNACE).unwrap();
    let town = state
        .entities
        .row(state.entities.slot_of(worker).unwrap())
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    let mut placed = None;
    for _ in 0..400 {
        crate::tick::run_tick(&mut state, &[]);
        placed = crate::villager::smelting::owned_furnaces(&state, town).next();
        if placed.is_some() {
            break;
        }
    }
    let at = placed.expect("ordinary builder must provision the workstation");
    let handle = state
        .settlements
        .get(town)
        .unwrap()
        .furnace_workstation
        .clone()
        .unwrap();
    let before = counts(&state);
    let registry = std::sync::Arc::clone(&state.registry);
    checkpoint(&mut state, &config);
    drop(state);
    let mut state = crate::core::recover_dimension_state(&config, Dimension::Overworld).unwrap();
    state.registry = registry;
    activate(&mut state);
    crate::tick::sync_chunk_block_entities(&mut state);
    assert_eq!(counts(&state), before);
    assert_eq!(
        state
            .settlements
            .get(town)
            .unwrap()
            .furnace_workstation
            .as_deref(),
        Some(handle.as_str())
    );
    assert_eq!(
        crate::villager::smelting::owned_furnaces(&state, town).collect::<Vec<_>>(),
        vec![at]
    );
    assert_eq!(
        crate::plugin_structures::settlement_builds::furnace_workstation(
            &state,
            crate::settlements::SettlementId(town.0 + 1),
            &handle
        ),
        None,
        "a handle alone cannot grant a different town custody"
    );
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.air);
    state.world.fold_overlay();
    assert!(
        crate::villager::smelting::owned_furnaces(&state, town)
            .next()
            .is_none(),
        "a durable record cannot authorize a missing furnace"
    );
}
