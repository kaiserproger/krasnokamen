//! Settlement migration policy and its derived, non-persisted observations.
//! Native membership and transit custody remain their existing owners.

use super::SettlementId;
use std::collections::BTreeMap;

const DAY_TICKS: u64 = 24_000;
const FOOD_PER_RESIDENT: f64 = 12.0;
const DEVELOPED_BUILDINGS: f64 = 12.0;
const OUTSIDE_ATTRACTIVENESS: f64 = 35.0;
const MIGRATION_THRESHOLD: f64 = 10.0;
const DISTANCE_COST_PER_BLOCK: f64 = 0.05;
const SCORE_SCALE: f64 = 100.0;
const HOUSING_WEIGHT: f64 = 0.30;
const FOOD_WEIGHT: f64 = 0.30;
const JOBS_WEIGHT: f64 = 0.15;
const SAFETY_WEIGHT: f64 = 0.15;
const DEVELOPMENT_WEIGHT: f64 = 0.10;

/// Current physical inputs; these observations never become membership authority.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Facts {
    pub(crate) members: usize,
    pub(crate) beds: usize,
    pub(crate) inbound: usize,
    pub(crate) food_points: u64,
    pub(crate) open_jobs: usize,
    pub(crate) recent_deaths: usize,
    pub(crate) completed_buildings: usize,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Attractiveness {
    pub(crate) housing: f64,
    pub(crate) food: f64,
    pub(crate) jobs: f64,
    pub(crate) safety: f64,
    pub(crate) development: f64,
}

impl Attractiveness {
    fn from_facts(facts: &Facts) -> Self {
        let population = facts.members.max(1) as f64;
        Self {
            housing: (facts
                .beds
                .saturating_sub(facts.members)
                .saturating_sub(facts.inbound) as f64
                / population)
                .clamp(0.0, 1.0),
            food: (facts.food_points as f64 / (FOOD_PER_RESIDENT * population)).clamp(0.0, 1.0),
            jobs: (facts.open_jobs as f64 / population).clamp(0.0, 1.0),
            safety: 1.0 - (facts.recent_deaths as f64 / population).clamp(0.0, 1.0),
            development: (facts.completed_buildings as f64 / DEVELOPED_BUILDINGS).clamp(0.0, 1.0),
        }
    }

    pub(crate) fn total(&self) -> f64 {
        SCORE_SCALE
            * (HOUSING_WEIGHT * self.housing
                + FOOD_WEIGHT * self.food
                + JOBS_WEIGHT * self.jobs
                + SAFETY_WEIGHT * self.safety
                + DEVELOPMENT_WEIGHT * self.development)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Observation {
    pub(crate) game_time: u64,
    pub(crate) facts: Facts,
    pub(crate) attractiveness: Attractiveness,
}

/// One host-owned runtime state. No serde, journal image, or saved sequence.
#[derive(Debug, Default)]
pub(crate) struct State {
    observations: BTreeMap<SettlementId, Observation>,
}

impl State {
    pub(crate) fn observe(&mut self, id: SettlementId, game_time: u64, facts: Facts) {
        self.observations.insert(
            id,
            Observation {
                game_time,
                facts,
                attractiveness: Attractiveness::from_facts(&facts),
            },
        );
    }

    pub(crate) fn observation(&self, id: SettlementId) -> Option<&Observation> {
        self.observations.get(&id)
    }

    pub(crate) fn remove(&mut self, id: SettlementId) {
        self.observations.remove(&id);
    }
}

fn phase(id: SettlementId) -> u64 {
    crate::natural_spawn::Rng(id.0).below(DAY_TICKS)
}

fn due(id: SettlementId, game_time: u64) -> bool {
    game_time % DAY_TICKS == phase(id)
}

fn gain(source: f64, destination: f64, distance: f64) -> f64 {
    destination - source - DISTANCE_COST_PER_BLOCK * distance
}

#[cfg(test)]
#[path = "migration_tests.rs"]
mod tests;
