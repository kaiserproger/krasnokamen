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
    assert!(!reported(&mut watch, Reason::SurfaceUnavailable, 128));
    assert!(reported(&mut watch, Reason::SurfaceUnavailable, 128));
    assert!(!reported(&mut watch, Reason::ProtectedObstruction, 128));
    assert!(
        reported(&mut watch, Reason::BuildingBuffer, 128),
        "a previous reason returning is another change"
    );
    assert!(!reported(&mut watch, Reason::SurfaceUnavailable, 128));
    assert!(!reported(&mut watch, Reason::SurfaceUnavailable, 192));
    assert!(!reported(&mut watch, Reason::SurfaceUnavailable, 192));
    assert!(
        reported(&mut watch, Reason::SurfaceUnavailable, 192),
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
    record.orders = vec![OwnerOrder {
        blueprint: "storage".into(),
        level: 1,
        building: None,
    }];
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
            record.projects.is_empty(),
            "protected lots do not admit autonomous work"
        );
        assert_eq!(record.orders[0].blueprint, "storage");
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
        state.settlements.get_mut(id).unwrap().orders = vec![OwnerOrder {
            blueprint: "storage".into(),
            level: 1,
            building: None,
        }];
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
        for _ in 0..256 {
            plan(&mut state, id);
            if !state.settlements.get(id).unwrap().projects.is_empty() {
                break;
            }
        }
        let record = state.settlements.get(id).unwrap();
        assert_eq!(record.projects[0].blueprint, "storage");
        assert!(record.orders.is_empty());
        assert!(!state.settlements.site_watch.contains_key(&id));
        assert_eq!(
            state.block_entities[&(1, 201, 1)].slots,
            stock,
            "site admission never debits materials"
        );
    });
}

#[test]
fn native_grown_planted_cells_are_clearable_by_the_owner_whole_tree_rule() {
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
    assert!(super::super::super::forestry::Clearance::default().clearable(&state, root, Some(lot)));
}

#[test]
fn off_lot_registered_sapling_reserves_its_crown_until_it_actually_grows() {
    let (mut state, id, _, _, root, _) = super::super::super::forestry::tests::forest();
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
    state.world.set_block(root[0], root[1], root[2], sapling);
    let origin = [root[0] + 2, 200, root[2] + 2];
    assert_eq!(
        check(
            &state,
            state.settlements.get(id).unwrap(),
            origin,
            [1, 12, 1],
            None,
            &mut Default::default(),
            None,
        ),
        Err(Reason::PlantingProvenance)
    );
    super::super::super::forestry::native_tree_tests::grow(&mut state, root, 5);
    assert!(
        check(
            &state,
            state.settlements.get(id).unwrap(),
            origin,
            [1, 12, 1],
            None,
            &mut Default::default(),
            None,
        )
        .is_ok(),
        "an actually grown registered tree is clearable by its owner lot"
    );
}

#[test]
fn native_overlapping_crowns_report_ambiguous_foliage_not_a_clearable_site() {
    for connected in [false, true] {
        let (state, id, _, witness) =
            super::super::super::forestry::native_tree_tests::overlapping(connected);
        assert_eq!(
            check(
                &state,
                state.settlements.get(id).unwrap(),
                [witness[0], 200, witness[2]],
                [1, 12, 1],
                None,
                &mut Default::default(),
                None,
            ),
            Err(Reason::AmbiguousFoliage),
            "touching trunks: {connected}"
        );
    }
}

#[test]
fn persistent_and_retained_player_foliage_report_protected_obstruction() {
    let (mut state, id, _, _, root, _) = super::super::super::forestry::tests::forest();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let at = [root[0] + 1, root[1] + 4, root[2]];
    let origin = [at[0], 200, at[2]];
    for retained in [false, true] {
        let leaf = crate::settlements::allocator::tests::block(
            &state,
            "minecraft:birch_leaves",
            &[("persistent", if retained { "false" } else { "true" })],
        );
        state.world.set_block(at[0], at[1], at[2], leaf);
        state.block_log = if retained {
            super::super::super::forestry::tests::player_history(&state, at, 0)
        } else {
            Default::default()
        };
        assert_eq!(
            check(
                &state,
                state.settlements.get(id).unwrap(),
                origin,
                [1, 12, 1],
                None,
                &mut Default::default(),
                None,
            ),
            Err(Reason::ProtectedObstruction),
            "retained edit: {retained}"
        );
    }
}

#[test]
fn natural_zone_terrain_keeps_its_shape_without_a_building_setback() {
    use crate::settlements::buildings::{Building, BuildingId, BuildingKind};
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let size = [3, 4, 3];
    let zone = |state: &mut TickState, kind, bounds: ScriptWorldBounds| {
        let zone_id = BuildingId((id.0 << 32) | 700);
        state.settlements.get_mut(id).unwrap().buildings.insert(
            zone_id,
            Building {
                id: zone_id,
                settlement: id,
                kind,
                level: 1,
                bounds,
                work_points: vec![[bounds.min[0], bounds.min[1] + 1, bounds.min[2]]],
                revision: 1,
                natural_zone: true,
            },
        );
    };
    let check_at = |state: &TickState, origin: [i32; 3], size: [u8; 3]| {
        check(
            state,
            state.settlements.get(id).unwrap(),
            origin,
            size,
            None,
            &mut Default::default(),
            None,
        )
        .map(|_| ())
    };
    let origin = [10, 201, 10];
    assert_eq!(
        check_at(&state, origin, size),
        Ok(()),
        "bare terrain admits the lot"
    );
    // A certified terrain zone one column away keeps only its own columns: its
    // one-block building setback is not a terrain rule.
    zone(
        &mut state,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([6, 200, 8], [9, 206, 14]).unwrap(),
    );
    assert_eq!(
        check_at(&state, origin, size),
        Ok(()),
        "a natural zone does not reserve the neighbouring lot"
    );
    // A zone the house would stand in is still an overlap.
    state.settlements.get_mut(id).unwrap().buildings.clear();
    zone(
        &mut state,
        BuildingKind::Mine,
        ScriptWorldBounds::new([11, 200, 11], [12, 206, 12]).unwrap(),
    );
    assert_eq!(
        check_at(&state, origin, size),
        Err(Reason::BuildingBuffer),
        "a zone the house would stand in still refuses"
    );
    // A zone below the floor that the fill would cross still refuses: the
    // preparation volume reaches down to the discovered ground, not only the
    // authored footprint (review 1.1). The lot is raised, so two fill rows exist
    // below its floor, and the zone's top sits between the ground and the floor.
    state.settlements.get_mut(id).unwrap().buildings.clear();
    zone(
        &mut state,
        BuildingKind::Mine,
        ScriptWorldBounds::new([11, 195, 11], [12, 201, 12]).unwrap(),
    );
    assert_eq!(
        check_at(&state, [10, 203, 10], size),
        Err(Reason::BuildingBuffer),
        "the fill below the floor may not cross a mine's zone"
    );
}
