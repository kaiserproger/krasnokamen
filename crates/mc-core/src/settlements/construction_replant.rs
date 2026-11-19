//! A lumberjack's replanting proof: the future crown must keep building access
//! and a free lot for every house blueprint, proved under the server's one
//! site budget (owner D42, `site_turn`).
use mc_script::ScriptWorldBounds;

use super::{executor, founding, site_free_claim, site_refusals, site_turn};
use crate::settlements::blueprints::{self, Rotation};
use crate::settlements::layout;
use crate::tick::TickState;

/// A lumberjack's replanting verdict at a root (owner D42).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Replant {
    Plant,
    /// The crown would take held ground, building access or the last free
    /// house lot, or that cannot be proved: the root is skipped.
    Refused,
    /// The house-lot proof needs site budget it has not had yet: the
    /// lumberjack keeps the stump and asks again on a later tick.
    NotYet,
}

/// Reuse the executor's reservation, including the admitted clearing corridor.
/// Finished structures stay protected; spent road and meadow work does not.
/// The house-lot proofs examine lots from `budget` (`site_turn`).
pub(super) fn replant_check(
    state: &TickState,
    root: [i32; 3],
    budget: &mut site_turn::SiteBudget,
) -> Replant {
    let crown = crate::settlements::forestry::PLANTED_CROWN_RADIUS as i32;
    let Ok(bounds) = ScriptWorldBounds::new(
        [
            root[0].saturating_sub(crown),
            root[1],
            root[2].saturating_sub(crown),
        ],
        [
            root[0].saturating_add(crown),
            root[1]
                .saturating_add(64)
                .min(state.world.geometry().max_y() - 1),
            root[2].saturating_add(crown),
        ],
    ) else {
        return Replant::Refused;
    };
    if executor::area_held(state, bounds) {
        return Replant::Refused;
    }
    state
        .settlements
        .records()
        .filter(|record| {
            // Street headroom and lots stay inside the territory's
            // horizontal extent. Outpost claims may lie outside it.
            let horizontal = ScriptWorldBounds::new(
                [bounds.min[0], record.anchor[1], bounds.min[2]],
                [bounds.max[0], record.anchor[1], bounds.max[2]],
            )
            .expect("valid crown projection");
            founding::sphere_overlaps(record.anchor, record.radius, horizontal)
                || record.founding.as_ref().is_some_and(|expedition| {
                    let claim = expedition.bounds();
                    [0, 2].into_iter().all(|axis| {
                        bounds.min[axis] <= claim.max[axis] && claim.min[axis] <= bounds.max[axis]
                    })
                })
        })
        .map(|record| layout::planting_verdict(state, record, bounds, budget))
        .find(|verdict| *verdict != Replant::Plant)
        .unwrap_or(Replant::Plant)
}

/// A street frontage: the road cell a lot's door faces and the lot's normal.
pub(in crate::settlements) type Front = ([i32; 3], [i32; 2]);

/// What a finished scan of a crowned walk found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::settlements) enum HouseLot {
    /// A free lot faces this frontage.
    Free(Front),
    /// Every frontage was examined and no lot is free.
    None,
}

/// How far one proposed crown's scan of its crowned walk has got for one
/// house blueprint (owner D42): a planting that waits ("not yet") continues
/// it on the lumberjack's next proof. Runtime only.
#[derive(Debug, Default)]
pub(crate) struct CrownScan {
    /// The frontage the running scan started from; `None` while no scan runs.
    from: Option<Front>,
    /// Frontages examined from it.
    done: u32,
    /// The tick of the last proof that used it.
    pub(super) touched: u64,
}

impl CrownScan {
    /// No scan of this crown's walk is running.
    pub(in crate::settlements) fn idle(&self) -> bool {
        self.from.is_none()
    }
}

/// The lot checks of one replanting proof: what its slice has left, and the
/// answer of every lot it already checked. A proof reads one tick's world,
/// so an answer holds for the whole proof, and two house blueprints of one
/// footprint share their lots and checks.
#[derive(Default)]
pub(in crate::settlements) struct LotChecks {
    /// Site checks the proof may still run.
    pub(in crate::settlements) lots: u32,
    survey: site_refusals::Survey,
    seen: std::collections::HashMap<([i32; 3], [u8; 3]), bool>,
}

impl LotChecks {
    pub(in crate::settlements) fn new(lots: u32) -> Self {
        Self {
            lots,
            ..Self::default()
        }
    }
}

/// Whether the lot of `blueprint` facing `front` on `network` is free;
/// geometrically feasible housing, independent of today's material balance.
/// A site check takes one of the proof's lots; `None` when none is left.
fn free_house_lot(
    state: &TickState,
    network: &mut layout::Network<'_>,
    blueprint: &blueprints::Blueprint,
    front: Front,
    checks: &mut LotChecks,
) -> Option<bool> {
    let Some(lot) = network.lot(blueprint, front.0, front.1) else {
        return Some(false);
    };
    let size = blueprint.footprint(Rotation::from_quarter_turns(lot.rotation));
    if let Some(&free) = checks.seen.get(&(lot.origin, size)) {
        return Some(free);
    }
    checks.lots = checks.lots.checked_sub(1)?;
    site_refusals::examined();
    let record = network.record();
    let free = site_free_claim(
        state,
        record,
        lot.origin,
        size,
        record.founding.as_ref(),
        &mut checks.survey,
    );
    checks.seen.insert((lot.origin, size), free);
    Some(free)
}

/// Whether the crowned walk (`network`, frontages `fronts`) holds the lot of
/// `blueprint` facing `front` and it is free now; `None` without a lot left.
pub(in crate::settlements) fn keeps_free_lot(
    state: &TickState,
    network: &mut layout::Network<'_>,
    fronts: &[Front],
    blueprint: &blueprints::Blueprint,
    front: Front,
    checks: &mut LotChecks,
) -> Option<bool> {
    if !fronts.contains(&front) {
        return Some(false);
    }
    free_house_lot(state, network, blueprint, front, checks)
}

/// Whether planting `crown` takes a lot of `blueprint` that is free now from
/// the plain walk `plain`: a lot whose frontage the crowned walk (frontages
/// `crowned`) lost, or whose footprint, door or verge column the crown covers.
/// Every other lot is the same on both walks. Checked in this proof only, so
/// the answer is never older than the proof; `None` when the proof's lots run
/// out first.
pub(in crate::settlements) fn takes_free_lot(
    state: &TickState,
    plain: &mut layout::Network<'_>,
    crowned: &[Front],
    crown: ScriptWorldBounds,
    blueprint: &blueprints::Blueprint,
    checks: &mut LotChecks,
) -> Option<bool> {
    let kept: std::collections::HashSet<&Front> = crowned.iter().collect();
    for front in plain.frontage() {
        if (!kept.contains(&front) || layout::crown_touches_lot(blueprint, front.0, front.1, crown))
            && free_house_lot(state, plain, blueprint, front, checks)?
        {
            return Some(true);
        }
    }
    Some(false)
}

/// Continue this crown's scan of its crowned walk (`network`, frontages
/// `fronts`) for a free lot of `blueprint`, in walking order from the
/// frontage the scan started at; a long walk is finished over several proofs
/// (`progress`). `None` when the proof's lots run out first; a finished scan
/// leaves `progress` idle.
pub(in crate::settlements) fn crowned_house_lot(
    state: &TickState,
    network: &mut layout::Network<'_>,
    fronts: &[Front],
    blueprint: &blueprints::Blueprint,
    progress: &mut CrownScan,
    checks: &mut LotChecks,
) -> Option<HouseLot> {
    if let Some(&first) = fronts.first() {
        let start = *progress.from.get_or_insert(first);
        let start = fronts.iter().position(|front| *front == start).unwrap_or(0);
        while (progress.done as usize) < fronts.len() {
            let front = fronts[(start + progress.done as usize) % fronts.len()];
            if free_house_lot(state, network, blueprint, front, checks)? {
                progress.from = None;
                progress.done = 0;
                return Some(HouseLot::Free(front));
            }
            progress.done += 1;
        }
    }
    progress.from = None;
    progress.done = 0;
    Some(HouseLot::None)
}
