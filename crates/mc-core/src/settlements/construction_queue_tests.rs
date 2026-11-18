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
    const CONN: crate::boundary::ConnectionId = crate::boundary::ConnectionId(7);
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
            connection: crate::boundary::ConnectionId(7),
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
