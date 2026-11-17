use super::tests::fixture;
use super::*;
use mc_data::ItemStack;
use std::io::Write;

#[derive(Clone, Default)]
struct Log(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);
impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn bed_project() -> (TickState, SettlementId, crate::entities::EntitySlot) {
    let (mut state, id, builder) = fixture();
    let bed = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:white_bed").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = ItemStack::new(bed, 1);
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 209, 14]).unwrap();
    let block =
        super::super::allocator::tests::block(&state, "minecraft:white_bed", &[("part", "foot")]);
    let structure = executor::prepare(
        &mut state,
        id,
        "spanish_fortress".into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            at: [10, 202, 10],
            state: block,
            resource: Some("minecraft:white_bed".into()),
            quantity: 1,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().project = Some(Project {
        settlement: id,
        blueprint: "spanish_fortress".into(),
        level: 1,
        origin: [4, 201, 4],
        rotation: 0,
        building: None,
        structure,
        ordered: true,
        borrowed_builder: None,
    });
    (state, id, builder)
}

fn log_stall(state: &mut TickState, id: SettlementId) -> String {
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        // Three windows distinguish the first stall, failed recovery and silence.
        for _ in 0..=450 {
            state.tick += 20;
            release_stalled_builder(state, id);
        }
    });
    String::from_utf8(log.0.lock().clone()).unwrap()
}

#[test]
fn a_material_stall_reports_the_item_source_route_and_actual_builder_release() {
    let (mut state, id, builder) = bed_project();
    let uuid = state.entities.row(builder).uuid;
    let v = state.entities.row_mut(builder).villager.as_mut().unwrap();
    v.navigation_target = Some(crate::boundary::BlockPos { x: 1, y: 201, z: 1 });
    v.navigation_search_failed = true;
    state.settlements.get_mut(id).unwrap().pending_order = Some(OwnerOrder {
        blueprint: "spanish_house".into(),
        level: 1,
        building: None,
    });
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let text = log_stall(&mut state, id);
    assert_eq!(
        text.matches("settlement_construction_stalled").count(),
        1,
        "{text}"
    );
    for evidence in [
        "collecting_material",
        "minecraft:white_bed",
        "required=Some(1)",
        "carried=Some(0)",
        "pickup=Some([1, 201, 1])",
        "navigation_failed=Some(true)",
        "recovery=\"builder_released\"",
        &uuid.to_string(),
    ] {
        assert!(text.contains(evidence), "missing {evidence}: {text}");
    }
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None
    );
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        record.project.as_ref().unwrap().structure,
        project.structure
    );
    assert_eq!(
        record.pending_order.as_ref().unwrap().blueprint,
        "spanish_house"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(
        executor::status(&state, id, &project.structure)
            .unwrap()
            .consumed
            .is_empty()
    );
}

#[test]
fn a_real_fetch_refusal_is_retained_for_the_stall_report_without_debiting_stock() {
    let (mut state, id, builder) = bed_project();
    let bread = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:bread").unwrap())
        .unwrap();
    let inventory = std::array::from_fn(|_| Some(ItemStack::new(bread, 64)));
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory = inventory.clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    release_stalled_builder(&mut state, id);
    crate::villager::building::tick(&mut state, id);
    assert_eq!(
        state.settlements.builder_watch[&id].refusal,
        Some("executor_capacity")
    );
    let text = log_stall(&mut state, id);
    assert!(
        text.contains("last_refusal=Some(\"executor_capacity\")"),
        "{text}"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .inventory,
        inventory
    );
}

#[test]
fn a_repeated_stall_names_the_failed_action_and_cell_once_without_discarding_work() {
    let (mut state, id, _) = bed_project();
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    let before = executor::status(&state, id, &project.structure).unwrap();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let text = log_stall(&mut state, id);
    assert_eq!(
        text.matches("settlement_construction_recovery_failed")
            .count(),
        1,
        "{text}"
    );
    let repeated = text
        .lines()
        .find(|line| line.contains("settlement_construction_recovery_failed"))
        .unwrap();
    for evidence in [
        "spanish_fortress",
        "next=Some([10, 202, 10])",
        "failed_action=\"builder_released\"",
        "cause=\"no_builder\"",
        "recovery=\"no_builder\"",
    ] {
        assert!(
            repeated.contains(evidence),
            "missing {evidence}: {repeated}"
        );
    }
    assert_eq!(
        state.settlements.get(id).unwrap().project.as_ref().unwrap(),
        &project
    );
    assert_eq!(
        executor::status(&state, id, &project.structure).unwrap(),
        before
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

#[test]
fn ordinary_grass_clearing_uses_work_reach_not_a_tree_felling_stand() {
    let (mut state, id, builder) = fixture();
    let next = [10, 201, 10];
    let grass = super::super::allocator::tests::block(&state, "minecraft:short_grass", &[]);
    state.world.set_block(next[0], next[1], next[2], grass);
    state.world.fold_overlay();
    let bounds = ScriptWorldBounds::new([4, 201, 4], [14, 209, 14]).unwrap();
    let air = state.registry.states.air;
    let structure = executor::prepare(
        &mut state,
        id,
        layout::MEADOW.into(),
        bounds,
        bounds,
        vec![executor::PlannedCell {
            at: next,
            state: air,
            resource: None,
            quantity: 0,
        }],
    )
    .unwrap();
    state.settlements.get_mut(id).unwrap().project = Some(Project {
        settlement: id,
        blueprint: layout::MEADOW.into(),
        level: 1,
        origin: bounds.min,
        rotation: 0,
        building: None,
        structure: structure.clone(),
        ordered: false,
        borrowed_builder: None,
    });
    state.entities.set_pos(builder, [5.5, 201.0, 10.5]);
    crate::villager::building::tick(&mut state, id);
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "a grass cell in ordinary work reach must not wait for an unrelated felling stand"
    );
    assert_eq!(
        executor::status(&state, id, &structure).unwrap().state,
        ScriptStructureState::Committed
    );
    assert_eq!(state.world.block_at(next[0], next[1], next[2]), Some(air));
}
