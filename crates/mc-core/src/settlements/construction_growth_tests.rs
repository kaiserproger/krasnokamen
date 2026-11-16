use super::tests::{complete, fixture, fund, open_plan, start};
use super::*;
use crate::settlements::allocator::tests::block;
use std::sync::{Arc, Mutex};

fn housing_fixture(spare_bed: bool) -> (TickState, SettlementId) {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    if spare_bed {
        let head = block(
            &state,
            "minecraft:white_bed",
            &[("facing", "north"), ("part", "head")],
        );
        let foot = block(
            &state,
            "minecraft:white_bed",
            &[("facing", "north"), ("part", "foot")],
        );
        state.world.set_block(1, 201, 10, head);
        state.world.set_block(1, 201, 11, foot);
        state
            .world
            .set_block(1, 204, 10, state.registry.states.stone);
        state.world.fold_overlay();
    }
    for _ in 0..40 {
        crate::tick::run_tick(&mut state, &[]);
    }
    assert!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some(),
        "the resident must actually claim its bed"
    );
    assert!(state.settlements.get(id).unwrap().project.is_none());
    stock_buildings(&mut state, &["spanish_house", "spanish_church"]);
    (state, id)
}

fn stock_buildings(state: &mut TickState, names: &[&str]) {
    let mut bill = std::collections::BTreeMap::<u32, u32>::new();
    for name in names {
        for &(item, count) in &blueprints::get(&state.registry, name)
            .unwrap()
            .level(1)
            .unwrap()
            .materials
        {
            *bill.entry(item).or_default() += count;
        }
    }
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    let mut index = 0;
    for (item, mut count) in bill {
        while count != 0 {
            let portion = count.min(64);
            slots[index] = mc_data::ItemStack::new(item, portion as i32);
            index += 1;
            count -= portion;
        }
    }
}

#[test]
fn exhausted_roofed_beds_put_another_house_before_a_queued_cultural_order() {
    let (mut state, id) = housing_fixture(false);
    // Keep the autonomous house untouched across several owner-order passes.
    state.day_time = 0;
    let result = request_build(&mut state, id, "spanish_church").unwrap();
    assert!(
        matches!(result, OrderOutcome::Queued { .. }),
        "growth housing must keep the cultural order queued: {result:?}"
    );
    for _ in 0..40 {
        crate::tick::run_tick(&mut state, &[]);
        if state.settlements.get(id).unwrap().project.is_some() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    let structure = record.project.as_ref().unwrap().structure.clone();
    for _ in 0..80 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project.as_ref().unwrap().structure, structure);
    assert_eq!(record.project.as_ref().unwrap().blueprint, "spanish_house");
    assert_eq!(
        record.pending_order.as_ref().unwrap().blueprint,
        "spanish_church"
    );
}

#[test]
fn a_real_spare_roofed_bed_keeps_the_cultural_order_reachable() {
    let (mut state, id) = housing_fixture(true);
    assert_eq!(
        request_build(&mut state, id, "spanish_church").unwrap(),
        OrderOutcome::Started
    );
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .blueprint,
        "spanish_church"
    );
}

fn house_without_site() -> (TickState, SettlementId) {
    let (mut state, id) = housing_fixture(false);
    stock_buildings(&mut state, &["spanish_house", "farm"]);
    // The Farm fits below this fluid ceiling; the taller House cannot.
    let water = block(&state, "minecraft:water", &[("level", "0")]);
    for x in -16..32 {
        for z in -16..32 {
            state.world.set_block(x, 208, z, water);
        }
    }
    state.world.fold_overlay();
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let house = priority_house(&state, record, &images).expect("a funded housing need");
    assert!(
        site_project(&state, record, std::iter::once(house), 0, u32::MAX, &images)
            .0
            .is_none(),
        "no loaded House site fits"
    );
    assert!(
        site_project(
            &state,
            record,
            std::iter::once(blueprints::get(&state.registry, "farm").unwrap()),
            0,
            SITE_ATTEMPTS,
            &images,
        )
        .0
        .is_some(),
        "the funded Farm has a legal site in this planning pass"
    );
    (state, id)
}

#[test]
fn a_house_without_a_site_does_not_hide_a_funded_autonomous_farm() {
    let (mut state, id) = house_without_site();
    plan(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .blueprint,
        "farm"
    );
}

#[test]
fn a_house_without_a_site_does_not_hold_a_direct_farm_order() {
    let (mut state, id) = house_without_site();
    assert_eq!(
        request_build(&mut state, id, "farm").unwrap(),
        OrderOutcome::Started
    );
}

#[test]
fn a_house_without_a_site_does_not_hold_a_pending_farm_order() {
    let (mut state, id) = house_without_site();
    state.settlements.get_mut(id).unwrap().pending_order = Some(OwnerOrder {
        blueprint: "farm".into(),
        level: 1,
        building: None,
    });
    assert!(plan_order(&mut state, id));
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project.as_ref().unwrap().blueprint, "farm");
    assert!(record.pending_order.is_none());
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl tracing::Subscriber for Warnings {
    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        *metadata.level() == tracing::Level::WARN
    }
    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, event: &tracing::Event<'_>) {
        struct Cause(String);
        impl tracing::field::Visit for Cause {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "cause" {
                    self.0 = format!("{value:?}");
                }
            }
        }
        let mut cause = Cause(String::new());
        event.record(&mut cause);
        if !cause.0.is_empty() {
            self.0.lock().unwrap().push(cause.0);
        }
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn a_stalled_project_warns_once_after_three_work_hours_without_a_builder() {
    let (mut state, id, builder) = fixture();
    fund(&mut state, "storage", 1);
    start(&mut state, id, "storage", 1, None);
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Farmer;
    state.day_time = 2_000;
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        release_stalled_builder(&mut state, id);
        for _ in 0..149 {
            state.tick += 20;
            state.day_time += 20;
            release_stalled_builder(&mut state, id);
        }
        assert!(warnings.0.lock().unwrap().is_empty());
        for _ in 0..180 {
            state.tick += 20;
            state.day_time += 20;
            release_stalled_builder(&mut state, id);
        }
    });
    let causes = warnings.0.lock().unwrap();
    assert_eq!(
        causes.len(),
        1,
        "one bounded warning per no-progress episode: {causes:?}"
    );
    assert!(causes[0].contains("no_builder"));
}

fn protected_preparation() -> (
    TickState,
    SettlementId,
    crate::entities::EntitySlot,
    Project,
    [i32; 3],
) {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    let root = [5, 201, 5];
    super::super::forestry::tests::grown_world(&mut state, vec![root], "birch");
    fund(&mut state, "storage", 1);
    start(&mut state, id, "storage", 1, None);
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    assert!(executor::clearing(&state, id, &project.structure));
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    state.entities.set_pos(
        builder,
        [
            f64::from(next[0]) + 1.5,
            f64::from(next[1]),
            f64::from(next[2]) + 0.5,
        ],
    );
    (state, id, builder, project, root)
}

fn stall_preparation(state: &mut TickState, id: SettlementId) -> Warnings {
    let warnings = Warnings::default();
    tracing::subscriber::with_default(warnings.clone(), || {
        release_stalled_builder(state, id);
        for _ in 0..=150 {
            crate::villager::building::tick(state, id);
            state.tick += 20;
            state.day_time += 20;
            release_stalled_builder(state, id);
        }
    });
    warnings
}

#[test]
fn a_protected_regrown_tree_requeues_unpaid_preparation_without_destroying_it() {
    let (mut state, id, builder, project, root) = protected_preparation();
    let tree = state.world.block_at(root[0], root[1], root[2]);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    // A rotated builder may still carry an older load during preparation.
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let stack = slots.iter_mut().find(|stack| !stack.is_empty()).unwrap();
    let carried = mc_data::ItemStack::new(stack.item_id, 1);
    stack.count -= 1;
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(carried);
    let warnings = stall_preparation(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(
        record.project.is_none(),
        "a permanently protected tree must not retain the project slot"
    );
    assert_eq!(record.pending_order.as_ref().unwrap().blueprint, "storage");
    assert_eq!(
        executor::status(&state, id, &project.structure)
            .unwrap()
            .state,
        ScriptStructureState::Cancelled
    );
    assert_eq!(state.world.block_at(root[0], root[1], root[2]), tree);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .iter()
            .flatten()
            .all(mc_data::ItemStack::is_empty),
        "retry returns the builder's carry before releasing its profession"
    );
    assert_eq!(
        &*warnings.0.lock().unwrap(),
        &["\"protected_tree_in_preparation\""]
    );
}

#[test]
fn a_protected_tree_never_cancels_preparation_with_consumed_materials() {
    let (mut state, id, _, project, root) = protected_preparation();
    // Clear cells are normally free. Exercise the defensive paid-work guard
    // using the canonical persisted ledger image, without changing its stage.
    let mut ledger = state
        .plugins
        .get_domain("@native-structures", "ledger")
        .unwrap()
        .clone();
    ledger["structures"][&project.structure]["snapshot"]["consumed"] =
        serde_json::json!([{"resource": "minecraft:oak_log", "quantity": 1}]);
    state
        .plugins
        .set_domain("@native-structures", "ledger", ledger);
    let before = executor::status(&state, id, &project.structure).unwrap();
    assert!(!before.consumed.is_empty());
    assert!(executor::clearing(&state, id, &project.structure));
    let tree = state.world.block_at(root[0], root[1], root[2]);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let warnings = stall_preparation(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        record.project.as_ref().unwrap().structure,
        project.structure
    );
    assert!(record.pending_order.is_none());
    let after = executor::status(&state, id, &project.structure).unwrap();
    assert_ne!(after.state, ScriptStructureState::Cancelled);
    assert_eq!(after.consumed, before.consumed);
    assert_eq!(state.world.block_at(root[0], root[1], root[2]), tree);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(
        &*warnings.0.lock().unwrap(),
        &["\"protected_tree_in_preparation\""]
    );
}

fn planted_sapling_before_admission(corridor: bool) {
    let (mut state, id, _, project, original_root, access) =
        super::super::forestry::tests::forest();
    let root = if corridor {
        access
    } else {
        [project.origin[0] - 1, 201, project.origin[2] + 2]
    };
    let centre = crate::boundary::ChunkPos {
        x: root[0].div_euclid(16),
        z: root[2].div_euclid(16),
    };
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter((-1..=1).flat_map(|dx| {
            (-1..=1).map(move |dz| {
                (
                    crate::boundary::ChunkPos {
                        x: centre.x + dx,
                        z: centre.z + dz,
                    },
                    0,
                )
            })
        })));
    // The mature trees have already been harvested. Only the registered
    // sapling predicts the future obstruction; no current crown can veto it.
    for cleared in [original_root, access] {
        for x in cleared[0] - 1..=cleared[0] + 1 {
            for z in cleared[2] - 1..=cleared[2] + 1 {
                for y in cleared[1]..=cleared[1] + 5 {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
    assert!(planned_cells(&state, &project, "oak").is_some());
    let dirt = block(&state, "minecraft:dirt", &[]);
    let sapling = block(&state, "minecraft:oak_sapling", &[("stage", "1")]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    state.world.set_block(root[0], root[1], root[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert!(
        planned_cells(&state, &project, "oak").is_none(),
        "future planted crown must exclude this site/corridor before it grows"
    );
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(sapling),
        "admission never cuts the sapling"
    );
    let at = crate::boundary::BlockPos {
        x: root[0],
        y: root[1],
        z: root[2],
    };
    let growth = (0..128)
        .find_map(|seed| {
            crate::plants::bonemeal(&state, at, seed).filter(|plan| plan.writes.len() > 1)
        })
        .expect("native tree growth plan");
    for (at, block) in growth.writes {
        state.world.set_block(at.x, at.y, at.z, block);
    }
    assert!(super::super::forestry::protected_tree(&state, root));
    assert!(
        !super::super::forestry::clearable(&state, root),
        "grown managed tree remains protected"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.settlements.get(id).unwrap().project.is_none());
}

#[test]
fn a_sapling_planted_first_keeps_its_future_crown_out_of_the_house_site() {
    planted_sapling_before_admission(false);
}

#[test]
fn a_sapling_planted_first_keeps_its_future_crown_out_of_the_access_corridor() {
    planted_sapling_before_admission(true);
}
