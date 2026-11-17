//! Runtime-only refusal evidence from the canonical site search. Complete
//! sweeps, not cursor batches, determine whether the reason has changed.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum Reason {
    NoFrontage,
    LotGeometry,
    OutpostClaim,
    BuildingBuffer,
    Territory,
    SurfaceUnavailable,
    HeightVariance,
    ProtectedFloor,
    Unloaded,
    ProtectedObstruction,
    ExecutorReservation,
    Preparation,
    MaterialBill,
}
const REASONS: [Reason; 13] = [
    Reason::NoFrontage,
    Reason::LotGeometry,
    Reason::OutpostClaim,
    Reason::BuildingBuffer,
    Reason::Territory,
    Reason::SurfaceUnavailable,
    Reason::HeightVariance,
    Reason::ProtectedFloor,
    Reason::Unloaded,
    Reason::ProtectedObstruction,
    Reason::ExecutorReservation,
    Reason::Preparation,
    Reason::MaterialBill,
];

#[derive(Default)]
pub(super) struct Report {
    pub(super) frontages: u32,
    pub(super) examined: u32,
    pub(super) counts: [u32; REASONS.len()],
    pub(super) examples: [Option<[i32; 3]>; REASONS.len()],
}
impl std::fmt::Debug for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut map = f.debug_map();
        for reason in REASONS {
            let index = reason as usize;
            if self.counts[index] != 0 {
                map.entry(&reason, &(self.counts[index], self.examples[index]));
            }
        }
        map.finish()
    }
}
impl Report {
    pub(super) fn reject(&mut self, reason: Reason, at: [i32; 3]) {
        let index = reason as usize;
        self.counts[index] += 1;
        self.examples[index].get_or_insert(at);
    }
    fn mask(&self) -> u16 {
        self.counts
            .iter()
            .enumerate()
            .fold(0, |mask, (index, &count)| {
                mask | (u16::from(count != 0) << index)
            })
    }
}

#[derive(Debug, Default)]
pub(crate) struct Watch {
    blueprint: String,
    sweep: Report,
    previous: Option<u16>,
}
impl Watch {
    fn record(&mut self, blueprint: &str, batch: Report) -> Option<Report> {
        if self.blueprint != blueprint {
            *self = Self {
                blueprint: blueprint.to_owned(),
                ..Self::default()
            };
        }
        if self.sweep.frontages != batch.frontages {
            self.sweep = Report::default();
        }
        self.sweep.frontages = batch.frontages;
        self.sweep.examined += batch.examined;
        for index in 0..REASONS.len() {
            self.sweep.counts[index] += batch.counts[index];
            if self.sweep.examples[index].is_none() {
                self.sweep.examples[index] = batch.examples[index];
            }
        }
        if self.sweep.examined < self.sweep.frontages {
            return None;
        }
        let report = std::mem::take(&mut self.sweep);
        // Retain the topology between sweeps so the first batch does not
        // invalidate an otherwise complete accumulation.
        self.sweep.frontages = report.frontages;
        let mask = report.mask();
        if self.previous == Some(mask) {
            return None;
        }
        self.previous = Some(mask);
        Some(report)
    }
}

pub(super) fn observe(state: &mut TickState, id: SettlementId, blueprint: &str, batch: Report) {
    if let Some(report) = state
        .settlements
        .site_watch
        .entry(id)
        .or_default()
        .record(blueprint, batch)
    {
        tracing::warn!(settlement = id.0, blueprint, level = 1,
            frontages = report.frontages, examined = report.examined,
            refusals = ?report, "settlement_construction_site_refused");
    }
}

pub(super) fn check(
    state: &TickState,
    record: &SettlementRecord,
    origin: [i32; 3],
    size: [u8; 3],
    expedition: Option<&FoundingExpedition>,
) -> Result<(), Reason> {
    if founding::site_reserved(state, record.id, origin, size) {
        return Err(Reason::OutpostClaim);
    }
    let max: [i32; 3] = std::array::from_fn(|axis| origin[axis] + i32::from(size[axis]) - 1);
    let lot = ScriptWorldBounds::new(origin, max).expect("blueprint bounds");
    let mut minimum = origin[1];
    let mut maximum = origin[1];
    if record.buildings.values().any(|building| {
        [0, 2].into_iter().all(|axis| {
            origin[axis] <= building.bounds.max[axis] + 1
                && building.bounds.min[axis] <= max[axis] + 1
        })
    }) {
        return Err(Reason::BuildingBuffer);
    }
    for x in origin[0]..=max[0] {
        for z in origin[2]..=max[2] {
            if [x, max[1], z]
                .into_iter()
                .zip(record.anchor)
                .map(|(value, anchor)| (i64::from(value) - i64::from(anchor)).pow(2))
                .sum::<i64>()
                > i64::from(record.radius).pow(2)
                && !expedition.is_some_and(|site| site.contains([x, max[1], z]))
            {
                return Err(Reason::Territory);
            }
            let anchor_y = if expedition.is_some_and(|site| site.contains(origin)) {
                origin[1]
            } else {
                record.anchor[1]
            };
            let height = surface_y_for_lot(state, anchor_y, x, z, Some(lot))
                .ok_or(Reason::SurfaceUnavailable)?;
            minimum = minimum.min(height);
            maximum = maximum.max(height);
            if maximum - minimum > SITE_VARIANCE {
                return Err(Reason::HeightVariance);
            }
            let ground = state
                .world
                .physics_block_at(x, height, z)
                .expect("loaded surface");
            if !floor_replaceable(state, ground) {
                return Err(Reason::ProtectedFloor);
            }
            for y in height.min(origin[1])..=max[1].max(height) {
                let block = state
                    .world
                    .physics_block_at(x, y, z)
                    .ok_or(Reason::Unloaded)?;
                if state.block_entities.contains_key(&(x, y, z))
                    || (super::super::forestry::tree_or_sapling(state, block)
                        && !super::super::forestry::clearable_on_lot(state, [x, y, z], Some(lot)))
                    || (y == origin[1]
                        && !floor_replaceable(state, block)
                        && !super::super::forestry::clearable_on_lot(state, [x, y, z], Some(lot)))
                    || (y > origin[1]
                        && !clearable(state, block)
                        && !excavatable(state, block)
                        && !super::super::forestry::clearable_on_lot(state, [x, y, z], Some(lot)))
                    || (y > height
                        && y <= origin[1]
                        && !clearable(state, block)
                        && !super::super::forestry::clearable_on_lot(state, [x, y, z], Some(lot)))
                {
                    return Err(Reason::ProtectedObstruction);
                }
            }
        }
    }
    if executor::area_reserved(
        state,
        ScriptWorldBounds::new([origin[0], minimum, origin[2]], max).expect("blueprint bounds"),
    ) {
        return Err(Reason::ExecutorReservation);
    }
    Ok(())
}

#[cfg(test)]
#[path = "construction_site_refusal_tests.rs"]
mod tests;
