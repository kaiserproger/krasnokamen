use super::*;
use crate::settlements::construction::tests::{fixture, fund, open_plan};
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

fn batch(reason: Reason, frontages: u32) -> Report {
    let mut report = Report {
        frontages,
        examined: 64,
        ..Default::default()
    };
    report.reject(reason, [10, 200, 20]);
    report
}

fn reported(watch: &mut Watch, reason: Reason, frontages: u32) -> bool {
    watch.record("storage", batch(reason, frontages)).is_some()
}

#[test]
fn complete_sweeps_report_reason_changes_not_cursor_batches() {
    let mut watch = Watch::default();
    assert!(!reported(&mut watch, Reason::BuildingBuffer, 128));
    let first = watch
        .record("storage", batch(Reason::ProtectedObstruction, 128))
        .unwrap();
    assert_eq!(first.examined, 128);
    assert_eq!(first.counts[Reason::BuildingBuffer as usize], 1);
    assert_eq!(
        first.examples[Reason::ProtectedObstruction as usize],
        Some([10, 200, 20])
    );
    assert!(!reported(&mut watch, Reason::ProtectedObstruction, 128));
    assert!(!reported(&mut watch, Reason::BuildingBuffer, 128));
    assert!(!reported(&mut watch, Reason::HeightVariance, 128));
    assert!(reported(&mut watch, Reason::HeightVariance, 128));
    assert!(!reported(&mut watch, Reason::ProtectedObstruction, 128));
    assert!(
        reported(&mut watch, Reason::BuildingBuffer, 128),
        "a previous reason returning is another change"
    );
    assert!(!reported(&mut watch, Reason::HeightVariance, 128));
    assert!(!reported(&mut watch, Reason::HeightVariance, 192));
    assert!(!reported(&mut watch, Reason::HeightVariance, 192));
    assert!(
        reported(&mut watch, Reason::HeightVariance, 192),
        "a changed network starts a complete new sweep"
    );
}

#[test]
fn pending_site_refusal_logs_once_and_keeps_fifo_and_real_stock() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    let record = state.settlements.get(id).unwrap();
    let blueprint = blueprints::get(&state.registry, "storage").unwrap();
    let mut network = layout::Network::for_construction(&state, record);
    let fronts = network.frontage();
    let roots = fronts
        .iter()
        .copied()
        .filter_map(|(at, normal)| network.lot(blueprint, at, normal).map(|lot| lot.origin))
        .collect();
    let passes = fronts.len().div_ceil(SITE_ATTEMPTS as usize) * 3;
    let record = state.settlements.get_mut(id).unwrap();
    record.planted_trees = roots;
    record.pending_order = Some(OwnerOrder {
        blueprint: "storage".into(),
        level: 1,
        building: None,
    });
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let log = Log::default();
    let writer = log.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        for _ in 0..passes {
            plan(&mut state, id);
        }
        let record = state.settlements.get(id).unwrap();
        assert!(
            record.project.is_none(),
            "protected lots do not admit autonomous work"
        );
        assert_eq!(record.pending_order.as_ref().unwrap().blueprint, "storage");
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
        assert!(state.settlements.site_watch.contains_key(&id));
        let text = String::from_utf8(log.0.lock().clone()).unwrap();
        assert_eq!(
            text.matches("settlement_construction_site_refused").count(),
            1,
            "{text}"
        );
        assert!(text.contains("ProtectedObstruction"), "{text}");
        assert!(text.contains("storage"), "{text}");
        cancel_pending(&mut state, id).unwrap();
        assert!(!state.settlements.site_watch.contains_key(&id));
        state.settlements.get_mut(id).unwrap().pending_order = Some(OwnerOrder {
            blueprint: "storage".into(),
            level: 1,
            building: None,
        });
        for _ in 0..passes {
            plan(&mut state, id);
        }
        let text = String::from_utf8(log.0.lock().clone()).unwrap();
        assert_eq!(
            text.matches("settlement_construction_site_refused").count(),
            2,
            "a new order is a new episode: {text}"
        );
        state.settlements.get_mut(id).unwrap().planted_trees.clear();
        plan(&mut state, id);
        let record = state.settlements.get(id).unwrap();
        assert_eq!(record.project.as_ref().unwrap().blueprint, "storage");
        assert!(record.pending_order.is_none());
        assert!(!state.settlements.site_watch.contains_key(&id));
        assert_eq!(
            state.block_entities[&(1, 201, 1)].slots,
            stock,
            "site admission never debits materials"
        );
    });
}

#[test]
fn native_grown_planted_cells_without_natural_baseline_have_named_refusal() {
    let (mut state, id, _, project, root, _) = super::super::super::forestry::tests::forest();
    super::super::super::forestry::tests::grown_world(&mut state, Vec::new(), "birch");
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let sapling = crate::settlements::allocator::tests::block(
        &state,
        "minecraft:birch_sapling",
        &[("stage", "1")],
    );
    let dirt = crate::settlements::allocator::tests::block(&state, "minecraft:dirt", &[]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    state.world.set_block(root[0], root[1], root[2], sapling);
    let at = crate::boundary::BlockPos {
        x: root[0],
        y: root[1],
        z: root[2],
    };
    let growth = (0..64)
        .find_map(|seed| crate::plants::bonemeal(&state, at, seed))
        .expect("native birch growth on its supported registered sapling");
    assert!(
        growth
            .writes
            .iter()
            .any(|(_, block)| { super::super::super::forestry::tree(&state, *block) })
    );
    for (at, block) in growth.writes {
        state.world.set_block(at.x, at.y, at.z, block);
    }
    let lot = super::super::super::forestry::lot_bounds(&state, &project).unwrap();
    assert!(!super::super::super::forestry::clearable_on_lot(
        &state,
        root,
        Some(lot)
    ));
    let blueprint = blueprints::get(&state.registry, &project.blueprint).unwrap();
    let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
    assert_eq!(
        check(
            &state,
            state.settlements.get(id).unwrap(),
            project.origin,
            size,
            None
        ),
        Err(Reason::PlantingProvenance),
    );
    assert!(planned_cells(&state, &project, "oak").is_none());
}
