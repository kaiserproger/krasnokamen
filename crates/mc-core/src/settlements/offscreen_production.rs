//! Game-time periods and one-shot resident reconciliation for production.
use super::*;

const DAY: u64 = 24_000;
/// Food points per living member per game day (bread=4, vegetables=1).
const DAILY_FOOD_POINTS: u64 = 1;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum Clock {
    WorldTicks,
    #[default]
    GameTicks,
}

pub(super) fn legacy_clock() -> Clock {
    Clock::WorldTicks
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Consumption {
    pub(super) next_due: Option<u64>,
    pub(super) debit: u64,
    /// Only already-paid food; never an unpaid demand carried to another day.
    #[serde(default)]
    pub(super) credit: u8,
    #[serde(skip)]
    retry_at: u64,
}

impl Consumption {
    pub(super) fn validate(&self) -> Result<(), &'static str> {
        // Demand is an exact native/script counter, never a saturating sentinel.
        if self.credit > 3 || self.debit > mc_script::MAX_SCRIPT_WORLD_TIME {
            return Err("invalid production consumption");
        }
        Ok(())
    }
}

pub(super) fn phase_after(now: u64, settlement: SettlementId, period: u64) -> u64 {
    let phase = settlement.0 % period;
    let remainder = now % period;
    let distance = if remainder < phase {
        phase - remainder
    } else {
        period - (remainder - phase)
    };
    now.saturating_add(distance)
}

pub(super) fn first_due(world: &TickState, settlement: SettlementId, period: u64) -> u64 {
    phase_after(
        world
            .tick_control
            .game_ticks(world.tick)
            .saturating_add(period - 1),
        settlement,
        period,
    )
}

pub(super) fn advance(job: &mut Job, now: u64) -> u64 {
    if now < job.next_due {
        return 0;
    }
    let periods = (now - job.next_due) / job.duration + 1;
    job.next_due = now.saturating_add(job.duration - (now - job.next_due) % job.duration);
    periods
}

pub(super) fn ready(job: &Job, now: u64) -> bool {
    if job.done >= job.count {
        return job.delivery_pending() && job.retry_at <= now;
    }
    (job.next_due <= now || job.deferred != 0 || job.paused.is_some()) && job.retry_at <= now
}

fn simulated(world: &TickState, at: [i32; 3]) -> bool {
    world
        .world
        .is_active(at[0].div_euclid(16), at[2].div_euclid(16))
}

/// Wait for every declared storage chunk, rather than treating missing chunks
/// as an empty larder. Destroyed resident containers are genuinely unavailable.
fn storage_resident(world: &TickState, settlement: SettlementId) -> bool {
    super::super::buildings::storage_points(world, settlement)
        .iter()
        .all(|at| {
            world
                .world
                .has_chunk(at[0].div_euclid(16), at[2].div_euclid(16))
        })
}

pub(super) fn visit(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
    now: u64,
) -> bool {
    if !simulated(world, job.station) {
        let waited = std::mem::take(&mut job.waiting_simulated);
        let periods = if job.done < job.count {
            advance(job, now)
        } else {
            0
        };
        let owed = if waited { periods.min(1) } else { periods };
        let before = job.deferred;
        job.deferred = job
            .deferred
            .saturating_add(owed.min(u64::from(u32::MAX)) as u32)
            .min(job.count - job.done);
        job.retry_at = now.saturating_add(RETRY_TICKS);
        return periods != 0 || before != job.deferred || waited;
    }
    let mut changed = false;
    if now < job.retry_at {
        return false;
    }
    if job.deferred != 0 && !storage_resident(world, settlement) {
        let periods = advance(job, now);
        job.deferred = job
            .deferred
            .saturating_add(periods.min(u64::from(u32::MAX)) as u32)
            .min(job.count - job.done);
        job.retry_at = now.saturating_add(RETRY_TICKS);
        return periods != 0;
    }
    if job.deferred != 0 {
        let portions = std::mem::take(&mut job.deferred);
        changed = true;
        // One reconciliation: successful portions are durable; all unaffordable
        // portions are dropped, never retried as deferred work.
        for _ in 0..portions {
            if reconcile_portion(world, settlement, job).is_err() {
                break;
            }
        }
    }
    if job.done < job.count || job.delivery_pending() {
        let portion_due = job.done < job.count && job.next_due <= now;
        let before = job.done;
        changed |= match tick_job(world, settlement, job, portion_due) {
            Ok(changed) => changed,
            Err(reason) => park(job, pause_reason(reason), now.saturating_add(RETRY_TICKS)),
        };
        if job.done > before {
            advance(job, now);
            job.waiting_simulated = false;
        } else if portion_due && !job.waiting_simulated {
            job.waiting_simulated = true;
            changed = true;
        }
    }
    changed
}

/// Same recipe planner and fenced physical writer as ordinary production, but
/// the inventory is real settlement storage rather than a moving worker carry.
fn reconcile_portion(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
) -> Result<(), Failure> {
    let facts = building_facts(world, settlement, &job.building)?;
    let recipe = recipe_of(world, &job.recipe)?;
    station_live(world, recipe, &facts, job.station, job.stand_at)?;
    let sources = inventory::town_storage(world, settlement);
    let mut slots = Vec::new();
    let mut available = Vec::new();
    for (_, image) in &sources {
        let mut stock = inventory::unreserved_stock(world, image)?;
        for stack in &image.slots {
            let free = stock.entry(stack.item_id).or_default();
            let count = (*free).min(stack.count.max(0) as u64);
            *free -= count;
            available.push(count as u32);
            slots.push(stack.clone());
        }
    }
    let (inputs, outputs, wear) = plan_contents(world, recipe, &mut slots, &mut [], &mut available)
        .map_err(|shortage| match shortage {
            Shortage::Input | Shortage::Tool => Failure::InsufficientItems,
            Shortage::Storage => Failure::Capacity,
        })?;
    let revision = recipe.revision.clone();
    let mut offset = 0;
    let changes = sources
        .into_iter()
        .map(|(_, before)| {
            let end = offset + before.slots.len();
            let after = slots[offset..end].to_vec();
            offset = end;
            inventory::Change { before, after }
        })
        .collect();
    inventory::commit(world, changes)?;
    job.recipe_revision = revision;
    job.done += 1;
    job.receipts.push(Receipt {
        job: job.id.clone(),
        sequence: job.done,
        recipe: job.recipe.clone(),
        recipe_revision: job.recipe_revision.clone(),
        inputs,
        outputs,
        tool_wear: wear,
    });
    Ok(())
}

pub(super) fn consume(
    world: &mut TickState,
    settlement: SettlementId,
    consumption: &mut Consumption,
) -> bool {
    let now = world.tick_control.game_ticks(world.tick);
    let Some(due) = consumption.next_due else {
        consumption.next_due = Some(first_due(world, settlement, DAY));
        return true;
    };
    let mut changed = false;
    if now >= due {
        let days = (now - due) / DAY + 1;
        consumption.next_due = Some(now.saturating_add(DAY - (now - due) % DAY));
        consumption.debit = consumption.debit.saturating_add(days.saturating_mul(
            (world.settlements.member_count(settlement) as u64).saturating_mul(DAILY_FOOD_POINTS),
        ));
        changed = true;
    }
    let Some(record) = world.settlements.get(settlement) else {
        return changed;
    };
    if consumption.debit == 0 || now < consumption.retry_at {
        return changed;
    }
    if !simulated(world, record.anchor) || !storage_resident(world, settlement) {
        consumption.retry_at = now.saturating_add(RETRY_TICKS);
        return changed;
    }
    consumption.retry_at = 0;
    let mut demand = std::mem::take(&mut consumption.debit);
    let prepaid = demand.min(u64::from(consumption.credit));
    consumption.credit -= prepaid as u8;
    demand -= prepaid;
    if demand == 0 {
        return true;
    }
    for (_, before) in inventory::town_storage(world, settlement) {
        let Ok(mut free) = inventory::unreserved_stock(world, &before) else {
            continue;
        };
        let mut after = before.slots.clone();
        let mut paid = 0;
        for stack in &mut after {
            let points = world
                .registry
                .items
                .name_of(stack.item_id)
                .map_or(0, |name| {
                    u64::from(crate::villager::brain::food_points_for_name(name.path()))
                });
            if points == 0 || stack.is_empty() {
                continue;
            }
            let available = free.entry(stack.item_id).or_default();
            let take = (demand - paid)
                .div_ceil(points)
                .min(*available)
                .min(stack.count.max(0) as u64);
            *available -= take;
            stack.count -= take as i32;
            if stack.is_empty() {
                *stack = ItemStack::EMPTY;
            }
            paid += take * points;
            if paid >= demand {
                break;
            }
        }
        if paid != 0 && inventory::commit(world, vec![inventory::Change { before, after }]).is_ok()
        {
            consumption.credit = paid.saturating_sub(demand) as u8;
            demand = demand.saturating_sub(paid);
        }
        if demand == 0 {
            break;
        }
    }
    true
}

pub(super) fn restore_clocks(world: &mut TickState) -> Result<(), crate::CoreError> {
    let now = world.tick;
    let game = world.tick_control.game_ticks(now);
    restore_dimension(world, now, game)?;
    for dimension in world.other_dimensions.values_mut() {
        restore_dimension(dimension, now, game)?;
    }
    Ok(())
}

fn restore_dimension(world: &mut TickState, now: u64, game: u64) -> Result<(), crate::CoreError> {
    for record in world.settlements.records() {
        let consumption = &record.economy.production.consumption;
        if consumption.validate().is_err()
            || consumption
                .next_due
                .is_some_and(|due| due.saturating_sub(game) >= 2 * DAY)
        {
            return Err(crate::CoreError::Storage(format!(
                "native settlement {}: invalid production consumption clock",
                record.id.0
            )));
        }
    }
    let ids: Vec<_> = world
        .settlements
        .records()
        .filter(|record| record.economy.production.clock == Clock::WorldTicks)
        .map(|record| record.id)
        .collect();
    for id in ids {
        let state = &mut world
            .settlements
            .get_mut(id)
            .expect("recovered settlement")
            .economy
            .production;
        for job in state.jobs.values_mut() {
            if job.deferred != 0
                || job.duration == 0
                || job.next_due.saturating_sub(now) > job.duration
            {
                return Err(crate::CoreError::Storage(format!(
                    "native settlement {} production {}: inconsistent legacy clock",
                    id.0, job.id
                )));
            }
            // Legacy production performed at most one portion on its next
            // visit, never every missed period. Preserve that opportunity only.
            job.next_due = game
                .checked_add(job.next_due.saturating_sub(now))
                .ok_or_else(|| {
                    crate::CoreError::Storage(format!(
                        "native settlement {} production {}: clock overflow",
                        id.0, job.id
                    ))
                })?;
        }
        state.clock = Clock::GameTicks;
    }
    Ok(())
}
