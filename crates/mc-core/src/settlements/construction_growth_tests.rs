use super::tests::{
    attempt, bind_project, complete, fixture, fund, open_plan, plan_until_project, request_started,
    start,
};
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
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    (state, id)
}

pub(super) fn stock_buildings(state: &mut TickState, levels: &[(&str, u8)]) {
    let mut bill = std::collections::BTreeMap::<u32, u32>::new();
    for &(name, level) in levels {
        for &(item, count) in &blueprints::get(&state.registry, name)
            .unwrap()
            .level(level)
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

/// The round-3 cadence proof, on the four-tick admission clock: only a
/// settlement whose own admission pass left an unfinished walk holding a legal
/// lot is stepped off that clock, one site slice a tick, so the walk still
/// finishes inside a quiet window between street changes and the tick keeps a
/// slice for the order turn (a change clears the flag until the next admission
/// pass, so the change lands 35 ticks into every 40). At the bare cadence (one
/// pass per 20 ticks) the same scripted village restarts its walk at every
/// change and never admits. The four-tick admission clock itself is asserted by
/// `only_a_holding_settlement_steps_and_construction_keeps_its_four_tick_clock`.
#[test]
fn a_holding_walk_steps_off_the_admission_clock_and_finishes_between_changes() {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    super::tests::complete(&mut state, id, builder);
    super::growth_tests::stock_buildings(&mut state, &[("spanish_house", 1), ("farm", 1)]);
    let anchor = state.settlements.get(id).unwrap().anchor;
    // The village's own work changes the walk's frontage list 35 ticks into
    // every 40: a change clears the flag until that settlement's next 20-tick
    // admission pass (review round 2 N3), so the quiet window has to outlast
    // one walk (~10 stepping slices) without landing on an admission pass.
    let roads: Vec<[i32; 2]> = (-14..=14)
        .flat_map(|x| (-14..=14).map(move |z| [x, z]))
        .filter(|[x, z]| matches!(layout::cell(anchor, *x, *z), layout::Cell::Road))
        .map(|[x, z]| [anchor[0] + x, anchor[2] + z])
        .collect();
    assert!(roads.len() > 8, "the plan has roads to change");
    let mut next_road = 0;
    let mut admitted = None;
    for tick in 1..=200u64 {
        state.tick = tick;
        if tick % 40 == 35 {
            let [x, z] = roads[next_road % roads.len()];
            next_road += 1;
            for y in 190..=202 {
                state.world.set_block(x, y, z, state.registry.states.stone);
            }
            state.world.fold_overlay();
        }
        let mut budget = site_turn::SiteBudget::default();
        super::planner_tick(&mut state, &mut budget);
        // A stepping tick takes one of the two slices and leaves a whole one
        // for the order turn and the replanting proofs.
        if !tick.is_multiple_of(4) {
            assert!(
                budget.slices() >= 1,
                "tick {tick} spent the slice the order turn needs"
            );
        }
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            admitted = Some(tick);
            break;
        }
    }
    // Control (recorded, review round 2 N5): with the off-clock step removed
    // (`planner_tick`'s off-cadence branch returning at once) this fails with
    // "got None" - at the bare cadence the street changes restart the walk
    // before it can finish.
    assert!(
        admitted.is_some_and(|tick| tick <= 80),
        "the walk must finish inside a quiet window between street changes, got {admitted:?}"
    );
}

/// The cadence contract with more than one settlement (review round 3 C1-C5):
/// only the settlement whose own admission pass left an unfinished walk that
/// already holds a legal lot is stepped, one site slice a tick; another
/// settlement's frozen holding walk and a live walk that holds nothing are
/// never stepped, even though their chunks are simulated; construction still
/// admits at most once per four ticks, and every stepping tick leaves a whole
/// site slice for the order turn.
///
/// Control (recorded, review round 2 N5): with the pre-review every-tick
/// construction admission restored (an off-clock `building::tick` for every
/// settlement holding a project) this fails at tick 9 with "tick 9 admitted
/// construction off the clock" (1 call instead of 0).
#[test]
fn only_a_holding_settlement_steps_and_construction_keeps_its_four_tick_clock() {
    let (mut state, id, _builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    // The walks are seeded before any project exists: an active project's
    // executor reservation cuts the street plan's reachable frontages, and the
    // stepping walks need the whole plan to stay unfinished.
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("farm", 1), ("mason", 1)],
    );
    // A second settlement in a simulated chunk: what keeps it out of the step
    // is its own flag, not residency.
    let second = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "steppees".into(),
            "owner".into(),
            [24, 201, 8],
            48,
        )
        .expect("second settlement");
    state.tickets.insert(
        "cadence-second".into(),
        (crate::boundary::ChunkPos { x: 1, z: 0 }, 0),
    );
    state.refresh_active_chunks(0);
    assert!(
        state.world.is_active(0, 0) && state.world.is_active(1, 0),
        "both settlements must be simulated for the cadence test"
    );
    let frontages = |state: &TickState, id: SettlementId| {
        u32::try_from(
            layout::Network::for_construction(state, state.settlements.get(id).unwrap())
                .frontage()
                .len(),
        )
        .unwrap()
    };
    assert!(
        frontages(&state, id) > 4 * super::SITE_ATTEMPTS,
        "the fixture's walk must outlast the stepping slices"
    );
    // The kinds this fixture actually funds: a seeded walk of an unfunded kind
    // is never advanced, so the seed must be the planner's own list.
    let funded: Vec<&'static blueprints::Blueprint> = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let (kinds, count) = super::site_refusals::funded_kinds(&state, record, id, &images);
        let funded: Vec<_> = kinds.into_iter().take(count).flatten().collect();
        assert!(
            !funded.is_empty(),
            "the fixture funds at least one autonomous kind"
        );
        funded
    };
    // The fixture village's own walk on the open ground: unfinished and holding
    // a legal lot, the state its admission pass leaves when it must be stepped.
    // The sweep map only has to carry that state; which settlement holds it is
    // the flag under test.
    let holding = |state: &mut TickState, kinds: &[&'static blueprints::Blueprint]| {
        let mut sweeps = site_refusals::Sweeps::default();
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(state, id);
        let lots = (frontages(state, id) / 3).max(1);
        for blueprint in kinds {
            let pass = site_project(
                state,
                record,
                std::iter::once(*blueprint),
                &mut sweeps,
                record.construction_cursor,
                lots,
                &images,
            );
            assert!(
                pass.site.is_none() && pass.running && pass.waiting,
                "the open ground leaves {} unfinished and holding after {lots} lots: \
                 site={} running={} waiting={} cursor={} report={:?}",
                blueprint.id,
                pass.site.is_some(),
                pass.running,
                pass.waiting,
                pass.cursor,
                pass.report,
            );
        }
        sweeps
    };
    let mut stepping = holding(&mut state, &funded);
    stepping.stepping = true;
    state.settlements.site_sweep.insert(id, stepping);
    let frozen = holding(&mut state, &funded[..1]);
    assert!(
        !frozen.stepping,
        "a seeded settlement never steps by itself"
    );
    state.settlements.site_sweep.insert(second, frozen);
    // The hopeless case: a pass whose budget was spent before its first lot (the
    // order turn can take the last slice) leaves a live, unfinished walk that
    // holds nothing.
    let hopeless_id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "hopeless".into(),
            "owner".into(),
            [-24, 201, -8],
            48,
        )
        .expect("third settlement");
    let hopeless = {
        let mut sweeps = site_refusals::Sweeps::default();
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let pass = site_project(
            &state,
            record,
            std::iter::once(funded[0]),
            &mut sweeps,
            record.construction_cursor,
            0,
            &images,
        );
        assert!(
            pass.site.is_none() && pass.running && !pass.waiting,
            "a spent budget leaves a walk that holds nothing: {:?}",
            pass.report
        );
        sweeps
    };
    state.settlements.site_sweep.insert(hopeless_id, hopeless);
    let summary = |state: &TickState, id: SettlementId| {
        state
            .settlements
            .site_sweep
            .get(&id)
            .map(site_refusals::Sweeps::summary)
            .unwrap_or_default()
    };
    let frozen_before = summary(&state, second);
    assert!(
        frozen_before
            .iter()
            .any(|(_, next, frontages, holds, _)| *holds && *next < *frontages),
        "the frozen settlement carries a live walk that holds a lot"
    );
    let hopeless_before = summary(&state, hopeless_id);
    assert!(
        hopeless_before
            .iter()
            .any(|(_, next, frontages, holds, _)| *next == 0 && !holds && *frontages > 0),
        "the hopeless settlement carries a live walk that holds nothing"
    );
    let mut admissions = 0;
    let mut steps = 0;
    for tick in 1..=12u64 {
        // From tick 9 the settlement has a project, so the four-tick clock has
        // something to admit; until then the stepping assertions run on the
        // whole plan.
        if tick == 9 {
            start(&mut state, id, "spanish_house", 1, None);
        }
        state.tick = tick;
        let stepping_before = state
            .settlements
            .site_sweep
            .get(&id)
            .is_some_and(|sweeps| sweeps.stepping);
        crate::villager::building::ADMISSIONS.with(|count| count.set(0));
        site_refusals::EXAMINED.with(|count| count.set(0));
        let mut budget = site_turn::SiteBudget::default();
        super::planner_tick(&mut state, &mut budget);
        let calls = crate::villager::building::ADMISSIONS.with(std::cell::Cell::get);
        let examined = site_refusals::EXAMINED.with(std::cell::Cell::get);
        if tick.is_multiple_of(4) {
            admissions += calls;
            continue;
        }
        assert_eq!(calls, 0, "tick {tick} admitted construction off the clock");
        assert!(
            budget.slices() >= 1,
            "tick {tick} spent the slice the order turn needs"
        );
        assert!(
            examined <= super::SITE_ATTEMPTS,
            "tick {tick} examined {examined} lots instead of one slice"
        );
        if stepping_before {
            assert!(
                examined > 0,
                "tick {tick} stepped a holding settlement to nothing"
            );
            steps += u32::from(tick < 9);
        } else {
            assert_eq!(examined, 0, "tick {tick} stepped without a holding walk");
        }
        assert_eq!(
            summary(&state, second),
            frozen_before,
            "tick {tick} stepped a settlement whose flag is clear"
        );
        assert_eq!(
            summary(&state, hopeless_id),
            hopeless_before,
            "tick {tick} stepped a walk that holds nothing"
        );
    }
    assert!(
        steps >= 5,
        "the holding settlement stepped only {steps} ticks"
    );
    assert_eq!(
        admissions, 1,
        "construction admitted {admissions} times in twelve ticks"
    );
}

/// The stepping flag is armed and cleared only by the passes that own it
/// (review round 2 N1-N3), through those real paths rather than by hand:
/// the settlement's own admission pass arms it, a step that restarted a walk
/// clears it, an owner order's scan and a storehouse wait never arm it, and a
/// pass that cannot scan (the builder gate) clears an armed one.
#[test]
fn the_stepping_flag_is_armed_and_cleared_by_its_own_passes() {
    let (mut state, id, _builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    let armed = |state: &TickState| {
        state
            .settlements
            .site_sweep
            .get(&id)
            .is_some_and(|sweeps| sweeps.stepping)
    };
    let examined = || site_refusals::EXAMINED.with(std::cell::Cell::get);
    let set_flag = |state: &mut TickState, value: bool| {
        state.settlements.site_sweep.entry(id).or_default().stepping = value;
    };
    // N2 (storehouse): `storage::plan` shares the order's rule and never arms
    // the flag. Reaching its scan needs a full bank behind a standing store
    // (an empty new store makes `needed` false), so the rule is not driven
    // here; the order turn below covers the same no-arming rule on the path
    // that runs every tick.
    // The settlement's own admission pass arms it when it leaves an unfinished
    // walk that already holds a legal lot.
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("farm", 1), ("mason", 1)],
    );
    assert!(!plan_pass(
        &mut state,
        id,
        &mut site_turn::SiteBudget::default()
    ));
    assert!(armed(&state), "an unfinished holding walk arms the step");
    // N3: a street change restarts the walk inside the step, which clears it.
    // The change blocks a street column the walk really uses (near the far end
    // of the frontage list), and the test proves it shortened the frontage list
    // before asserting the flag.
    let frontages = |state: &TickState| {
        let record = state.settlements.get(id).unwrap();
        layout::Network::for_construction(state, record)
            .frontage()
            .len()
    };
    let before = frontages(&state);
    let cut = {
        let record = state.settlements.get(id).unwrap();
        let fronts = layout::Network::for_construction(&state, record).frontage();
        fronts[fronts.len().saturating_sub(3)].0
    };
    for y in cut[1] - 2..=cut[1] + 2 {
        state
            .world
            .set_block(cut[0], y, cut[2], state.registry.states.stone);
    }
    state.world.fold_overlay();
    let after = frontages(&state);
    assert!(
        after < before,
        "blocking the street column {cut:?} must shorten the walk ({before} -> {after})"
    );
    state.tick = 1;
    super::planner_tick(&mut state, &mut site_turn::SiteBudget::default());
    // The restarted walk still holds a legal lot while the flag is clear, so
    // `scan.waiting` is true: the only thing that can have cleared the flag is
    // the restart rule this asserts (review round 3 T1).
    let summary = |state: &TickState| {
        state
            .settlements
            .site_sweep
            .get(&id)
            .map(site_refusals::Sweeps::summary)
            .unwrap_or_default()
    };
    assert!(
        summary(&state)
            .iter()
            .any(|(_, next, frontages, holds, _)| *holds && next < frontages),
        "the step restarted a walk that still holds a lot: {:?}",
        summary(&state)
    );
    assert!(
        !armed(&state),
        "a step that restarted a walk clears the flag"
    );
    // The next admission pass re-arms the restarted walk once it holds a lot.
    assert!(!plan_pass(
        &mut state,
        id,
        &mut site_turn::SiteBudget::default()
    ));
    assert!(armed(&state), "the next pass re-arms the restarted walk");
    // N2 (owner order): the order's own scan really scans, opens no project,
    // leaves the farm walk unfinished and holding, and never arms the flag - so
    // the removed arming line (`stepping = pass.waiting && pass.site.is_none()`)
    // would have armed it (review round 3 T2).
    set_flag(&mut state, false);
    site_refusals::EXAMINED.with(|count| count.set(0));
    let scan = super::start_build(&mut state, id, "farm", 64);
    assert!(
        scan.is_err(),
        "a 64-lot order scan cannot finish this village's walk: {scan:?}"
    );
    assert!(examined() > 0, "the order's scan examined lots");
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "the order scan opened no project"
    );
    assert!(
        summary(&state)
            .iter()
            .any(|(kind, next, frontages, holds, _)| kind == "farm"
                && *holds
                && *next < *frontages),
        "the order scan left the farm walk unfinished and holding: {:?}",
        summary(&state)
    );
    assert!(!armed(&state), "an owner order's scan never arms the step");
    // N1: a pass that cannot reach the scan (the builder gate, with an active
    // project) clears an armed flag instead of leaving it standing.
    start(&mut state, id, "spanish_house", 1, None);
    set_flag(&mut state, true);
    assert!(!plan_pass(
        &mut state,
        id,
        &mut site_turn::SiteBudget::default()
    ));
    assert!(!armed(&state), "a pass that cannot scan clears the flag");
}

/// The work-budget pin (CIV-ADOPTED-CLOSURE-ROOT-TICK-COST): a step off the
/// admission clock reuses the funded verdict the settlement's own admission
/// pass proved, so a held walk's stepped ticks add no funding proof and no
/// recipe scan; the next admission pass re-proves and overwrites it.
#[test]
fn stepped_ticks_add_no_funding_proofs() {
    let (mut state, id, _builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    let armed = |state: &TickState| {
        state
            .settlements
            .site_sweep
            .get(&id)
            .is_some_and(|sweeps| sweeps.stepping)
    };
    // The kinds the walk must hold a lot for (the flag test's own seeding).
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("farm", 1), ("mason", 1)],
    );
    // The admission pass proves the kinds' bills and leaves a holding walk.
    assert!(!plan_pass(
        &mut state,
        id,
        &mut site_turn::SiteBudget::default()
    ));
    assert!(armed(&state), "the admission pass arms the step");
    // Count only the off-clock steps: ticks 1, 2, 3, 5, 6, 7 are not on the
    // four-tick construction clock and never on the 20-tick admission clock.
    crate::settlements::construction_crafting::reset_scan_stats();
    site_refusals::EXAMINED.with(|count| count.set(0));
    for tick in [1u64, 2, 3, 5, 6, 7] {
        state.tick = tick;
        super::planner_tick(&mut state, &mut site_turn::SiteBudget::default());
    }
    assert!(
        site_refusals::EXAMINED.with(std::cell::Cell::get) > 0,
        "the steps really examined lots"
    );
    assert!(armed(&state), "the walk still holds, so it keeps stepping");
    let work = crate::settlements::construction_crafting::scan_stats();
    assert_eq!(
        (work.funding_calls, work.memo_hits, work.recipes_scanned),
        (0, 0, 0),
        "a stepped tick runs no funding proof at all (a memo hit is still a proof \
         attempt): {work:?}"
    );
}

#[test]
fn an_autonomous_house_selected_first_keeps_its_slot_before_a_later_cultural_order() {
    let (mut state, id) = housing_fixture(false);
    // One house's worth of stock, doubled so a second lot's foundation can be
    // paid: the later church is still refused by the joint proof, so the
    // earlier house keeps its slot (D8).
    stock_buildings(&mut state, &[("spanish_house", 1)]);
    for stack in state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .iter_mut()
        .filter(|stack| stack.count > 0)
    {
        stack.count = (stack.count * 2).min(64);
    }
    plan_until_project(&mut state, id);
    let project = &state.settlements.get(id).unwrap().projects[0];
    assert_eq!(project.blueprint, "spanish_house");
    assert!(!project.ordered);
    // Keep the earlier autonomous house untouched across later order passes.
    state.day_time = 0;
    let result = request_build(&mut state, id, "spanish_church").unwrap();
    assert!(
        matches!(result, OrderOutcome::Queued { .. }),
        "the earlier house must retain its FIFO slot: {result:?}"
    );
    for _ in 0..40 {
        crate::tick::run_tick(&mut state, &[]);
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    let structure = record.projects[0].structure.clone();
    for _ in 0..80 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].structure, structure);
    assert_eq!(record.projects[0].blueprint, "spanish_house");
    assert_eq!(record.orders[0].blueprint, "spanish_church");
}

#[test]
fn a_pending_cultural_order_starts_before_a_later_autonomous_house() {
    let (mut state, id) = housing_fixture(false);
    state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .fill(mc_data::ItemStack::EMPTY);
    assert!(matches!(
        request_build(&mut state, id, "spanish_fortress").unwrap(),
        OrderOutcome::Queued { reason } if reason.starts_with("not enough:")
    ));
    stock_buildings(&mut state, &[("spanish_house", 1), ("spanish_fortress", 1)]);
    state.day_time = 0;
    for _ in 0..256 {
        crate::tick::run_tick(&mut state, &[]);
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
    }
    let record = &state.settlements.get(id).unwrap();
    let project = &record.projects[0];
    assert_eq!(project.blueprint, "spanish_fortress");
    assert!(project.ordered);
    assert!(record.orders.is_empty());
}

#[test]
fn a_real_spare_roofed_bed_keeps_the_cultural_order_reachable() {
    let (mut state, id) = housing_fixture(true);
    stock_buildings(&mut state, &[("spanish_church", 1)]);
    request_started(&mut state, id, "spanish_church");
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].blueprint,
        "spanish_church"
    );
}

fn house_without_site() -> (TickState, SettlementId) {
    let (mut state, id) = housing_fixture(false);
    stock_buildings(&mut state, &[("spanish_house", 1), ("farm", 1)]);
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
    let mut sweeps = site_refusals::Sweeps::default();
    assert!(
        site_project(
            &state,
            record,
            std::iter::once(house),
            &mut sweeps,
            0,
            u32::MAX,
            &images
        )
        .site
        .is_none(),
        "no loaded House site fits"
    );
    assert!(
        site_project(
            &state,
            record,
            std::iter::once(blueprints::get(&state.registry, "farm").unwrap()),
            &mut sweeps,
            0,
            u32::MAX,
            &images,
        )
        .site
        .is_some(),
        "a complete sweep finds the funded Farm a legal site"
    );
    (state, id)
}

#[test]
fn a_house_without_a_site_does_not_hide_a_funded_autonomous_farm() {
    let (mut state, id) = house_without_site();
    plan_until_project(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].blueprint,
        "farm"
    );
}

#[test]
fn a_house_without_a_site_does_not_hold_a_direct_farm_order() {
    let (mut state, id) = house_without_site();
    request_started(&mut state, id, "farm");
}

#[test]
fn a_house_without_a_site_does_not_hold_a_pending_farm_order() {
    let (mut state, id) = house_without_site();
    state.settlements.get_mut(id).unwrap().orders = vec![OwnerOrder {
        seq: 0,
        blueprint: "farm".into(),
        level: 1,
        building: None,
    }];
    for _ in 0..256 {
        assert!(plan_order(
            &mut state,
            id,
            &mut site_turn::SiteBudget::default()
        ));
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].blueprint, "farm");
    assert!(record.orders.is_empty());
}

fn roofed_house_beds(state: &TickState, id: SettlementId) -> usize {
    let mut beds = 0;
    for house in state.settlements.get(id).unwrap().buildings.values() {
        if house.kind != BuildingKind::House {
            continue;
        }
        for x in house.bounds.min[0]..=house.bounds.max[0] {
            for y in house.bounds.min[1]..=house.bounds.max[1] {
                for z in house.bounds.min[2]..=house.bounds.max[2] {
                    let pos = crate::boundary::BlockPos { x, y, z };
                    beds += usize::from(
                        super::super::allocator::bed_pair(&state.world, &state.registry, pos)
                            .is_some()
                            && super::super::allocator::roofed(&state.world, &state.registry, pos),
                    );
                }
            }
        }
    }
    beds
}

#[test]
fn funded_housing_upgrades_gain_spare_beds_when_new_buildings_have_no_lot() {
    for level in 1..=2 {
        let (mut state, id, builder) = fixture();
        // Autonomous upgrades bypass fund(level > 1), but still need real
        // canonical destruction loot for the standing house's replaced cells.
        state.registry = std::sync::Arc::new(
            crate::registry::CoreRegistry::load(Some(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/vanilla"),
            ))
            .unwrap(),
        );
        open_plan(&mut state);
        fund(&mut state, "spanish_house", 1);
        start(&mut state, id, "spanish_house", 1, None);
        complete(&mut state, id, builder);
        let house = *state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .keys()
            .next()
            .unwrap();
        if level == 2 {
            fund(&mut state, "spanish_house", 2);
            start(&mut state, id, "spanish_house", 2, Some(house));
            complete(&mut state, id, builder);
            let resident = super::super::allocator::tests::villager(&mut state, [1.5, 201.0, 3.5]);
            let entity = state.entities.body(resident).id;
            super::super::bind(&mut state, id, entity).unwrap();
        }
        assert_eq!(roofed_house_beds(&state, id), usize::from(level));
        assert_eq!(state.settlements.member_count(id), usize::from(level));

        // Retain the standing house's maximum footprint and dry Builder
        // approaches, but surround it with water: no new lot can fit.
        let water = block(&state, "minecraft:water", &[("level", "0")]);
        for x in -16..32 {
            for z in -16..32 {
                if !(0..17).contains(&x) || !(0..14).contains(&z) {
                    state.world.set_block(x, 201, z, water);
                }
            }
        }
        state.world.fold_overlay();
        stock_buildings(
            &mut state,
            &[
                ("spanish_house", 1),
                ("spanish_house", level + 1),
                ("farm", 1),
            ],
        );
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let farm = blueprints::get(&state.registry, "farm").unwrap();
        assert!(funded_variant(&state, id, &farm.level(1).unwrap().materials, &images).is_some());
        let mut sweeps = site_refusals::Sweeps::default();
        assert!(
            site_project(
                &state,
                record,
                std::iter::once(farm),
                &mut sweeps,
                0,
                u32::MAX,
                &images
            )
            .site
            .is_none(),
            "the funded missing kind has no legal lot"
        );
        let stock = state.block_entities[&(1, 201, 1)].slots.clone();
        plan(&mut state, id);
        let project = &state.settlements.get(id).unwrap().projects[0];
        assert_eq!(project.blueprint, "spanish_house");
        assert_eq!(project.building, Some(house));
        assert_eq!(project.level, level + 1);
        assert!(
            !project.ordered,
            "the real autonomous planner admitted the upgrade"
        );
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
        complete(&mut state, id, builder);
        assert_eq!(
            state.settlements.get(id).unwrap().buildings[&house].level,
            level + 1
        );
        assert_eq!(roofed_house_beds(&state, id), usize::from(level + 1));
        assert!(roofed_house_beds(&state, id) > state.settlements.member_count(id));
    }
}

#[derive(Clone, Default)]
pub(super) struct Warnings(pub(super) Arc<Mutex<Vec<String>>>);

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
        if event.metadata().fields().field("cause").is_none() {
            return;
        }
        struct Fields(String);
        impl tracing::field::Visit for Fields {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                use std::fmt::Write as _;
                write!(&mut self.0, "{}={value:?} ", field.name()).unwrap();
            }
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.0.lock().unwrap().push(fields.0);
    }
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

#[test]
fn a_stalled_project_reports_initial_and_repeated_stalls_without_a_builder() {
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
        2,
        "one initial stall and one failed recovery per no-progress episode: {causes:?}"
    );
    assert!(causes.iter().all(|cause| cause.contains("no_builder")));
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
    let project = state.settlements.get(id).unwrap().projects[0].clone();
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
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    retain_player_edit(&mut state, next);
    territory_elsewhere(&mut state, id, next);
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
        record.projects.is_empty(),
        "a permanently protected tree must not retain the project slot"
    );
    assert_eq!(record.orders[0].blueprint, "storage");
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
    let events = warnings.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].contains("cause=\"protected_tree_in_preparation\""));
}

#[test]
fn a_protected_tree_never_cancels_preparation_with_consumed_materials() {
    let (mut state, id, _, project, root) = protected_preparation();
    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    retain_player_edit(&mut state, next);
    territory_elsewhere(&mut state, id, next);
    // Clear cells are normally free. Exercise the defensive paid-work guard
    // using the canonical persisted structure record, without changing its stage.
    let key = format!("structure/{}", project.structure);
    let mut record = state
        .plugins
        .get_domain("@native-structures", &key)
        .unwrap()
        .clone();
    record["snapshot"]["consumed"] =
        serde_json::json!([{"resource": "minecraft:oak_log", "quantity": 1}]);
    state.plugins.set_domain("@native-structures", &key, record);
    let before = executor::status(&state, id, &project.structure).unwrap();
    assert!(!before.consumed.is_empty());
    assert!(executor::clearing(&state, id, &project.structure));
    let tree = state.world.block_at(root[0], root[1], root[2]);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let warnings = stall_preparation(&mut state, id);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].structure, project.structure);
    assert!(record.orders.is_empty());
    let after = executor::status(&state, id, &project.structure).unwrap();
    assert_ne!(after.state, ScriptStructureState::Cancelled);
    assert_eq!(after.consumed, before.consumed);
    assert_eq!(state.world.block_at(root[0], root[1], root[2]), tree);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    let events = warnings.0.lock().unwrap();
    assert_eq!(events.len(), 1);
    assert!(events[0].contains("cause=\"protected_tree_in_preparation\""));
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
    assert!(planned_cells(&state, &project, "oak", &mut Default::default()).is_ok());
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
    let planned = planned_cells(&state, &project, "oak", &mut Default::default());
    if corridor {
        // Owner D7: a planted sapling on the walking corridor is felled like a
        // tree on the lot, as the first sapling cell of the preparation.
        let (_, cells) = planned.expect("the corridor sapling is clearance work");
        assert!(cells.iter().any(|cell| {
            cell.at == root
                && cell.portion
                    == crate::plugin_structures::settlement_builds::PortionKind::Saplings
        }));
    } else {
        // The sapling beside the lot is no corridor work; its future crown over
        // the lot is the site check's refusal.
        let size = blueprints::get(&state.registry, &project.blueprint)
            .unwrap()
            .footprint(Rotation::from_quarter_turns(project.rotation));
        assert_eq!(
            site_refusals::check(
                &state,
                state.settlements.get(id).unwrap(),
                project.origin,
                size,
                None,
                &mut Default::default(),
                None,
            ),
            Err(site_refusals::Reason::PlantingProvenance),
            "future planted crown must exclude this site before it grows"
        );
    }
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
        !super::super::forestry::clearable(&state, root, true),
        "grown managed tree remains protected"
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
}

#[test]
fn a_sapling_planted_first_keeps_its_future_crown_out_of_the_house_site() {
    planted_sapling_before_admission(false);
}

#[test]
fn a_sapling_planted_first_on_the_access_corridor_is_felled_with_the_preparation() {
    planted_sapling_before_admission(true);
}

/// Seed-100 laptop trial 2 had funded, free lots, but every preparation
/// envelope enclosed a managed tree beside (not on) its winding access walk.
#[test]
fn managed_crown_in_empty_access_envelope_does_not_block_paid_house() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    let blueprint = blueprints::get(&state.registry, "spanish_house").unwrap();
    let record = state.settlements.get(id).unwrap();
    let mut network = layout::Network::for_construction(&state, record);
    let (project, root, envelope) = network
        .frontage()
        .into_iter()
        .find_map(|(front, normal)| {
            let lot = network.lot(blueprint, front, normal)?;
            let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
            if !site_free(&state, record, lot.origin, size) {
                return None;
            }
            let project = Project {
                settlement: id,
                blueprint: blueprint.id.clone(),
                level: 1,
                origin: lot.origin,
                rotation: lot.rotation,
                building: None,
                structure: String::new(),
                ordered: true,
                builder: None,
                order_seq: None,
            };
            let (bounds, _) =
                planned_cells(&state, &project, "oak", &mut Default::default()).ok()?;
            let route = network.construction_access(&project)?;
            let crown = super::super::forestry::PLANTED_CROWN_RADIUS as i32;
            let root = (bounds.min[0]..=bounds.max[0]).find_map(|x| {
                (bounds.min[2]..=bounds.max[2]).find_map(|z| {
                    let outside_lot = [0, 2].into_iter().any(|axis| {
                        let value = if axis == 0 { x } else { z };
                        value + crown < lot.origin[axis]
                            || value - crown >= lot.origin[axis] + i32::from(size[axis])
                    });
                    (outside_lot
                        && route
                            .iter()
                            .all(|at| (x - at[0]).abs() > crown || (z - at[2]).abs() > crown)
                        && state.world.block_at(x, 201, z) == Some(state.registry.states.air))
                    .then_some([x, 201, z])
                })
            })?;
            Some((project, root, bounds))
        })
        .expect("a bent access walk has empty space inside its rectangular envelope");
    drop(network);
    let sapling = block(&state, "minecraft:oak_sapling", &[("stage", "1")]);
    let dirt = block(&state, "minecraft:dirt", &[]);
    state.world.set_block(root[0], root[1] - 1, root[2], dirt);
    state.world.set_block(root[0], root[1], root[2], sapling);
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    assert!(super::super::forestry::planted_crown_overlaps(
        &state, envelope
    ));
    let size = blueprint.footprint(Rotation::from_quarter_turns(project.rotation));
    assert!(site_free(
        &state,
        state.settlements.get(id).unwrap(),
        project.origin,
        size
    ));
    assert!(
        super::super::forestry::cells(
            &state,
            &project,
            size,
            &mut super::super::forestry::Clearance::default()
        )
        .is_some()
    );
    let (bounds, cells) = planned_cells(&state, &project, "oak", &mut Default::default())
        .expect("a crown outside the lot and actual access walk must not veto their envelope");
    fund(&mut state, "spanish_house", 1);
    assert!(begin_cells(&mut state, project, bounds, cells));
    complete(&mut state, id, builder);
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .any(|b| {
                placement(&state, id, b).is_some_and(|(bp, _, _)| bp.id == "spanish_house")
            })
    );
    assert_eq!(
        state.world.block_at(root[0], root[1], root[2]),
        Some(sapling)
    );
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
}

fn retain_player_edit(state: &mut TickState, at: [i32; 3]) {
    state.block_log = crate::block_log::BlockLog::restore(
        crate::config::BlockLogSettings {
            retention_secs: 0,
            ..Default::default()
        },
        vec![crate::block_log::Delta {
            records: vec![crate::block_log::Record {
                seq: 0,
                tick: Some(1),
                time: 0,
                actor: Arc::from("player"),
                pos: at,
                change: crate::block_log::Change::Block {
                    before: state.registry.states.air,
                    after: state.world.block_at(at[0], at[1], at[2]).unwrap(),
                    entity: None,
                },
                rolled_back: false,
            }],
            marks: Vec::new(),
            coverage: 0,
        }],
    );
}

/// After admission, move the settlement's territory away from `at`: a player
/// edit there is outside the territory and keeps its protection (owner D27.4).
fn territory_elsewhere(state: &mut TickState, id: SettlementId, at: [i32; 3]) {
    let record = state.settlements.get_mut(id).unwrap();
    assert!(record.contains(at));
    record.anchor = [at[0] + 1_000, at[1], at[2]];
    record.radius = 8;
}

fn proven_lot_is_felled_and_built(sapling: bool, native: bool) {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    // The tree grows on the first legal lot in walking order; once the world
    // is regrown, every other lot is refused, so the ranked sweep takes it.
    let site = super::site_tests::first_legal_lot(&state, id, "spanish_fortress");
    let (lot, size) = (site.lot, site.size);
    let expected = Project {
        settlement: id,
        blueprint: "spanish_fortress".into(),
        level: 1,
        origin: lot.origin,
        rotation: lot.rotation,
        building: None,
        structure: String::new(),
        ordered: true,
        builder: None,
        order_seq: None,
    };
    let root = [
        lot.origin[0] + i32::from(size[0]) / 2,
        201,
        lot.origin[2] + i32::from(size[2]) / 2,
    ];
    super::super::forestry::tests::grown_world(&mut state, vec![root], "birch");
    if native {
        super::super::forestry::tests::grown_world(&mut state, Vec::new(), "birch");
        super::super::forestry::native_tree_tests::grow(&mut state, root, 5);
    } else if sapling {
        for x in root[0] - 1..=root[0] + 1 {
            for z in root[2] - 1..=root[2] + 1 {
                for y in root[1]..=root[1] + 4 {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
        let block = block(&state, "minecraft:birch_sapling", &[]);
        state.world.set_block(root[0], root[1], root[2], block);
    } else {
        // A grown, nonpersistent leaf where regeneration had air must use its
        // own placement-state evidence, not the agreeing trunk baseline.
        let at = [root[0] + 3, root[1] + 3, root[2]];
        let leaves = block(&state, "minecraft:birch_leaves", &[("persistent", "false")]);
        state.world.set_block(at[0], at[1], at[2], leaves);
    }
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    super::site_tests::pin_lot(&mut state, id, "spanish_fortress", &site);
    let bounds = super::super::forestry::lot_bounds(&state, &expected).unwrap();
    assert!(!super::super::forestry::clearable(&state, root, true));
    assert!(!super::super::forestry::Clearance::default().clearable(&state, root, None));
    assert!(super::super::forestry::Clearance::default().clearable(&state, root, Some(bounds)));
    assert_eq!(
        surface_y_for_lot(
            &state,
            200,
            root[0],
            root[2],
            Some(bounds),
            &mut super::super::forestry::Clearance::default()
        ),
        Some(200)
    );
    assert!(site_free(
        &state,
        state.settlements.get(id).unwrap(),
        lot.origin,
        size
    ));
    retain_player_edit(&mut state, root);
    assert!(!super::super::forestry::Clearance::default().clearable(&state, root, Some(bounds)));
    assert!(
        site_free(&state, state.settlements.get(id).unwrap(), lot.origin, size),
        "inside the territory the player's edit is the village's property (owner D27.4)"
    );
    state.block_log = Default::default();
    fund(&mut state, "spanish_fortress", 1);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    let original = state.world.block_at(root[0], root[1], root[2]);
    request_started(&mut state, id, "spanish_fortress");
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    assert_eq!(
        (project.origin, project.rotation),
        (expected.origin, expected.rotation)
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    assert_eq!(state.world.block_at(root[0], root[1], root[2]), original);
    assert!(executor::clearing(&state, id, &project.structure));

    // Proven natural cells and registered saplings must not trigger protected
    // cancellation when no adult can advance their preparation.
    state
        .entities
        .row_mut(builder)
        .villager
        .as_mut()
        .unwrap()
        .age = -1;
    let warnings = stall_preparation(&mut state, id);
    assert_eq!(
        state.settlements.get(id).unwrap().projects[0].structure,
        project.structure
    );
    assert!(
        !warnings
            .0
            .lock()
            .unwrap()
            .iter()
            .any(|event| event.contains("protected_tree_in_preparation"))
    );
    let villager = state.entities.row_mut(builder).villager.as_mut().unwrap();
    villager.age = 0;
    villager.data.profession = Profession::Builder;

    let next = executor::next_cell(&state, id, &project.structure)
        .unwrap()
        .unwrap();
    let previous = state.world.block_at(next[0], next[1], next[2]);
    retain_player_edit(&mut state, next);
    // Outside the territory the edit stops the debit (owner D27.4 keeps the
    // veto there); inside it the debit credits the cell.
    let record = state.settlements.get(id).unwrap().clone();
    territory_elsewhere(&mut state, id, next);
    let uuid = state.entities.row(builder).uuid;
    // The executor only advances a project for its bound builder (P2-5).
    bind_project(&mut state, id, &project.structure, builder);
    assert_eq!(
        executor::advance(&mut state, id, &project.structure, uuid),
        Err(mc_script::ScriptOperationFailure::Blocked)
    );
    assert_eq!(state.world.block_at(next[0], next[1], next[2]), previous);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    *state.settlements.get_mut(id).unwrap() = record;
    state.block_log = Default::default();
    let revision = state.settlements.get(id).unwrap().revision;

    for _ in 0..4096 {
        if !executor::clearing(&state, id, &project.structure) {
            break;
        }
        let next = executor::next_cell(&state, id, &project.structure)
            .unwrap()
            .unwrap();
        let previous = state.world.block_at(next[0], next[1], next[2]).unwrap();
        let counts = |state: &TickState| {
            let mut counts = std::collections::BTreeMap::<u32, i32>::new();
            for stack in &state.block_entities[&(1, 201, 1)].slots {
                if !stack.is_empty() {
                    *counts.entry(stack.item_id).or_default() += stack.count;
                }
            }
            counts
        };
        let mut expected_stock = counts(&state);
        state.tick += 20;
        for stack in super::super::forestry::drops(
            &state,
            next,
            previous,
            Some(bounds),
            &mut super::super::forestry::Clearance::verify(),
        )
        .unwrap()
        {
            *expected_stock.entry(stack.item_id).or_default() += stack.count;
        }
        state.tick -= 20;
        attempt(&mut state, id, builder);
        assert_eq!(
            counts(&state),
            expected_stock,
            "managed-tree drops at {next:?}"
        );
        if next == root {
            // The root keeps its registration until its whole trunk column is
            // empty; the retire pass after the loop removes it.
            let record = state.settlements.get(id).unwrap();
            assert!(
                record.planted_trees.contains(&root),
                "a partially felled trunk keeps its registered root"
            );
        }
    }
    assert!(!executor::clearing(&state, id, &project.structure));
    // The root keeps its registration until its whole trunk column is empty;
    // then the ordinary retire pass removes it.
    state.tick = 20;
    super::super::forestry::retire_missing(&mut state);
    assert!(
        state.settlements.get(id).unwrap().revision > revision,
        "root retirement is journaled"
    );
    assert!(
        !state
            .settlements
            .get(id)
            .unwrap()
            .planted_trees
            .contains(&root)
    );
    complete(&mut state, id, builder);
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
    assert!(
        state
            .settlements
            .get(id)
            .unwrap()
            .buildings
            .values()
            .any(|building| {
                placement(&state, id, building)
                    .is_some_and(|(blueprint, _, _)| blueprint.id == "spanish_fortress")
            })
    );
}

#[test]
fn natural_baseline_tree_in_planted_halo_is_felled_conserved_and_built() {
    proven_lot_is_felled_and_built(false, false);
}

#[test]
fn managed_sapling_on_needed_lot_is_felled_conserved_and_built() {
    proven_lot_is_felled_and_built(true, false);
}

#[test]
fn native_grown_registered_tree_is_felled_conserved_and_built() {
    proven_lot_is_felled_and_built(false, true);
}

#[test]
fn upgrade_clear_accepts_a_proven_natural_cell_and_refuses_the_fence() {
    use crate::settlements::forestry;
    let (mut state, id, _) = fixture();
    let root = [8, 201, 8];
    // The generator itself grows the birch here, so baseline evidence agrees.
    forestry::tests::grown_world(&mut state, vec![root], "birch");
    let at = [8, 203, 8];
    let lot = ScriptWorldBounds::new([4, 201, 4], [14, 222, 14]).unwrap();
    let cell = executor::PlannedCell {
        portion: executor::PortionKind::Trees,
        at,
        state: state.registry.states.air,
        resource: None,
        quantity: 0,
    };
    // No committed build on this site yet: the tree fence has no authored beam
    // to defer to, exactly as before the evidence map became a parameter.
    let evidence =
        executor::previous_committed(&state, id, "house", root).expect("committed chain");
    let cleared = executor::upgrade_trees_clear(
        &state,
        &evidence,
        std::slice::from_ref(&cell),
        Some(lot),
        &mut forestry::Clearance::default(),
    )
    .expect("proven natural forest in an upgrade's new cells is clear work");
    assert_eq!(cleared.len(), 1);
    assert_eq!(cleared[0].at, at);
    assert_eq!(cleared[0].state, state.registry.states.air);
    assert!(cleared[0].resource.is_none());

    // A retained player placement is never natural forest.
    retain_player_edit(&mut state, at);
    assert!(matches!(
        executor::upgrade_trees_clear(
            &state,
            &evidence,
            std::slice::from_ref(&cell),
            Some(lot),
            &mut forestry::Clearance::default(),
        ),
        Err(mc_script::ScriptOperationFailure::Forbidden)
    ));

    // Persistent leaves are a player-facing veto, not clearance work.
    state.block_log = Default::default();
    let persistent = block(&state, "minecraft:birch_leaves", &[("persistent", "true")]);
    state.world.set_block(at[0], at[1], at[2], persistent);
    assert!(matches!(
        executor::upgrade_trees_clear(
            &state,
            &evidence,
            std::slice::from_ref(&cell),
            Some(lot),
            &mut forestry::Clearance::default(),
        ),
        Err(mc_script::ScriptOperationFailure::Blocked)
    ));

    // Without generator evidence a tree cell is unproven and refuses.
    state
        .world
        .set_block(at[0], at[1], at[2], state.registry.states.air);
    let unproven = block(&state, "minecraft:oak_log", &[]);
    state.world.set_block(at[0], at[1], at[2], unproven);
    assert!(matches!(
        executor::upgrade_trees_clear(
            &state,
            &evidence,
            std::slice::from_ref(&cell),
            Some(lot),
            &mut forestry::Clearance::default(),
        ),
        Err(mc_script::ScriptOperationFailure::Blocked)
    ));
}

#[test]
fn upgrades_never_overwrite_tree_or_sapling_cells_in_their_diff() {
    let (mut state, id, builder) = fixture();
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    complete(&mut state, id, builder);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    start(&mut state, id, "house", 2, Some(building));
    let project = state.settlements.get(id).unwrap().projects[0].clone();
    let (_, planned) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    let at = planned
        .iter()
        .find(|cell| {
            state
                .world
                .block_at(cell.at[0], cell.at[1], cell.at[2])
                .is_some_and(|current| !super::super::forestry::tree_or_sapling(&state, current))
        })
        .expect("upgrade has a cell that was not an authored beam")
        .at;
    let original = state.world.block_at(at[0], at[1], at[2]).unwrap();
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    // Unproven cells (no generator evidence, not the building's own authored
    // state) still refuse; a proven natural cell is clear work instead.
    for name in [
        "minecraft:birch_sapling",
        "minecraft:oak_log",
        "minecraft:birch_log",
        "minecraft:birch_leaves",
    ] {
        let tree = block(&state, name, &[]);
        state.world.set_block(at[0], at[1], at[2], tree);
        // Inside the territory (owner D27.4) the unproven cell is the
        // village's property: clear work with conserved drops.
        let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default())
            .unwrap_or_else(|reason| panic!("inside the territory {name}: {reason:?}"));
        assert!(
            cells
                .iter()
                .any(|cell| cell.at == at && cell.state == state.registry.states.air),
            "inside the territory {name} is clear work"
        );
        let record = state.settlements.get(id).unwrap().clone();
        territory_elsewhere(&mut state, id, at);
        assert!(
            matches!(
                planned_cells(&state, &project, "oak", &mut Default::default()),
                Err(site_refusals::Reason::ProtectedObstruction)
            ),
            "outside the territory an unproven tree cell in the upgrade work diff is refused as ProtectedObstruction: {name}"
        );
        *state.settlements.get_mut(id).unwrap() = record;
        assert_eq!(state.world.block_at(at[0], at[1], at[2]), Some(tree));
        assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    }
    state.world.set_block(at[0], at[1], at[2], original);
    assert!(planned_cells(&state, &project, "oak", &mut Default::default()).is_ok());
}

fn upgrade_beam() -> (
    TickState,
    SettlementId,
    crate::entities::EntitySlot,
    Project,
    [i32; 3],
) {
    let (mut state, id, builder) = fixture();
    start(&mut state, id, "house", 1, None);
    let original = state.settlements.get(id).unwrap().projects[0].clone();
    fund(&mut state, "house", 1);
    complete(&mut state, id, builder);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    let project = Project {
        level: 2,
        building: Some(building),
        structure: String::new(),
        ..original
    };
    // L1's horizontal oak beam is replaced by L2 cobblestone.
    let beam = [
        project.origin[0] + 5,
        project.origin[1] + 4,
        project.origin[2],
    ];
    let current = state.world.block_at(beam[0], beam[1], beam[2]).unwrap();
    assert_eq!(
        crate::farming::prop(&state.registry.blocks, current, "axis"),
        Some("x")
    );
    assert_eq!(
        state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(current))
            .unwrap()
            .block
            .id
            .path(),
        "oak_log"
    );
    let (_, cells) = planned_cells(&state, &project, "oak", &mut Default::default()).unwrap();
    assert!(cells.iter().any(|cell| {
        cell.at == beam
            && state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(cell.state))
                .unwrap()
                .block
                .id
                .path()
                == "cobblestone"
    }));
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert([beam[0] + 1, beam[1], beam[2] + 1]);
    (state, id, builder, project, beam)
}

/// Owner D27.4: inside the territory a retained player edit of the building's
/// own beam is the village's property; outside it the attribution still
/// refuses before the authored exemption, until the attribution expires.
#[test]
fn retained_identical_player_beam_refuses_upgrade_outside_the_territory_only() {
    let (mut state, id, _, project, beam) = upgrade_beam();
    retain_player_edit(&mut state, beam);
    assert!(planned_cells(&state, &project, "oak", &mut Default::default()).is_ok());
    territory_elsewhere(&mut state, id, beam);
    assert!(matches!(
        planned_cells(&state, &project, "oak", &mut Default::default()),
        Err(site_refusals::Reason::ProtectedObstruction)
    ));
    fund(&mut state, "house", 2);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        start_upgrade(&mut state, id, project.building.unwrap()),
        Err("Construction admission refused: ProtectedObstruction.".into())
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    // Main's explicit upgrade contract: an indistinguishable authored-state
    // replacement may upgrade once its player attribution is no longer live.
    state.block_log = super::super::forestry::tests::player_history(&state, beam, 1);
    state.block_log.evict(u64::MAX);
    assert!(!state.block_log.player_touched(beam));
    assert!(planned_cells(&state, &project, "oak", &mut Default::default()).is_ok());
}

#[test]
fn disabled_history_with_live_player_refuses_upgrade_outside_the_territory_only() {
    let (mut state, id, slot, project, beam) = upgrade_beam();
    let entity = state.entities.body(slot).id;
    let connection = crate::boundary::ConnectionId(999);
    state
        .entities
        .set_kind(slot, crate::entities::EntityKind::Player { connection });
    state.entities.row_mut(slot).villager = None;
    state.players.insert(
        entity,
        crate::player::PlayerState::new(connection, crate::commands::GameMode::Survival),
    );
    state.entities.set_pos(slot, beam.map(f64::from));
    assert!(!state.block_log.enabled());
    assert!(
        planned_cells(&state, &project, "oak", &mut Default::default()).is_ok(),
        "inside the territory the live-player veto is not consulted (owner D27.4)"
    );
    territory_elsewhere(&mut state, id, beam);
    assert!(matches!(
        planned_cells(&state, &project, "oak", &mut Default::default()),
        Err(site_refusals::Reason::ProtectedObstruction)
    ));
    state.entities.set_pos(
        slot,
        [
            f64::from(beam[0]) + 33.0,
            f64::from(beam[1]),
            f64::from(beam[2]),
        ],
    );
    assert!(planned_cells(&state, &project, "oak", &mut Default::default()).is_ok());
}

/// Committee ruling: an unfinished sweep holds the project slot only while it
/// already holds a legal candidate. An empty one must not starve the upgrade,
/// meadow, fortification or street planning.
#[test]
fn an_empty_unfinished_sweep_does_not_hold_the_slot() {
    let (mut state, id, builder) = fixture();
    // A plan wider than one pass's share of candidates.
    super::terrain_tests::wide_plan(&mut state);
    super::terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    let house = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    // Keep the standing house's maximum footprint and dry Builder approaches;
    // every other lot column is solid through the terrain window. Roads,
    // verges and the plaza stay walkable, so the sweep still walks every
    // frontage and refuses each lot.
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -48..64 {
        for z in -48..64 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && !((0..17).contains(&x) && (0..14).contains(&z))
            {
                for y in 201..=250 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("spanish_house", 2), ("farm", 1)],
    );
    // Premise: one planning pass cannot finish the funded Farm's sweep, that
    // pass finds nothing legal, and the whole sweep has no legal lot either.
    {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let farm = blueprints::get(&state.registry, "farm").unwrap();
        assert!(funded_variant(&state, id, &farm.level(1).unwrap().materials, &images).is_some());
        let pass = site_project(
            &state,
            record,
            std::iter::once(farm),
            &mut site_refusals::Sweeps::default(),
            record.construction_cursor,
            SITE_ATTEMPTS,
            &images,
        );
        assert!(
            pass.site.is_none() && pass.running,
            "one pass leaves the sweep unfinished"
        );
        assert!(
            !pass.waiting,
            "that unfinished sweep holds no legal candidate"
        );
        let whole = site_project(
            &state,
            record,
            std::iter::once(farm),
            &mut site_refusals::Sweeps::default(),
            0,
            u32::MAX,
            &images,
        );
        assert!(
            whole.site.is_none(),
            "the funded Farm has no legal lot at all"
        );
    }
    assert!(upgrade_available(&state, id, house));
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    plan(&mut state, id);
    let project = state
        .settlements
        .get(id)
        .unwrap()
        .projects
        .first()
        .expect("the empty unfinished sweep left the slot to the funded upgrade");
    assert_eq!(project.blueprint, "spanish_house");
    assert_eq!(project.building, Some(house));
    assert_eq!(project.level, 2);
    assert!(!project.ordered, "the autonomous planner admitted it");
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

/// The fixture the sweep-hold pair shares: a standing funded House L1 on open
/// ground long enough to seed a real incomplete Farm sweep that already holds a
/// legal candidate, then every other lot column solid so the funded House
/// search runs wide and finds no best. House L1 and L2 stay paid; the Farm is
/// funded only while the seed is produced.
fn sweep_hold_fixture() -> (TickState, SettlementId, BuildingId, site_refusals::Sweeps) {
    let (mut state, id, builder) = fixture();
    // A plan wider than one pass's share of candidates.
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    let house = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    // One bed for two residents: the planner wants another House L1 site,
    // which it searches wide below, while the standing house's L2 upgrade
    // stays admissible.
    let resident = crate::settlements::allocator::tests::villager(&mut state, [1.5, 201.0, 3.5]);
    let entity = state.entities.body(resident).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    // The planner itself produces the stale scratch on the open ground: one
    // examined candidate is enough for the Farm sweep to hold its own best
    // while the whole sweep stays unfinished.
    stock_buildings(&mut state, &[("spanish_house", 1), ("farm", 1)]);
    let seeded = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let farm = blueprints::get(&state.registry, "farm").unwrap();
        let frontages = u32::try_from(
            layout::Network::for_construction(&state, record)
                .frontage()
                .len(),
        )
        .unwrap();
        let mut found = None;
        for start in 0..frontages.min(256) {
            let mut sweeps = site_refusals::Sweeps::default();
            let pass = site_project(
                &state,
                record,
                std::iter::once(farm),
                &mut sweeps,
                start,
                1,
                &images,
            );
            if pass.site.is_none() && pass.running && pass.waiting {
                found = Some(sweeps);
                break;
            }
        }
        found.expect("the open ground gives an unfinished Farm sweep holding a candidate")
    };
    // Keep the standing house's own footprint and dry Builder approaches; every
    // other lot column is solid through the terrain window, so the funded
    // House search walks every frontage and finds no best.
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -48..64 {
        for z in -48..64 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && !((0..17).contains(&x) && (0..14).contains(&z))
            {
                for y in 201..=250 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    // House L1 and L2 paid; the Farm is not funded in the plan run.
    stock_buildings(&mut state, &[("spanish_house", 1), ("spanish_house", 2)]);
    (state, id, house, seeded)
}

/// One pass with the whole budget admits the cheapest lot of the whole street;
/// the same village walked in small steps must admit exactly that lot, and no
/// step may examine more lots than its share allows. This is the determinism
/// guard for the carried-best change: a stable village keeps the rule.
#[test]
fn stepped_sweeps_admit_the_same_cheapest_lot_as_one_full_pass() {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    stock_buildings(&mut state, &[("spanish_house", 1), ("farm", 1)]);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let farm = blueprints::get(&state.registry, "farm").unwrap();
    let frontages = u32::try_from(
        layout::Network::for_construction(&state, record)
            .frontage()
            .len(),
    )
    .unwrap();
    let one_pass = site_project(
        &state,
        record,
        std::iter::once(farm),
        &mut site_refusals::Sweeps::default(),
        0,
        u32::MAX,
        &images,
    );
    let full_origin = one_pass
        .site
        .as_ref()
        .map(|site| (site.bounds.min, site.project.rotation))
        .expect("the open ground admits one Farm");
    // The same walk in steps of four lots, each step examined against its share.
    let step = 32;
    let mut sweeps = site_refusals::Sweeps::default();
    let mut steps = 0u32;
    let stepped_origin = loop {
        steps += 1;
        assert!(
            u64::from(steps) <= u64::from(frontages / step) + 2,
            "the stepped walk must finish within its frontage count"
        );
        let pass = site_project(
            &state,
            record,
            std::iter::once(farm),
            &mut sweeps,
            0,
            step,
            &images,
        );
        assert!(
            pass.report.examined <= step,
            "a pass examined {} lots with a {step}-lot budget",
            pass.report.examined
        );
        if let Some(site) = pass.site {
            break (site.bounds.min, site.project.rotation);
        }
    };
    assert_eq!(
        stepped_origin, full_origin,
        "the stepped walk admitted a different lot than the whole-street pass"
    );
}

/// A pass hands its whole budget out: with five lots and two kinds the leftover
/// lot goes to the first kind instead of being dropped. (The hand-out is the
/// only thing this changes; no kind used to be starved by the kinds before it -
/// each already had its own share.)
#[test]
fn a_pass_spends_its_whole_budget_across_its_kinds() {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    stock_buildings(&mut state, &[("spanish_house", 1), ("farm", 1)]);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let kinds = [
        blueprints::get(&state.registry, "spanish_house").unwrap(),
        blueprints::get(&state.registry, "farm").unwrap(),
    ];
    let pass = site_project(
        &state,
        record,
        kinds.into_iter(),
        &mut site_refusals::Sweeps::default(),
        0,
        5,
        &images,
    );
    assert_eq!(
        pass.report.examined, 5,
        "a five-lot pass over two funded kinds must examine all five lots"
    );
}

/// The hold belongs to the kind whose own sweep is still running: a stale
/// sweep of another, now-unfunded kind that holds a candidate must not starve a
/// funded House upgrade. The pair differs only in that scratch, and both start
/// the same L2.
#[test]
fn a_stale_unfunded_sweep_does_not_hold_the_slot_for_a_funded_upgrade() {
    let (mut plain, plain_id, plain_house, _unused) = sweep_hold_fixture();
    let (mut stale, stale_id, stale_house, seeded) = sweep_hold_fixture();
    assert!(upgrade_available(&plain, plain_id, plain_house));
    assert!(upgrade_available(&stale, stale_id, stale_house));
    // The only difference: the live sweep of a kind this pass never advances.
    stale.settlements.site_sweep.insert(stale_id, seeded);
    let plain_started = plan_pass(&mut plain, plain_id, &mut site_turn::SiteBudget::default());
    let stale_started = plan_pass(&mut stale, stale_id, &mut site_turn::SiteBudget::default());
    assert_eq!(
        stale_started, plain_started,
        "the stale scratch of an unfunded kind changed the planner's decision"
    );
    assert!(plain_started, "the funded House upgrade starts");
    for (state, id, house) in [
        (&plain, plain_id, plain_house),
        (&stale, stale_id, stale_house),
    ] {
        let project = state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .expect("the funded House upgrade starts in both runs");
        assert_eq!(project.blueprint, "spanish_house");
        assert_eq!(project.level, 2);
        assert_eq!(project.building, Some(house));
        assert!(!project.ordered, "the autonomous planner admitted it");
    }
}

/// A funded kind whose own sweep ran unfinished this pass and already holds a
/// legal candidate owns the hold: the slot waits for its cheapest lot.
#[test]
fn a_funded_sweep_that_still_holds_a_candidate_keeps_the_slot() {
    let (mut state, id, _house, seeded) = sweep_hold_fixture();
    state.settlements.site_sweep.insert(id, seeded);
    // Fund the Farm again: the stale sweep now belongs to a funded kind.
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("spanish_house", 2), ("farm", 1)],
    );
    assert!(
        !plan_pass(&mut state, id, &mut site_turn::SiteBudget::default()),
        "the funded sweep's own candidate holds the slot"
    );
    assert!(state.settlements.get(id).unwrap().projects.is_empty());
}

/// The guard the removed terrace test held (review round 3 C7): a funded kind
/// that won its whole walk and was refused only in `materialize` must not
/// release an earlier kind's hold in the same pass.
///
/// The refusal is materialize-only. A registered planted tree stands beside the
/// winner's walking corridor and its crown hangs over a corridor column at head
/// height inside the territory, but one branch log in a second column leaves
/// the whole component unprovable, so `forestry::cells` refuses with
/// `whole_tree_unproven`. `check` and the sweep's route guard (`lot_has_tree`
/// with no route) cannot see that cell, so the lot still wins the ranking.
#[test]
fn a_refused_materialization_keeps_the_earlier_hold() {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    let house = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    // One bed against two residents: the planner still wants a House, so the
    // funded scan advances the House walk before the Farm.
    let resident = crate::settlements::allocator::tests::villager(&mut state, [1.5, 201.0, 3.5]);
    let entity = state.entities.body(resident).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    stock_buildings(
        &mut state,
        &[("spanish_house", 1), ("spanish_house", 2), ("farm", 1)],
    );
    let farm = blueprints::get(&state.registry, "farm").unwrap();
    let house_blueprint = catalog(&state, state.settlements.get(id).unwrap())
        .find(|blueprint| blueprint.kind == BuildingKind::House)
        .expect("catalog House");
    // Where the Farm would win on the open ground and how its walk reaches it.
    let winner = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        site_project(
            &state,
            record,
            std::iter::once(farm),
            &mut site_refusals::Sweeps::default(),
            0,
            u32::MAX,
            &images,
        )
        .site
        .expect("the open ground admits one Farm")
    };
    let corridor = layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
        .construction_access(&winner.project)
        .expect("the winning lot is reachable");
    let street = corridor[0];
    // A registered planted tree two columns off that walking column, its crown
    // over the column at head height; a branch log in a second column makes the
    // whole component ambiguous, so no cell of it can be proved owned.
    let log = block(&state, "minecraft:oak_log", &[("axis", "y")]);
    let leaves = block(&state, "minecraft:oak_leaves", &[("persistent", "false")]);
    let root = [street[0] + 2, street[1] + 1, street[2] + 2];
    for y in root[1]..=root[1] + 3 {
        state.world.set_block(root[0], y, root[2], log);
    }
    state
        .world
        .set_block(root[0] + 1, root[1] + 2, root[2], log);
    for y in street[1] + 1..=street[1] + 2 {
        state.world.set_block(street[0], y, street[2], leaves);
    }
    state.world.fold_overlay();
    state
        .settlements
        .get_mut(id)
        .unwrap()
        .planted_trees
        .insert(root);
    // Real passes seed both live sweeps: the House stops halfway holding its
    // ranked lot, the Farm stops one slice short of its winner.
    let frontages = u32::try_from(
        layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
            .frontage()
            .len(),
    )
    .unwrap();
    let mut sweeps = site_refusals::Sweeps::default();
    let house_seed = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        site_project(
            &state,
            record,
            std::iter::once(house_blueprint),
            &mut sweeps,
            0,
            frontages / 2,
            &images,
        )
    };
    assert!(
        house_seed.site.is_none() && house_seed.running && house_seed.waiting,
        "the House sweep stays unfinished while holding its ranked lot"
    );
    let farm_seed = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        site_project(
            &state,
            record,
            std::iter::once(farm),
            &mut sweeps,
            0,
            frontages - 8,
            &images,
        )
    };
    assert!(
        farm_seed.site.is_none() && farm_seed.running && farm_seed.waiting,
        "the Farm sweep stops one slice short of completing with its ranked lot"
    );
    state.settlements.site_sweep.insert(id, sweeps);
    assert!(upgrade_available(&state, id, house));
    let _ = super::super::forestry::take_last_tree_stage();
    assert!(
        !plan_pass(&mut state, id, &mut site_turn::SiteBudget::default()),
        "the refused sweep is not an admission"
    );
    assert_eq!(
        super::super::forestry::take_last_tree_stage(),
        Some("whole_tree_unproven"),
        "the winner's corridor tree is what refuses the plan"
    );
    assert!(
        state.settlements.get(id).unwrap().projects.is_empty(),
        "the House sweep's own hold survives the later kind's refused materialization"
    );
}

/// The priority scan (physical beds) and the ordinary funded scan (certificate
/// levels) fetch the same catalog House in one planning pass. A long real
/// planner pass leaves that House sweep unfinished while already holding its
/// ranked lot; an ordinary pass then finishes the sweep and its recheck finds
/// the world moved. The ordinary visit owns the final state, so the hold dies
/// and the funded House L2 upgrade starts.
fn priority_house_boundary() -> (TickState, SettlementId, BuildingId, site_refusals::Sweeps) {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    fund(&mut state, "spanish_house", 1);
    start(&mut state, id, "spanish_house", 1, None);
    complete(&mut state, id, builder);
    let house = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    // One physical bed against two residents and one certificate level: the
    // priority scan and the ordinary funded scan both fetch this catalog House.
    let resident = crate::settlements::allocator::tests::villager(&mut state, [1.5, 201.0, 3.5]);
    let entity = state.entities.body(resident).id;
    crate::settlements::bind(&mut state, id, entity).unwrap();
    // The POI scan gates the priority pass; drive its canonical bounded refresh
    // so it holds a real bed inventory rather than an uninitialized cache.
    let mut cache = std::mem::take(&mut state.villager_pois);
    cache.refresh_poi_revisions(&state);
    state.villager_pois = cache;
    for _ in 0..64 {
        if state.villager_pois.pending_chunks.is_empty() {
            break;
        }
        let mut cache = std::mem::take(&mut state.villager_pois);
        cache.refresh(&state);
        state.villager_pois = cache;
    }
    stock_buildings(&mut state, &[("spanish_house", 1), ("spanish_house", 2)]);
    // A real planner pass ranks the cheapest legal lot and stops a hundred
    // candidates short of the street's end: the priority grant (64) keeps it
    // unfinished and holding, the ordinary grant (64) finishes it.
    let frontages = u32::try_from(
        layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
            .frontage()
            .len(),
    )
    .unwrap();
    assert!(frontages > 128, "the boundary needs two full site grants");
    let seeded = {
        let record = state.settlements.get(id).unwrap();
        let images = executor::source_images(&state, id);
        let house_blueprint = catalog(&state, record)
            .find(|blueprint| blueprint.kind == BuildingKind::House)
            .unwrap();
        let mut sweeps = site_refusals::Sweeps::default();
        let pass = site_project(
            &state,
            record,
            std::iter::once(house_blueprint),
            &mut sweeps,
            0,
            frontages - 100,
            &images,
        );
        assert!(
            pass.site.is_none() && pass.running && pass.waiting,
            "the long pass leaves the House sweep unfinished while holding its ranked lot"
        );
        sweeps
    };
    // Keep the standing house's own footprint and dry Builder approaches; every
    // other lot column is solid, so the ranked lot cannot survive the recheck
    // and the candidates left to both grants refuse.
    let anchor = state.settlements.get(id).unwrap().anchor;
    for x in -48..64 {
        for z in -48..64 {
            if layout::cell(anchor, x, z) == layout::Cell::Lot
                && !((0..17).contains(&x) && (0..14).contains(&z))
            {
                for y in 201..=250 {
                    state.world.set_block(x, y, z, state.registry.states.stone);
                }
            }
        }
    }
    state.world.fold_overlay();
    (state, id, house, seeded)
}

#[test]
fn an_ordinary_revisit_of_the_priority_house_owns_the_hold() {
    let (mut plain, plain_id, plain_house, _unused) = priority_house_boundary();
    let (mut stale, stale_id, stale_house, seeded) = priority_house_boundary();
    for (state, id) in [(&plain, plain_id), (&stale, stale_id)] {
        let images = executor::source_images(state, id);
        assert!(
            priority_house(state, state.settlements.get(id).unwrap(), &images).is_some(),
            "the priority scan reaches the same catalog House"
        );
    }
    assert!(upgrade_available(&plain, plain_id, plain_house));
    assert!(upgrade_available(&stale, stale_id, stale_house));
    stale.settlements.site_sweep.insert(stale_id, seeded);
    let plain_started = plan_pass(&mut plain, plain_id, &mut site_turn::SiteBudget::default());
    let stale_started = plan_pass(&mut stale, stale_id, &mut site_turn::SiteBudget::default());
    assert_eq!(
        stale_started, plain_started,
        "the ordinary pass's finished, moved sweep must not leave the priority hold standing"
    );
    assert!(plain_started, "the funded House upgrade starts");
    for (state, id, house) in [
        (&plain, plain_id, plain_house),
        (&stale, stale_id, stale_house),
    ] {
        let project = state
            .settlements
            .get(id)
            .unwrap()
            .projects
            .first()
            .expect("the funded House upgrade starts in both runs");
        assert_eq!(project.blueprint, "spanish_house");
        assert_eq!(project.level, 2);
        assert_eq!(project.building, Some(house));
        assert!(!project.ordered, "the autonomous planner admitted it");
    }
}

/// A settlement whose planner must see a real upgrade: a built mason L1, the
/// POI bed cache ready, and a chest the test fills itself.
fn upgrade_fixture(build: &str) -> (TickState, SettlementId) {
    let (mut state, id, builder) = fixture();
    terrain_tests::wide_plan(&mut state);
    terrain_tests::restore_stock(&mut state);
    open_plan(&mut state);
    fund(&mut state, build, 1);
    start(&mut state, id, build, 1, None);
    complete(&mut state, id, builder);
    // The bed count is the POI scan's; drive its canonical bounded refresh.
    let mut cache = std::mem::take(&mut state.villager_pois);
    cache.refresh_poi_revisions(&state);
    state.villager_pois = cache;
    for _ in 0..64 {
        if state.villager_pois.pending_chunks.is_empty() {
            break;
        }
        let mut cache = std::mem::take(&mut state.villager_pois);
        cache.refresh(&state);
        state.villager_pois = cache;
    }
    (state, id)
}

/// Stock the chosen `name` level bill item-wise: every item the upgrade needs,
/// plus the House reserve's count for items the two bills share. Items that
/// exist only in the House bill (its bed) are never stocked - the property the
/// per-item rule exists for.
fn stock_bill_and_reserve(
    state: &mut TickState,
    id: SettlementId,
    name: &str,
    level: u8,
    rename: Option<&str>,
    headroom: bool,
) -> Vec<(u32, u32)> {
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(state, id);
    let reserved = house_reserve_bill(state, record, &images);
    let blueprint = blueprints::get(&state.registry, name).unwrap();
    let bill = selected_bill(
        state,
        id,
        &blueprint.level(level).unwrap().materials,
        &images,
    )
    .unwrap()
    .into_owned();
    let mut amounts: std::collections::BTreeMap<u32, u32> = std::collections::BTreeMap::new();
    for (item, count) in bill.iter().copied() {
        *amounts.entry(item).or_default() += count;
    }
    if headroom {
        for (item, count) in reserved.iter().copied() {
            if let Some(needed) = amounts.get_mut(&item) {
                *needed = needed.saturating_add(count);
            }
        }
    }
    let renamed = |item: u32| match rename {
        None => item,
        Some(to) => state
            .registry
            .items
            .name_of(item)
            .and_then(|name| name.path().split_once('_'))
            .and_then(|(_, rest)| mc_data::Identifier::parse(format!("minecraft:{to}_{rest}")).ok())
            .and_then(|identifier| state.registry.items.id_of(&identifier))
            .unwrap_or(item),
    };
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    let mut index = 0;
    for (item, mut count) in amounts {
        let item = renamed(item);
        while count != 0 && index < slots.len() {
            let portion = count.min(64);
            slots[index] = mc_data::ItemStack::new(item, portion as i32);
            index += 1;
            count -= portion;
        }
    }
    bill.into_iter().collect()
}

/// The House reserve is netted per item of the upgrade's own bill: a shortage
/// of items only the House bill carries (its bed, never stocked autonomously)
/// must not refuse an upgrade that never asks for them.
#[test]
fn the_house_reserve_is_netted_per_item_of_the_upgrade_bill() {
    let (mut state, id) = upgrade_fixture("mason");
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let reserved = house_reserve_bill(&state, record, &images);
    assert!(
        !reserved.is_empty(),
        "a village short of roofed beds reserves"
    );
    let bed = reserved.iter().find(|(item, _)| {
        state
            .registry
            .items
            .name_of(*item)
            .is_some_and(|name| name.path().ends_with("_bed"))
    });
    assert!(bed.is_some(), "the House bill carries a bed");
    // Stock the shared items with the reserve's headroom; the bed stays absent.
    stock_bill_and_reserve(&mut state, id, "mason", 2, None, true);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let admitted = preparation::upgrade_project(&state, record, &images)
        .expect("shared stock covers the upgrade and its share of the reserve");
    assert_eq!(admitted.0.blueprint, "mason");
    assert_eq!(admitted.0.level, 2);
}

/// An upgrade OF a house adds beds and is the documented fallback when no new
/// House has a legal lot, so the House reserve never applies to it. The test
/// only discriminates if the reserved bill really shares an item with the
/// house's own bill: the shared item then carries the reservation in the
/// whole-bill form (control: with the exemption off the upgrade is refused).
#[test]
fn a_house_upgrade_is_not_reserved_against_itself() {
    let (mut state, id) = upgrade_fixture("spanish_house");
    // Exactly the chosen L2 bill: no reserve headroom at all.
    let _ = stock_bill_and_reserve(&mut state, id, "spanish_house", 2, None, false);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let reserved = house_reserve_bill(&state, record, &images);
    assert!(!reserved.is_empty(), "the village is short of roofed beds");
    let blueprint = blueprints::get(&state.registry, "spanish_house").unwrap();
    let own = selected_bill(&state, id, &blueprint.level(2).unwrap().materials, &images)
        .expect("the chosen L2 bill");
    let shared = own
        .iter()
        .filter(|(item, _)| reserved.iter().any(|(known, _)| known == item))
        .count();
    assert!(
        shared != 0,
        "the reserve must share an item with the house bill, or this test cannot discriminate"
    );
    let admitted = preparation::upgrade_project(&state, record, &images)
        .expect("a house upgrade is exempt from the House reserve");
    assert_eq!(admitted.0.blueprint, "spanish_house");
    assert_eq!(admitted.0.level, 2);
}

/// The reserve still bites on the items the upgrade and the House share: with
/// exactly the upgrade's bill in store the upgrade waits for the House share.
#[test]
fn the_house_reserve_refuses_an_upgrade_on_its_shared_items() {
    let (mut state, id) = upgrade_fixture("mason");
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    assert!(
        !house_reserve_bill(&state, record, &images).is_empty(),
        "a village short of roofed beds reserves"
    );
    stock_buildings(&mut state, &[("mason", 2)]);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    assert!(
        preparation::upgrade_project(&state, record, &images).is_none(),
        "the mason upgrade would starve the House lane"
    );
}

/// The stock-only rule is checked on the CHOSEN bill, so a village that stocked
/// birch variants passes on its own species.
#[test]
fn a_birch_village_admits_its_upgrade() {
    let (mut state, id) = upgrade_fixture("mason");
    stock_bill_and_reserve(&mut state, id, "mason", 2, Some("birch"), true);
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    let admitted = preparation::upgrade_project(&state, record, &images)
        .expect("the chosen birch bill is in store");
    assert_eq!(admitted.1, "birch");
}

/// Logs alone do not admit an upgrade: the bill's planks, stairs, slabs, doors
/// and fences are craftable at haul, but the stock-only rule waits until they
/// are in store. Every non-wood item is stocked literally, so funding itself
/// passes and only the stock rule refuses. Control: with the stock check
/// removed the same upgrade is admitted (observed in the row-2 reproducer).
#[test]
fn logs_only_do_not_admit_an_upgrade() {
    let (mut state, id) = upgrade_fixture("mason");
    let blueprint = blueprints::get(&state.registry, "mason").unwrap();
    let materials = &blueprint.level(2).unwrap().materials;
    let log = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:oak_log").unwrap())
        .unwrap();
    let wood_craftable = |name: &str| {
        ["planks", "stairs", "slab", "door", "fence", "trapdoor"]
            .iter()
            .any(|kind| name.ends_with(kind))
    };
    let mut literal: Vec<(u32, u32)> = Vec::new();
    for &(item, count) in materials {
        let name = state
            .registry
            .items
            .name_of(item)
            .map(|name| name.path().to_owned())
            .unwrap_or_default();
        if !wood_craftable(&name) {
            literal.push((item, count));
        }
    }
    let slots = &mut state.block_entities.get_mut(&(1, 201, 1)).unwrap().slots;
    slots.fill(mc_data::ItemStack::EMPTY);
    let mut index = 0;
    for (item, mut count) in literal {
        while count != 0 && index < slots.len() {
            let portion = count.min(64);
            slots[index] = mc_data::ItemStack::new(item, portion as i32);
            index += 1;
            count -= portion;
        }
    }
    while index < 20 {
        slots[index] = mc_data::ItemStack::new(log, 64);
        index += 1;
    }
    let record = state.settlements.get(id).unwrap();
    let images = executor::source_images(&state, id);
    assert!(
        funded_variant(&state, record.id, materials, &images).is_some(),
        "logs fund the wooden part, so funding itself passes"
    );
    assert!(
        preparation::upgrade_project(&state, record, &images).is_none(),
        "logs alone are craftable but not stocked"
    );
}
