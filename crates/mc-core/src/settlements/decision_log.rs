//! Runtime-only observations for `MC_CORE_PHASE_LOG`. No simulation dependencies.
//!
//! The planner is the settlement's slowest and quietest decider: a town can sit
//! a whole run without a project, and the existing `tracing::debug!` lines say
//! what one branch refused but not which branch the pass took. This module
//! records that verdict, and an aggregate of the courtship gates per town, on
//! the phase log's cadence. It decides nothing: every value is read from state
//! the pass already computed, and nothing here is persisted.

pub(crate) fn enabled() -> bool {
    static ENABLED: std::sync::LazyLock<bool> =
        std::sync::LazyLock::new(|| std::env::var_os("MC_CORE_PHASE_LOG").is_some());
    #[cfg(test)]
    if FORCED.load(std::sync::atomic::Ordering::SeqCst) {
        return true;
    }
    *ENABLED
}

/// Test-only override for [`enabled`]: the environment is read once per
/// process, so a test cannot set `MC_CORE_PHASE_LOG` for its own run. The guard
/// clears the flag again on drop, including on a panic.
#[cfg(test)]
static FORCED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[cfg(test)]
pub(crate) struct ForcedEnabled;

#[cfg(test)]
impl ForcedEnabled {
    /// Observe as `MC_CORE_PHASE_LOG=1` would, until the guard is dropped.
    #[allow(dead_code)]
    pub(crate) fn take() -> Self {
        FORCED.store(true, std::sync::atomic::Ordering::SeqCst);
        Self
    }
}

#[cfg(test)]
impl Drop for ForcedEnabled {
    fn drop(&mut self) {
        FORCED.store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Rate limit for one settlement's sampled records: the first eligible tick,
/// then the next eligible tick at least a minute of simulation later.
#[derive(Debug, Default)]
pub(crate) struct Window(Option<u64>);

impl Window {
    /// Take the tick when it is at least 1200 ticks past the last one taken.
    /// Round-robin towns need not visit tick % 1200, so eligibility is checked
    /// when the town is visited instead of on a fixed tick.
    pub(super) fn claim(&mut self, tick: u64) -> bool {
        if self.0.is_some_and(|last| tick.saturating_sub(last) < 1200) {
            return false;
        }
        self.0 = Some(tick);
        true
    }
}

/// One sampled planner pass: which branch of `plan_pass` decided, and the site
/// facts of the branches that searched one.
#[derive(Default)]
pub(super) struct Plan {
    pub(super) verdict: &'static str,
    pub(super) storage_slots: Option<(usize, usize)>,
    pub(super) storage_site: Option<Site>,
    pub(super) priority_house: Option<Site>,
    pub(super) ordinary_house_funded: Option<bool>,
    pub(super) ordinary_house_sweep: Option<&'static str>,
    pub(super) ordinary_site: Option<Site>,
    pub(super) upgrades: Upgrades,
}

/// Upgrade candidates examined by the pass in one bounded loop. The planner
/// caps that loop at `construction::UPGRADE_CANDIDATES`; the record keeps the
/// same bound so a refusal list stays small on disk.
#[derive(Default)]
pub(super) struct Upgrades {
    pub(super) eligible: usize,
    pub(super) funded: [(u64, &'static str); 4],
    pub(super) count: usize,
}

impl Plan {
    pub(super) fn emit(&self, town: u64, tick: u64, game_time: u64, started: bool) {
        tracing::info!(
            settlement = town,
            tick,
            game_time,
            started,
            verdict = self.verdict,
            storage_free_total = ?self.storage_slots,
            storage_site = ?self.storage_site,
            priority_house = ?self.priority_house,
            ordinary_house_funded = ?self.ordinary_house_funded,
            ordinary_house_sweep = ?self.ordinary_house_sweep,
            ordinary_site = ?self.ordinary_site,
            eligible_upgrades = self.upgrades.eligible,
            funded_upgrades = ?&self.upgrades.funded[..self.upgrades.count],
            "settlement_planner_verdict"
        );
    }
}

/// How one funded-kind sweep ended, as the planner saw it.
#[derive(Clone, Copy)]
pub(super) struct Site {
    pub(super) outcome: &'static str,
    pub(super) frontages: u32,
    pub(super) examined: u32,
    pub(super) unfunded: u32,
}

impl std::fmt::Debug for Site {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sweep")
            .field("outcome", &self.outcome)
            .field("frontages", &self.frontages)
            .field("examined", &self.examined)
            .field("unfunded", &self.unfunded)
            .finish()
    }
}

/// One town's courtship gates, aggregated between two sampled windows.
#[derive(Debug, Default)]
pub(crate) struct Courtship {
    window: Window,
    pub(crate) counts: CourtshipCounts,
}

/// Every gate a courtship decision passes, so a town that courts but never
/// pairs, or pairs but never births, names the step that stopped it.
#[derive(Debug, Default)]
pub(crate) struct CourtshipCounts {
    pub(crate) actors: u64,
    pub(crate) admitted: u64,
    pub(crate) unpaired_on_cadence: u64,
    pub(crate) willing: u64,
    pub(crate) ineligible: WillingRefusals,
    pub(crate) neighbors_in_range: u64,
    pub(crate) willing_neighbors: u64,
    pub(crate) compatible_pair: u64,
    pub(crate) ration_wait: u64,
    pub(crate) pairing_bed: BedCounts,
    pub(crate) started: u64,
    pub(crate) paired: u64,
    pub(crate) missing_partner: u64,
    pub(crate) invalid_pair: u64,
    pub(crate) due: u64,
    pub(crate) due_in_range: u64,
    pub(crate) birth_bed: BedCounts,
    pub(crate) born: u64,
}

#[derive(Debug, Default)]
pub(crate) struct WillingRefusals {
    pub(crate) child: u64,
    pub(crate) cooldown: u64,
    pub(crate) sleeping: u64,
    pub(crate) panic: u64,
    pub(crate) food: u64,
    pub(crate) reserved_food: u64,
}

/// Outcomes of one spare-bed search (`allocator::search_free_bed`).
#[derive(Debug, Default)]
pub(crate) struct BedCounts {
    pub(crate) found: u64,
    pub(crate) no_candidate: u64,
    pub(crate) route_failed: u64,
    pub(crate) budget_deferred: u64,
}

impl Courtship {
    /// Sample a town once per eligible window: take the counts accumulated
    /// since the previous sample and log them. The first window has no earlier
    /// sample to close, so it only arms the window.
    pub(crate) fn observe(&mut self, settlement: u64, tick: u64, game_time: u64) {
        let previous = self.window.0;
        if !self.window.claim(tick) {
            return;
        }
        let Some(from_tick) = previous else { return };
        let c = std::mem::take(&mut self.counts);
        tracing::info!(
            settlement,
            from_tick,
            tick,
            game_time,
            actors = c.actors,
            admitted = c.admitted,
            unpaired_on_cadence = c.unpaired_on_cadence,
            willing = c.willing,
            neighbors_in_range = c.neighbors_in_range,
            initiator_refusals = ?c.ineligible,
            willing_neighbors = c.willing_neighbors,
            compatible_pair = c.compatible_pair,
            ration_wait = c.ration_wait,
            pairing_bed_found = c.pairing_bed.found,
            pairing_bed_no_candidate = c.pairing_bed.no_candidate,
            pairing_bed_route_failed = c.pairing_bed.route_failed,
            pairing_bed_budget_deferred = c.pairing_bed.budget_deferred,
            started = c.started,
            paired = c.paired,
            missing_partner = c.missing_partner,
            invalid_pair = c.invalid_pair,
            due = c.due,
            due_in_range = c.due_in_range,
            birth_bed_found = c.birth_bed.found,
            birth_bed_no_candidate = c.birth_bed.no_candidate,
            birth_bed_route_failed = c.birth_bed.route_failed,
            birth_bed_budget_deferred = c.birth_bed.budget_deferred,
            born = c.born,
            "settlement_courtship_summary"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Write};
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Output(Arc<Mutex<Vec<u8>>>);

    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().write(bytes)
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn courtship_summary_emits_once_per_eligible_window_per_town() {
        let output = Output::default();
        let writer = output.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .without_time()
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let mut a = Courtship::default();
            let mut b = Courtship::default();
            a.observe(1, 43, 10);
            b.observe(2, 80, 20);
            a.counts.pairing_bed.route_failed = 1;
            b.counts.birth_bed.budget_deferred = 1;
            for tick in [100, 1_242, 1_243, 1_243, 1_260, 1_279, 1_280, 2_442, 2_443] {
                a.observe(1, tick, tick / 2);
                b.observe(2, tick, tick / 2);
            }
        });
        let text = String::from_utf8(output.0.lock().unwrap().clone()).unwrap();
        let events: Vec<_> = text
            .lines()
            .filter(|line| line.contains("settlement_courtship_summary"))
            .collect();
        assert_eq!(events.len(), 3, "{text}");
        assert!(events[0].contains("settlement=1 from_tick=43 tick=1243 game_time=621"));
        assert!(events[0].contains("pairing_bed_route_failed=1"));
        assert!(events[1].contains("settlement=2 from_tick=80 tick=1280 game_time=640"));
        assert!(events[1].contains("birth_bed_budget_deferred=1"));
        assert!(events[2].contains("settlement=1 from_tick=1243 tick=2443 game_time=1221"));
        assert!(events[2].contains("pairing_bed_route_failed=0"));
    }
}
