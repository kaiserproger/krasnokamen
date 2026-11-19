//! D8 round-3 production-path proofs (D8Tests slice): the parallel admission
//! clock (R2-2), the joint funding proof through the site search (R2-3), retry
//! metadata on parallel identical orders (R2-7/A1), FIFO against a waiting head
//! (R2-7/A2), per-binding advancement (R2-7/SOL-1), the owner-visible
//! stall-watch retry refusal (A4), the submission place across a native
//! save/reopen (R2-proof), and the per-key checkpoint/WAL fences (R2-8).
//!
//! Every proof enters through the production path: `construction::tick` the
//! settlement tick calls, the planner's own site search, `request_build`,
//! `retry_preparation`, and the native checkpoint/WAL readers. Fixtures only
//! prepare state a running world would already hold (a stocked chest, residents,
//! a bound builder) and stand builders where the brain would have walked them.
use super::tests::{bind_project, complete, fixture, fund, open_plan, request_started};
use super::*;
use crate::boundary::{BlockPos, ChangeSet};
use crate::entities::EntitySlot;
use crate::journal::{JournalEntry, Replay};
use crate::settlements::allocator::tests::{block, test_state, villager};
use crate::settlements::persistence::{
    checkpoint_rows, hydrate_dimension, journal_entries, row_key,
};
use crate::villager::Activity;
use mc_data::ItemStack;

const CHEST: (i32, i32, i32) = (1, 201, 1);
const SHIFT_DAY_TIME: i64 = 3_000;
const OWNER: uuid::Uuid = uuid::Uuid::from_u128(0x0000_d80e_57a1);

fn chest_slots(state: &mut TickState) -> &mut Vec<ItemStack> {
    &mut state
        .block_entities
        .get_mut(&CHEST)
        .expect("fixture chest")
        .slots
}

fn clear_chest(state: &mut TickState) {
    chest_slots(state).fill(ItemStack::EMPTY);
}

/// Stock `copies` complete material bills of one authored level, one stack per
/// entry so a bill larger than a stack still fits.
fn stock(state: &mut TickState, blueprint: &str, level: u8, copies: usize) {
    let materials = blueprints::get(&state.registry, blueprint)
        .expect("known blueprint")
        .level(level)
        .expect("authored level")
        .materials
        .clone();
    clear_chest(state);
    let slots = chest_slots(state);
    let mut index = 0;
    for _ in 0..copies {
        for (item, quantity) in &materials {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                assert!(
                    index < slots.len(),
                    "the fixture chest holds {copies} × {blueprint} L{level}"
                );
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
}

/// Stock a bill into a settlement's own storage chest.
fn stock_at(
    state: &mut TickState,
    chest: (i32, i32, i32),
    blueprint: &str,
    level: u8,
    copies: usize,
) {
    let materials = blueprints::get(&state.registry, blueprint)
        .expect("known blueprint")
        .level(level)
        .expect("authored level")
        .materials
        .clone();
    let slots = &mut state
        .block_entities
        .get_mut(&chest)
        .expect("fixture chest")
        .slots;
    slots.fill(ItemStack::EMPTY);
    let mut index = 0;
    for _ in 0..copies {
        for (item, quantity) in &materials {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                assert!(
                    index < slots.len(),
                    "the fixture chest holds {copies} × {blueprint} L{level}"
                );
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
}

/// Stock `copies` complete bills of one authored level in one wood species: every
/// `oak_*` material swapped to `{species}_*`, as the admission itself would.
fn stock_species(state: &mut TickState, blueprint: &str, level: u8, copies: usize, species: &str) {
    let materials = blueprints::get(&state.registry, blueprint)
        .expect("known blueprint")
        .level(level)
        .expect("authored level")
        .materials
        .clone();
    let bill: Vec<(u32, u32)> = materials
        .iter()
        .map(|&(item, count)| {
            (
                species_item(state, item, species).expect("a species variant exists"),
                count,
            )
        })
        .collect();
    let slots = chest_slots(state);
    let mut index = 0;
    for _ in 0..copies {
        for (item, quantity) in &bill {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                assert!(
                    index < slots.len(),
                    "the fixture chest holds {copies} × {blueprint} L{level} in {species}"
                );
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
}

/// Add `copies` bills of one level in one species to the stock already there.
fn add_species_stock(
    state: &mut TickState,
    blueprint: &str,
    level: u8,
    copies: usize,
    species: &str,
) {
    let materials = blueprints::get(&state.registry, blueprint)
        .expect("known blueprint")
        .level(level)
        .expect("authored level")
        .materials
        .clone();
    let bill: Vec<(u32, u32)> = materials
        .iter()
        .map(|&(item, count)| (species_item(state, item, species).expect("variant"), count))
        .collect();
    let slots = chest_slots(state);
    for _ in 0..copies {
        for (item, quantity) in &bill {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                match slots.iter_mut().find(|stack| {
                    stack.item_id == *item && stack.count > 0 && stack.count + count as i32 <= 64
                }) {
                    Some(stack) => stack.count += count as i32,
                    None => {
                        let empty = slots
                            .iter_mut()
                            .find(|stack| stack.is_empty())
                            .expect("chest room");
                        *empty = ItemStack::new(*item, count as i32);
                    }
                }
                quantity -= count;
            }
        }
    }
}

/// A second (or third) resident of the settlement, hired as a Builder.
fn builder_at(state: &mut TickState, id: SettlementId, pos: [f64; 3]) -> EntitySlot {
    let slot = villager(state, pos);
    let entity = state.entities.body(slot).id;
    crate::settlements::bind(state, id, entity).expect("member villager");
    let v = state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .expect("villager");
    v.data.profession = Profession::Builder;
    v.activity = Activity::Work;
    slot
}

fn carried(v: &crate::villager::VillagerState, item: u32, count: u32) -> bool {
    v.inventory
        .iter()
        .flatten()
        .filter(|stack| stack.item_id == item && stack.count > 0)
        .map(|stack| stack.count as u32)
        .sum::<u32>()
        >= count
}

/// Admit one real structure ledger and return its handle: the fixture's stand-in
/// for the planner, which every direct executor test also needs.
fn begin_at(state: &mut TickState, id: SettlementId, origin: [i32; 3], blueprint: &str) -> String {
    assert!(
        begin(
            state,
            Project {
                settlement: id,
                blueprint: blueprint.into(),
                level: 1,
                origin,
                rotation: 0,
                building: None,
                structure: String::new(),
                ordered: false,
                builder: None,
                order_seq: None,
            },
            "oak",
            0,
        ),
        "{blueprint} at {origin:?} has a legal site"
    );
    access::projects_of(&state.settlements, id)
        .last()
        .expect("admitted project")
        .structure
        .clone()
}

fn handle_of(state: &TickState, id: SettlementId, index: usize) -> String {
    access::projects_of(&state.settlements, id)
        .nth(index)
        .unwrap_or_else(|| panic!("settlement {} has project #{index}", id.0))
        .structure
        .clone()
}

/// Work units finished so far: completed stages plus the current stage's count.
fn work_done(state: &TickState, id: SettlementId, handle: &str) -> u64 {
    let status = executor::status(state, id, handle).expect("live ledger");
    status
        .stages
        .iter()
        .take(status.current_stage_index as usize)
        .map(|stage| stage.work_units)
        .sum::<u64>()
        + status.completed_work_units
}

/// Stand the bound builder where its own project's work wants it: the pickup
/// neighbour of the storage while it must fetch, else the exact perimeter
/// stand. Production's brain walks there; a unit fixture has no navigation.
fn stand(state: &mut TickState, id: SettlementId, slot: EntitySlot) {
    let uuid = state.entities.row(slot).uuid;
    let Some(mut v) = state.entities.row_mut(slot).villager.take() else {
        return;
    };
    let material = state
        .settlements
        .get(id)
        .and_then(|record| project_of_builder(record, uuid))
        .and_then(|project| {
            executor::next_material(state, id, &project.structure)
                .ok()
                .flatten()
        });
    let pickup = material.is_some_and(|(item, count)| !carried(&v, item, count));
    let target = crate::villager::building::work(state, slot, &mut v, &mut ChangeSet::default())
        .and_then(|target| {
            if pickup {
                Some(BlockPos {
                    z: target.pos.z + 1,
                    ..target.pos
                })
            } else {
                target.exact.then_some(target.pos)
            }
        });
    state.entities.row_mut(slot).villager = Some(v);
    if let Some(BlockPos { x, y, z }) = target {
        state
            .entities
            .set_pos(slot, [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
    }
}

/// Real construction admissions without the planning pass: the fixture sets the
/// projects and bindings up itself, so no autonomous project may join a proof.
fn drive(state: &mut TickState, shifts: &[(SettlementId, EntitySlot)], admissions: usize) {
    let mut round = 1u64;
    let mut done = 0;
    while done < admissions {
        let tick = round * 4;
        round += 1;
        if tick.is_multiple_of(PLAN_CADENCE) {
            continue;
        }
        state.tick = tick;
        state.day_time = SHIFT_DAY_TIME;
        super::tick(state);
        for (id, slot) in shifts {
            stand(state, *id, *slot);
        }
        done += 1;
    }
}

/// The planner's site sweep is bounded and resumable: one pass may only advance
/// its cursor, so admission needs several passes (as `plan_until_project` in the
/// shared fixtures shows).
fn plan_until(state: &mut TickState, id: SettlementId, projects: usize) {
    for _ in 0..256 {
        plan(state, id);
        if access::projects_of(&state.settlements, id).count() >= projects {
            return;
        }
    }
    panic!(
        "the planner admitted {projects} projects within 256 bounded passes; it has {}",
        access::projects_of(&state.settlements, id).count()
    );
}

/// R2-2: two settlements × two projects through the real admission tick; every
/// structure handle advances, whichever settlement and project the clock picks.
#[test]
fn two_settlements_of_two_projects_all_four_handles_advance() {
    let (mut state, first, first_builder) = fixture();
    open_plan(&mut state);
    let second = state
        .settlements
        .create(
            &mut state.settlement_sequence,
            "farmers".into(),
            "owner".into(),
            [24, 201, 8],
            48,
        )
        .expect("second settlement");
    // Its anchor chunk must be simulated, or the admission clock never reaches it.
    state.tickets.insert(
        "d8-second".into(),
        (crate::boundary::ChunkPos { x: 1, z: 0 }, 0),
    );
    state.refresh_active_chunks(0);
    // One storage chest and two builders per settlement.
    let second_chest = (30, 201, 2);
    let cobble = block(&state, "minecraft:chest", &[]);
    state
        .world
        .set_block(second_chest.0, second_chest.1, second_chest.2, cobble);
    state.block_entities.insert(
        second_chest,
        crate::tick::BlockEntityState {
            slots: vec![ItemStack::EMPTY; 27],
            furnace: None,
        },
    );
    state
        .settlements
        .get_mut(second)
        .expect("created settlement")
        .bootstrap_storage = Some([second_chest.0, second_chest.1, second_chest.2]);
    stock(&mut state, "house", 1, 2);
    stock_at(&mut state, second_chest, "house", 1, 2);
    let builders = [
        (first, first_builder),
        (first, builder_at(&mut state, first, [2.5, 201.0, 4.5])),
        (second, builder_at(&mut state, second, [18.5, 201.0, 4.5])),
        (second, builder_at(&mut state, second, [18.5, 201.0, 6.5])),
    ];
    // The planner chooses the lots; both settlements are funded separately.
    for (id, _) in [(first, ()), (second, ())] {
        for (order, _) in [("house", ()), ("house", ())] {
            assert!(
                matches!(
                    request_build(&mut state, id, order).expect("funded request"),
                    OrderOutcome::Started | OrderOutcome::Queued { .. }
                ),
                "a funded house is admitted or queued"
            );
        }
        plan_until(&mut state, id, 2);
    }
    let handles: Vec<(SettlementId, String)> = [first, second]
        .into_iter()
        .map(|id| (id, (handle_of(&state, id, 0), handle_of(&state, id, 1))))
        .flat_map(|(id, (a, b))| [(id, a), (id, b)])
        .collect();
    let mut shifts = Vec::new();
    for (id, slot) in builders {
        shifts.push((id, slot));
    }
    for ((id, handle), (_, slot)) in handles.iter().zip(builders) {
        bind_project(&mut state, *id, handle, slot);
    }
    let before: Vec<u64> = handles
        .iter()
        .map(|(id, handle)| work_done(&state, *id, handle))
        .collect();
    drive(&mut state, &shifts, 60);
    for ((id, handle), before) in handles.iter().zip(before) {
        let after = work_done(&state, *id, handle);
        assert!(
            after > before,
            "settlement {} project {handle} never advanced: {before} -> {after}",
            id.0
        );
    }
}

/// A flat plain, ten residents and one stocked chest: the authored fortification
/// fixture, which is what makes a wall stretch reachable at all.
fn walled_town() -> (TickState, SettlementId, EntitySlot) {
    let (mut state, id, builder) = fixture();
    state
        .world
        .ensure_blocking(&rustc_hash::FxHashMap::from_iter((-3..=3).flat_map(|x| {
            (-3..=3).map(move |z| (crate::boundary::ChunkPos { x, z }, 0))
        })));
    let grass = block(&state, "minecraft:grass_block", &[]);
    for x in -40..=56 {
        for z in -40..=56 {
            state.world.set_block(x, 200, z, grass);
            for y in 201..224 {
                if (x, y, z) != CHEST {
                    state.world.set_block(x, y, z, state.registry.states.air);
                }
            }
        }
    }
    state.world.fold_overlay();
    for n in 1..10 {
        builder_at(&mut state, id, [4.5 + n as f64 * 0.5, 201.0, 4.5]);
    }
    let (cobble, door) = (
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:cobblestone").unwrap())
            .unwrap(),
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:oak_door").unwrap())
            .unwrap(),
    );
    let slots = chest_slots(&mut state);
    for slot in slots.iter_mut().take(14) {
        *slot = ItemStack::new(cobble, 64);
    }
    slots[14] = ItemStack::new(door, 4);
    (state, id, builder)
}

/// The planner's own wall search skips what already stands; the fixture builds
/// the well and the four gatehouses first, as a real town does.
fn finish(state: &mut TickState, id: SettlementId, slot: EntitySlot) {
    let v = state
        .entities
        .row_mut(slot)
        .villager
        .as_mut()
        .expect("builder");
    v.data.profession = Profession::Builder;
    v.activity = Activity::Work;
    complete(state, id, slot);
}

fn wall_stretch(state: &mut TickState, id: SettlementId) -> (String, mc_script::ScriptWorldBounds) {
    // The well and the gatehouses come first, in the planner's own order; each
    // one is admitted by the same body the planner uses and finished on site.
    for _ in 0..12 {
        let (kind, bounds, cells) = {
            let record = state.settlements.get(id).expect("settlement");
            let Some((kind, work)) = layout::fortification(state, record)
                .find(|(_, work)| !executor::area_held(state, work.bounds))
            else {
                panic!("a walled town always owes its perimeter");
            };
            (kind.to_owned(), work.bounds, work.cells)
        };
        if kind == layout::PERIMETER {
            return (kind, bounds);
        }
        assert!(
            begin_cells(
                state,
                Project {
                    settlement: id,
                    blueprint: kind.clone(),
                    level: 1,
                    origin: bounds.min,
                    rotation: 0,
                    building: None,
                    structure: String::new(),
                    ordered: false,
                    builder: None,
                    order_seq: None,
                },
                bounds,
                cells,
            ),
            "the planner admits {kind}"
        );
        let builder = crate::settlements::construction::residents(state, id)
            .next()
            .expect("a resident");
        finish(state, id, builder);
    }
    panic!("the perimeter never comes up");
}

/// R2-3 (wall): with an active underfunded project holding the same cobblestone
/// the joint proof refuses a wall stretch that is fundable on its own; drop that
/// project and the same stock admits it.
#[test]
fn a_wall_is_refused_by_the_joint_proof_not_by_its_own_bill() {
    let (mut state, id, _builder) = walled_town();
    let (kind, bounds) = wall_stretch(&mut state, id);
    assert_eq!(
        kind,
        layout::PERIMETER,
        "the well and the gates already stand"
    );
    let cells = {
        let record = state.settlements.get(id).expect("settlement");
        layout::fortification(&state, record)
            .find(|(work_kind, work)| *work_kind == kind && work.bounds == bounds)
            .expect("the candidate again")
            .1
            .cells
    };
    // Stock exactly one wall stretch: the site is affordable on its own.
    let bill = cell_bill(&state, &cells).expect("a wall bill");
    clear_chest(&mut state);
    {
        let slots = chest_slots(&mut state);
        let mut index = 0;
        for (item, quantity) in &bill {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
    let images = executor::source_images(&state, id);
    let record = state.settlements.get(id).expect("settlement").clone();
    let alone = fortification_site(&state, &record, 10, &images)
        .expect("the wall stretch is fundable on its own bill");
    assert_eq!(alone.project.blueprint, kind);
    assert_eq!(alone.bounds, bounds);
    // A running house claims the same cobblestone: the joint proof refuses it.
    let house = begin_at(&mut state, id, [20, 200, 20], "house");
    let images = executor::source_images(&state, id);
    let refused = fortification_site(&state, &record, 10, &images)
        .map(|site| (site.project.blueprint, site.bounds));
    assert!(
        refused.as_ref() != Some(&(kind.clone(), bounds)),
        "a wall funded alone must still lose the joint proof while the house holds its stock: {refused:?}"
    );
    // The same stock, with no other active project: the wall is admitted.
    cancel_project(&mut state, id, &house).expect("cancel the rival project");
    let record = state.settlements.get(id).expect("settlement").clone();
    let admitted = fortification_site(&state, &record, 10, &images)
        .expect("the wall is fundable on its own bill");
    assert_eq!(admitted.project.blueprint, kind);
    assert_eq!(admitted.bounds, bounds);
}

/// R2-3 (lantern): the same joint proof through the street search. The walk to
/// the storage's door is blocked by a trunk, so the next street work is the
/// verge lamp post rather than paving.
#[test]
fn a_lantern_is_refused_by_the_joint_proof_not_by_its_own_bill() {
    let (mut state, id, worker) = fixture();
    open_plan(&mut state);
    fund(&mut state, "storage", 1);
    request_started(&mut state, id, "storage");
    complete(&mut state, id, worker);
    state.entities.set_pos(worker, [1.5, 201.0, 2.5]);
    state
        .world
        .set_block(3, 200, 8, block(&state, "minecraft:dirt", &[]));
    for y in 201..205 {
        state
            .world
            .set_block(3, y, 8, block(&state, "minecraft:birch_log", &[]));
    }
    state
        .world
        .set_block(3, 205, 8, block(&state, "minecraft:birch_leaves", &[]));
    state.world.fold_overlay();
    // A lamp post is two fences and a torch.
    clear_chest(&mut state);
    for (index, (name, count)) in [("birch_fence", 8), ("torch", 4)].into_iter().enumerate() {
        let item = state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
            .expect("known item");
        chest_slots(&mut state)[index] = ItemStack::new(item, count);
    }
    let images = executor::source_images(&state, id);
    let record = state.settlements.get(id).expect("settlement").clone();
    let Some(site) = street_site(&state, &record, &images) else {
        panic!("the built storage owes a lamp post");
    };
    assert_eq!(
        site.project.blueprint,
        layout::LAMP,
        "the lamp is reachable"
    );
    let bill = cell_bill(&state, &site.cells).expect("a lamp bill");
    // Stock exactly that bill.
    clear_chest(&mut state);
    {
        let slots = chest_slots(&mut state);
        let mut index = 0;
        for (item, quantity) in &bill {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
    // Alone the lamp is affordable: the site search still offers it.
    let images = executor::source_images(&state, id);
    let record = state.settlements.get(id).expect("settlement").clone();
    assert!(
        street_site(&state, &record, &images)
            .is_some_and(|site| site.project.blueprint == layout::LAMP),
        "the lamp is fundable on its own bill"
    );
    // A real rival project claims the same fence and torch stock: a house the
    // planner admits on a legal lot next to the storage.
    clear_chest(&mut state);
    stock(&mut state, "house", 1, 1);
    // The rival house starts.
    request_started(&mut state, id, "house");
    let house = handle_of(&state, id, 0);
    // Only the lamp's own bill is left, in the same wood species the vanishing
    // stock allowed, so the joint proof is the only reason left to refuse it.
    clear_chest(&mut state);
    {
        let slots = chest_slots(&mut state);
        let mut index = 0;
        for (item, quantity) in &bill {
            let mut quantity = *quantity;
            while quantity > 0 {
                let count = quantity.min(64);
                slots[index] = ItemStack::new(*item, count as i32);
                quantity -= count;
                index += 1;
            }
        }
    }
    let images = executor::source_images(&state, id);
    let record = state.settlements.get(id).expect("settlement").clone();
    assert!(
        street_site(&state, &record, &images)
            .is_none_or(|site| site.project.blueprint != layout::LAMP),
        "a lamp funded alone must still lose the joint proof while the house holds its stock"
    );
    // Drop the rival project: the same stock admits the lamp again.
    cancel_project(&mut state, id, &house).expect("cancel the rival project");
    let record = state.settlements.get(id).expect("settlement").clone();
    assert!(
        street_site(&state, &record, &images)
            .is_some_and(|site| site.project.blueprint == layout::LAMP),
        "with the joint bill gone the lamp is admitted"
    );
}

/// R2-7/A1: two identical houses admitted in parallel each carry the sequence of
/// the request they were lifted from, and a retry restores that place.
#[test]
fn two_identical_houses_take_their_own_retry_metadata() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    clear_chest(&mut state);
    // Both requests wait for materials first, in submission order.
    assert!(matches!(
        request_build(&mut state, id, "house").expect("first request"),
        OrderOutcome::Queued { .. }
    ));
    assert!(matches!(
        request_build(&mut state, id, "house").expect("second request"),
        OrderOutcome::Queued { .. }
    ));
    let seqs: Vec<u64> = orders(state.settlements.get(id).expect("settlement"))
        .map(|order| order.seq)
        .collect();
    assert_eq!(seqs, [1, 2], "each submission takes its own sequence");
    stock(&mut state, "house", 1, 2);
    // Both funded requests are admitted in queue order.
    plan_until(&mut state, id, 1);
    plan_until(&mut state, id, 2);
    let projects: Vec<(String, Option<u64>)> = access::projects_of(&state.settlements, id)
        .map(|project| (project.structure.clone(), project.order_seq))
        .collect();
    assert_eq!(projects.len(), 2, "two identical houses build at once");
    assert_eq!(
        projects.iter().map(|(_, seq)| *seq).collect::<Vec<_>>(),
        [Some(1), Some(2)],
        "each created handle carries its own request's sequence"
    );
    // The second house's stall retry restores its own place, not the tail.
    let second = projects[1].0.clone();
    let project = access::projects_of(&state.settlements, id)
        .find(|project| project.structure == second)
        .cloned()
        .expect("second project");
    retry_preparation(&mut state, id, &project).expect("retry the second house");
    let queued: Vec<(String, u64)> = orders(state.settlements.get(id).expect("settlement"))
        .map(|order| (order.blueprint.clone(), order.seq))
        .collect();
    assert_eq!(queued, [("house".to_owned(), 2)], "the retry keeps its seq");
    assert!(
        access::projects_of(&state.settlements, id)
            .any(|project| project.structure == projects[0].0),
        "the other identical house is untouched"
    );
}

/// R2-7/A2: a new request never overtakes a waiting head, even once the head is
/// funded; the head starts first and keeps the queue behind it.
#[test]
fn a_new_order_never_overtakes_a_waiting_funded_head() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    clear_chest(&mut state);
    assert!(matches!(
        request_build(&mut state, id, "house").expect("head request"),
        OrderOutcome::Queued { .. }
    ));
    let queued = request_build(&mut state, id, "storage").expect("later request");
    let OrderOutcome::Queued { reason } = &queued else {
        panic!("the later request waits behind the head: {queued:?}");
    };
    assert!(
        reason.contains("not enough"),
        "the wait names materials, not the head's turn: {reason}"
    );
    assert_eq!(
        orders(state.settlements.get(id).expect("settlement"))
            .map(|order| (order.blueprint.as_str(), order.seq))
            .collect::<Vec<_>>(),
        [("house", 1), ("storage", 2)],
        "submission order holds"
    );
    // Fund only the head.
    stock(&mut state, "house", 1, 1);
    plan_until(&mut state, id, 1);
    let started = access::projects_of(&state.settlements, id)
        .next()
        .cloned()
        .expect("the funded head starts");
    assert_eq!(
        started.blueprint, "house",
        "the head, not the later request"
    );
    assert_eq!(started.order_seq, Some(1));
    assert_eq!(
        orders(state.settlements.get(id).expect("settlement"))
            .map(|order| order.blueprint.as_str())
            .collect::<Vec<_>>(),
        ["storage"],
        "the later request is still waiting, not promoted"
    );
}

/// R2-7/SOL-1: two bound builders through the real admission tick; each project
/// advances with its own binding, so both handles move.
#[test]
fn two_bound_builders_advance_both_handles() {
    let (mut state, id, first_builder) = fixture();
    open_plan(&mut state);
    stock(&mut state, "house", 1, 2);
    let first = begin_at(&mut state, id, [3, 200, 3], "house");
    let second = begin_at(&mut state, id, [3, 200, 12], "house");
    let second_builder = builder_at(&mut state, id, [12.5, 201.0, 14.5]);
    bind_project(&mut state, id, &first, first_builder);
    bind_project(&mut state, id, &second, second_builder);
    let before = [
        work_done(&state, id, &first),
        work_done(&state, id, &second),
    ];
    drive(&mut state, &[(id, first_builder), (id, second_builder)], 40);
    let after = [
        work_done(&state, id, &first),
        work_done(&state, id, &second),
    ];
    assert!(
        after[0] > before[0],
        "the first builder's project advanced: {} -> {}",
        before[0],
        after[0]
    );
    assert!(
        after[1] > before[1],
        "the second builder's project advanced: {} -> {}",
        before[1],
        after[1]
    );
}

/// SOL-2b (1): another project's oak obligation is never re-coloured to the
/// candidate's birch, so an unfunded oak joint refuses the candidate instead of
/// admitting it on substituted birch.
#[test]
fn a_missing_oak_obligation_is_not_replaced_by_birch() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    // The storage project is admitted in oak and keeps that obligation.
    stock_species(&mut state, "storage", 1, 1, "oak");
    // The first project starts in oak.
    request_started(&mut state, id, "storage");
    let head = handle_of(&state, id, 0);
    let oak_obligation: Vec<String> = project_bill(&state, id, &head)
        .expect("a running bill")
        .into_iter()
        .filter_map(|(item, _)| {
            state
                .registry
                .items
                .name_of(item)
                .map(|n| n.as_str().to_owned())
        })
        .filter(|name| {
            wood_species_name(&state, &mc_data::Identifier::parse(name).unwrap()).is_some()
        })
        .collect();
    assert!(
        oak_obligation
            .iter()
            .all(|name| name.starts_with("minecraft:oak_")),
        "the running project owes oak: {oak_obligation:?}"
    );
    // Only birch is left: a second house may not pay the oak obligation with it.
    clear_chest(&mut state);
    stock_species(&mut state, "house", 1, 1, "birch");
    let queued = request_build(&mut state, id, "house").expect("the request is answered");
    assert!(
        matches!(&queued, OrderOutcome::Queued { .. }),
        "the oak joint is unfunded, so the house waits: {queued:?}"
    );
    assert_eq!(
        access::projects_of(&state.settlements, id).count(),
        1,
        "nothing was admitted on substituted birch"
    );
    let stock_after: Vec<(u32, i32)> = chest_slots(&mut state)
        .iter()
        .filter(|stack| !stack.is_empty())
        .map(|stack| (stack.item_id, stack.count))
        .collect();
    assert!(
        stock_after.iter().any(|(item, _)| state
            .registry
            .items
            .name_of(*item)
            .is_some_and(|n| n.as_str().contains("birch_"))),
        "the birch stock is untouched by the refused admission"
    );
}

/// SOL-2b (2): a project holding oak does not stop the next candidate from
/// using the birch it does have; each species is proved against the exact joint.
#[test]
fn a_kept_oak_obligation_does_not_hide_the_candidates_birch() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    // One full storage bill in oak (the joint) plus one house bill in birch.
    stock_species(&mut state, "storage", 1, 1, "oak");
    // The oak project starts.
    request_started(&mut state, id, "storage");
    // The candidate's own bill has headroom; the oak side is exactly the joint,
    // so an oak-variant admission cannot fit.
    add_species_stock(&mut state, "house", 1, 2, "birch");
    let head = handle_of(&state, id, 0);
    // The house is admitted on birch beside the oak project.
    request_started(&mut state, id, "house");
    assert_eq!(
        access::projects_of(&state.settlements, id).count(),
        2,
        "both projects build at once"
    );
    let house = handle_of(&state, id, 1);
    let birch: Vec<String> = project_bill(&state, id, &house)
        .expect("the house bill")
        .into_iter()
        .filter_map(|(item, _)| {
            state
                .registry
                .items
                .name_of(item)
                .map(|n| n.as_str().to_owned())
        })
        .filter(|name| name.starts_with("minecraft:birch_"))
        .collect();
    assert!(
        !birch.is_empty(),
        "the admitted house is the birch variant: {:?}",
        project_bill(&state, id, &house)
    );
    let oak_still: Vec<String> = project_bill(&state, id, &head)
        .expect("the storage bill")
        .into_iter()
        .filter_map(|(item, _)| {
            state
                .registry
                .items
                .name_of(item)
                .map(|n| n.as_str().to_owned())
        })
        .filter(|name| name.starts_with("minecraft:oak_"))
        .collect();
    assert!(
        !oak_still.is_empty(),
        "the oak project keeps its own obligation"
    );
}

/// A4: a stall-watch retry the full queue refuses reaches the owner, again on
/// the next recovery; the paid project and the queue are left untouched.
#[test]
fn a_full_queue_retry_refusal_reaches_the_owner_repeatedly() {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some(OWNER.to_string());
    let entity = state.entities.body(builder).id;
    state.connections.insert(
        crate::boundary::ConnectionId(1),
        entity,
        OWNER,
        "owner".into(),
    );
    // One funded order starts; the rest of the queue fills behind it.
    stock(&mut state, "house", 1, 1);
    request_started(&mut state, id, "house");
    let handle = handle_of(&state, id, 0);
    bind_project(&mut state, id, &handle, builder);
    clear_chest(&mut state);
    for _ in 0..MAX_ORDERS {
        let _ = request_build(&mut state, id, "storage");
    }
    assert_eq!(
        orders(state.settlements.get(id).expect("settlement")).count(),
        MAX_ORDERS,
        "the queue is full"
    );
    let queued_before: Vec<u64> = orders(state.settlements.get(id).expect("settlement"))
        .map(|order| order.seq)
        .collect();
    // The builder stands away from the site: no cell ever advances, so every
    // stall episode retries the paid preparation.
    state.entities.set_pos(builder, [30.5, 201.0, 30.5]);
    let mut episodes = 0;
    let mut notifications = Vec::new();
    for call in 0..120u64 {
        state.tick = 100 + call * 100;
        state.day_time = SHIFT_DAY_TIME;
        super::tick(&mut state);
        let said: Vec<String> = state
            .outbox
            .get(&crate::boundary::ConnectionId(1))
            .into_iter()
            .flatten()
            .filter_map(|change| match change {
                crate::boundary::PlayerChange::Chat { message } => Some(message.clone()),
                _ => None,
            })
            .filter(|message| message.contains("could not be retried"))
            .collect();
        if said.len() > notifications.len() {
            episodes += 1;
            notifications = said;
        }
    }
    assert!(
        notifications.len() >= 2,
        "the refused retry reaches the owner on repeated recovery: {notifications:?}"
    );
    assert!(
        notifications
            .iter()
            .all(|message| message.contains("queue is full")),
        "the message names the full queue: {notifications:?}"
    );
    assert!(episodes >= 2, "more than one recovery episode: {episodes}");
    assert!(
        access::projects_of(&state.settlements, id).any(|project| project.structure == handle),
        "the paid project was never dropped by a refused retry"
    );
    assert_eq!(
        orders(state.settlements.get(id).expect("settlement"))
            .map(|order| order.seq)
            .collect::<Vec<_>>(),
        queued_before,
        "the queue is unchanged by a refused retry"
    );
}

/// R2-proof (order): Started A → queued B/C → native save/reopen → retry A
/// restores A's own submission place, ahead of B and C.
#[test]
fn a_started_order_keeps_its_place_across_a_native_reopen() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    stock(&mut state, "house", 1, 1);
    // The funded head starts.
    request_started(&mut state, id, "house");
    let handle = handle_of(&state, id, 0);
    let project = access::projects_of(&state.settlements, id)
        .next()
        .cloned()
        .expect("started project");
    assert_eq!(
        project.order_seq,
        Some(1),
        "the started order keeps its seq"
    );
    clear_chest(&mut state);
    for blueprint in ["storage", "farm"] {
        assert!(
            matches!(
                request_build(&mut state, id, blueprint).expect("unfunded request"),
                OrderOutcome::Queued { .. }
            ),
            "{blueprint} waits for materials"
        );
    }
    // Native save: the per-record checkpoint row plus the plugin custody image
    // the same checkpoint carries (the structure ledger lives there).
    let rows = checkpoint_rows(&state, 4).expect("checkpoint rows");
    let custody = serde_json::to_vec(&state.plugins).expect("custody image");
    let sequence = state.settlement_sequence;
    let mut reopened = test_state();
    reopened.plugins = serde_json::from_slice(&custody).expect("custody reopens");
    hydrate_dimension(&mut reopened, &rows, &Replay::default(), sequence)
        .expect("the record reopens");
    let restored = reopened.settlements.get(id).expect("restored record");
    assert_eq!(
        orders(restored)
            .map(|order| (order.seq, order.blueprint.as_str()))
            .collect::<Vec<_>>(),
        [(2, "storage"), (3, "farm")],
        "the queue reopens untouched"
    );
    let restored_project = access::projects_of(&reopened.settlements, id)
        .next()
        .cloned()
        .expect("the started project reopens");
    assert_eq!(restored_project.structure, handle);
    assert_eq!(restored_project.order_seq, Some(1));
    retry_preparation(&mut reopened, id, &restored_project).expect("retry the reopened project");
    assert_eq!(
        orders(reopened.settlements.get(id).expect("restored record"))
            .map(|order| (order.seq, order.blueprint.as_str()))
            .collect::<Vec<_>>(),
        [(1, "house"), (2, "storage"), (3, "farm")],
        "the retried order keeps its own submission place"
    );
}

/// R2-proof (full queue): the refusal happens before anything is returned or
/// cancelled, so the project and the queue are byte-identical.
#[test]
fn a_full_queue_refuses_the_retry_before_anything_changes() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    stock(&mut state, "house", 1, 1);
    request_started(&mut state, id, "house");
    let handle = handle_of(&state, id, 0);
    clear_chest(&mut state);
    for _ in 0..MAX_ORDERS {
        let _ = request_build(&mut state, id, "storage");
    }
    let project = access::projects_of(&state.settlements, id)
        .next()
        .cloned()
        .expect("running project");
    let before = state.settlements.get(id).expect("settlement").clone();
    let refused = retry_preparation(&mut state, id, &project);
    assert!(
        refused
            .as_ref()
            .is_err_and(|error| error.contains("queue is full")),
        "a full queue refuses the retry: {refused:?}"
    );
    assert_eq!(
        state.settlements.get(id).expect("settlement"),
        &before,
        "a refused retry leaves the project, the queue and the revision untouched"
    );
    assert!(access::projects_of(&state.settlements, id).any(|p| p.structure == handle));
}

/// Checkpoint-row fences (R2-8): every new required key refuses on its own.
#[test]
fn each_new_required_key_is_fenced_in_a_checkpoint_row() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    stock(&mut state, "house", 1, 1);
    request_started(&mut state, id, "house");
    let handle = handle_of(&state, id, 0);
    let builder = fixture_builder(&state, id);
    bind_project(&mut state, id, &handle, builder);
    clear_chest(&mut state);
    request_build(&mut state, id, "storage").expect("queued request");
    let rows = checkpoint_rows(&state, 4).expect("checkpoint rows");
    let raw: serde_json::Value =
        serde_json::from_slice(&rows[&row_key(id)].payload).expect("row json");
    // The keys exist before they are removed: the fence tests the absence.
    assert!(raw["value"]["order_seq"].is_u64());
    assert!(raw["value"]["orders"][0]["seq"].is_u64());
    assert!(raw["value"]["projects"][0]["order_seq"].is_u64());
    assert!(!raw["value"]["projects"][0]["builder"].is_null());
    for path in [
        ["value", "order_seq"].as_slice(),
        ["value", "orders", "0", "seq"].as_slice(),
        ["value", "projects", "0", "order_seq"].as_slice(),
        ["value", "projects", "0", "builder"].as_slice(),
    ] {
        let mut damaged = raw.clone();
        remove_key(&mut damaged, path);
        let mut damaged_rows = rows.clone();
        damaged_rows.get_mut(&row_key(id)).expect("row").payload =
            serde_json::to_vec(&damaged).expect("damaged row octets");
        let error = hydrate_dimension(&mut test_state(), &damaged_rows, &Replay::default(), 1)
            .expect_err(&format!("a row without {} must refuse", path.join(".")));
        let text = error.to_string();
        assert!(
            text.contains(&format!("row settlement/{}", id.0)),
            "the refusal names the record for {}: {text}",
            path.join(".")
        );
    }
}

/// WAL-tail fences (R2-8): every new required key refuses on its own, with batch
/// and record attribution, through the real journal reader.
#[test]
fn each_new_required_key_is_fenced_in_a_wal_tail() {
    let (mut state, id, _slot) = fixture();
    open_plan(&mut state);
    state.settlements.get_mut(id).expect("settlement").owner = Some("owner".into());
    stock(&mut state, "house", 1, 1);
    request_started(&mut state, id, "house");
    let handle = handle_of(&state, id, 0);
    let builder = fixture_builder(&state, id);
    bind_project(&mut state, id, &handle, builder);
    clear_chest(&mut state);
    request_build(&mut state, id, "storage").expect("queued request");
    let mut entries = Vec::new();
    journal_entries(&mut state, 1, &mut entries);
    let index = entries
        .iter()
        .position(|entry| matches!(entry, JournalEntry::Settlement { image, .. } if image.id == id))
        .expect("the touched settlement image");
    let image: serde_json::Value = serde_json::to_value(&entries[index]).expect("entry json");
    for path in [
        ["Settlement", "image", "value", "order_seq"].as_slice(),
        ["Settlement", "image", "value", "orders", "0", "seq"].as_slice(),
        ["Settlement", "image", "value", "projects", "0", "order_seq"].as_slice(),
        ["Settlement", "image", "value", "projects", "0", "builder"].as_slice(),
    ] {
        let mut damaged = image.clone();
        remove_key(&mut damaged, path);
        let error = wal_refusal(&entries, index, &damaged, id);
        let text = error.to_string();
        let attributed =
            text.contains("kind Settlement") || text.contains(&format!("WAL image id {}", id.0));
        assert!(
            attributed,
            "the refusal names the batch record or WAL image for {}: {text}",
            path.join(".")
        );
    }
}

fn fixture_builder(state: &TickState, id: SettlementId) -> EntitySlot {
    crate::settlements::construction::residents(state, id)
        .find(|slot| {
            state
                .entities
                .row(*slot)
                .villager
                .as_ref()
                .is_some_and(|v| {
                    v.settlement == Some(id) && v.data.profession == Profession::Builder
                })
        })
        .expect("the fixture builder")
}

/// Drop the key at a JSON path; a numeric step indexes an array.
fn remove_key(value: &mut serde_json::Value, path: &[&str]) {
    let (last, parents) = path.split_last().expect("non-empty path");
    let mut cursor = value;
    for step in parents {
        cursor = match step.parse::<usize>() {
            Ok(index) if cursor.is_array() => &mut cursor[index],
            _ => cursor
                .get_mut(*step)
                .unwrap_or_else(|| panic!("json key {step}")),
        };
    }
    let object = cursor
        .as_object_mut()
        .unwrap_or_else(|| panic!("the object holding {last}"));
    assert!(
        object.remove(*last).is_some(),
        "json key {last} is present before the fence"
    );
}

/// Write one damaged settlement image as a real published batch and read it back
/// the way recovery does. Returns the refusal: either the batch reader rejects
/// the record or hydration rejects the WAL image.
fn wal_refusal(
    entries: &[JournalEntry],
    index: usize,
    damaged: &serde_json::Value,
    id: SettlementId,
) -> crate::config::CoreError {
    let mut payload = String::new();
    for (position, entry) in entries.iter().enumerate() {
        if position == index {
            payload.push_str(&serde_json::to_string(damaged).expect("damaged entry"));
        } else {
            payload.push_str(&serde_json::to_string(entry).expect("entry"));
        }
        payload.push('\n');
    }
    let frame = crate::journal::frame_batch(
        1,
        1,
        payload.as_bytes(),
        crate::journal::WalIdentity::fixture(),
    )
    .expect("framed batch");
    let dir = std::env::temp_dir().join(format!(
        "krasnokamen-d8-wal-{}-{}",
        std::process::id(),
        index
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("batch dir");
    let name = format!("batch-{:020}-{:020}.bin", 1, 1);
    std::fs::write(dir.join(&name), &frame).expect("batch file");
    let read = crate::journal::read_complete_batches_after(&dir, 0, crate::journal::WalIdentity::fixture());
    let _ = std::fs::remove_dir_all(&dir);
    match read {
        Err(error) => error,
        Ok(batches) => {
            let replay = crate::journal::apply_batch_images(Replay::default(), &batches);
            hydrate_dimension(&mut test_state(), &Default::default(), &replay, 1).expect_err(
                &format!("a WAL image without the key refuses (id {})", id.0),
            )
        }
    }
}
