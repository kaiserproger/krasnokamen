//! Bounded production record of at-risk exposure for the dismounted-rider
//! bound (`BATTLE-DISMOUNTED-RIDER-SKATES-TO-SWEEP-POINT`).
//!
//! `cavalry::sweep_drives` drives the steady, untargeted, unwounded, unrouted
//! members of a regiment with a sweep plan toward the plan point. Before the
//! seat check, an unseated member took that heading — a displacement in blocks —
//! as a drive in blocks per second, and skated (up to 107 blocks per second in
//! the field-1k takes). The client wire cannot see the plan, `active`,
//! `wounded` or `routing`, and its "untargeted" proxy measured 0.36/0.18
//! precision against server truth, so the campaign bound could not be counted
//! from it.
//!
//! This module records the server's own exposure: an episode is a maximal run
//! of consecutive battle ticks in which one member passes exactly the
//! `sweep_drives` guard set and is **not** seated — the member the pre-fix code
//! would have driven.
//!
//! ## Phase contract
//!
//! The guard set is evaluated in the battle step of server tick `T`, which runs
//! before the entity motion of that tick (`tick/entity_phase.rs`), and the
//! motion it causes is published under tick `T+1` (the recorder's
//! `krasnokamen:tick` marker, `fanout.rs`, `TickOutput::tick`). So an episode
//! whose condition holds on ticks `T0..=Tn` covers the motion published under
//! `T0+1..=Tn+1`; the line names both: `tick=T0`, `ticks=Tn-T0+1`,
//! `motion=T0+1-Tn+1`. The positions are sampled at the battle steps, and the
//! closing sample is taken when the episode ends, so the movement of the last
//! exposed tick — the one the pre-fix drive would have flung — is always
//! inside `peak` and `travel`.
//!
//! ## Bounds
//!
//! One `info` line per closed episode, capped per run at [`LINES_MAX`] lines and
//! [`BODIES_MAX`] distinct bodies; what the caps drop is counted as `suppressed`
//! and reported by the one summary line of the run. A run whose summary is
//! missing, whose `episodes` disagree with its lines, or whose `suppressed` is
//! non-zero is not usable for a bound. Nothing is written back into the
//! simulation and nothing is persisted (`BattleState::exposure` is
//! `serde(skip)`), so a recovered battle starts with a fresh tracker, which the
//! summary's `start=` makes visible.

use super::Columns;
use super::combat::{Action, Fighter};
use super::formation::FormationRuntime;
use crate::boundary::EntityId;
use crate::entities::EntityStore;

/// Episode lines one run may log: a run holds a few dozen (8-15 bodies per
/// field-1k take), so this bounds a pathological flapper without cutting a
/// real battle.
const LINES_MAX: u32 = 256;
/// Distinct bodies one run may report. One body can flap; the bound counts
/// bodies as well as lines so a single flapper cannot exhaust the lines.
const BODIES_MAX: usize = 64;
/// Open episodes one run tracks at once: the memory bound of the tracker.
const OPEN_MAX: usize = 1024;
/// A single-tick step above this is the server's own excess signal (the D1
/// bound is 2 blocks between two packets at most 6 ticks apart). Reported, not
/// the verdict: knockback moves a body too, and the campaign decides on the
/// wire step inside the episode.
pub(super) const EXCESS_BLOCKS: f64 = 2.0;

/// Which sweep plan the member's regiment rides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Plan {
    Chase,
    Flank,
}

impl Plan {
    fn name(self) -> &'static str {
        match self {
            Self::Chase => "chase",
            Self::Flank => "flank",
        }
    }
}

/// Why an episode closed. `None` from [`classify`] means the member still
/// passes the guards unseated: the episode continues.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Outcome {
    Reseated,
    Dead,
    Wounded,
    Routing,
    Inactive,
    Targeted,
    PlanEnd,
    Stopped,
    Gone,
}

impl Outcome {
    fn name(self) -> &'static str {
        match self {
            Self::Reseated => "reseated",
            Self::Dead => "dead",
            Self::Wounded => "wounded",
            Self::Routing => "routing",
            Self::Inactive => "inactive",
            Self::Targeted => "targeted",
            Self::PlanEnd => "plan_end",
            Self::Stopped => "stopped",
            Self::Gone => "gone",
        }
    }
}

/// One open exposure episode. `member` is the formation index it was last
/// observed at; `id` is the identity that outlives a repack.
#[derive(Debug, Clone, Copy)]
struct Episode {
    id: EntityId,
    member: usize,
    first_tick: u64,
    last_tick: u64,
    from: [f64; 3],
    to: [f64; 3],
    peak: f64,
    plan: Plan,
}

/// The member's state at classification time, gathered outside so the mapping
/// is a pure table.
#[derive(Debug, Clone, Copy)]
struct State {
    alive: bool,
    seated: bool,
    wounded: bool,
    routing: bool,
    active: bool,
    targeted: bool,
    planned: bool,
}

fn classify(state: State) -> Option<Outcome> {
    if !state.alive {
        return Some(Outcome::Dead);
    }
    if state.seated {
        return Some(Outcome::Reseated);
    }
    if state.wounded {
        return Some(Outcome::Wounded);
    }
    if state.routing {
        return Some(Outcome::Routing);
    }
    if !state.active {
        return Some(Outcome::Inactive);
    }
    if state.targeted {
        return Some(Outcome::Targeted);
    }
    if !state.planned {
        return Some(Outcome::PlanEnd);
    }
    None
}

fn horizontal(from: [f64; 3], to: [f64; 3]) -> f64 {
    mc_physics::mth::length2(to[0] - from[0], to[2] - from[2])
}

/// The body positions and battle tables a closing sample is read from. Passed
/// by the battle step for a live close and by the run boundaries (pause,
/// `battle reset`, shutdown) that close episodes outside a step.
pub(super) struct Sample<'a> {
    pub entities: &'a EntityStore,
    pub formation: &'a FormationRuntime,
    pub columns: &'a Columns,
}

/// Index-aligned at-risk exposure tracker. Lookup by member index is O(1)
/// through `aligned` (the `combat::prepare` idiom: an aligned side table, reused
/// buffers, no per-tick allocation), and one pass over the bounded slot table
/// closes everything that ended.
#[derive(Debug)]
pub(crate) struct Exposure {
    server_tick: u64,
    first_tick: Option<u64>,
    /// Open episodes by slot; `None` is a free slot, reused by the next one.
    episodes: Vec<Option<Episode>>,
    /// Member index -> slot + 1, or 0 when that member has no open episode.
    aligned: Vec<u32>,
    /// Open episodes among the slots, kept as a count so the cap is O(1).
    open: usize,
    /// Bodies reported this run, in first-exposure order.
    bodies: Vec<EntityId>,
    emitted: u32,
    suppressed: u32,
    closed: u64,
    member_ticks: u64,
    excess: u64,
    #[cfg(test)]
    enabled: bool,
    #[cfg(test)]
    lines: Vec<String>,
}

impl Default for Exposure {
    fn default() -> Self {
        Self {
            server_tick: 0,
            first_tick: None,
            episodes: Vec::new(),
            aligned: Vec::new(),
            open: 0,
            bodies: Vec::new(),
            emitted: 0,
            suppressed: 0,
            closed: 0,
            member_ticks: 0,
            excess: 0,
            #[cfg(test)]
            enabled: true,
            #[cfg(test)]
            lines: Vec::new(),
        }
    }
}

impl Exposure {
    /// Once per battle step, before the kernels: the global server tick whose
    /// guard set the step evaluates (its motion is published under this + 1).
    pub(super) fn begin(&mut self, server_tick: u64) {
        self.server_tick = server_tick;
        self.first_tick.get_or_insert(server_tick);
    }

    /// One exposed member-tick: the member passes `sweep_drives`' guard set and
    /// takes no sweep heading because he is not seated.
    pub(super) fn exposed(&mut self, id: EntityId, member: usize, pos: [f64; 3], plan: Plan) {
        #[cfg(test)]
        if !self.enabled {
            return;
        }
        self.member_ticks += 1;
        self.align(member);
        if let Some(slot) = self.open_slot(member, id) {
            let episode = self.episodes[slot]
                .as_mut()
                .expect("open slot holds an episode");
            episode.peak = episode.peak.max(horizontal(episode.to, pos));
            episode.to = pos;
            episode.last_tick = self.server_tick;
            episode.plan = plan;
            return;
        }
        if self.bodies.len() >= BODIES_MAX || self.open >= OPEN_MAX {
            self.suppressed += 1;
            return;
        }
        let slot = self.free_slot();
        self.aligned[member] = slot as u32 + 1;
        self.open += 1;
        if !self.bodies.contains(&id) {
            self.bodies.push(id);
        }
        self.episodes[slot] = Some(Episode {
            id,
            member,
            first_tick: self.server_tick,
            last_tick: self.server_tick,
            from: pos,
            to: pos,
            peak: 0.0,
            plan,
        });
    }

    /// Closes every episode whose member left the exposed set, sampling the
    /// body's position first so the motion of the last exposed tick is in the
    /// episode. Called once per battle step, after the cavalry pass.
    pub(super) fn close(
        &mut self,
        sample: &Sample<'_>,
        fighters: &[Fighter],
        plans: &[Option<super::cavalry::Sweep>],
    ) {
        for slot in 0..self.episodes.len() {
            let Some(episode) = self.episodes[slot] else {
                continue;
            };
            if episode.last_tick == self.server_tick {
                continue;
            }
            if let Some(pos) = body_position(&episode, sample) {
                let episode = self.episodes[slot]
                    .as_mut()
                    .expect("open slot holds an episode");
                episode.peak = episode.peak.max(horizontal(episode.to, pos));
                episode.to = pos;
            }
            let episode = self.episodes[slot]
                .as_mut()
                .expect("open slot holds an episode");
            let state = member_state(episode, sample, fighters, plans);
            let outcome = match state {
                None => Some(Outcome::Gone),
                Some(state) => classify(state),
            };
            let Some(outcome) = outcome else {
                // Still exposed, only the record is a tick behind: keep counting.
                continue;
            };
            let episode = self.episodes[slot]
                .take()
                .expect("open slot holds an episode");
            self.open -= 1;
            if self.aligned[episode.member] == slot as u32 + 1 {
                self.aligned[episode.member] = 0;
            }
            self.emit_episode(episode, outcome);
        }
    }

    /// A run boundary the battle step does not own: pause, `battle reset`,
    /// shutdown or the journal halt. Closes what is open, reports, and starts a
    /// fresh run for a resumed battle. `sample` is absent only where no body
    /// position is reachable.
    pub(super) fn halt(&mut self, sample: Option<&Sample<'_>>) {
        for slot in 0..self.episodes.len() {
            let Some(episode) = self.episodes[slot] else {
                continue;
            };
            if let Some(sample) = sample
                && let Some(pos) = body_position(&episode, sample)
            {
                let episode = self.episodes[slot]
                    .as_mut()
                    .expect("open slot holds an episode");
                episode.peak = episode.peak.max(horizontal(episode.to, pos));
                episode.to = pos;
            }
            let episode = self.episodes[slot]
                .take()
                .expect("open slot holds an episode");
            self.open -= 1;
            if self.aligned[episode.member] == slot as u32 + 1 {
                self.aligned[episode.member] = 0;
            }
            self.emit_episode(episode, Outcome::Stopped);
        }
        if self.closed > 0 {
            let capped = self.emitted >= LINES_MAX || self.suppressed > 0;
            let (start, closed, bodies) =
                (self.first_tick.unwrap_or(0), self.closed, self.bodies.len());
            let (member_ticks, excess, suppressed) =
                (self.member_ticks, self.excess, self.suppressed);
            self.emit(format_args!(
                "Battle exposure summary: start={} episodes={} bodies={} member_ticks={} excess_episodes={} suppressed={} capped={}",
                start, closed, bodies, member_ticks, excess, suppressed, capped
            ));
        }
        self.first_tick = None;
        self.emitted = 0;
        self.suppressed = 0;
        self.closed = 0;
        self.member_ticks = 0;
        self.excess = 0;
        self.open = 0;
        self.bodies.clear();
        for slot in &mut self.episodes {
            *slot = None;
        }
        self.aligned.clear();
    }

    fn emit_episode(&mut self, episode: Episode, outcome: Outcome) {
        self.closed += 1;
        if episode.peak > EXCESS_BLOCKS {
            self.excess += 1;
        }
        if self.emitted >= LINES_MAX {
            self.suppressed += 1;
            return;
        }
        self.emitted += 1;
        let ticks = episode.last_tick.saturating_sub(episode.first_tick) + 1;
        let travel = horizontal(episode.from, episode.to);
        let (from, to) = (episode.from, episode.to);
        let motion = episode.first_tick + 1;
        self.emit(format_args!(
            "Battle exposure: id={} tick={} ticks={} motion={}-{} plan={} from=[{:.3},{:.3},{:.3}] to=[{:.3},{:.3},{:.3}] travel={:.3} peak={:.3} outcome={}",
            episode.id.0,
            episode.first_tick,
            ticks,
            motion,
            motion + ticks - 1,
            episode.plan.name(),
            from[0],
            from[1],
            from[2],
            to[0],
            to[1],
            to[2],
            travel,
            episode.peak,
            outcome.name()
        ));
    }

    /// One line, no formatting allocation in a production build (the event
    /// writes the fields itself); test builds keep the text for assertions.
    fn emit(&mut self, line: std::fmt::Arguments<'_>) {
        #[cfg(test)]
        self.lines.push(line.to_string());
        tracing::info!("{line}");
    }

    fn open_slot(&self, member: usize, id: EntityId) -> Option<usize> {
        let slot = usize::try_from(*self.aligned.get(member)?)
            .ok()?
            .checked_sub(1)?;
        let episode = self.episodes.get(slot)?.as_ref()?;
        (episode.id == id).then_some(slot)
    }

    /// A free slot, growing the table up to its bounded open count and reusing a
    /// closed one after that.
    fn free_slot(&mut self) -> usize {
        if let Some(slot) = self.episodes.iter().position(Option::is_none) {
            return slot;
        }
        self.episodes.push(None);
        self.episodes.len() - 1
    }

    fn open_count(&self) -> usize {
        self.open
    }

    /// The member's formation index, grown on demand by [`Self::exposed`].
    fn align(&mut self, member: usize) {
        if self.aligned.len() <= member {
            self.aligned.resize(member + 1, 0);
        }
    }

    #[cfg(test)]
    pub(super) fn is_exposed(&self, id: EntityId) -> bool {
        self.episodes
            .iter()
            .flatten()
            .any(|episode| episode.id == id)
    }

    #[cfg(test)]
    pub(super) fn lines(&self) -> &[String] {
        &self.lines
    }

    /// The first condition tick of the current run, which the summary reports:
    /// the phase contract is observable without an episode.
    #[cfg(test)]
    pub(super) fn run_start(&self) -> Option<u64> {
        self.first_tick
    }

    #[cfg(test)]
    pub(super) fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }
}

/// A tracker that the run ends without closing (a path the tick loop does not
/// own) still reports what it saw instead of losing it silently.
impl Drop for Exposure {
    fn drop(&mut self) {
        if self.closed > 0 || self.open_count() > 0 {
            self.halt(None);
        }
    }
}

/// The body's current position, from the columns row when the formation still
/// knows the member and from the entity store otherwise.
fn body_position(episode: &Episode, sample: &Sample<'_>) -> Option<[f64; 3]> {
    if let Some(member) = sample.formation.members.get(episode.member)
        && member.id == episode.id
    {
        if let Some(pos) = sample.columns.positions.get(episode.member) {
            return Some(*pos);
        }
    }
    let slot = sample
        .formation
        .members
        .get(episode.member)
        .and_then(|m| m.slot)?;
    Some(sample.entities.pos(slot))
}

/// The member's state for [`classify`], or `None` when the identity left the
/// formation (repacked out, removed or the battle was reset).
fn member_state(
    episode: &Episode,
    sample: &Sample<'_>,
    fighters: &[Fighter],
    plans: &[Option<super::cavalry::Sweep>],
) -> Option<State> {
    let member = sample
        .formation
        .members
        .get(episode.member)
        .filter(|member| member.id == episode.id)?;
    let bi = episode.member;
    let columns = sample.columns;
    Some(State {
        alive: member.alive,
        seated: member.slot.is_some_and(|slot| {
            sample
                .entities
                .row(slot)
                .entity_life
                .mount
                .vehicle
                .is_some()
        }),
        wounded: member.wounded
            || fighters
                .get(bi)
                .is_some_and(|fighter| matches!(fighter.action, Action::Wounded)),
        routing: member.routing,
        active: columns.active.get(bi).copied().unwrap_or(false),
        targeted: columns.targets.get(bi).is_some_and(Option::is_some),
        planned: plans
            .get(member.regiment)
            .is_some_and(|plan| plan.is_some()),
    })
}

#[cfg(test)]
#[path = "exposure_tests.rs"]
mod tests;
