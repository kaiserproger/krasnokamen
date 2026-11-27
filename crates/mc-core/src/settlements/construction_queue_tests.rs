//! D8 queue rules: several owner requests, funded ones lift, the entry order
//! otherwise holds, and the queue is bounded.
use super::tests::{fixture, fund, fund_at, open_plan};
use super::*;
use crate::settlements::allocator::tests::block;
use mc_data::ItemStack;

/// Owner D8: a request whose materials are in storage moves ahead of unfunded
/// ones, so a funded request queued behind an unfunded head starts first while
/// the head keeps its position.
#[test]
fn a_funded_queued_order_lifts_over_an_unfunded_head() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    // The head cannot be funded from this fixture's stock; the house can.
    let head = OwnerOrder {
        seq: 0,
        blueprint: "spanish_plaza".into(),
        level: 1,
        building: None,
    };
    let lifted = OwnerOrder {
        seq: 0,
        blueprint: "house".into(),
        level: 1,
        building: None,
    };
    {
        let record = state.settlements.get_mut(id).unwrap();
        access::push_order(record, head);
        access::push_order(record, lifted);
    }
    fund(&mut state, "house", 1);
    // The site sweep may need several passes before the funded lot is found.
    for _ in 0..64 {
        plan(&mut state, id);
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record)
            .map(|project| project.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"],
        "the funded request started while the unfunded head waited"
    );
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["spanish_plaza"],
        "the unfunded head keeps its place"
    );
}

/// The queue is bounded and the refusal names the limit.
#[test]
fn the_queue_refuses_an_order_beyond_its_limit() {
    let (mut state, id, _) = fixture();
    let record = state.settlements.get_mut(id).unwrap();
    for _ in 0..MAX_ORDERS {
        access::push_order(
            record,
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    let refused = request_build(&mut state, id, "house").unwrap_err();
    assert!(
        refused.contains(&MAX_ORDERS.to_string()),
        "the refusal names the queue limit: {refused}"
    );
}

/// One admission per planning pass, and a settlement may hold several projects
/// once each is funded on top of the others (D8).
#[test]
fn a_pass_admits_one_project_and_a_second_pass_admits_the_next() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    for _ in 0..2 {
        access::push_order(
            state.settlements.get_mut(id).unwrap(),
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    // Two houses' worth: the fixture writes one bill, so double every stack.
    fund(&mut state, "house", 1);
    for stack in state
        .block_entities
        .get_mut(&(1, 201, 1))
        .unwrap()
        .slots
        .iter_mut()
        .filter(|stack| stack.count > 0)
    {
        stack.count *= 2;
    }
    let mut passes = 0;
    while access::projects(state.settlements.get(id).unwrap()).count() == 0 && passes < 64 {
        plan(&mut state, id);
        passes += 1;
    }
    assert_eq!(
        access::projects(state.settlements.get(id).unwrap()).count(),
        1,
        "the first admission"
    );
    let mut passes = 0;
    while access::projects(state.settlements.get(id).unwrap()).count() == 1 && passes < 64 {
        plan(&mut state, id);
        passes += 1;
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record).count(),
        2,
        "the next pass admits the second funded project"
    );
    let mut handles: Vec<&str> = access::projects(record)
        .map(|project| project.structure.as_str())
        .collect();
    handles.sort_unstable();
    handles.dedup();
    assert_eq!(handles.len(), 2, "each project owns its own structure");
}

/// The joint proof: a second project is not admitted out of the first one's
/// materials, even though it would be fundable on its own.
#[test]
fn the_joint_proof_keeps_a_second_project_out_of_the_first_ones_stock() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    for _ in 0..2 {
        access::push_order(
            state.settlements.get_mut(id).unwrap(),
            OwnerOrder {
                seq: 0,
                blueprint: "house".into(),
                level: 1,
                building: None,
            },
        );
    }
    fund(&mut state, "house", 1);
    for _ in 0..64 {
        plan(&mut state, id);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record).count(),
        1,
        "the second house is not funded on top of the first"
    );
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"],
        "the refused request keeps its place in the queue"
    );
}

/// The fixture town with its owner joined inside it, a house funded after the
/// join tick (whose planning pass then has no sweep to start), and the number
/// of street frontages its site sweep walks.
fn owner_in_open_plan() -> (TickState, SettlementId, usize) {
    const CONN: mc_domain::ConnectionId = mc_domain::ConnectionId(7);
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    crate::tick::run_tick(
        &mut state,
        &[crate::boundary::PlayerInput::Join {
            connection: CONN,
            uuid: uuid::Uuid::nil(),
            name: "Mayor".into(),
            permission_level: 0,
        }],
    );
    state.settlements.get_mut(id).unwrap().owner = Some(uuid::Uuid::nil().to_string());
    let entity = state.connections.entity_of(CONN).unwrap();
    let slot = state.entities.slot_of(entity).unwrap();
    state.entities.set_pos(slot, [8.5, 201.0, 8.5]);
    fund(&mut state, "house", 1);
    assert_eq!(examined(&state, id), None);
    let frontages = layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
        .frontage()
        .len();
    assert!(
        frontages > 2 * SITE_ATTEMPTS as usize,
        "the sweep must outlast two slices: {frontages} frontages"
    );
    (state, id, frontages)
}

fn examined(state: &TickState, id: SettlementId) -> Option<u32> {
    state
        .settlements
        .site_sweep
        .get(&id)
        .and_then(|sweeps| sweeps.examined("house"))
}

/// One whole server tick and the chat lines it sent.
fn tick_chat(state: &mut TickState, inputs: &[crate::boundary::PlayerInput]) -> Vec<String> {
    let (_, outbox, _) = crate::tick::run_tick(state, inputs);
    outbox
        .into_iter()
        .flat_map(|(_, changes)| changes)
        .filter_map(|change| match change {
            crate::boundary::PlayerChange::Chat { message } => Some(message),
            _ => None,
        })
        .collect()
}

/// Lots examined since the last call (site search cost observer).
fn lots_examined() -> u32 {
    site_refusals::EXAMINED.with(|lots| lots.replace(0))
}

/// Owner D42, after pin 7e421d34c060 (one 4.3 s command tick) and 38c3f1cdf
/// (one inline site slice per command): the command queues the order and
/// answers at once; order turns, one per tick from the tick's site budget,
/// take one slice each and start it as soon as its sweep is complete.
#[test]
fn an_owner_order_answers_at_once_and_its_turns_start_it() {
    let (mut state, id, frontages) = owner_in_open_plan();
    let bound = site_turn::SITE_SLICES * SITE_ATTEMPTS;
    lots_examined();
    let chat = tick_chat(
        &mut state,
        &[crate::boundary::PlayerInput::Command {
            connection: mc_domain::ConnectionId(7),
            command: "settlement build house".into(),
            permission_level: 0,
        }],
    );
    assert_eq!(chat, ["Queued house: waiting for a site"]);
    let lots = lots_examined();
    assert!(lots <= bound, "the command tick examined {lots} lots");
    let record = state.settlements.get(id).unwrap();
    assert!(record.projects.is_empty());
    assert_eq!(
        access::orders(record)
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["house"]
    );
    let slices = frontages.div_ceil(SITE_ATTEMPTS as usize);
    let mut ticks = 0;
    let mut chat = Vec::new();
    while state.settlements.get(id).unwrap().projects.is_empty() {
        chat.extend(tick_chat(&mut state, &[]));
        ticks += 1;
        let lots = lots_examined();
        assert!(
            lots <= site_turn::SITE_SLICES * SITE_ATTEMPTS,
            "tick {ticks} examined {lots} lots"
        );
        assert!(
            ticks <= slices,
            "{ticks} ticks after the command for a sweep of {slices} slices ({frontages} frontages)"
        );
    }
    assert!(
        chat.iter()
            .any(|line| line == "Started house construction.")
    );
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].blueprint, "house");
    assert!(record.projects[0].ordered);
    assert_eq!(access::orders(record).count(), 0);
}

/// A settlement six chunks east per `index` with the fixture's own layout: its
/// three-by-three chunks loaded flat, its anchor chunk simulated and a chest
/// holding one house bill. Plans of radius 48 then never overlap.
fn crowd_settlement(state: &mut TickState, index: i32) -> SettlementId {
    let (cx, x0) = (6 * index, 96 * index);
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter(
            (cx - 1..=cx + 1)
                .flat_map(|x| (-1..=1).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))),
        ));
    for x in x0 - 16..x0 + 32 {
        for z in -16i32..32 {
            state
                .world
                .set_block(x, 200, z, state.registry.states.stone);
            for y in 201..224 {
                state.world.set_block(x, y, z, state.registry.states.air);
            }
        }
    }
    let chest = (x0 + 1, 201, 1);
    state.world.set_block(
        chest.0,
        chest.1,
        chest.2,
        block(state, "minecraft:chest", &[]),
    );
    state.world.fold_overlay();
    state.block_entities.insert(
        chest,
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 27],
            furnace: None,
        },
    );
    state.tickets.insert(
        format!("crowd{index}"),
        (crate::boundary::ChunkPos { x: cx, z: 0 }, 0),
    );
    state.refresh_active_chunks(0);
    let id = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            format!("crowd{index}"),
            "owner".into(),
            [x0 + 8, 201, 8],
            48,
        )
        .unwrap();
    state.settlements.get_mut(id).unwrap().bootstrap_storage = Some([chest.0, chest.1, chest.2]);
    fund_at(state, chest, "house", 1);
    id
}

/// Owner D42: many owners ordering in one tick share the server's one site
/// budget. No tick examines more than its slices, however many orders wait;
/// the order turns serve the settlements in turn, and every order starts.
#[test]
fn simultaneous_orders_share_one_bounded_site_budget_and_all_start() {
    let (mut state, first, _) = owner_in_open_plan();
    let mut ids = vec![first];
    for index in 1..4 {
        ids.push(crowd_settlement(&mut state, index));
    }
    let bound = site_turn::SITE_SLICES * SITE_ATTEMPTS;
    lots_examined();
    // Every order arrives in the same tick: the requests and that tick's site
    // work share one budget.
    for &id in &ids {
        assert_eq!(
            request_build(&mut state, id, "house").unwrap(),
            OrderOutcome::Queued {
                reason: site_refusals::WAITING_FOR_SITE.into()
            }
        );
    }
    let mut turns = Vec::new();
    for tick in 0..400 {
        crate::settlements::construction::tick(&mut state);
        let lots = lots_examined();
        assert!(lots <= bound, "tick {tick} examined {lots} lots");
        turns.push(state.settlements.site_turn);
        if ids.iter().all(|&id| {
            access::projects(state.settlements.get(id).unwrap())
                .next()
                .is_some()
        }) {
            break;
        }
        state.tick += 1;
    }
    assert_eq!(
        turns[..ids.len()],
        ids.iter().copied().map(Some).collect::<Vec<_>>(),
        "the first turns serve every settlement once, in id order"
    );
    for id in ids {
        let record = state.settlements.get(id).unwrap();
        assert_eq!(
            access::projects(record)
                .map(|project| project.blueprint.as_str())
                .collect::<Vec<_>>(),
            ["house"],
            "settlement {id:?} started its order: {:?}; refusals {:?}",
            pending_reason_at(&state, id, 0),
            state.settlements.order_refusal
        );
        assert_eq!(access::orders(record).count(), 0);
    }
}

/// Owner D42, upgrades too: a funded upgrade's command checks and queues it
/// and admits nothing, so no project exists before its order turn; the turn,
/// with a slice of the shared budget, starts it without any debit.
#[test]
fn a_funded_upgrade_is_queued_and_starts_on_its_order_turn() {
    let (mut state, id, slot) = super::tests::fixture();
    super::tests::start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    super::tests::complete(&mut state, id, slot);
    let building = *state
        .settlements
        .get(id)
        .unwrap()
        .buildings
        .keys()
        .next()
        .unwrap();
    state.tickets.insert(
        "upgrade".into(),
        (crate::boundary::ChunkPos { x: 0, z: 0 }, 0),
    );
    state.refresh_active_chunks(0);
    fund(&mut state, "house", 2);
    let stock = state.block_entities[&(1, 201, 1)].slots.clone();
    assert_eq!(
        request_upgrade(&mut state, id, building).unwrap(),
        OrderOutcome::Queued {
            reason: READY.into()
        }
    );
    let record = state.settlements.get(id).unwrap();
    assert!(record.projects.is_empty(), "the command admits nothing");
    assert_eq!(
        access::orders(record)
            .map(|order| (order.level, order.building))
            .collect::<Vec<_>>(),
        [(2, Some(building))]
    );
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
    // Only the order turn runs this tick, not the four-tick planner pass.
    state.tick = state.tick / 4 * 4 + 1;
    crate::settlements::construction::tick(&mut state);
    let record = state.settlements.get(id).unwrap();
    assert_eq!(
        access::projects(record)
            .map(|project| (project.level, project.building, project.ordered))
            .collect::<Vec<_>>(),
        [(2, Some(building), true)]
    );
    assert_eq!(access::orders(record).count(), 0);
    assert_eq!(state.block_entities[&(1, 201, 1)].slots, stock);
}

/// A crown deep inside the lot block north-east of the fixture's bell: on
/// the fixture's loaded ground it takes no lot, road, verge or access.
const BLOCK_ROOT: [i32; 3] = [24, 201, 24];

/// The fixture town in its open plan with more street frontages than one
/// tick's site budget.
fn town_with_long_walk() -> (TickState, SettlementId) {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    let frontages = layout::Network::for_construction(&state, state.settlements.get(id).unwrap())
        .frontage()
        .len();
    assert!(
        frontages > (site_turn::SITE_SLICES * SITE_ATTEMPTS) as usize,
        "{frontages} frontages"
    );
    (state, id)
}

/// Whether house blueprint `blueprint` has a free lot anywhere on the
/// settlement's walk: one whole scan.
fn free_lot_anywhere(
    state: &TickState,
    id: SettlementId,
    blueprint: &blueprints::Blueprint,
) -> bool {
    let mut network = layout::Network::for_construction(state, state.settlements.get(id).unwrap());
    let fronts = network.frontage();
    matches!(
        crowned_house_lot(
            state,
            &mut network,
            &fronts,
            blueprint,
            &mut CrownScan::default(),
            &mut LotChecks::new(u32::MAX)
        ),
        Some(HouseLot::Free(_))
    )
}

fn houses(state: &TickState) -> impl Iterator<Item = &'static blueprints::Blueprint> {
    blueprints::all(&state.registry)
        .iter()
        .filter(|blueprint| blueprint.kind == BuildingKind::House)
}

/// A crown over the south lot row of that block, clear of its door row: it
/// takes the row's lots but no road, verge or building access.
const EDGE_ROOT: [i32; 3] = [24, 201, 16];

/// Owner D42: a replanting proof is site search under the tick's shared
/// budget. In the open plan every lot is free: the first proof takes a slice,
/// finds that its crown takes a free lot and scans its crowned walk to
/// another, which becomes the village's hint; the next proof re-checks only
/// the hint (house blueprints of one footprint share the check); a tick's
/// third proof finds no slice left and waits ("not yet").
#[test]
fn replanting_proofs_with_a_free_lot_plant_within_the_tick_budget() {
    let (mut state, _) = town_with_long_walk();
    super::tests::fresh_site_budget(&mut state);
    lots_examined();
    assert_eq!(
        replant_verdict(&mut state, EDGE_ROOT),
        Replant::Plant,
        "a free house lot remains, so the crown is planted"
    );
    let first = lots_examined();
    assert!((1..=SITE_ATTEMPTS).contains(&first), "{first} lots");
    assert_eq!(replant_verdict(&mut state, EDGE_ROOT), Replant::Plant);
    let second = lots_examined();
    assert!(
        (1..=2).contains(&second),
        "one crowned check of the hint lot per house blueprint: {second}"
    );
    assert_eq!(
        replant_verdict(&mut state, EDGE_ROOT),
        Replant::NotYet,
        "the tick's two slices are spent"
    );
    assert_eq!(lots_examined(), 0);
    super::tests::fresh_site_budget(&mut state);
    assert_eq!(
        replant_verdict(&mut state, EDGE_ROOT),
        Replant::Plant,
        "the next tick plants"
    );
}

/// Owner D42: with no free house lot anywhere (every lot block holds a
/// building) nothing can be lost. A proof checks only the lots its crown
/// takes, all in this proof and within one slice, and plants; it never scans
/// the walk. A crown that takes no lot needs no check at all.
#[test]
fn replanting_without_a_free_lot_checks_only_the_lots_its_crown_takes() {
    let (mut state, id) = town_with_long_walk();
    build_over_lot_blocks(&mut state, id, None);
    for blueprint in houses(&state) {
        assert!(
            !free_lot_anywhere(&state, id, blueprint),
            "{}",
            blueprint.id
        );
    }
    super::tests::fresh_site_budget(&mut state);
    lots_examined();
    assert_eq!(replant_verdict(&mut state, EDGE_ROOT), Replant::Plant);
    let lots = lots_examined();
    assert!((1..=SITE_ATTEMPTS).contains(&lots), "{lots} lots");
    assert_eq!(replant_verdict(&mut state, BLOCK_ROOT), Replant::Plant);
    assert_eq!(lots_examined(), 0, "the deeper crown takes no lot");
}

/// Every lot row deeper than the door row of the fixture's 4 x 4 lot blocks
/// (but `spare`) is built over, so every lot of every house blueprint there
/// meets a building; roads, verges and door rows stay free and the walk keeps
/// every frontage.
fn build_over_lot_blocks(state: &mut TickState, id: SettlementId, spare: Option<[i32; 2]>) {
    let record = state.settlements.get_mut(id).unwrap();
    let anchor = record.anchor;
    for (index, [i, j]) in (-2..2)
        .flat_map(|i| (-2..2).map(move |j| [i, j]))
        .enumerate()
        .filter(|(_, block)| Some(*block) != spare)
    {
        let min = [
            anchor[0] + i * layout::PERIOD + 5,
            anchor[2] + j * layout::PERIOD + 5,
        ];
        let max = [min[0] + layout::PERIOD - 10, min[1] + layout::PERIOD - 10];
        let building = BuildingId((id.0 << 32) | (index as u64 + 1));
        record.buildings.insert(
            building,
            buildings::Building {
                id: building,
                settlement: id,
                kind: BuildingKind::House,
                level: 1,
                bounds: ScriptWorldBounds::new([min[0], 200, min[1]], [max[0], 204, max[1]])
                    .unwrap(),
                work_points: Vec::new(),
                revision: 1,
                natural_zone: false,
            },
        );
    }
}

/// One proof of `crown` for settlement `id` with the tick's shared budget.
fn crown_proof(state: &mut TickState, id: SettlementId, crown: ScriptWorldBounds) -> Replant {
    let mut budget = std::mem::take(&mut state.settlements.site_budget);
    let verdict = layout::planting_verdict(
        state,
        state.settlements.get(id).unwrap(),
        crown,
        &mut budget,
    );
    state.settlements.site_budget = budget;
    verdict
}

/// Owner D42 and the stale-proof review: a proof never plants on an earlier
/// answer. With every lot built over a proof plants (nothing to lose); then
/// the north-east block is cleared down to its south row, and a crown over
/// that row would take the village's only free lots. Its lumberjack, far
/// from every player, asks once per brain run (32 ticks apart, with the
/// server's site work between): the first proof finds the taken free lots,
/// the crowned walk's scan for another needs more than that proof's lots and
/// continues over the next runs, each proof within its slice; no proof
/// plants, and the walk scanned whole refuses. Negative controls: a stale
/// "no free lot" plants at once; a scan that restarts every proof, or is
/// pruned between brain runs, never refuses.
#[test]
fn a_crown_over_the_last_free_lots_is_refused_within_the_tick_budget() {
    let (mut state, id) = town_with_long_walk();
    build_over_lot_blocks(&mut state, id, None);
    super::tests::fresh_site_budget(&mut state);
    assert_eq!(
        replant_verdict(&mut state, BLOCK_ROOT),
        Replant::Plant,
        "no free lot, nothing to lose"
    );
    let anchor = state.settlements.get(id).unwrap().anchor;
    // The block's first lot column, and the last column a lot of its south
    // row reaches (a lot is at most `LOT_DEPTH` deep): the block's building
    // now starts beyond that and the one-block building buffer.
    let first = [anchor[0] + 3, anchor[2] + 3];
    let row = {
        let record = state.settlements.get_mut(id).unwrap();
        let building = record
            .buildings
            .values_mut()
            .find(|building| {
                let bounds = building.bounds;
                [0, 2]
                    .into_iter()
                    .all(|axis| (bounds.min[axis]..=bounds.max[axis]).contains(&BLOCK_ROOT[axis]))
            })
            .unwrap();
        let [x, y, z] = building.bounds.min;
        let deeper = first[1] + layout::LOT_DEPTH + 1;
        assert!(deeper > z && deeper <= building.bounds.max[2]);
        building.bounds = ScriptWorldBounds::new([x, y, deeper], building.bounds.max).unwrap();
        deeper - 2
    };
    assert!(
        houses(&state).any(|blueprint| free_lot_anywhere(&state, id, blueprint)),
        "the south row is free again"
    );
    let crown = ScriptWorldBounds::new(
        [first[0], 201, first[1]],
        [anchor[0] + layout::PERIOD - 3, 265, row],
    )
    .unwrap();
    state.tick = 1;
    let mut verdicts = Vec::new();
    for run in 0..64 {
        // Never a planner tick: the proof has the tick's whole budget.
        state.tick += 32;
        crate::settlements::construction::tick(&mut state);
        lots_examined();
        let verdict = crown_proof(&mut state, id, crown);
        let lots = lots_examined();
        assert!(
            lots <= SITE_ATTEMPTS,
            "run {run}: the proof examined {lots} lots"
        );
        assert_ne!(verdict, Replant::Plant, "run {run}");
        verdicts.push(verdict);
        if verdict == Replant::Refused {
            break;
        }
    }
    assert_eq!(
        verdicts.first(),
        Some(&Replant::NotYet),
        "the crowned walk takes more than one proof"
    );
    assert_eq!(
        verdicts.last(),
        Some(&Replant::Refused),
        "the crowned walk was scanned whole: {verdicts:?}"
    );
}

/// The sweep is runtime scratch: a restart mid-sweep keeps only the saved
/// order, whose next planning pass re-derives the sweep from its first slice
/// and still starts it.
#[test]
fn a_restart_mid_sweep_rederives_the_search_and_starts_the_order() {
    let (mut state, id, frontages) = owner_in_open_plan();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: site_refusals::WAITING_FOR_SITE.into()
        }
    );
    assert_eq!(examined(&state, id), None, "the command examines no lot");
    plan(&mut state, id);
    plan(&mut state, id);
    assert_eq!(examined(&state, id), Some(2 * SITE_ATTEMPTS));
    let restored: crate::settlements::SettlementRecord =
        serde_json::from_slice(&serde_json::to_vec(state.settlements.get(id).unwrap()).unwrap())
            .unwrap();
    crate::settlements::validate_record(&restored).unwrap();
    state.settlements.site_sweep.clear();
    state.settlements.site_watch.clear();
    state.settlements.order_refusal.clear();
    state.settlements.site_orders.clear();
    state
        .settlements
        .apply_image(crate::settlements::RecordImage {
            id,
            value: Some(restored),
        });
    plan(&mut state, id);
    assert_eq!(examined(&state, id), Some(SITE_ATTEMPTS));
    for _ in 0..frontages {
        if !state.settlements.get(id).unwrap().projects.is_empty() {
            break;
        }
        plan(&mut state, id);
    }
    let record = state.settlements.get(id).unwrap();
    assert_eq!(record.projects[0].blueprint, "house");
    assert_eq!(access::orders(record).count(), 0);
}

/// The house lots the order's first sweep slice examines, in sweep order.
fn first_slice_lots(state: &TickState, id: SettlementId) -> Vec<layout::Lot> {
    let blueprint = blueprints::get(&state.registry, "house").unwrap();
    let mut network = layout::Network::for_construction(state, state.settlements.get(id).unwrap());
    network
        .frontage()
        .into_iter()
        .take(SITE_ATTEMPTS as usize)
        .filter_map(|(front, normal)| network.lot(blueprint, front, normal))
        .collect()
}

/// Pin 38c3f1cdf: a lot that covers a village path proves the walk around it,
/// and every such candidate rescanned the whole territory's walk from the
/// world (70% of the probe's 0.93 s order tick). The walk does not depend on
/// the lot, so one site pass scans it once however many of its lots cut a
/// path; a lone check of each lot still scans it for that lot.
#[test]
fn a_site_pass_scans_the_village_walk_once() {
    let (mut state, id, _) = owner_in_open_plan();
    let path = relayout::dirt_path(&state).expect("dirt path state");
    for lot in first_slice_lots(&state, id) {
        state
            .world
            .set_block(lot.origin[0] + 1, lot.origin[1], lot.origin[2] + 1, path);
    }
    state.world.fold_overlay();
    assert_eq!(
        request_build(&mut state, id, "house").unwrap(),
        OrderOutcome::Queued {
            reason: site_refusals::WAITING_FOR_SITE.into()
        }
    );
    relayout::WALKS.with(|walks| walks.set(0));
    plan(&mut state, id);
    assert_eq!(examined(&state, id), Some(SITE_ATTEMPTS));
    assert_eq!(
        relayout::WALKS.with(std::cell::Cell::get),
        1,
        "the order's site pass scans the village walk once"
    );
    relayout::WALKS.with(|walks| walks.set(0));
    let blueprint = blueprints::get(&state.registry, "house").unwrap();
    let record = state.settlements.get(id).unwrap();
    for lot in first_slice_lots(&state, id) {
        let _ = site_refusals::check(
            &state,
            record,
            lot.origin,
            blueprint.footprint(Rotation::from_quarter_turns(lot.rotation)),
            record.founding.as_ref(),
            &mut Default::default(),
            Some(&site_refusals::floor_columns(blueprint, lot.rotation)),
        );
    }
    let alone = relayout::WALKS.with(std::cell::Cell::get);
    assert!(
        alone > 1,
        "the slice holds several path-cutting lots, each proved alone: {alone}"
    );
}
