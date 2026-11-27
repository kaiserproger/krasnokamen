use super::*;

fn rich() -> Facts {
    Facts {
        members: 10,
        beds: 20,
        free_beds: 10,
        inbound: 0,
        food_points: 120,
        open_jobs: 10,
        recent_deaths: 0,
        completed_buildings: 12,
    }
}

#[test]
fn attractiveness_components_use_physical_surplus_and_clamp() {
    let full = Attractiveness::from_facts(&rich());
    assert_eq!(full.housing, 1.0);
    assert_eq!(full.food, 1.0);
    assert_eq!(full.jobs, 1.0);
    assert_eq!(full.safety, 1.0);
    assert_eq!(full.development, 1.0);
    assert!((full.total() - 100.0).abs() < 1e-9);
    let empty = Attractiveness::from_facts(&Facts {
        members: 10,
        recent_deaths: 10,
        ..Facts::default()
    });
    assert_eq!(empty.total(), 0.0);
    let oversupplied = Attractiveness::from_facts(&Facts {
        beds: usize::MAX,
        food_points: u64::MAX,
        open_jobs: usize::MAX,
        completed_buildings: usize::MAX,
        ..rich()
    });
    assert_eq!(oversupplied, full);
}

#[test]
fn inbound_members_reserve_housing_before_they_arrive() {
    let baseline = Attractiveness::from_facts(&Facts { beds: 15, ..rich() });
    let reserved = Attractiveness::from_facts(&Facts {
        beds: 15,
        inbound: 5,
        ..rich()
    });
    assert_eq!(baseline.housing, 0.5);
    assert_eq!(reserved.housing, 0.0);
    assert!((baseline.total() - reserved.total() - 15.0).abs() < 1e-9);
    let overfull = Attractiveness::from_facts(&Facts {
        beds: 0,
        inbound: usize::MAX,
        ..rich()
    });
    assert_eq!(overfull.housing, 0.0);
}

#[test]
fn zero_population_uses_one_as_the_denominator() {
    let score = Attractiveness::from_facts(&Facts {
        members: 0,
        beds: 1,
        food_points: 12,
        open_jobs: 1,
        completed_buildings: 12,
        ..Facts::default()
    });
    assert!(score.total().is_finite());
    assert!((score.total() - 100.0).abs() < 1e-9);
}

#[test]
fn observations_are_replaced_and_unknown_destinations_stay_unknown() {
    let mut state = State::default();
    let town = SettlementId(7);
    assert!(state.observation(town).is_none());
    state.observe(town, 123, rich());
    let first = state.observation(town).unwrap();
    assert_eq!(first.game_time, 123);
    assert_eq!(first.facts, rich());
    state.observe(town, 456, Facts::default());
    let current = state.observation(town).unwrap();
    assert_eq!(current.game_time, 456);
    assert_eq!(current.facts, Facts::default());
    assert_eq!(current.attractiveness.housing, 0.0);
    state.remove(town);
    assert!(state.observation(town).is_none());
}

#[test]
fn daily_phase_is_stable_staggered_and_has_no_catchup_iterations() {
    let mut phases = std::collections::BTreeSet::new();
    for id in (1..=64).map(SettlementId) {
        let at = phase(id);
        assert!(at < DAY_TICKS);
        assert!(due(id, at));
        assert!(due(id, at + DAY_TICKS));
        assert!(due(id, at + 100 * DAY_TICKS));
        assert!(!due(id, at + DAY_TICKS + 1));
        phases.insert(at);
    }
    assert!(phases.len() > 40);
}

#[test]
fn distance_cost_and_outside_score_do_not_hide_the_threshold() {
    assert_eq!(gain(35.0, 45.0, 0.0), MIGRATION_THRESHOLD);
    assert_eq!(gain(35.0, 60.0, 200.0), 15.0);
    assert!(gain(35.0, 60.0, 512.0) < MIGRATION_THRESHOLD);
}

#[test]
fn native_router_cap_and_uncapped_home_search_share_terrain_and_budget() {
    use super::super::allocator::{self, PoiCache, RouteCtx};
    use crate::boundary::BlockPos;
    use crate::tick_budget::{Consumer, Kind};
    let mut state = allocator::tests::test_state();
    let mut cache = PoiCache::default();
    let mut path = Vec::new();
    let target = BlockPos {
        x: 13,
        y: 201,
        z: 13,
    };
    state.tick_budget.reset(state.tick);
    let mut ctx = RouteCtx::tick(&mut state, Consumer::Brain);
    assert!(ctx.admit());
    let before = ctx
        .budget
        .allowance(ctx.dimension, ctx.consumer, Kind::PathNode);
    assert!(!allocator::find_path_limited_ctx(
        &mut ctx,
        &mut cache,
        [2.5, 201.0, 2.5],
        target,
        0.3,
        1.95,
        &mut path,
        true,
        48,
        16,
        true,
        3,
    ));
    assert_eq!(
        before
            - ctx
                .budget
                .allowance(ctx.dimension, ctx.consumer, Kind::PathNode),
        3
    );
    assert!(allocator::find_path_ctx(
        &mut ctx,
        &mut cache,
        [2.5, 201.0, 2.5],
        target,
        0.3,
        1.95,
        &mut path,
        true,
        48,
        16,
        true,
    ));
    assert_eq!(path.first(), Some(&target));
}

#[test]
fn home_search_observation_is_allocator_completion_not_a_brain_scan() {
    use super::super::allocator;
    use crate::boundary::ChangeSet;
    let mut state = allocator::tests::test_state();
    let slot = allocator::tests::villager(&mut state, [2.5, 201.0, 2.5]);
    let id = state.entities.body(slot).id;
    for _ in 0..240 {
        state.tick_budget.reset(state.tick);
        allocator::tick(&mut state, &mut ChangeSet::default());
        if state.villager_pois.home_search.contains_key(&id) {
            break;
        }
        state.tick += 1;
    }
    let result = state
        .villager_pois
        .home_search
        .get(&id)
        .expect("Home searched");
    assert_eq!(result.tick, state.tick);
    assert!(result.no_candidates);
    allocator::tests::bed(&mut state, 8, 8);
    state.world.fold_overlay();
    for _ in 0..480 {
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
    let result = state.villager_pois.home_search.get(&id).unwrap();
    assert_eq!(result.tick, state.tick);
    assert!(!result.no_candidates);
}
