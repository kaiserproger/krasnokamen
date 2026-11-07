use super::tests::{attempt, complete, fixture, fund, open_plan, start};
use super::*;
use mc_script::ScriptWorldBounds;

fn certificate(record: &mut SettlementRecord, kind: BuildingKind, level: u8, natural_zone: bool) {
    record.building_sequence += 1;
    let id = BuildingId((record.id.0 << 32) | record.building_sequence);
    record.buildings.insert(
        id,
        buildings::Building {
            id,
            settlement: record.id,
            kind,
            level,
            bounds: ScriptWorldBounds::new([30, 200, 30], [32, 203, 32]).unwrap(),
            work_points: Vec::new(),
            revision: 1,
            natural_zone,
        },
    );
}

#[test]
fn active_stone_stairs_do_not_mask_the_project_wood_shortfall() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "barracks", 1);
    start(&mut state, id, "barracks", 1, None);
    let plank = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_planks").unwrap())
        .unwrap();
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.item_id == plank {
            *stack = mc_data::ItemStack::EMPTY;
        }
    }
    assert_eq!(needed_gatherer(&state, id), Some(Profession::Lumberjack));
    assert_eq!(active_project_wood(&state, id), Some("oak"));
}

#[test]
fn partial_birch_house_bill_drives_the_same_family_as_funded_construction() {
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    let materials = fund(&mut state, "house", 1);
    let birch_bill = species_bill(&state, &materials, "birch").unwrap();
    let replacements: Vec<_> = state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .map(|stack| {
            let mut stack = stack.clone();
            if !stack.is_empty() {
                stack.item_id = species_item(&state, stack.item_id, "birch").unwrap();
                if state.registry.items.name_of(stack.item_id).unwrap().path() == "birch_log" {
                    stack.count = 19;
                }
            }
            stack
        })
        .collect();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots = replacements;
    assert!(!super::super::construction_crafting::can_fund(
        &state,
        &birch_bill,
        &executor::source_images(&state, id),
    ));
    assert!(matches!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued { .. }
    ));
    let waiting = waiting(&state, id);
    let house = waiting
        .iter()
        .find(|kind| kind.blueprint == "house")
        .unwrap();
    assert!(
        house
            .needs
            .iter()
            .any(|need| need.item == "minecraft:birch_log")
    );
    assert!(
        house
            .needs
            .iter()
            .all(|need| !need.item.starts_with("minecraft:oak_"))
    );
    assert_eq!(needed_gatherer(&state, id), Some(Profession::Lumberjack));

    let birch_log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:birch_log").unwrap())
        .unwrap();
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .iter_mut()
        .find(|stack| stack.item_id == birch_log)
        .unwrap()
        .count = 36;
    plan_until_started(&mut state, id);
    complete(&mut state, id, worker);
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .any(|building| building.kind == BuildingKind::House)
    );
    assert!(state.plugin_block_writes.iter().any(|(pos, _)| {
        state
            .world
            .block_at(pos.x, pos.y, pos.z)
            .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
            .is_some_and(|block| block.block.id.path() == "white_bed")
    }));
}

#[test]
fn optional_lamps_do_not_borrow_the_worker_funding_the_next_house() {
    use crate::settlements::allocator::tests::block;
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    request_build(&mut state, id, "storage").unwrap();
    complete(&mut state, id, worker);
    state.entities.set_pos(worker, [1.5, 201.0, 2.5]);
    state
        .world
        .set_block(3, 200, 8, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(3, y, 8, block(&state, "minecraft:birch_log", &[]));
    }
    state
        .world
        .set_block(3, 205, 8, block(&state, "minecraft:birch_leaves", &[]));
    state.world.fold_overlay();
    buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([2, 200, 8], [3, 206, 8]).unwrap(),
        [2, 201, 8],
    )
    .unwrap();
    let v = state.entities.row_mut(worker).villager.as_mut().unwrap();
    v.data.profession = Profession::Lumberjack;
    v.job_site = Some(crate::boundary::BlockPos { x: 2, y: 201, z: 8 });
    v.job_site_dimension = state.dimension.name().into();
    let stock: Vec<_> = [
        ("birch_log", 10),
        ("cobblestone", 64),
        ("white_wool", 3),
        ("coal", 1),
        ("birch_fence", 2),
        ("torch", 1),
    ]
    .into_iter()
    .map(|(name, count)| {
        mc_data::ItemStack::new(
            state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
                .unwrap(),
            count,
        )
    })
    .collect();
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    slots[..stock.len()].clone_from_slice(&stock);
    let images = executor::source_images(&state, id);
    assert_eq!(needed_gatherer(&state, id), Some(Profession::Lumberjack));
    assert_eq!(
        street_site(&state, state.settlements.get(id).unwrap(), &images)
            .expect("a funded optional lamp is genuinely available")
            .project
            .blueprint,
        layout::LAMP
    );
    state.tick = PLAN_CADENCE;
    tick(&mut state);
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "optional infrastructure must not monopolize housing's material worker"
    );
    assert_eq!(
        state
            .entities
            .row(worker)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Lumberjack
    );
}

#[test]
fn funded_seed24_candidate_starts_without_borrowing_its_priority_gatherer() {
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    certificate(
        state.settlements.get_mut(id).unwrap(),
        BuildingKind::Storage,
        1,
        false,
    );
    certificate(
        state.settlements.get_mut(id).unwrap(),
        BuildingKind::LumberCamp,
        1,
        true,
    );
    let job = [30, 201, 30];
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .buildings
        .values_mut()
        .find(|building| building.kind == BuildingKind::LumberCamp)
        .unwrap()
        .work_points
        .push(job);
    let v = state.entities.row_mut(worker).villager.as_mut().unwrap();
    v.data.profession = Profession::Lumberjack;
    v.job_site = Some(crate::boundary::BlockPos {
        x: job[0],
        y: job[1],
        z: job[2],
    });
    v.job_site_dimension = state.dimension.name().into();
    let builder = crate::settlements::allocator::tests::villager(&mut state, [2.5, 201.0, 2.5]);
    let entity = state.entities.body(builder).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let stock = [
        ("oak_log", 34),
        ("bread", 16),
        ("raw_iron", 6),
        ("coal", 7),
        ("cobblestone", 264),
        ("oak_planks", 2),
        ("stick", 1),
        ("oak_fence", 1),
        ("torch", 3),
        ("white_wool", 4),
        ("shears", 1),
        ("birch_log", 83),
        ("dirt", 16),
    ];
    let stacks: Vec<_> = stock
        .into_iter()
        .flat_map(|(name, count)| {
            let item = state
                .registry
                .items
                .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
                .unwrap();
            (0..count)
                .step_by(64)
                .map(move |offset| mc_data::ItemStack::new(item, (count - offset).min(64)))
        })
        .collect();
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    slots[..stacks.len()].clone_from_slice(&stacks);
    assert!(has_funded_candidate(&state, id));
    assert_eq!(needed_gatherer(&state, id), Some(Profession::Lumberjack));
    plan(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(
        record.project.is_some() || record.construction_cursor != 0,
        "the funded planner must search sites instead of returning at the gatherer guard"
    );
    plan_until_started(&mut state, id);
    assert!(
        state.settlements.get(id).unwrap().project.is_some(),
        "funded construction must not remain idle behind the priority gatherer guard"
    );
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Nitwit;
    assign_builder(&mut state, id);
    assert_eq!(
        state
            .entities
            .row(worker)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Lumberjack
    );
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    assign_builder(&mut state, id);
    assert_eq!(
        state
            .entities
            .row(builder)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder
    );
}
#[test]
fn iron_demand_tracks_open_house_and_real_shears_or_wool() {
    let (mut state, id, worker) = fixture();
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    state.settlements.get_mut(id).unwrap().pending_order = Some(OwnerOrder {
        blueprint: "house".into(),
        level: 1,
        building: None,
    });
    assert_eq!(iron_demand(&state, id, None), 2);
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Builder;
    assert_eq!(
        iron_demand(&state, id, None),
        2,
        "demand does not require an eligible resident"
    );
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    let wool = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:white_wool").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = mc_data::ItemStack::new(wool, 1);
    assert_eq!(
        iron_demand(&state, id, None),
        2,
        "partial bed wool still needs shears"
    );
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = mc_data::ItemStack::new(wool, 3);
    assert_eq!(
        iron_demand(&state, id, None),
        0,
        "the wool bill is already satisfied"
    );
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = mc_data::ItemStack::EMPTY;
    let shears = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:shears").unwrap())
        .unwrap();
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .inventory[0] = Some(mc_data::ItemStack::new(shears, 1));
    assert_eq!(
        iron_demand(&state, id, None),
        0,
        "a usable tool removes the iron target"
    );
    let detached = state.entities.row_mut(worker).villager.take().unwrap();
    assert_eq!(
        iron_demand(&state, id, None),
        2,
        "detached tool is not visible in the resident index"
    );
    assert_eq!(
        iron_demand(&state, id, Some(&detached)),
        0,
        "the current worker's shears stop excess smelting"
    );
}

#[test]
fn autonomous_needs_cover_every_authored_kind_and_ignore_natural_zones() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get_mut(id).unwrap();
    assert_eq!(needed_kinds(record, 10).next(), Some(BuildingKind::Storage));
    certificate(record, BuildingKind::Storage, 1, false);
    for _ in 0..4 {
        certificate(record, BuildingKind::House, 3, false);
    }
    certificate(record, BuildingKind::Mine, 1, true);
    certificate(record, BuildingKind::LumberCamp, 1, true);
    for kind in [
        BuildingKind::Workshop(Profession::Farmer),
        BuildingKind::LumberCamp,
        BuildingKind::Mine,
        BuildingKind::Workshop(Profession::Builder),
        BuildingKind::Barracks,
        BuildingKind::Tower,
        BuildingKind::Wall,
    ] {
        assert_eq!(needed_kinds(record, 10).next(), Some(kind));
        certificate(record, kind, 1, false);
    }
    assert_eq!(needed_kinds(record, 10).next(), None);
    for (kind, threshold) in [
        (BuildingKind::Barracks, 6),
        (BuildingKind::Tower, 8),
        (BuildingKind::Wall, 10),
    ] {
        let building = record
            .buildings
            .values()
            .find(|building| building.kind == kind)
            .unwrap()
            .clone();
        record.buildings.remove(&building.id);
        assert_eq!(needed_kinds(record, threshold - 1).next(), None);
        assert_eq!(needed_kinds(record, threshold).next(), Some(kind));
        record.buildings.insert(building.id, building);
    }
    let authored: Vec<_> = blueprints::all(&state.registry)
        .iter()
        .map(|blueprint| blueprint.kind)
        .collect();
    assert_eq!(authored.len(), 10);
    assert!(
        authored
            .into_iter()
            .filter(|kind| *kind != BuildingKind::Workshop(Profession::Shepherd))
            .all(|kind| {
                state
                    .settlements
                    .get(id)
                    .unwrap()
                    .buildings
                    .values()
                    .any(|building| building.kind == kind && !building.natural_zone)
            })
    );
}

fn plan_until_started(state: &mut TickState, id: SettlementId) {
    for _ in 0..2000 {
        plan(state, id);
        if state.settlements.get(id).unwrap().project.is_some() {
            return;
        }
    }
    panic!("bounded site search did not find the fixture's free area");
}

/// Seed 87122 replay: a funded hillside village never started its Storage in
/// 100k ticks. Candidate sites sat on the bell's own plane, and only 12 of
/// 61k columns had free ground anywhere within ten blocks of it. The planner
/// must follow the terrain to a flat site above or below the bell.
#[test]
fn funded_planner_follows_terrain_to_a_free_site_off_the_bell_plane() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    // Raise the plan area five blocks everywhere but the chest corner: the
    // bell plane (y=200) has no clearable headroom in any footprint.
    for x in -16..32 {
        for z in -16..32 {
            if (0..4).contains(&x) && (0..4).contains(&z) {
                continue;
            }
            for y in 201..=205 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    state.world.fold_overlay();
    plan_until_started(&mut state, id);
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.blueprint, "storage");
    assert_eq!(
        project.origin[1], 205,
        "the raised surface, not the bell plane"
    );
}

/// Seed 87122 replay: the builder slot went to an unemployed resident whose
/// only house door was blocked by a tree crown. It sat in a ten-cell pocket
/// with the project active and every free villager idle. A resident that cannot
/// walk to storage is passed over for one that can.
#[test]
fn builder_choice_skips_a_resident_who_cannot_walk_to_storage() {
    builder_choice_skips_the_trapped_resident(false);
}

/// Buildings QA farm world (freeze 73e1c530): villagers and the allocator spend
/// the whole per-tick route allowance before the planning pass runs, so every
/// reachability probe was refused and no resident was ever made the builder
/// (0/226 placed for 55 minutes). The one-shot probe does not draw on the
/// shared tick allowance: an exhausted tick still picks the reachable resident.
#[test]
fn builder_choice_does_not_depend_on_the_shared_tick_route_allowance() {
    builder_choice_skips_the_trapped_resident(true);
}

fn builder_choice_skips_the_trapped_resident(starved_tick_budget: bool) {
    use crate::settlements::allocator::tests::villager;
    let (mut state, id, trapped) = fixture();
    state
        .entities
        .row_mut(trapped)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    // Shut the first resident in a stone cell at (10, 201..202, 10).
    for x in 9..=11 {
        for z in 9..=11 {
            for y in 201..=203 {
                if (x, z) != (10, 10) || y == 203 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    state.entities.set_pos(trapped, [10.5, 201.0, 10.5]);
    let free = villager(&mut state, [4.5, 201.0, 4.5]);
    let entity = state.entities.body(free).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    state
        .entities
        .row_mut(free)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Mason;
    if starved_tick_budget {
        for kind in [
            crate::tick_budget::Kind::Decision,
            crate::tick_budget::Kind::PathNode,
        ] {
            state.tick_budget.exhaust_for_test(kind);
        }
    }
    assign_builder(&mut state, id);
    let profession = |slot| {
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession
    };
    assert_eq!(profession(trapped), Profession::None);
    assert_eq!(profession(free), Profession::Builder);
}

#[test]
fn inactive_incumbent_yields_to_an_active_builder_and_construction_advances() {
    use crate::boundary::{ChangeSet, ChunkPos};
    use crate::settlements::allocator::tests::villager;
    let (mut state, id, incumbent) = fixture();
    let inactive = ChunkPos { x: 1, z: 0 };
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter([(inactive, 0)]));
    for x in 16..32 {
        for z in 0..16 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..224 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    state.world.fold_overlay();
    state.entities.set_pos(incumbent, [20.5, 201.0, 2.5]);
    state
        .entities
        .row_mut(incumbent)
        .villager
        .as_mut()
        .unwrap()
        .activity = crate::villager::Activity::Idle;
    let candidate = villager(&mut state, [1.5, 201.0, 2.5]);
    let entity = state.entities.body(candidate).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    let v = state.entities.row_mut(candidate).villager.as_mut().unwrap();
    v.data.profession = Profession::Mason;
    v.activity = crate::villager::Activity::Work;
    assert!(!state.world.is_active(inactive.x, inactive.z));
    assert!(state.world.is_active(0, 0));
    start(&mut state, id, "storage", 1, None);
    fund(&mut state, "storage", 1);
    let handle = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap()
        .structure
        .clone();
    for now in 0..400 {
        state.tick = now;
        state.tick_budget.reset(now);
        tick(&mut state);
        crate::villager::brain::tick(&mut state, &mut ChangeSet::default());
        state.world.fold_overlay();
    }
    let status = executor::status(&state, id, &handle).unwrap();
    let total: u64 = status.stages.iter().map(|stage| stage.work_units).sum();
    eprintln!("INACTIVE_BUILDER progress={}/{total}", status.watermark);
    assert!(
        status.watermark > 0,
        "inactive incumbent keeps the funded Storage at 0/{total}"
    );
    assert_eq!(
        state
            .entities
            .row(incumbent)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::None,
        "the inactive resident must not be selected again"
    );
    assert_eq!(
        state
            .entities
            .row(candidate)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder
    );
}

#[test]
fn funded_owner_order_starts_the_single_project_without_world_or_stock_debit() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    fund(&mut state, "watch_tower", 1);
    let before_stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let before_block = state.world.block_at(8, 201, 8);
    request_build(&mut state, id, "watch_tower").unwrap();
    let record = state.settlements.get(id).unwrap();
    let project = record
        .project
        .as_ref()
        .expect("funded owner order starts immediately");
    assert_eq!(project.blueprint, "watch_tower");
    assert_eq!(project.level, 1);
    assert!(
        !project.structure.is_empty(),
        "canonical structure authority"
    );
    assert!(executor::status(&state, id, &project.structure).is_ok());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, before_stock);
    assert_eq!(state.world.block_at(8, 201, 8), before_block);
    assert!(
        state.plugin_block_writes.is_empty(),
        "admission never places blocks"
    );
}

#[test]
fn pending_owner_order_blocks_autonomy_then_starts_after_funding() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert!(
        request_build(&mut state, id, "house").is_ok(),
        "unfunded order is queued"
    );
    assert!(state.settlements.get(id).unwrap().project.is_none());
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    fund(&mut state, "storage", 1);
    for _ in 0..200 {
        plan(&mut state, id);
    }
    assert!(
        state.settlements.get(id).unwrap().project.is_none(),
        "autonomy must wait"
    );
    fund(&mut state, "house", 1);
    plan_until_started(&mut state, id);
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.blueprint, "house");
    assert!(project.ordered);
}

#[test]
fn funded_pending_owner_order_searches_sites_in_bounded_planning_passes() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    assert!(matches!(
        request_build(&mut state, id, "farm").unwrap(),
        OrderOutcome::Queued { .. }
    ));
    // Fill every lot of the plan area through the terrain search band; roads,
    // verges and the plaza stay open, and the storage chest stays reachable
    // for the funding check.
    let anchor = state.settlements.get(id).unwrap().anchor;
    let lots =
        move |x: i32, z: i32| layout::cell(anchor, x, z) == layout::Cell::Lot && (x, z) != (1, 1);
    for x in -16..32 {
        for z in -16..32 {
            if lots(x, z) {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    fund(&mut state, "farm", 1);

    let before = state.settlements.get(id).unwrap().clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        pending_reason(&state, id).as_deref(),
        Some("waiting for a site")
    );
    assert_eq!(state.settlements.get(id).unwrap(), &before);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);

    plan(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.construction_cursor, SITE_ATTEMPTS);
    assert_eq!(record.pending_order, before.pending_order);
    assert!(record.project.is_none(), "autonomy must not take the slot");

    // Lots open beyond the first bounded search. Reporting a reason is
    // observational, even when a suitable site now exists.
    for x in -16..32 {
        for z in 16..32 {
            if lots(x, z) {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
    state.world.fold_overlay();
    let before_reason = state.settlements.get(id).unwrap().clone();
    assert_eq!(
        pending_reason(&state, id).as_deref(),
        Some("waiting for a site")
    );
    assert_eq!(state.settlements.get(id).unwrap(), &before_reason);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);

    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(record.pending_order.is_none());
    let project = record.project.as_ref().unwrap();
    assert_eq!(project.blueprint, "farm");
    assert_eq!(project.level, 1);
    assert!(project.ordered);
    assert!(
        project.origin[2] >= 16,
        "site must be on the newly opened lots"
    );
}

#[test]
fn active_project_queues_owner_build_and_invalid_upgrade_keeps_it() {
    let (mut state, id, _) = fixture();
    start(&mut state, id, "storage", 1, None);
    fund(&mut state, "storage", 1);
    let before = state.settlements.get(id).unwrap().project.clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: false,
        }
    );
    let record = state.settlements.get(id).unwrap();
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 1, None)
    );
    assert!(request_upgrade(&mut state, id, BuildingId(123)).is_err());
    assert!(request_build(&mut state, id, "not-a-blueprint").is_err());
    let record = state.settlements.get(id).unwrap();
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 1, None)
    );
    assert_eq!(record.project, before);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.plugin_block_writes.is_empty());
}

#[test]
fn requested_upgrade_queues_without_materials_then_starts_after_funding() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    complete(&mut state, id, slot);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    assert!(upgrade_available(&state, id, building));
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .fill(mc_data::ItemStack::EMPTY);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let writes = state.plugin_block_writes.len();
    let outcome = request_upgrade(&mut state, id, building).unwrap();
    assert!(
        matches!(outcome, OrderOutcome::Queued { ref reason, replaced: false } if reason.starts_with("not enough:"))
    );
    let record = state.settlements.get(id).unwrap();
    assert!(record.project.is_none());
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 2, Some(building))
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(state.plugin_block_writes.len(), writes);
    fund(&mut state, "house", 2);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(record.pending_order.is_none());
    let project = record.project.as_ref().unwrap();
    assert_eq!(project.blueprint, "house");
    assert_eq!(project.building, Some(building));
    assert_eq!(project.level, 2);
    assert!(project.ordered);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    complete(&mut state, id, slot);
    let record = state.settlements.get(id).unwrap();
    assert!(record.project.is_none());
    assert_eq!(record.buildings[&building].level, 2);
    assert_eq!(record.buildings.len(), 1);
}

#[test]
fn queued_upgrade_targets_next_level_after_active_upgrade_finishes() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    complete(&mut state, id, slot);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    fund(&mut state, "house", 2);
    assert_eq!(
        request_upgrade(&mut state, id, building).unwrap(),
        OrderOutcome::Started
    );
    let active = state.settlements.get(id).unwrap().project.clone();
    assert_eq!(active.as_ref().unwrap().level, 2);
    assert_eq!(
        request_upgrade(&mut state, id, building).unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: false,
        }
    );
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project, active);
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 3, Some(building))
    );
    complete(&mut state, id, slot);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        2
    );
    fund(&mut state, "house", 3);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(record.pending_order.is_none());
    let project = record.project.as_ref().unwrap();
    assert_eq!(
        (project.blueprint.as_str(), project.level, project.building),
        ("house", 3, Some(building))
    );
    assert!(project.ordered);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(
        request_upgrade(&mut state, id, building).unwrap_err(),
        "No blueprint upgrade available for this building."
    );
    complete(&mut state, id, slot);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&building].level,
        3
    );
}

#[test]
fn stale_pending_upgrade_is_dropped_and_autonomous_planning_resumes() {
    for remove_building in [true, false] {
        let (mut state, id, slot) = fixture();
        open_plan(&mut state);
        start(&mut state, id, "house", 1, None);
        fund(&mut state, "house", 1);
        complete(&mut state, id, slot);
        let building = *state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .keys()
            .next()
            .unwrap();
        state
            .block_entities
            .get_mut(&(1, 201, 1))
            .unwrap()
            .slots
            .fill(mc_data::ItemStack::EMPTY);
        assert!(matches!(
            request_upgrade(&mut state, id, building).unwrap(),
            OrderOutcome::Queued {
                replaced: false,
                ..
            }
        ));
        let record = state.settlements.get_mut(id).unwrap();
        if remove_building {
            record.buildings.remove(&building);
        } else {
            record.buildings.get_mut(&building).unwrap().level = 3;
        }
        fund(&mut state, "storage", 1);
        let stock = state.block_entities[&(1, 201, 1)].slots.clone();
        plan_until_started(&mut state, id);
        let record = state.settlements.get(id).unwrap();
        assert!(
            record.pending_order.is_none(),
            "remove_building={remove_building}"
        );
        let project = record.project.as_ref().unwrap();
        assert_eq!(project.blueprint, "storage");
        assert!(!project.ordered);
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    }
}

#[test]
fn autonomous_upgrade_requires_blueprint_provenance_and_materials() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    complete(&mut state, id, slot);
    let house = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    let record = state.settlements.get_mut(id).unwrap();
    record.owner = None;
    for kind in [
        BuildingKind::Storage,
        BuildingKind::Workshop(Profession::Farmer),
        BuildingKind::LumberCamp,
        BuildingKind::Mine,
        BuildingKind::Workshop(Profession::Builder),
    ] {
        certificate(record, kind, 1, false);
    }
    assert_eq!(needed_kinds(record, 1).next(), None);
    let player_certificate = record
        .buildings
        .values()
        .find(|building| building.kind == BuildingKind::Storage)
        .unwrap()
        .id;
    assert!(!upgrade_available(&state, id, player_certificate));
    plan(&mut state, id);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    fund(&mut state, "house", 2);
    plan(&mut state, id);
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.building, Some(house));
    assert_eq!(project.level, 2);
    complete(&mut state, id, slot);
    assert_eq!(
        state.settlements.get(id).unwrap().buildings[&house].level,
        2
    );
}

#[test]
fn partial_delivery_funds_a_blueprint_proven_upgrade_without_mutating_stock() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "storage", 1, None);
    fund(&mut state, "storage", 1);
    complete(&mut state, id, slot);
    for [x, y, z] in buildings::storage_points(&state, id) {
        if let Some(container) = state.block_entities.get_mut(&(x, y, z)) {
            container.slots.fill(mc_data::ItemStack::EMPTY);
        }
    }
    fund(&mut state, "storage", 2);
    let plank = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_planks").unwrap())
        .unwrap();
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let index = slots.iter().position(|s| s.item_id == plank).unwrap();
    let carried = slots[index].clone();
    slots[index] = mc_data::ItemStack::EMPTY;
    let stock = slots.clone();
    assert!(!has_funded_candidate(&state, id));
    assert!(candidate_funded_by_delivery(&state, id, &[Some(carried)]));
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.settlements.get(id).unwrap().project.is_none());
}

#[test]
fn an_already_funded_candidate_does_not_send_each_harvested_log_home() {
    for active in [false, true] {
        let (mut state, id, _) = fixture();
        fund(&mut state, "storage", 1);
        if active {
            start(&mut state, id, "storage", 1, None);
            assert!(super::super::construction_crafting::can_fund(
                &state,
                &active_project_bill(&state, id).unwrap(),
                &executor::source_images(&state, id),
            ));
        } else {
            assert!(has_funded_candidate(&state, id));
        }
        let log = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
            .unwrap();
        let stock = state.block_entities[&(1, 201, 1)].slots.clone();
        assert!(
            !candidate_funded_by_delivery(&state, id, &[Some(mc_data::ItemStack::new(log, 1))]),
            "a funded project must not force a partial delivery (active={active})",
        );
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    }
}

#[test]
fn invalid_or_stale_owner_targets_leave_no_request_to_obstruct_planning() {
    let (mut state, id, _) = fixture();
    open_plan(&mut state);
    let record = state.settlements.get_mut(id).unwrap();
    certificate(record, BuildingKind::Mine, 1, true);
    let mine = *record.buildings.keys().next().unwrap();
    let revision = record.revision;
    assert!(!upgrade_available(&state, id, mine));
    state.settlements.take_images();
    assert!(request_build(&mut state, id, "not-a-blueprint").is_err());
    assert!(request_upgrade(&mut state, id, BuildingId(123)).is_err());
    assert!(request_upgrade(&mut state, id, mine).is_err());
    assert_eq!(state.settlements.get(id).unwrap().revision, revision);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    assert!(state.settlements.take_images().is_empty());
    fund(&mut state, "storage", 1);
    plan_until_started(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .blueprint,
        "storage"
    );
}

#[test]
fn funded_owner_order_without_a_free_site_leaves_no_saved_request_or_cursor() {
    let (mut state, id, _) = fixture();
    fund(&mut state, "watch_tower", 1);
    // No lot fits the fixture's one loaded chunk, and the plaza and roads are
    // solid through the whole terrain band.
    for x in 0..16 {
        for z in 0..16 {
            if (x, z) != (1, 1) {
                for y in 201..=216 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    let record = state.settlements.get(id).unwrap();
    let revision = record.revision;
    let cursor = record.construction_cursor;
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    state.settlements.take_images();
    let error = request_build(&mut state, id, "watch_tower").unwrap_err();
    assert!(
        error.contains("No free loaded construction site"),
        "{error}"
    );
    let record = state.settlements.get(id).unwrap();
    assert!(record.project.is_none());
    assert_eq!(record.revision, revision);
    assert_eq!(record.construction_cursor, cursor);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.settlements.take_images().is_empty());
}

#[test]
fn unfunded_house_skips_to_funded_farm_without_debiting_stock() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "storage", 1, None);
    fund(&mut state, "storage", 1);
    complete(&mut state, id, slot);
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .any(|building| building.kind == BuildingKind::Storage && !building.natural_zone),
        "storage is really built, not certified"
    );
    // Order is unchanged: the house is still the top need.
    assert_eq!(
        needed_kinds(state.settlements.get(id).unwrap(), 1).next(),
        Some(BuildingKind::House)
    );
    // No sheep in the fixture, so wool has no source here.
    // Fund only the later kind's real materials: no wool, no bed.
    fund(&mut state, "farm", 1);
    let house_materials = blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .materials
        .clone();
    assert!(
        !super::super::construction_crafting::can_fund(
            &state,
            &house_materials,
            &executor::source_images(&state, id)
        ),
        "house must be unfundable without wool"
    );
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    let project = record
        .project
        .as_ref()
        .expect("planner must start the next fundable need");
    assert_eq!(
        project.blueprint, "farm",
        "unfunded house must not block a funded later need"
    );
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots,
        before,
        "selection must not debit stock"
    );
    let status = project_status(&state, id).join("\n");
    assert!(
        status.contains("house"),
        "waiting explanation names the skipped kind:\n{status}"
    );
    assert!(
        status.contains("white_wool"),
        "waiting explanation names the missing wool:\n{status}"
    );
    assert!(
        status.to_lowercase().contains("no source"),
        "waiting explanation reports no wool source:\n{status}"
    );
}

#[test]
fn funded_house_retains_priority_over_later_needs() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    start(&mut state, id, "storage", 1, None);
    fund(&mut state, "storage", 1);
    complete(&mut state, id, slot);
    fund(&mut state, "house", 1);
    let house_materials = blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .materials
        .clone();
    assert!(
        super::super::construction_crafting::can_fund(
            &state,
            &house_materials,
            &executor::source_images(&state, id)
        ),
        "exact house stock funds the house"
    );
    let before = state.block_entities[&(1, 201, 1)].slots.clone();
    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    let project = record.project.as_ref().expect("funded house must start");
    assert_eq!(
        project.blueprint, "house",
        "a truly funded house keeps priority"
    );
    assert_eq!(
        state.block_entities[&(1, 201, 1)].slots,
        before,
        "selection must not debit stock"
    );
}

#[test]
fn colored_wool_funds_and_builds_matching_bed_without_mixing() {
    let (mut state, id, builder) = fixture();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    open_plan(&mut state);
    fund(&mut state, "house", 1);
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
            .unwrap()
    };
    let white_bed = item("white_bed");
    let red_wool = item("red_wool");
    let blue_wool = item("blue_wool");
    let planks = item("oak_planks");
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    let bed = slots
        .iter()
        .position(|stack| stack.item_id == white_bed)
        .unwrap();
    slots[bed] = mc_data::ItemStack::new(red_wool, 2);
    let empty = slots.iter().position(mc_data::ItemStack::is_empty).unwrap();
    slots[empty] = mc_data::ItemStack::new(blue_wool, 1);
    let empty = slots.iter().position(mc_data::ItemStack::is_empty).unwrap();
    slots[empty] = mc_data::ItemStack::new(planks, 3);
    assert!(wool_demand(&state, id), "mixed wool cannot satisfy a bed");
    assert!(
        funding_error(
            &state,
            id,
            &blueprints::get(&state.registry, "house")
                .unwrap()
                .level(1)
                .unwrap()
                .materials
        )
        .is_some()
    );
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[bed].count = 3;
    assert!(
        !wool_demand(&state, id),
        "three matching wool satisfy the bed"
    );
    request_build(&mut state, id, "house").unwrap();
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    let level = blueprints::get(&state.registry, "house")
        .unwrap()
        .level(1)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(project.rotation));
    let beds: Vec<_> = level
        .blocks
        .iter()
        .filter_map(|(offset, block)| {
            state
                .registry
                .blocks
                .by_id(*block)
                .filter(|entry| entry.block.id.path() == "white_bed")
                .map(|_| level.world(project.origin, *offset))
        })
        .collect();
    assert_eq!(beds.len(), 2);
    complete(&mut state, id, builder);
    assert!(
        beds.iter().any(|&[x, y, z]| {
            super::super::allocator::bed_pair(
                &state.world,
                &state.registry,
                crate::boundary::BlockPos { x, y, z },
            )
            .is_some()
        }),
        "the placed bed is a valid matched pair"
    );
    for [x, y, z] in beds {
        let placed = state.world.block_at(x, y, z).unwrap();
        assert_eq!(
            state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(placed))
                .unwrap()
                .block
                .id
                .path(),
            "red_bed"
        );
    }
    assert_eq!(
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| stack.item_id == red_wool)
            .map(|stack| stack.count)
            .sum::<i32>(),
        0
    );
}

#[test]
fn colored_sheep_is_the_house_wool_source() {
    let (mut state, id, worker) = fixture();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla");
    state.registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(Some(&root)).unwrap());
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    let sheep = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:sheep").unwrap())
        .unwrap();
    let entity = state.spawn_mob(sheep, [8.5, 201.0, 8.5], 8.0);
    let slot = state.entities.slot_of(entity).unwrap();
    crate::tick::set_wool(
        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance),
        14,
    );
    let iron = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_ingot").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[0] = mc_data::ItemStack::new(iron, 2);
    let planks = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_planks").unwrap())
        .unwrap();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots[1] =
        mc_data::ItemStack::new(planks, 3);
    state.settlements.get_mut(id).unwrap().pending_order = Some(OwnerOrder {
        blueprint: "house".into(),
        level: 1,
        building: None,
    });
    assert!(wool_demand(&state, id));
    let house = waiting(&state, id);
    let bed = house
        .iter()
        .flat_map(|kind| &kind.needs)
        .find(|need| need.item.ends_with("_bed"))
        .unwrap();
    assert_eq!(bed.item, "minecraft:red_bed");
    assert_eq!(
        bed.no_source, None,
        "red sheep and real iron supply red wool"
    );
}

/// Merge several blueprints' level-one materials into the fixture chest.
fn fund_all(state: &mut TickState, blueprints_: &[&str]) {
    let mut total: Vec<(u32, u32)> = Vec::new();
    for name in blueprints_ {
        for (item, count) in &blueprints::get(&state.registry, name)
            .unwrap()
            .level(1)
            .unwrap()
            .materials
        {
            match total.iter_mut().find(|(id, _)| id == item) {
                Some((_, sum)) => *sum += count,
                None => total.push((*item, *count)),
            }
        }
    }
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    let mut index = 0;
    for (item, mut quantity) in total {
        while quantity > 0 {
            let count = quantity.min(64);
            slots[index] = mc_data::ItemStack::new(item, count as i32);
            quantity -= count;
            index += 1;
        }
    }
}

fn autonomous_storage(state: &mut TickState, id: SettlementId) {
    open_plan(state);
    fund_all(state, &["storage", "house"]);
    plan_until_started(state, id);
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.blueprint, "storage");
    assert!(!project.ordered, "the planner's project is autonomous");
}

#[test]
fn owner_order_preempts_an_autonomous_project_that_placed_nothing() {
    let (mut state, id, _) = fixture();
    autonomous_storage(&mut state, id);
    let old = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap()
        .structure
        .clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Started
    );
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(project.blueprint, "house");
    assert!(project.ordered);
    assert_ne!(project.structure, old);
    assert_eq!(
        executor::status(&state, id, &old).unwrap().state,
        mc_script::ScriptStructureState::Cancelled
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.plugin_block_writes.is_empty());
}

#[test]
fn owner_order_is_busy_once_an_autonomous_project_placed_a_block() {
    let (mut state, id, slot) = fixture();
    autonomous_storage(&mut state, id);
    let handle = state
        .settlements
        .get(id)
        .unwrap()
        .project
        .as_ref()
        .unwrap()
        .structure
        .clone();
    for _ in 0..400 {
        super::tests::attempt(&mut state, id, slot);
        if executor::status(&state, id, &handle)
            .unwrap()
            .completed_work_units
            > 0
        {
            break;
        }
    }
    assert!(
        executor::status(&state, id, &handle)
            .unwrap()
            .completed_work_units
            > 0
    );
    let before = state.settlements.get(id).unwrap().project.clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let writes = state.plugin_block_writes.len();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: false,
        }
    );
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project, before);
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 1, None)
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(state.plugin_block_writes.len(), writes);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .structure,
        handle
    );
}

#[test]
fn ordered_project_finishes_before_replacement_order_starts() {
    let (mut state, id, slot) = fixture();
    open_plan(&mut state);
    fund_all(&mut state, &["storage", "house"]);
    assert_eq!(
        request_build(&mut state, id, "storage").unwrap(),
        OrderOutcome::Started
    );
    let before = state.settlements.get(id).unwrap().project.clone();
    assert!(before.as_ref().unwrap().ordered);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        request_build(&mut state, id, "watch_tower").unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: false,
        }
    );
    let pending = state
        .settlements
        .get(id)
        .unwrap()
        .pending_order
        .as_ref()
        .unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("watch_tower", 1, None)
    );
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: true,
        }
    );
    for _ in 0..200 {
        plan(&mut state, id);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project, before);
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("house", 1, None)
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.plugin_block_writes.is_empty());
    complete(&mut state, id, slot);
    assert!(state.settlements.get(id).unwrap().project.is_none());
    fund(&mut state, "house", 1);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    plan_until_started(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert!(record.pending_order.is_none());
    let project = record.project.as_ref().unwrap();
    assert_eq!(
        (project.blueprint.as_str(), project.level, project.building),
        ("house", 1, None)
    );
    assert!(project.ordered);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

#[test]
fn unfunded_owner_order_keeps_the_autonomous_project() {
    let (mut state, id, _) = fixture();
    autonomous_storage(&mut state, id);
    let before = state.settlements.get(id).unwrap().project.clone();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let outcome = request_build(&mut state, id, "watch_tower").unwrap();
    assert!(
        matches!(outcome, OrderOutcome::Queued { ref reason, replaced: false } if reason.starts_with("not enough:"))
    );
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.project, before);
    let pending = record.pending_order.as_ref().unwrap();
    assert_eq!(
        (pending.blueprint.as_str(), pending.level, pending.building),
        ("watch_tower", 1, None)
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.plugin_block_writes.is_empty());
}

#[test]
fn preempting_returns_the_builders_hauled_materials_before_funding_the_order() {
    let (mut state, id, slot) = fixture();
    autonomous_storage(&mut state, id);
    let stock_total = |state: &TickState| -> i32 {
        state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .map(|stack| stack.count)
            .sum()
    };
    let before = stock_total(&state);
    // The builder hauls the autonomous project's materials out of the chest.
    crate::villager::building::tick(&mut state, id);
    let carried = |state: &TickState| -> i32 {
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .iter()
            .flatten()
            .map(|stack| stack.count)
            .sum()
    };
    assert!(carried(&state) > 0, "the haul left the chest");
    assert!(stock_total(&state) < before);
    request_build(&mut state, id, "house").expect("hauled stock still funds the order");
    assert_eq!(carried(&state), 0, "carry went back to storage");
    assert_eq!(stock_total(&state), before, "nothing lost or duplicated");
}

#[test]
fn upgrade_is_offered_for_an_ordered_house_that_replaced_an_autonomous_site() {
    let (mut state, id, slot) = fixture();
    autonomous_storage(&mut state, id);
    fund_all(&mut state, &["house"]);
    request_build(&mut state, id, "house").unwrap();
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    complete(&mut state, id, slot);
    let (bid, building) = state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .iter()
        .find(|(_, b)| b.kind == BuildingKind::House)
        .map(|(id, b)| (*id, b.clone()))
        .expect("house certified");
    let house = blueprints::get(&state.registry, "house").unwrap();
    let turn = Rotation::from_quarter_turns(project.rotation);
    let placed = |level: u8| {
        house
            .level(level)
            .unwrap()
            .rotated(turn)
            .bounds(project.origin)
    };
    assert_eq!(
        building.bounds,
        placed(1),
        "certificate follows the executor origin and street-facing rotation"
    );
    assert!(executor::built_at(&state, id, "house", project.origin));
    assert!(upgrade_available(&state, id, bid), "L2 upgrade row exists");
    // The row is orderable and the upgraded house offers L3 the same way.
    for level in [2, 3] {
        fund(&mut state, "house", level);
        request_upgrade(&mut state, id, bid).unwrap();
        complete(&mut state, id, slot);
        let upgraded = &state.settlements.get(id).unwrap().buildings[&bid];
        assert_eq!(upgraded.level, level);
        assert_eq!(upgraded.bounds, placed(level));
        assert_eq!(upgrade_available(&state, id, bid), level < 3);
    }
}

/// Real-client QA farm L1_blockmatch read 12 air cells at origin.x + 6. L1 was
/// whole (every planned cell placed, structure committed); the surplus QA stock
/// let the planner start the L2 upgrade at once, and its clear stage removed
/// exactly the L1 east-wall cells that L2 does not keep.
#[test]
fn farm_level_one_is_fully_placed_and_a_funded_upgrade_clears_only_its_removals() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "farm", 1, None);
    fund(&mut state, "farm", 1);
    complete(&mut state, id, slot);
    let origin = [3, 200, 3];
    let farm = blueprints::get(&state.registry, "farm").unwrap();
    let level1 = farm
        .level(1)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(0));
    let level2 = farm
        .level(2)
        .unwrap()
        .rotated(Rotation::from_quarter_turns(0));
    let at = |offset: &[i8; 3]| level1.world(origin, *offset);
    let mismatched = |state: &TickState| -> std::collections::BTreeSet<[i32; 3]> {
        level1
            .blocks
            .iter()
            .map(|(offset, block)| (at(offset), block))
            .filter(|(pos, block)| state.world.block_at(pos[0], pos[1], pos[2]) != Some(block.0))
            .map(|(pos, _)| pos)
            .collect()
    };
    assert!(
        level1.blocks.iter().any(|(offset, _)| offset[0] == 6),
        "the blueprint has a last column"
    );
    assert!(
        mismatched(&state).is_empty(),
        "certified L1 holds every blueprint cell, last column included"
    );
    // Surplus stock and every other need already met: the planner upgrades.
    let record = state.settlements.get_mut(id).unwrap();
    record.owner = None;
    for kind in [
        BuildingKind::Storage,
        BuildingKind::House,
        BuildingKind::LumberCamp,
        BuildingKind::Mine,
        BuildingKind::Workshop(Profession::Builder),
    ] {
        certificate(record, kind, 1, false);
    }
    fund(&mut state, "farm", 2);
    plan(&mut state, id);
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    assert_eq!((project.level, project.ordered), (2, false));
    for _ in 0..4096 {
        let status = executor::status(&state, id, &project.structure).unwrap();
        if status.current_stage_index >= 1 {
            break;
        }
        attempt(&mut state, id, slot);
    }
    let removals: std::collections::BTreeSet<[i32; 3]> = level2
        .removals
        .iter()
        .map(|offset| level2.world(origin, *offset))
        .collect();
    assert_eq!(mismatched(&state), removals, "the clear stage only removes");
    assert_eq!(removals.len(), 12);
    assert!(removals.iter().all(|pos| pos[0] == origin[0] + 6));
}

/// Buildings QA farm world (freeze bc04b53b): a certified farm and an autonomous
/// L2 project appeared while the L1 structure was believed to be Running. A
/// building certifies only through a Committed structure, and the single
/// project slot keeps the planner from admitting L+1 before L is Committed.
#[test]
fn running_structure_neither_certifies_nor_admits_an_upgrade() {
    let (mut state, id, slot) = fixture();
    start(&mut state, id, "farm", 1, None);
    fund(&mut state, "farm", 1);
    for _ in 0..64 {
        attempt(&mut state, id, slot);
    }
    let project = state.settlements.get(id).unwrap().project.clone().unwrap();
    let status = executor::status(&state, id, &project.structure).unwrap();
    assert_eq!(status.state, ScriptStructureState::Running);
    assert!(status.completed_work_units > 0, "the build is under way");
    let record = state.settlements.get_mut(id).unwrap();
    record.owner = None;
    for kind in [
        BuildingKind::Storage,
        BuildingKind::House,
        BuildingKind::LumberCamp,
        BuildingKind::Mine,
        BuildingKind::Workshop(Profession::Builder),
    ] {
        certificate(record, kind, 1, false);
    }
    let certified = state.settlements.get(id).unwrap().buildings.len();
    fund(&mut state, "farm", 2);
    assert!(!finish_project(&mut state, id), "Running never certifies");
    plan(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.buildings.len(), certified, "no farm certificate yet");
    let kept = record.project.as_ref().unwrap();
    assert_eq!(
        (kept.level, kept.structure.as_str()),
        (1, project.structure.as_str())
    );
}

#[test]
fn owner_order_does_not_preempt_after_a_completed_clear_stage() {
    let (mut state, id, slot) = fixture();
    state
        .world
        .set_block(3, 201, 3, state.registry.states.stone);
    state.world.fold_overlay();
    start(&mut state, id, "storage", 1, None);
    let record = state.settlements.get_mut(id).unwrap();
    record.project.as_mut().unwrap().ordered = false;
    let project = record.project.clone().unwrap();
    for _ in 0..400 {
        let snapshot = executor::status(&state, id, &project.structure).unwrap();
        if snapshot.current_stage_index > 0 && snapshot.completed_work_units == 0 {
            break;
        }
        attempt(&mut state, id, slot);
    }
    let snapshot = executor::status(&state, id, &project.structure).unwrap();
    assert!(snapshot.current_stage_index > 0, "{snapshot:?}");
    assert_eq!(snapshot.completed_work_units, 0);
    fund(&mut state, "house", 1);
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: "busy".into(),
            replaced: false,
        }
    );
    plan(&mut state, id);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .structure,
        project.structure
    );
}

#[test]
fn sole_resource_worker_changes_to_the_missing_housing_material_through_ticks() {
    use crate::settlements::allocator::tests::block;
    let (mut state, id, worker) = fixture();
    let enclosed = worker;
    state.entities.set_pos(enclosed, [14.5, 201.0, 1.5]);
    state
        .entities
        .row_mut(enclosed)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::None;
    for x in 13..=15 {
        for z in 0..=2 {
            for y in 201..=204 {
                if x != 14 || z != 1 || y == 204 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    let worker = super::super::allocator::tests::villager(&mut state, [1.5, 201.0, 2.5]);
    let entity = state.entities.body(worker).id;
    super::super::bind(&mut state, id, entity).unwrap();
    fund(&mut state, "house", 1);
    let cobble = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:cobblestone").unwrap())
        .unwrap();
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.item_id == cobble {
            stack.count = 24;
        }
    }
    for x in 5..=14 {
        for z in 5..=14 {
            for y in 190..201 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
        }
    }
    state
        .world
        .set_block(3, 200, 8, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(3, y, 8, block(&state, "minecraft:oak_log", &[]));
    }
    state
        .world
        .set_block(3, 205, 8, block(&state, "minecraft:oak_leaves", &[]));
    state.world.fold_overlay();
    let camp = buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([2, 200, 8], [3, 206, 8]).unwrap(),
        [2, 201, 8],
    )
    .unwrap();
    buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::Mine,
        ScriptWorldBounds::new([5, 190, 5], [14, 202, 14]).unwrap(),
        [6, 201, 6],
    )
    .unwrap();
    let v = state.entities.row_mut(worker).villager.as_mut().unwrap();
    v.data.profession = Profession::Lumberjack;
    v.job_site = Some(crate::boundary::BlockPos { x: 2, y: 201, z: 8 });
    v.job_site_dimension = state.dimension.name().into();
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .contains_key(&camp)
    );
    assert!(matches!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued { .. }
    ));
    let mut mined = false;
    for _ in 0..6000 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        mined |= state
            .entities
            .row(worker)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession
            == Profession::Miner
            && !output.changes.blocks.is_empty();
        let delivered = state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| stack.item_id == cobble)
            .map(|stack| stack.count)
            .sum::<i32>()
            >= 43;
        if mined && (delivered || state.settlements.get(id).unwrap().project.is_some()) {
            assert_eq!(
                state
                    .entities
                    .row(enclosed)
                    .villager
                    .as_ref()
                    .unwrap()
                    .data
                    .profession,
                Profession::None,
                "a disconnected adult must not become a phantom miner and exclude the reachable gatherer"
            );
            return;
        }
    }
    panic!(
        "the sole reachable gatherer must mine and deliver the missing stone: mined={mined}, pos={:?}, worker={:?}, stock={:?}",
        state.entities.pos(worker),
        state.entities.row(worker).villager,
        state.block_entities[&(1, 201, 1)].slots,
    );
}

#[test]
fn funded_house_keeps_an_ordinary_recruit_in_the_builder_career() {
    for profession in [Profession::None, Profession::Mason] {
        let (mut state, id, worker) = fixture();
        open_plan(&mut state);
        fund(&mut state, "house", 1);
        state
            .entities
            .row_mut(worker)
            .villager
            .as_mut()
            .unwrap()
            .data
            .profession = profession;
        request_build(&mut state, id, "house").unwrap();
        state.tick = PLAN_CADENCE;
        tick(&mut state);
        assert_eq!(
            state
                .settlements
                .get(id)
                .unwrap()
                .project
                .as_ref()
                .unwrap()
                .borrowed_builder,
            None,
            "ordinary recruits take the existing builder career; only resource gatherers are borrowed"
        );
        complete(&mut state, id, worker);
        assert_eq!(
            state
                .entities
                .row(worker)
                .villager
                .as_ref()
                .unwrap()
                .data
                .profession,
            Profession::Builder,
            "finishing an authored building must not erase an ordinary recruit's builder career"
        );
        assert!(!state.plugin_block_writes.is_empty());
    }
}

#[test]
fn funded_house_can_borrow_the_only_untraded_gatherer_as_builder() {
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    fund(&mut state, "house", 1);
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Lumberjack;
    request_build(&mut state, id, "house").unwrap();
    state.tick = PLAN_CADENCE;
    tick(&mut state);
    assert_eq!(
        state
            .entities
            .row(worker)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Builder
    );
    let encoded = serde_json::to_vec(state.settlements.get(id).unwrap()).unwrap();
    let restored: SettlementRecord = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(
        restored.project.as_ref().unwrap().borrowed_builder,
        Some((state.entities.row(worker).uuid, Profession::Lumberjack))
    );
    *state.settlements.get_mut(id).unwrap() = restored;
    complete(&mut state, id, worker);
    assert_eq!(
        state
            .entities
            .row(worker)
            .villager
            .as_ref()
            .unwrap()
            .data
            .profession,
        Profession::Lumberjack,
        "an ordered house must release its borrowed gatherer, not permanently strand extraction"
    );
}

#[test]
fn borrowed_lumberjack_replenishes_an_unfunded_active_project_through_ticks() {
    use crate::settlements::allocator::tests::block;
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    fund(&mut state, "lumber_camp", 1);
    let replacements: Vec<_> = state.block_entities[&(1, 201, 1)]
        .slots
        .iter()
        .map(|stack| {
            let mut stack = stack.clone();
            if !stack.is_empty() {
                stack.item_id = species_item(&state, stack.item_id, "birch").unwrap();
            }
            stack
        })
        .collect();
    state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots = replacements;
    state
        .world
        .set_block(3, 200, 8, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(3, y, 8, block(&state, "minecraft:birch_log", &[]));
    }
    state
        .world
        .set_block(3, 205, 8, block(&state, "minecraft:birch_leaves", &[]));
    state
        .world
        .set_block(2, 200, 9, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(2, y, 9, block(&state, "minecraft:oak_log", &[]));
    }
    state
        .world
        .set_block(2, 205, 9, block(&state, "minecraft:oak_leaves", &[]));
    state.world.fold_overlay();
    buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([2, 200, 8], [3, 206, 8]).unwrap(),
        [2, 201, 8],
    )
    .unwrap();
    let v = state.entities.row_mut(worker).villager.as_mut().unwrap();
    v.data.profession = Profession::Lumberjack;
    v.job_site = Some(crate::boundary::BlockPos { x: 2, y: 201, z: 8 });
    v.job_site_dimension = state.dimension.name().into();
    request_build(&mut state, id, "lumber_camp").unwrap();
    state.tick = PLAN_CADENCE;
    tick(&mut state);
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert_eq!(
        project.borrowed_builder,
        Some((state.entities.row(worker).uuid, Profession::Lumberjack))
    );
    let plank = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:birch_planks").unwrap())
        .unwrap();
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.item_id == plank {
            stack.count -= 9;
        }
    }
    let project = state.settlements.get(id).unwrap().project.as_ref().unwrap();
    assert!(
        !executor::missing(&state, id, &project.structure)
            .unwrap()
            .is_empty()
    );
    let birch = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:birch_log").unwrap())
        .unwrap();
    let mut restored = false;
    let mut harvested = false;
    let mut resumed = false;
    for _ in 0..36000 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let v = state.entities.row(worker).villager.as_ref().unwrap();
        restored |= v.data.profession == Profession::Lumberjack;
        harvested |= v.data.profession == Profession::Lumberjack
            && (output.changes.blocks.iter().any(|change| {
                change.position.x == 3
                    && change.position.z == 8
                    && (201..205).contains(&change.position.y)
                    && change.state == state.registry.states.air
            }) || v
                .inventory
                .iter()
                .flatten()
                .any(|stack| stack.item_id == birch && stack.count > 0));
        resumed |= restored && harvested && v.data.profession == Profession::Builder;
        if state.settlements.get(id).unwrap().project.is_none() {
            assert!(
                restored && harvested && resumed,
                "the real worker must gather, deliver and resume construction"
            );
            assert!(
                state
                    .settlements
                    .get(id)
                    .unwrap()
                    .buildings
                    .values()
                    .any(|building| building.kind == BuildingKind::LumberCamp
                        && !building.natural_zone)
            );
            assert_eq!(
                state.world.block_at(2, 201, 9),
                Some(block(&state, "minecraft:oak_log", &[])),
                "the active birch project must not consume the earlier oak tree"
            );
            return;
        }
    }
    panic!(
        "active project did not recover: restored={restored} harvested={harvested} resumed={resumed}"
    );
}

#[test]
fn active_wood_shortfall_can_retask_a_borrowed_miner_through_ticks() {
    use crate::settlements::allocator::tests::block;
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    fund(&mut state, "lumber_camp", 1);
    state
        .world
        .set_block(3, 200, 8, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(3, y, 8, block(&state, "minecraft:oak_log", &[]));
    }
    state
        .world
        .set_block(3, 205, 8, block(&state, "minecraft:oak_leaves", &[]));
    state.world.fold_overlay();
    buildings::certify_zone(
        &mut state,
        id,
        BuildingKind::LumberCamp,
        ScriptWorldBounds::new([2, 200, 8], [3, 206, 8]).unwrap(),
        [2, 201, 8],
    )
    .unwrap();
    state
        .entities
        .row_mut(worker)
        .villager
        .as_mut()
        .unwrap()
        .data
        .profession = Profession::Miner;
    request_build(&mut state, id, "lumber_camp").unwrap();
    state.tick = PLAN_CADENCE;
    tick(&mut state);
    assert_eq!(
        state
            .settlements
            .get(id)
            .unwrap()
            .project
            .as_ref()
            .unwrap()
            .borrowed_builder,
        Some((state.entities.row(worker).uuid, Profession::Miner))
    );
    let plank = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_planks").unwrap())
        .unwrap();
    for stack in &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots {
        if stack.item_id == plank {
            stack.count -= 9;
        }
    }
    assert_eq!(
        needed_gatherer(&state, id),
        Some(Profession::Lumberjack),
        "stock withdrawal must leave a wood shortfall"
    );
    let mut restored = false;
    let mut harvested = false;
    let mut resumed = false;
    for _ in 0..36000 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let v = state.entities.row(worker).villager.as_ref().unwrap();
        restored |= v.data.profession == Profession::Lumberjack;
        harvested |= restored
            && output.changes.blocks.iter().any(|change| {
                change.position.x == 3
                    && change.position.z == 8
                    && (201..205).contains(&change.position.y)
                    && change.state == state.registry.states.air
            });
        resumed |= harvested && v.data.profession == Profession::Builder;
        if state.settlements.get(id).unwrap().project.is_none() {
            assert!(
                restored && harvested && resumed,
                "worker must gather, deliver and resume"
            );
            return;
        }
    }
    panic!(
        "borrowed miner did not recover: restored={restored}, harvested={harvested}, resumed={resumed}, worker={:?}",
        state.entities.row(worker).villager.as_ref().unwrap()
    );
}

#[path = "construction_delivery_tests.rs"]
mod delivery_ownership;
