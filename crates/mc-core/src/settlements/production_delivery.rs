//! Per-final-job output obligation. Native carry/container images remain the
//! inventory authority; these are owed quantities, not a second inventory.
use mc_data::ItemStack;
use mc_script::ScriptOperationFailure as Failure;
use serde::{Deserialize, Serialize};

use super::{
    Job, RETRY_TICKS, SettlementId, TickState, inventory, park, plan_store, reach, supply_stand,
    worker_pos,
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lot {
    /// Exact authored output components; count is the still-owed quantity.
    pub stack: ItemStack,
    /// Same-component units which predated this portion, never shipped for it.
    pub retained: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Delivery {
    pub lots: Vec<Lot>,
    /// Incremented only with an actual successful inventory store commit.
    pub sequence: u64,
    /// Loss, substitution or unexplained same-component gain is not repairable
    /// by refilling the inventory. Cancel explicitly; never forge fulfillment.
    pub invalidated: bool,
}

fn matches(a: &ItemStack, b: &ItemStack) -> bool {
    mc_data::inventory_semantics_26_1_2::can_stack(a, b)
}

fn quantity(slots: &[ItemStack], key: &ItemStack) -> u64 {
    slots
        .iter()
        .filter(|stack| !stack.is_empty() && matches(stack, key))
        .map(|stack| stack.count.max(0) as u64)
        .sum()
}

impl Delivery {
    pub(super) fn pending(&self) -> bool {
        !self.lots.is_empty()
    }

    fn intact(&self, carry: &[ItemStack]) -> bool {
        !self.invalidated
            && self.lots.iter().all(|lot| {
                quantity(carry, &lot.stack) == u64::from(lot.retained) + lot.stack.count as u64
            })
    }

    /// Same-component units are fungible. Protect the entire mixed pool until
    /// this portion's obligation clears: that is deliberately conservative,
    /// independent of routing, pause, and whether the next portion is due.
    /// Unrelated food, tools and cargo are neither protected nor selected here.
    pub(super) fn protect(&self, carry: &[Option<ItemStack>; 8], protected: &mut [u32; 8]) {
        for (index, stack) in carry.iter().enumerate() {
            if let Some(stack) = stack
                && self.lots.iter().any(|lot| matches(stack, &lot.stack))
            {
                protected[index] = stack.count.max(0) as u32;
            }
        }
    }

    pub(super) fn validate(&self) -> Result<(), &'static str> {
        if self.lots.len() > 8 || (self.invalidated && self.lots.is_empty()) {
            return Err("invalid delivery obligation");
        }
        for (index, lot) in self.lots.iter().enumerate() {
            if lot.stack.is_empty()
                || lot.stack.count <= 0
                || lot.retained.checked_add(lot.stack.count as u32).is_none()
                || self.lots[..index]
                    .iter()
                    .any(|old| matches(&old.stack, &lot.stack))
            {
                return Err("invalid delivery lot");
            }
        }
        Ok(())
    }
}

/// Plan the obligation BEFORE mutating inventory; install it only after the
/// production commit succeeds. The caller supplies authored recipe.outputs,
/// not authored remainders (returned buckets remain the worker's own carry).
/// One portion is delivered before another begins; intermediate chain jobs
/// never call this owner, so dependency outputs stay available as before.
pub(super) fn after_production(
    previous: &Delivery,
    carry_after: &[ItemStack],
    outputs: &[ItemStack],
) -> Result<Delivery, Failure> {
    if previous.pending() || previous.invalidated {
        return Err(Failure::Blocked);
    }
    let mut next = Delivery {
        lots: Vec::with_capacity(outputs.len()),
        sequence: previous.sequence,
        invalidated: false,
    };
    for output in outputs {
        if output.is_empty() || output.count <= 0 {
            return Err(Failure::InvalidRequest);
        }
        if let Some(lot) = next.lots.iter_mut().find(|lot| matches(&lot.stack, output)) {
            lot.stack.count = lot
                .stack
                .count
                .checked_add(output.count)
                .ok_or(Failure::Capacity)?;
        } else {
            next.lots.push(Lot {
                stack: output.clone(),
                retained: 0,
            });
        }
    }
    for lot in &mut next.lots {
        let before_output = quantity(carry_after, &lot.stack)
            .checked_sub(lot.stack.count as u64)
            .ok_or(Failure::InvalidRequest)?;
        lot.retained = u32::try_from(before_output).map_err(|_| Failure::Capacity)?;
    }
    next.validate().map_err(|_| Failure::InvalidRequest)?;
    Ok(next)
}

/// Select ONLY owed output counts; all other carry units are floor-protected
/// in this plan. A complete transfer leaves every retained same-component unit
/// and every unrelated item in carry, even if storage cannot accept them.
fn kept_for_delivery(delivery: &Delivery, carry: &[ItemStack]) -> [u32; 8] {
    let mut kept = std::array::from_fn(|index| carry[index].count.max(0) as u32);
    for lot in &delivery.lots {
        let mut left = lot.stack.count as u32;
        for (index, stack) in carry.iter().enumerate() {
            if left == 0 {
                break;
            }
            if !stack.is_empty() && matches(stack, &lot.stack) {
                let take = left.min(kept[index]);
                kept[index] -= take;
                left -= take;
            }
        }
    }
    kept
}

/// The caller owns a live, noncancelled pending job. A standing cell is NOT a
/// reachability proof: we preserve native navigation and expose a pending
/// "awaiting output storage reach" boundary rather than invent another router.
pub(super) fn store(
    world: &mut TickState,
    settlement: SettlementId,
    job: &mut Job,
) -> Result<bool, Failure> {
    let retry = world.tick.saturating_add(RETRY_TICKS);
    let carry = inventory::load(
        world,
        &inventory::Endpoint::VillagerCarry { uuid: job.worker },
    )?;
    let delivery = job.delivery.as_ref().ok_or(Failure::InvalidRequest)?;
    if !delivery.intact(&carry.slots) {
        job.delivery.as_mut().expect("delivery checked").invalidated = true;
        job.fetch_at = None;
        return Ok(park(job, "delivery cargo changed; cancel required", retry));
    }
    let kept = kept_for_delivery(delivery, &carry.slots);
    let pos = worker_pos(world, job.worker).ok_or(Failure::NotFound)?;
    let mut walk: Option<([i32; 3], f64)> = None;
    for (position, target) in inventory::town_storage(world, settlement) {
        let Some((target_after, carry_after)) =
            plan_store(world, &carry.slots, &kept, &target.slots)
        else {
            continue;
        };
        let distance = reach(pos, position);
        if distance <= 6.25 {
            let mut after = job.delivery.as_ref().expect("delivery checked").clone();
            after.sequence = after.sequence.checked_add(1).ok_or(Failure::Capacity)?;
            for lot in &mut after.lots {
                let moved = quantity(&carry.slots, &lot.stack)
                    .checked_sub(quantity(&carry_after, &lot.stack))
                    .ok_or(Failure::InvalidRequest)?;
                let moved = i32::try_from(moved).map_err(|_| Failure::Capacity)?;
                lot.stack.count = lot
                    .stack
                    .count
                    .checked_sub(moved)
                    .filter(|count| *count >= 0)
                    .ok_or(Failure::InvalidRequest)?;
            }
            after.lots.retain(|lot| lot.stack.count > 0);
            if !after.intact(&carry_after) {
                return Err(Failure::InvalidRequest);
            }
            inventory::commit(
                world,
                vec![
                    inventory::Change {
                        before: target,
                        after: target_after,
                    },
                    inventory::Change {
                        before: carry,
                        after: carry_after,
                    },
                ],
            )?;
            // Inventory post-images, fence metadata and this policy post-image
            // are captured by the same native tick/WAL/checkpoint authority.
            let pending = after.pending();
            job.delivery = Some(after);
            job.fetch_at = None;
            if pending {
                park(job, "awaiting output storage", retry);
            } else {
                job.paused = None;
                job.next_due = retry;
            }
            return Ok(true);
        }
        if walk.is_none_or(|(_, nearest)| distance < nearest)
            && let Some(stand) = supply_stand(world, job.worker, position)
        {
            walk = Some((stand, distance));
        }
    }
    let Some((stand, _)) = walk else {
        job.fetch_at = None;
        return Ok(park(job, "no output storage", retry));
    };
    job.fetch_at = Some(stand);
    Ok(park(job, "awaiting output storage reach", retry))
}
