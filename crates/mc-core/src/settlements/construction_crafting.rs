//! Bounded builder crafting over canonical storage images.
//!
//! Pure planner: borrows storage [`inventory::Image`]s, consumes real stock
//! and adds exact vanilla batch yields (plus surplus) to a cloned post-image.
//! Never writes or commits; the caller transfers the requested unit to
//! builder carry and commits atomically. Any shortage or capacity failure
//! returns `None` and leaves the caller's images untouched.
//!
//! Only grid crafting (`Shaped`/`Shapeless`) from the vanilla registry is
//! used, so furnace-only transformations never apply. Ingredient matching
//! reuses the vanilla helpers, preserving colored wool/wood alternatives.

use super::inventory;
use crate::tick::TickState;
use mc_data::ItemStack;
use mc_data::recipes::{Ingredient, Recipe, RecipeKind};
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;

/// Deepest allowed craft chain (log -> planks -> door is depth 2).
const MAX_DEPTH: u8 = 8;
/// Total ingredient/batch operations across one `craft` call.
const MAX_WORK: usize = 4096;
/// Batches for a single recipe step; a full carried stack needs up to 22.
const MAX_BATCHES: i32 = 32;
/// Native container slot cap mirrors the fence ledger (`min(99)`).
const MAX_SLOT: i32 = 99;

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct ScanStats {
    pub(crate) funding_calls: usize,
    pub(crate) recipes_scanned: usize,
    pub(crate) result_resolutions: usize,
    pub(crate) index_queries: usize,
    pub(crate) index_builds: usize,
    pub(crate) memo_hits: usize,
    pub(crate) memo_misses: usize,
    pub(crate) wood_memo_hits: usize,
    pub(crate) wood_memo_misses: usize,
    pub(crate) unreachable_rejections: usize,
}

#[cfg(test)]
std::thread_local! {
    static SCAN_STATS: std::cell::Cell<ScanStats> = std::cell::Cell::new(ScanStats::default());
}

#[cfg(test)]
fn count_scan(update: impl FnOnce(&mut ScanStats)) {
    SCAN_STATS.with(|cell| {
        let mut stats = cell.get();
        update(&mut stats);
        cell.set(stats);
    });
}

#[cfg(test)]
pub(crate) fn reset_scan_stats() {
    SCAN_STATS.with(|cell| cell.set(ScanStats::default()));
}

#[cfg(test)]
pub(crate) fn scan_stats() -> ScanStats {
    SCAN_STATS.with(std::cell::Cell::get)
}

/// Narrow measured optimization: on the adopted-village closure test most
/// funding time went to repeating identical boolean and wood-reservation
/// proofs. The settlement Registry owns at most 16 derived results, never
/// persisted or journaled.
///
/// A hit requires the probe kind, exact ordered bill, every slot/stack component
/// by value, and the same strongly held immutable recipe index (items, facts,
/// tags and recipes are rebuilt together). Endpoints, fences, time and settlement
/// rows are not read by either pure proof. The wood probe starts after the
/// wood-specific projection, which still runs every time; other bed colors do
/// not change that ordered sub-bill. The memo stores only canonical proof results
/// and cannot schedule or choose work.
///
/// Native callers run on the serial tick owner; entity workers receive no
/// settlement Registry. Locks cover lookup/insert only. Concurrent callers
/// could change hit/eviction order and compute a miss twice, but both compute
/// the identical result. Replacement mixes a counter, not gameplay RNG.
///
/// The closure test's full-proof ceiling fails when every probe misses. Remove
/// the two wrappers and the Registry field when a replacement proof keeps that
/// test under its ceiling without them; both canonical uncached proofs remain
/// below.
#[derive(Default)]
pub(super) struct FundingMemo {
    entries: Vec<FundingProof>,
    replacements: u64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ProbeKind {
    Boolean,
    ReservedWood,
}

#[derive(Clone)]
enum MemoResult {
    Boolean(bool),
    ReservedWood(Result<BTreeMap<u32, i32>, Option<crate::villager::VillagerProfession>>),
}

impl MemoResult {
    fn kind(&self) -> ProbeKind {
        match self {
            Self::Boolean(_) => ProbeKind::Boolean,
            Self::ReservedWood(_) => ProbeKind::ReservedWood,
        }
    }
}

struct FundingProof {
    index: std::sync::Arc<Index>,
    bill: Vec<(u32, u32)>,
    slots: Vec<Vec<ItemStack>>,
    result: MemoResult,
}

impl std::fmt::Debug for FundingMemo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FundingMemo")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl FundingMemo {
    fn get<'a>(
        &self,
        index: &std::sync::Arc<Index>,
        bill: &[(u32, u32)],
        slots: impl ExactSizeIterator<Item = &'a [ItemStack]> + Clone,
        kind: ProbeKind,
    ) -> Option<&MemoResult> {
        let position = self.entries.iter().rposition(|entry| {
            entry.result.kind() == kind
                && std::sync::Arc::ptr_eq(&entry.index, index)
                && entry.bill == bill
                && entry.slots.len() == slots.len()
                && entry.slots.iter().zip(slots.clone()).all(|(a, b)| a == b)
        })?;
        self.entries.get(position).map(|entry| &entry.result)
    }

    fn remember<'a>(
        &mut self,
        index: &std::sync::Arc<Index>,
        bill: &[(u32, u32)],
        slots: impl ExactSizeIterator<Item = &'a [ItemStack]> + Clone,
        result: MemoResult,
    ) {
        // A concurrent miss may already have installed this exact pure result.
        if self
            .get(index, bill, slots.clone(), result.kind())
            .is_some()
        {
            return;
        }
        if self.entries.is_empty() {
            self.entries.reserve(16);
        }
        if self.entries.len() == 16 {
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&self.replacements, &mut hash);
            self.replacements = self.replacements.wrapping_add(1);
            let victim = (std::hash::Hasher::finish(&hash) & 15) as usize;
            self.entries.swap_remove(victim);
        }
        self.entries.push(FundingProof {
            index: std::sync::Arc::clone(index),
            bill: bill.to_vec(),
            slots: slots.map(<[ItemStack]>::to_vec).collect(),
            result,
        });
    }
}

/// Immutable candidate lists derived from the registry's recipe and tag order.
/// Rebuilt by the registry owner together with the other recipe facts.
#[derive(Debug, Default)]
pub(crate) struct Index {
    recipes: Vec<RecipePlan>,
    by_result: BTreeMap<u32, Vec<usize>>,
    ingredients: Vec<IngredientPlan>,
    has_model_outputs: bool,
}

#[derive(Debug)]
struct RecipePlan {
    recipe: usize,
    result: u32,
    count: i32,
    ingredients: Option<Vec<usize>>,
}

#[derive(Debug)]
struct IngredientPlan {
    ingredient: Ingredient,
    recipes: Vec<usize>,
    items: Vec<u32>,
    accepted_items: Vec<u32>,
}

impl Index {
    pub(crate) fn build(
        items: &mc_data::items::ItemRegistry,
        facts: &mc_data::item_components::ItemFactsTable,
        tags: &mc_data::tags::TagsData,
        recipes: &[Recipe],
    ) -> Self {
        #[cfg(test)]
        count_scan(|stats| stats.index_builds += 1);
        let mut index = Self::default();
        for (position, recipe) in recipes.iter().enumerate() {
            if !is_grid(recipe) {
                continue;
            }
            let Some((result, count)) = result_identity(items, facts, recipe) else {
                continue;
            };
            index.has_model_outputs |= facts.custom(&recipe.result.item).is_some();
            let ingredients = expanded(recipe).map(|ingredients| {
                ingredients
                    .into_iter()
                    .map(|ingredient| {
                        if let Some(position) = index
                            .ingredients
                            .iter()
                            .position(|plan| plan.ingredient == ingredient)
                        {
                            return position;
                        }
                        let position = index.ingredients.len();
                        index.ingredients.push(IngredientPlan {
                            ingredient,
                            recipes: Vec::new(),
                            items: Vec::new(),
                            accepted_items: Vec::new(),
                        });
                        position
                    })
                    .collect()
            });
            index
                .by_result
                .entry(result)
                .or_default()
                .push(index.recipes.len());
            index.recipes.push(RecipePlan {
                recipe: position,
                result,
                count,
                ingredients,
            });
        }
        for plan in &mut index.ingredients {
            for alternative in &plan.ingredient.alternatives {
                match alternative {
                    mc_data::recipes::IngredientAlternative::Item(item) => {
                        if let Some(item) = items.id_of(item) {
                            plan.accepted_items.push(item);
                        }
                    }
                    mc_data::recipes::IngredientAlternative::Tag(tag) => {
                        if let Some(members) =
                            tags.raw_ids(mc_data::tags::ITEM_REGISTRY, tag.as_str())
                        {
                            plan.accepted_items.extend(
                                members.iter().filter_map(|&item| u32::try_from(item).ok()),
                            );
                        }
                    }
                }
            }
            plan.accepted_items.sort_unstable();
            plan.accepted_items.dedup();
            plan.items = items
                .iter()
                .filter_map(|(_, item)| {
                    plan.accepted_items
                        .binary_search(&item)
                        .is_ok()
                        .then_some(item)
                })
                .collect();
            plan.recipes = index
                .recipes
                .iter()
                .enumerate()
                .filter_map(|(position, recipe)| {
                    plan.accepted_items
                        .binary_search(&recipe.result)
                        .is_ok()
                        .then_some(position)
                })
                .collect();
        }
        index
    }

    fn for_item(&self, item: u32) -> &[usize] {
        #[cfg(test)]
        count_scan(|stats| stats.index_queries += 1);
        self.by_result.get(&item).map_or(&[], Vec::as_slice)
    }
}

/// Per-proof counts derived from the slot images. Slots still determine which
/// ingredient is consumed and where each result is placed. Keep the unsaturated
/// sum so rolling back a write also restores totals above `i32::MAX` exactly.
#[derive(Default)]
struct SlotTotals(FxHashMap<u32, u128>);

impl SlotTotals {
    fn from_slots(post: &[Vec<ItemStack>]) -> Self {
        let mut totals = Self::default();
        for stack in post.iter().flatten().filter(|stack| !stack.is_empty()) {
            *totals.0.entry(stack.item_id).or_default() += stack.count as u128;
        }
        totals
    }

    fn get(&self, item: u32) -> i32 {
        i32::try_from(self.0.get(&item).copied().unwrap_or(0)).unwrap_or(i32::MAX)
    }

    fn replace(&mut self, previous: &ItemStack, item: u32, count: i32) {
        if !previous.is_empty() {
            *self
                .0
                .get_mut(&previous.item_id)
                .expect("positive slot has a derived total") -= previous.count as u128;
        }
        if count > 0 {
            *self.0.entry(item).or_default() += count as u128;
        }
    }
}

#[derive(Default)]
struct Work<'a> {
    count: usize,
    trace: bool,
    missing_gatherer: Option<crate::villager::VillagerProfession>,
    reserved: Option<&'a BTreeMap<u32, i32>>,
    undo: Vec<(usize, usize, ItemStack)>,
    totals: SlotTotals,
}

impl Work<'_> {
    fn rollback(&mut self, post: &mut [Vec<ItemStack>], mark: usize) {
        while self.undo.len() > mark {
            let (image, slot, previous) = self.undo.pop().expect("undo entry after mark");
            self.totals
                .replace(&post[image][slot], previous.item_id, previous.count);
            post[image][slot] = previous;
        }
    }
}

/// Plan post-craft slots in exact image order, guaranteeing one unit of
/// `item` across the returned images. Returns `None` when stock, recipes or
/// room cannot cover it. Already-stocked items return unchanged slot clones.
pub(crate) fn craft(
    state: &TickState,
    item: u32,
    images: &[inventory::Image],
) -> Option<Vec<Vec<ItemStack>>> {
    let mut post: Vec<Vec<ItemStack>> = images.iter().map(|image| image.slots.clone()).collect();
    if total(&post, item) >= 1 {
        return Some(post);
    }
    if post.is_empty() {
        return None;
    }
    state.registry.items.name_of(item)?;
    let mut work = Work {
        totals: SlotTotals::from_slots(&post),
        ..Work::default()
    };
    if ensure(state, &mut post, &mut work, item, 1, 0) && work.totals.get(item) >= 1 {
        Some(post)
    } else {
        None
    }
}

/// Craft until `need` units of `item` exist in `post` (a haul of many units).
pub(crate) fn ensure_units(
    state: &TickState,
    post: &mut Vec<Vec<ItemStack>>,
    item: u32,
    need: i32,
    reserved: &BTreeMap<u32, i32>,
) -> bool {
    let mut work = Work {
        reserved: Some(reserved),
        totals: SlotTotals::from_slots(post),
        ..Work::default()
    };
    ensure(state, post, &mut work, item, need, 0)
}

/// Single-authority probe: delegates to [`craft`] so no second recipe
/// authority can diverge.
pub(crate) fn can_craft(state: &TickState, item: u32, images: &[inventory::Image]) -> bool {
    craft(state, item, images).is_some()
}

/// A negative-only dependency proof over the same compiled ingredients. It
/// ignores quantities, reservations and capacity, so a possible result still
/// needs the canonical ordered simulation. Cycles and the depth boundary are
/// deliberately possible, never grounds for rejecting a bill.
fn unreachable_requirement(
    index: &Index,
    requirements: &[(u32, u32)],
    images: &[inventory::Image],
) -> bool {
    if index.has_model_outputs
        || images
            .iter()
            .flat_map(|image| &image.slots)
            .any(|stack| !stack.is_empty() && stack.item_model.is_some())
    {
        // Custom models can match ingredients independently of their carrier.
        return false;
    }
    let mut possible: FxHashMap<_, _> = images
        .iter()
        .flat_map(|image| &image.slots)
        .filter(|stack| !stack.is_empty())
        .map(|stack| (stack.item_id, true))
        .collect();
    requirements
        .iter()
        .any(|&(item, count)| count > 0 && !possibly_reachable(index, item, &mut possible, 0))
}

fn possibly_reachable(
    index: &Index,
    item: u32,
    memo: &mut FxHashMap<u32, bool>,
    depth: u8,
) -> bool {
    if depth > MAX_DEPTH {
        return true;
    }
    if let Some(&possible) = memo.get(&item) {
        return possible;
    }
    // An unfinished dependency is conservatively possible. A cached false
    // therefore always has a finite proof ending at unavailable raw stock.
    memo.insert(item, true);
    let possible = index.for_item(item).iter().any(|&candidate| {
        index.recipes[candidate]
            .ingredients
            .as_ref()
            .is_some_and(|ingredients| {
                ingredients.iter().all(|&ingredient| {
                    index.ingredients[ingredient]
                        .accepted_items
                        .iter()
                        .any(|&item| possibly_reachable(index, item, memo, depth + 1))
                })
            })
    });
    memo.insert(item, possible);
    possible
}

/// Read-only funding proof for a whole upgrade. Virtual consumption prevents
/// the same logs/intermediate recipe surplus from funding two requirements.
pub(crate) fn can_fund(
    state: &TickState,
    requirements: &[(u32, u32)],
    images: &[inventory::Image],
) -> bool {
    // A grid recipe cannot create a result absent from its result index.
    // Boolean-only rejection preserves the traced proof's first-failure order.
    let raw_shortage = requirements.iter().any(|&(item, need)| {
        need > 0
            && !state
                .registry
                .construction_crafting
                .by_result
                .contains_key(&item)
            && images
                .iter()
                .flat_map(|image| &image.slots)
                .filter(|stack| !stack.is_empty() && stack.item_id == item)
                .map(|stack| u64::try_from(stack.count.max(0)).unwrap_or(0))
                .sum::<u64>()
                < u64::from(need)
    });
    let index = &state.registry.construction_crafting;
    let slots = || images.iter().map(|image| image.slots.as_slice());
    let cached = if raw_shortage {
        None
    } else {
        state
            .settlements
            .funding
            .lock()
            .expect("funding memo poisoned")
            .get(index, requirements, slots(), ProbeKind::Boolean)
            .cloned()
    };
    let funded = if raw_shortage {
        false
    } else if let Some(MemoResult::Boolean(funded)) = cached {
        #[cfg(test)]
        count_scan(|stats| stats.memo_hits += 1);
        funded
    } else {
        #[cfg(test)]
        count_scan(|stats| stats.memo_misses += 1);
        if unreachable_requirement(index, requirements, images) {
            #[cfg(test)]
            count_scan(|stats| stats.unreachable_rejections += 1);
            false
        } else {
            let funded = funding(state, requirements, images, false).0;
            state
                .settlements
                .funding
                .lock()
                .expect("funding memo poisoned")
                .remember(index, requirements, slots(), MemoResult::Boolean(funded));
            funded
        }
    };
    funded
}

/// The first missing extraction capability in the same whole-bill proof.
pub(crate) fn needed_gatherer(
    state: &TickState,
    requirements: &[(u32, u32)],
    images: &[inventory::Image],
) -> Option<crate::villager::VillagerProfession> {
    funding(state, requirements, images, true).1
}

fn funding(
    state: &TickState,
    requirements: &[(u32, u32)],
    images: &[inventory::Image],
    trace: bool,
) -> (bool, Option<crate::villager::VillagerProfession>) {
    let mut post: Vec<_> = images.iter().map(|image| image.slots.clone()).collect();
    let result = simulate_bill(state, requirements, &mut post, trace, true);
    #[cfg(test)]
    count_scan(|stats| stats.funding_calls += 1);
    result
}

fn simulate_bill(
    state: &TickState,
    requirements: &[(u32, u32)],
    post: &mut Vec<Vec<ItemStack>>,
    trace: bool,
    protect: bool,
) -> (bool, Option<crate::villager::VillagerProfession>) {
    if requirements
        .iter()
        .map(|(_, count)| u64::from(*count))
        .sum::<u64>()
        > 65_536
    {
        return (false, None);
    }
    let mut totals = SlotTotals::from_slots(post);
    for (index, (item, count)) in requirements.iter().enumerate() {
        let reserve_future = protect && !wood_specific(state, *item);
        let mut remaining = *count;
        while remaining > 0 {
            let reserved = if reserve_future {
                match reserve_wood(state, &requirements[index + 1..], post) {
                    Ok(reserved) => reserved,
                    Err(missing) => return (false, missing),
                }
            } else {
                BTreeMap::new()
            };
            let mut work = Work {
                trace,
                reserved: Some(&reserved),
                totals: std::mem::take(&mut totals),
                ..Work::default()
            };
            if !ensure(state, post, &mut work, *item, 1, 0) {
                return (false, work.missing_gatherer);
            }
            let mut available =
                (work.totals.get(*item) - reserved.get(item).copied().unwrap_or(0)).max(0);
            for stack in post
                .iter_mut()
                .flatten()
                .filter(|stack| stack.item_id == *item && stack.count > 0)
            {
                let take = remaining.min(stack.count as u32).min(available as u32);
                remaining -= take;
                available -= take as i32;
                work.totals
                    .replace(stack, stack.item_id, stack.count - take as i32);
                stack.count -= take as i32;
                if stack.count == 0 {
                    *stack = ItemStack::EMPTY;
                }
                if remaining == 0 || available == 0 {
                    break;
                }
            }
            totals = work.totals;
        }
    }
    (true, None)
}

pub(crate) fn wood_specific(state: &TickState, item: u32) -> bool {
    super::construction::wood_species(state, item).is_some()
}

/// Reserve actual inputs consumed by the same proof for later wood-specific
/// cells. Generic recipes may use only the unreserved physical remainder.
pub(crate) fn reserve_wood(
    state: &TickState,
    requirements: &[(u32, u32)],
    post: &[Vec<ItemStack>],
) -> Result<BTreeMap<u32, i32>, Option<crate::villager::VillagerProfession>> {
    let strict: Vec<_> = requirements
        .iter()
        .copied()
        .filter(|(item, count)| *count > 0 && wood_specific(state, *item))
        .collect();
    if strict.is_empty() {
        return Ok(BTreeMap::new());
    }
    reserve_strict_wood(state, &strict, post)
}

fn reserve_strict_wood(
    state: &TickState,
    strict: &[(u32, u32)],
    post: &[Vec<ItemStack>],
) -> Result<BTreeMap<u32, i32>, Option<crate::villager::VillagerProfession>> {
    let index = &state.registry.construction_crafting;
    let cached = state
        .settlements
        .funding
        .lock()
        .expect("funding memo poisoned")
        .get(
            index,
            strict,
            post.iter().map(Vec::as_slice),
            ProbeKind::ReservedWood,
        )
        .cloned();
    if let Some(MemoResult::ReservedWood(result)) = cached {
        #[cfg(test)]
        count_scan(|stats| stats.wood_memo_hits += 1);
        return result;
    }
    #[cfg(test)]
    count_scan(|stats| stats.wood_memo_misses += 1);
    let result = reserve_strict_wood_uncached(state, strict, post);
    state
        .settlements
        .funding
        .lock()
        .expect("funding memo poisoned")
        .remember(
            index,
            strict,
            post.iter().map(Vec::as_slice),
            MemoResult::ReservedWood(result.clone()),
        );
    result
}

fn reserve_strict_wood_uncached(
    state: &TickState,
    strict: &[(u32, u32)],
    post: &[Vec<ItemStack>],
) -> Result<BTreeMap<u32, i32>, Option<crate::villager::VillagerProfession>> {
    let mut after = post.to_vec();
    let (funded, missing) = simulate_bill(state, strict, &mut after, true, false);
    if !funded {
        return Err(missing);
    }
    let mut reserved = BTreeMap::new();
    for stack in post.iter().flatten().filter(|stack| !stack.is_empty()) {
        reserved.entry(stack.item_id).or_insert_with(|| {
            total(post, stack.item_id).saturating_sub(total(&after, stack.item_id))
        });
    }
    reserved.retain(|_, count| *count > 0);
    Ok(reserved)
}

/// Simple stock gap for explicit requirements: `(item, still-missing)` in
/// input order. Pure counting, no recipe logic.
pub(crate) fn missing_base(
    _state: &TickState,
    requirements: &[(u32, u32)],
    images: &[inventory::Image],
) -> Vec<(u32, u32)> {
    let mut missing = Vec::new();
    for &(item, need) in requirements {
        let need = u64::from(need);
        let mut have = 0u64;
        for image in images {
            for stack in &image.slots {
                if !stack.is_empty() && stack.item_id == item {
                    have += u64::try_from(stack.count.max(0)).unwrap_or(0);
                }
            }
        }
        if have < need {
            missing.push((item, u32::try_from(need - have).unwrap_or(u32::MAX)));
        }
    }
    missing
}

fn total(post: &[Vec<ItemStack>], item: u32) -> i32 {
    let mut sum = 0i32;
    for image in post {
        for stack in image {
            if !stack.is_empty() && stack.item_id == item {
                sum = sum.saturating_add(stack.count.max(0));
            }
        }
    }
    sum
}

fn available(item: u32, work: &Work<'_>) -> i32 {
    (work.totals.get(item)
        - work
            .reserved
            .and_then(|reserved| reserved.get(&item))
            .copied()
            .unwrap_or(0))
    .max(0)
}

/// Consume one unit satisfying a vanilla ingredient, scanning images then
/// slots in order for determinism.
fn take_matching(
    state: &TickState,
    post: &mut [Vec<ItemStack>],
    work: &mut Work<'_>,
    ingredient: &IngredientPlan,
) -> bool {
    for image in 0..post.len() {
        for index in 0..post[image].len() {
            let slot = &post[image][index];
            if slot.is_empty() {
                continue;
            }
            let accepted = if slot.item_model.is_some() {
                mc_data::recipes::ingredient_accepts_stack(
                    &state.registry.items,
                    &state.registry.item_facts,
                    &state.registry.tags,
                    slot,
                    &ingredient.ingredient,
                )
            } else {
                ingredient
                    .accepted_items
                    .binary_search(&slot.item_id)
                    .is_ok()
            };
            if !accepted
                || (work
                    .reserved
                    .is_some_and(|reserved| reserved.contains_key(&slot.item_id))
                    && available(slot.item_id, work) == 0)
            {
                continue;
            }
            work.undo.push((image, index, slot.clone()));
            work.totals.replace(slot, slot.item_id, slot.count - 1);
            let slot = &mut post[image][index];
            slot.count -= 1;
            if slot.count <= 0 {
                *slot = ItemStack::EMPTY;
            }
            return true;
        }
    }
    false
}
/// Capacity-checked insert mirroring the native ledger: merge first, then
/// empty slots, both in image/slot order.
pub(crate) fn add_stack(
    state: &TickState,
    post: &mut [Vec<ItemStack>],
    template: &ItemStack,
) -> bool {
    add_stack_recorded(state, post, template, |_, _, _, _, _| {})
}

fn add_stack_recorded(
    state: &TickState,
    post: &mut [Vec<ItemStack>],
    template: &ItemStack,
    mut before_write: impl FnMut(usize, usize, &ItemStack, u32, i32),
) -> bool {
    let mut remaining = template.count;
    if remaining <= 0 {
        return false;
    }
    let max = state
        .registry
        .item_rules()
        .max_stack(template)
        .min(MAX_SLOT);
    if max <= 0 {
        return false;
    }
    for empty in [false, true] {
        for (image_index, image) in post.iter_mut().enumerate() {
            for (slot_index, slot) in image.iter_mut().enumerate() {
                if remaining <= 0 {
                    return true;
                }
                if slot.is_empty() != empty {
                    continue;
                }
                if empty {
                    let place = remaining.min(max);
                    before_write(image_index, slot_index, slot, template.item_id, place);
                    let mut placed = template.clone();
                    placed.count = place;
                    *slot = placed;
                    remaining -= place;
                } else {
                    if !mc_data::inventory_semantics_26_1_2::can_stack(slot, template) {
                        continue;
                    }
                    if slot.count >= max {
                        continue;
                    }
                    let place = (max - slot.count).min(remaining);
                    if place <= 0 {
                        continue;
                    }
                    before_write(
                        image_index,
                        slot_index,
                        slot,
                        slot.item_id,
                        slot.count + place,
                    );
                    slot.count += place;
                    remaining -= place;
                }
            }
        }
    }
    remaining <= 0
}

/// Expanded one-unit-per-entry ingredient list; `None` for non-grid or
/// malformed recipes (never crafted here).
fn expanded(recipe: &mc_data::recipes::Recipe) -> Option<Vec<Ingredient>> {
    match &recipe.kind {
        RecipeKind::Shaped(shaped) => {
            let mut out = Vec::new();
            for row in &shaped.pattern {
                for ch in row.chars() {
                    if ch == ' ' {
                        continue;
                    }
                    out.push(shaped.key.get(&ch)?.clone());
                }
            }
            if out.is_empty() || out.len() > 9 {
                return None;
            }
            Some(out)
        }
        RecipeKind::Shapeless(shapeless) => {
            if shapeless.ingredients.is_empty() || shapeless.ingredients.len() > 9 {
                return None;
            }
            Some(shapeless.ingredients.clone())
        }
        _ => None,
    }
}

fn is_grid(recipe: &mc_data::recipes::Recipe) -> bool {
    recipe.id.as_str().starts_with("minecraft:")
        && matches!(
            recipe.kind,
            RecipeKind::Shaped(_) | RecipeKind::Shapeless(_)
        )
}

/// Concrete result identity; custom-model results match by carrier id.
fn result_identity(
    items: &mc_data::items::ItemRegistry,
    facts: &mc_data::item_components::ItemFactsTable,
    recipe: &Recipe,
) -> Option<(u32, i32)> {
    #[cfg(test)]
    count_scan(|stats| stats.result_resolutions += 1);
    let count = i32::try_from(recipe.result.count).ok().filter(|c| *c > 0)?;
    if let Some(id) = items.id_of(&recipe.result.item) {
        return Some((id, count));
    }
    let stack = recipe.result.to_stack(items, facts)?;
    if stack.count <= 0 {
        return None;
    }
    Some((stack.item_id, count))
}

/// Ensure `need` units of a concrete item exist in `post`, crafting grid
/// batches recursively. Tries every vanilla recipe for the item in registry
/// order with backtracking; surplus from exact batch yields stays in `post`.
fn ensure(
    state: &TickState,
    post: &mut Vec<Vec<ItemStack>>,
    work: &mut Work<'_>,
    target: u32,
    need: i32,
    depth: u8,
) -> bool {
    if need <= 0 {
        return true;
    }
    if depth > MAX_DEPTH || work.count > MAX_WORK {
        return false;
    }
    if available(target, work) >= need {
        return true;
    }
    let deficit = need - available(target, work);
    let previous_missing = work.missing_gatherer;
    let index = &state.registry.construction_crafting;
    for &candidate in index.for_item(target) {
        #[cfg(test)]
        count_scan(|stats| stats.recipes_scanned += 1);
        let plan = &index.recipes[candidate];
        let recipe = &state.registry.recipes[plan.recipe];
        let yield_count = plan.count;
        let batches = deficit / yield_count + i32::from(deficit % yield_count != 0);
        if batches <= 0 || batches > MAX_BATCHES {
            continue;
        }
        let Some(ingredients) = &plan.ingredients else {
            continue;
        };
        let mark = work.undo.len();
        let mut ok = true;
        for _ in 0..batches {
            work.count = work.count.saturating_add(1);
            if work.count > MAX_WORK {
                ok = false;
                break;
            }
            for &ingredient in ingredients {
                let ingredient = &index.ingredients[ingredient];
                work.count = work.count.saturating_add(1);
                if work.count > MAX_WORK {
                    ok = false;
                    break;
                }
                if take_matching(state, post, work, ingredient) {
                    continue;
                }
                if !ensure_ingredient(state, post, work, ingredient, depth + 1) {
                    ok = false;
                    break;
                }
            }
            if !ok {
                break;
            }
            let Some(output) = recipe
                .result
                .to_stack(&state.registry.items, &state.registry.item_facts)
            else {
                ok = false;
                break;
            };
            if output.count <= 0
                || !add_stack_recorded(
                    state,
                    post,
                    &output,
                    |image, slot, previous, item, count| {
                        work.undo.push((image, slot, previous.clone()));
                        work.totals.replace(previous, item, count);
                    },
                )
            {
                ok = false;
                break;
            }
        }
        if ok && available(target, work) >= need {
            work.missing_gatherer = previous_missing;
            return true;
        }
        work.rollback(post, mark);
        if work.count > MAX_WORK {
            return false;
        }
    }
    if work.trace && work.missing_gatherer.is_none() {
        work.missing_gatherer = super::construction::gatherer_for_item(state, target);
    }
    false
}

/// Satisfy one generic ingredient unit: consume matching stock, else craft a
/// matching grid result (registry order) and consume the crafted unit.
fn ensure_ingredient(
    state: &TickState,
    post: &mut Vec<Vec<ItemStack>>,
    work: &mut Work<'_>,
    ingredient: &IngredientPlan,
    depth: u8,
) -> bool {
    if take_matching(state, post, work, ingredient) {
        return true;
    }
    if depth > MAX_DEPTH || work.count > MAX_WORK {
        return false;
    }
    let previous_missing = work.missing_gatherer;
    #[cfg(test)]
    count_scan(|stats| stats.index_queries += 1);
    for &candidate in &ingredient.recipes {
        #[cfg(test)]
        count_scan(|stats| stats.recipes_scanned += 1);
        let id = state.registry.construction_crafting.recipes[candidate].result;
        let mark = work.undo.len();
        if ensure(state, post, work, id, 1, depth + 1)
            && take_matching(state, post, work, ingredient)
        {
            work.missing_gatherer = previous_missing;
            return true;
        }
        work.rollback(post, mark);
        if work.count > MAX_WORK {
            return false;
        }
    }
    if work.trace && work.missing_gatherer.is_none() {
        work.missing_gatherer = ingredient
            .items
            .iter()
            .find_map(|&item| super::construction::gatherer_for_item(state, item));
    }
    false
}

#[cfg(test)]
#[path = "construction_crafting_tests.rs"]
mod tests;
