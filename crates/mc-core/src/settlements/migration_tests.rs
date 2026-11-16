use super::*;

fn rich() -> Facts {
    Facts {
        members: 10,
        beds: 20,
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
    assert_eq!(OUTSIDE_ATTRACTIVENESS, 35.0);
    assert_eq!(gain(35.0, 45.0, 0.0), MIGRATION_THRESHOLD);
    assert_eq!(gain(35.0, 60.0, 200.0), 15.0);
    assert!(gain(35.0, 60.0, 512.0) < MIGRATION_THRESHOLD);
}
