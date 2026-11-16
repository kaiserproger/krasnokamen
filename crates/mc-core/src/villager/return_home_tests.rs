use super::*;
use crate::boundary::{ChangeSet, EntityId};
use crate::settlements::migration::return_tests::{fixture, house, resident, town};
use crate::tick_budget::Kind;
use crate::villager::Activity;

fn run(state: &mut TickState, id: EntityId, useful: bool) -> bool {
    let slot = state.entities.slot_of(id).unwrap();
    let mut v = state
        .entities
        .row_mut_unmarked(slot)
        .villager
        .take()
        .unwrap();
    let mut cache = std::mem::take(&mut state.villager_pois);
    let owned = step(state, &mut cache, slot, &mut v, useful);
    if owned {
        let unchanged = !allocator::route_changed(state, &v);
        crate::villager::brain::move_committed_with_route(
            state,
            slot,
            &mut v,
            Activity::Work,
            unchanged,
        );
    }
    state.entities.row_mut_unmarked(slot).villager = Some(v);
    state.villager_pois = cache;
    owned
}

fn home_search(state: &mut TickState, id: EntityId) {
    for _ in 0..20 {
        state.tick_budget.reset(state.tick);
        allocator::tick(state, &mut ChangeSet::default());
        state.tick += 1;
        if state.villager_pois.home_search.contains_key(&id) {
            run(state, id, false); // establish the restart-safe initial stagger
            return;
        }
    }
    panic!("native Home search did not finish");
}

fn blocked() -> (TickState, EntityId, uuid::Uuid) {
    let mut state = fixture();
    let town = town(&mut state, "blocked", 8, 32);
    let id = resident(&mut state, town, 8.5);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, [90.5, 64.0, 8.5]);
    let uuid = state.entities.row(slot).uuid;
    for y in 64..84 {
        for z in -16..32 {
            state.world.set_block(50, y, z, state.registry.states.stone);
        }
    }
    state.world.fold_overlay();
    home_search(&mut state, id);
    state.tick = 80;
    (state, id, uuid)
}

#[test]
fn capped_truncation_spends_three_retries_then_one_search_per_hold_cadence() {
    let (mut state, id, uuid) = blocked();
    let mut distribution = Vec::new();
    for attempt in 0..3 {
        state.tick_budget.reset(state.tick);
        let before = state
            .tick_budget
            .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
        assert!(run(&mut state, id, false));
        let used = before
            - state
                .tick_budget
                .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
        distribution.push(used);
        let memory = &crate::settlements::migration::return_memory(&mut state)[&uuid];
        assert_eq!(memory.failures, attempt + 1);
        assert_eq!(
            used, NODE_LIMIT,
            "open reachable region is larger than the capped search"
        );
        let next = memory.next;
        let failures = memory.failures;
        state.tick = next - 1;
        state.tick_budget.reset(state.tick);
        let queued_before =
            state
                .tick_budget
                .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
        assert!(run(&mut state, id, false));
        assert_eq!(
            crate::settlements::migration::return_memory(&mut state)[&uuid].failures,
            failures
        );
        assert_eq!(
            state
                .tick_budget
                .allowance(state.dimension, Consumer::Brain, Kind::PathNode),
            queued_before
        );
        state.tick = next;
    }
    assert!(crate::settlements::migration::return_memory(&mut state)[&uuid].hold);
    assert_eq!(
        distribution.iter().sum::<usize>(),
        MAX_FAILURES * NODE_LIMIT
    );
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    let next = crate::settlements::migration::return_memory(&mut state)[&uuid].next;
    assert!(next >= state.tick + RETRY_TICKS);
    eprintln!("return search nodes: {distribution:?}; Hold next={next}");
}

#[test]
fn admission_denial_does_not_spend_a_failure_or_bypass_the_retry_deadline() {
    let (mut state, id, uuid) = blocked();
    state.tick_budget.reset(state.tick);
    state.tick_budget.exhaust_for_test(Kind::PathNode);
    run(&mut state, id, false);
    assert_eq!(
        crate::settlements::migration::return_memory(&mut state)[&uuid].failures,
        0
    );
    state.tick += 1;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    let next = crate::settlements::migration::return_memory(&mut state)[&uuid].next;
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, [100.5, 64.0, 8.5]);
    state.tick = next - 1;
    state.tick_budget.reset(state.tick);
    let before = state
        .tick_budget
        .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
    run(&mut state, id, false);
    assert_eq!(
        state
            .tick_budget
            .allowance(state.dimension, Consumer::Brain, Kind::PathNode),
        before
    );
}

#[test]
fn found_but_stalled_segment_fails_and_useful_work_preempts_without_searching() {
    let mut state = fixture();
    let town = town(&mut state, "return", 8, 32);
    let id = resident(&mut state, town, 8.5);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, [180.5, 64.0, 8.5]);
    let uuid = state.entities.row(slot).uuid;
    home_search(&mut state, id);
    state.tick = 80;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    let deadline = crate::settlements::migration::return_memory(&mut state)[&uuid]
        .segment
        .expect("native route")
        .1;
    state.tick = deadline;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    assert_eq!(
        crate::settlements::migration::return_memory(&mut state)[&uuid].failures,
        1
    );
    let before = state
        .tick_budget
        .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
    assert!(!run(&mut state, id, true));
    assert_eq!(
        state
            .tick_budget
            .allowance(state.dimension, Consumer::Brain, Kind::PathNode),
        before
    );
}

#[test]
fn arrival_wakes_one_observed_home_search_and_inside_no_beds_stays_normal() {
    let mut state = fixture();
    let town = town(&mut state, "return", 8, 200);
    house(&mut state, town, 8);
    let id = resident(&mut state, town, 120.5);
    home_search(&mut state, id);
    let slot = state.entities.slot_of(id).unwrap();
    let uuid = state.entities.row(slot).uuid;
    state.entities.set_pos(slot, [36.5, 64.0, 8.5]);
    state.tick = 80;
    state.tick_budget.reset(state.tick);
    assert!(run(&mut state, id, false));
    let wake = crate::settlements::migration::return_memory(&mut state)[&uuid]
        .waiting
        .unwrap();
    assert_eq!(wake, state.tick + 1);
    for _ in 0..3 {
        state.tick += 1;
        state.tick_budget.reset(state.tick);
        allocator::tick(&mut state, &mut ChangeSet::default());
        if state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
        {
            break;
        }
    }
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
    );
    assert!(!run(&mut state, id, false));
    assert!(!crate::settlements::migration::return_memory(&mut state).contains_key(&uuid));

    let mut state = fixture();
    let town = crate::settlements::migration::return_tests::town(&mut state, "full", 8, 200);
    let id = resident(&mut state, town, 100.5);
    home_search(&mut state, id);
    state.tick = 80;
    state.tick_budget.reset(state.tick);
    assert!(!run(&mut state, id, false));
    for _ in 0..10 {
        state.tick += 1;
        assert!(!run(&mut state, id, false));
    }
}

#[test]
fn supported_waypoint_arrival_uses_native_tolerance_and_no_waypoint_is_a_failure() {
    let mut state = fixture();
    let town = town(&mut state, "return", 8, 32);
    let id = resident(&mut state, town, 8.5);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, [180.5, 64.0, 8.5]);
    let uuid = state.entities.row(slot).uuid;
    home_search(&mut state, id);
    state.tick = 80;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    let target = crate::settlements::migration::return_memory(&mut state)[&uuid]
        .segment
        .unwrap()
        .0;
    state.entities.set_pos(
        slot,
        [
            f64::from(target.x) + 0.5 + 0.29,
            f64::from(target.y),
            f64::from(target.z) + 0.5,
        ],
    );
    state.tick += 1;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    let memory = &crate::settlements::migration::return_memory(&mut state)[&uuid];
    assert_eq!(memory.failures, 0);
    assert_ne!(memory.segment.unwrap().0, target);

    state.tickets.clear();
    let tick = state.tick;
    state.refresh_active_chunks(tick);
    crate::settlements::migration::return_memory(&mut state)
        .get_mut(&uuid)
        .unwrap()
        .segment = None;
    state.tick_budget.reset(state.tick);
    run(&mut state, id, false);
    assert_eq!(
        crate::settlements::migration::return_memory(&mut state)[&uuid].failures,
        1
    );
}

#[test]
fn two_hundred_returners_leave_an_ordinary_inside_route_available_within_eighty_ticks() {
    let (mut state, first, _) = blocked();
    let settlement = state
        .entities
        .row(state.entities.slot_of(first).unwrap())
        .villager
        .as_ref()
        .unwrap()
        .settlement
        .unwrap();
    let mut returners = vec![first];
    for _ in 1..200 {
        let id = resident(&mut state, settlement, 8.5);
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_pos(slot, [90.5, 64.0, 8.5]);
        returners.push(id);
    }
    state.settlements.get_mut(settlement).unwrap().radius = 32;
    for _ in 0..20 {
        state.tick_budget.reset(state.tick);
        allocator::tick(&mut state, &mut ChangeSet::default());
        state.tick += 1;
    }
    assert!(returners.iter().all(|id| {
        state
            .villager_pois
            .home_search
            .get(id)
            .is_some_and(|r| r.no_candidates)
    }));
    state.tick = 160;
    state.tick_budget.reset(state.tick);
    for &id in &returners {
        run(&mut state, id, false);
    }
    let mut arrived_at = None;
    // Even adversarial first-come ordering drains the 200-returner initial
    // wave in fewer than 80 ticks with 1024-node searches; an uncapped wave
    // can consume one entire tick per resident.
    for offset in 0..80 {
        state.tick = 200 + offset;
        state.tick_budget.reset(state.tick);
        for &id in &returners {
            run(&mut state, id, false);
        }
        let mut cache = std::mem::take(&mut state.villager_pois);
        let mut path = Vec::new();
        let mut ctx = RouteCtx::tick(&mut state, Consumer::Brain);
        let found = ctx.admit()
            && allocator::find_path_ctx(
                &mut ctx,
                &mut cache,
                [30.5, 64.0, 8.5],
                BlockPos { x: 8, y: 64, z: 8 },
                0.3,
                1.95,
                &mut path,
                true,
                48,
                16,
                true,
            );
        state.villager_pois = cache;
        if found {
            assert_eq!(path.first(), Some(&BlockPos { x: 8, y: 64, z: 8 }));
            arrived_at = Some(offset);
            break;
        }
    }
    assert!(
        arrived_at.is_some(),
        "ordinary inside route must be admitted within 80 ticks behind 200 returners, not merely stay under global allowance"
    );
}

fn row_images(entries: &[crate::journal::JournalEntry], id: EntityId) -> usize {
    use crate::journal::JournalEntry;
    entries
        .iter()
        .map(|entry| match entry {
            JournalEntry::Dimension { entries, .. } => row_images(entries, id),
            JournalEntry::Entity { id: changed, .. }
            | JournalEntry::EntityState { id: changed, .. }
                if *changed == id.0 =>
            {
                1
            }
            _ => 0,
        })
        .sum()
}

#[test]
fn hold_retries_do_not_emit_native_entity_row_postimages() {
    let (mut state, id, uuid) = blocked();
    for _ in 0..3 {
        state.tick_budget.reset(state.tick);
        run(&mut state, id, false);
        state.tick = crate::settlements::migration::return_memory(&mut state)[&uuid].next;
    }
    assert!(crate::settlements::migration::return_memory(&mut state)[&uuid].hold);
    // Establish the real journal's entity baseline, then advance Hold memory,
    // including a capped retry, without changing any durable component.
    let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let baseline = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
    assert!(
        row_images(&baseline, id) > 0,
        "the native journal actually observed this entity"
    );
    let retry = crate::settlements::migration::return_memory(&mut state)[&uuid].next;
    for _ in 0..RETRY_TICKS + STAGGER_TICKS {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        let entries = crate::core::journal_entries_for_tick(&mut state, &output).unwrap();
        assert_eq!(
            row_images(&entries, id),
            0,
            "Hold memory must not generate native row images: {entries:?}"
        );
    }
    assert!(
        crate::settlements::migration::return_memory(&mut state)[&uuid].next > retry,
        "the quiet native tick sequence includes an actual capped Hold retry"
    );
}

#[test]
fn persisted_membership_restarts_an_inside_edge_return_without_a_saved_intent() {
    let mut state = fixture();
    let settlement = town(&mut state, "return", 8, 200);
    house(&mut state, settlement, 8);
    let id = resident(&mut state, settlement, 120.5);
    for _ in 0..300 {
        crate::settlements::migration::return_tests::step(&mut state);
    }
    let slot = state.entities.slot_of(id).unwrap();
    assert!(
        state.entities.pos(slot)[0] < 120.5,
        "physical approach began"
    );
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_none(),
        "restart occurs mid-walk"
    );
    let saved = serde_json::to_vec(state.entities.row(slot).villager.as_ref().unwrap()).unwrap();
    state.entities.row_mut(slot).villager = Some(serde_json::from_slice(&saved).unwrap());
    crate::settlements::migration::return_memory(&mut state).clear();
    state.villager_pois.home_search.clear();
    state.villager_pois.search_at.insert(id, state.tick);
    state.villager_pois.allocator_updates.push(id);
    for _ in 0..5000 {
        crate::settlements::migration::return_tests::step(&mut state);
        if state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
        {
            break;
        }
    }
    assert_eq!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .settlement,
        Some(settlement)
    );
    assert!(
        state
            .entities
            .row(slot)
            .villager
            .as_ref()
            .unwrap()
            .home
            .is_some()
    );
}

#[test]
fn restored_homeless_body_has_one_capped_search_per_twelve_hundred_ticks() {
    let (mut state, id, uuid) = blocked();
    for _ in 0..3 {
        state.tick_budget.reset(state.tick);
        run(&mut state, id, false);
        state.tick = crate::settlements::migration::return_memory(&mut state)[&uuid].next;
    }
    assert!(crate::settlements::migration::return_memory(&mut state)[&uuid].hold);
    let slot = state.entities.slot_of(id).unwrap();
    let persisted =
        serde_json::to_vec(state.entities.row(slot).villager.as_ref().unwrap()).unwrap();
    state.entities.row_mut(slot).villager = Some(serde_json::from_slice(&persisted).unwrap());
    crate::settlements::migration::return_memory(&mut state).clear();
    let start = state.tick;
    let mut searches = 0;
    let mut nodes = 0;
    for offset in 0..RETRY_TICKS {
        state.tick = start + offset;
        state.tick_budget.reset(state.tick);
        let before = state
            .tick_budget
            .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
        run(&mut state, id, false);
        let used = before
            - state
                .tick_budget
                .allowance(state.dimension, Consumer::Brain, Kind::PathNode);
        searches += usize::from(used != 0);
        nodes += used;
    }
    assert_eq!(searches, 1);
    assert_eq!(nodes, NODE_LIMIT);
}
