//! The twentieth-tick item merge against the per-probe binary search it
//! replaced: identical stacks, ages, delays, removals and publication order on
//! seeded dense item sets.

use std::sync::Arc;

use mc_data::{Identifier, ItemStack};

use super::{item_cell, merge_amount, merge_boxes_overlap, merge_neighbours};
use crate::entities::{EntitySlot, StackRow};
use crate::registry::CoreRegistry;
use crate::tick::TickState;
use crate::world::ChunkWorld;

/// Merge before the cursor runs, verbatim: two binary searches over the
/// sorted items per neighbour cell, each probe re-reading a position.
fn reference_merge(state: &mut TickState) {
    if !state.tick.is_multiple_of(20) {
        return;
    }
    let mut slots: Vec<EntitySlot> = state
        .entities
        .item_slots()
        .filter(|slot| {
            state.entities.body(*slot).alive
                && state
                    .entities
                    .row(*slot)
                    .item
                    .as_ref()
                    .is_some_and(|item| item.count > 0)
        })
        .collect();
    slots.sort_unstable_by_key(|slot| item_cell(state, *slot));
    for receiver in slots.iter().copied() {
        if !state.entities.body(receiver).alive {
            continue;
        }
        let position = state.entities.pos(receiver);
        let receiver_id = state.entities.body(receiver).id;
        let cell = item_cell(state, receiver);
        let mut receiver_changed = false;
        for y in cell.1 - 1..=cell.1 + 1 {
            for z in cell.2 - 1..=cell.2 + 1 {
                for x in cell.0 - 1..=cell.0 + 1 {
                    let candidate_cell = (x, y, z);
                    let start =
                        slots.partition_point(|slot| item_cell(state, *slot) < candidate_cell);
                    let end =
                        slots.partition_point(|slot| item_cell(state, *slot) <= candidate_cell);
                    for source in slots[start..end].iter().copied() {
                        if source.0 <= receiver.0 || !state.entities.body(source).alive {
                            continue;
                        }
                        if !merge_boxes_overlap(
                            position,
                            state.entities.pos(source),
                            state.entities.body(receiver).half_width,
                            state.entities.body(receiver).height,
                            state.entities.body(source).half_width,
                            state.entities.body(source).height,
                        ) {
                            continue;
                        }
                        let Some((moved, pickup_delay, age, source_id)) =
                            merge_amount(state, receiver, source)
                        else {
                            continue;
                        };
                        {
                            let row = state.entities.row_mut(receiver);
                            let item = row.item.as_mut().unwrap();
                            item.count += moved;
                            item.stack.as_mut().unwrap().count += moved;
                            row.age = age;
                        }
                        let source_empty = {
                            let item = state.entities.row_mut(source).item.as_mut().unwrap();
                            item.count -= moved;
                            item.stack.as_mut().unwrap().count = item.count;
                            if item.count == 0 {
                                item.stack = Some(ItemStack::EMPTY);
                            }
                            item.count == 0
                        };
                        state.entities.body_mut(receiver).pickup_delay = pickup_delay;
                        receiver_changed = true;
                        if source_empty {
                            state.entities.body_mut(source).alive = false;
                            state.merge_consumed_items.insert(source);
                        } else {
                            let row = state.entities.row_mut(source);
                            Arc::make_mut(&mut row.appearance).item =
                                row.item.as_ref().map(|item| item.full_stack());
                            state.appearance_changed.push(source_id);
                        }
                    }
                }
            }
        }
        if receiver_changed {
            let row = state.entities.row_mut(receiver);
            Arc::make_mut(&mut row.appearance).item =
                row.item.as_ref().map(|item| item.full_stack());
            state.appearance_changed.push(receiver_id);
        }
    }
}

/// splitmix64: a fixed stream per seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn stack(registry: &CoreRegistry, name: &str, count: i32) -> ItemStack {
    let id = registry
        .items
        .id_of(&Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap();
    ItemStack::new(id, count)
}

/// `items` item entities in a 12x3x12 block box straddling x = z = 0, so most
/// have partners in their own and neighbouring cells: four kinds (one with
/// components, one stacking to 16), partial counts, delays, ages and a few
/// dead rows.
fn seeded(registry: &Arc<CoreRegistry>, seed: u64, items: usize) -> TickState {
    let terrain = mc_worldgen::TerrainGenerator::new(42, Arc::clone(&registry.blocks));
    let generator: Arc<dyn mc_world::ChunkGenerator> = Arc::new(terrain);
    let world = ChunkWorld::new(
        42,
        generator,
        registry,
        mc_world::BlockStateId(registry.states.air),
        [0, 200, 0],
        mc_world::OVERWORLD_GEOMETRY,
    );
    let mut state = TickState::new(Arc::clone(registry), world, 2, 512);
    let kinds = [
        stack(registry, "dirt", 1),
        stack(registry, "stone", 1),
        stack(registry, "egg", 1),
        stack(registry, "stone", 1).with_custom_name("named"),
    ];
    let mut rng = Rng(seed);
    for _ in 0..items {
        let kind = &kinds[rng.below(kinds.len() as u64) as usize];
        let max = if kind.item_id == kinds[2].item_id {
            16
        } else {
            64
        };
        let mut item = kind.clone();
        item.count = 1 + rng.below(max) as i32;
        let pos = [
            rng.unit() * 12.0 - 6.0,
            200.0 + rng.below(3) as f64 + 0.1 * rng.below(3) as f64,
            rng.unit() * 12.0 - 6.0,
        ];
        let id = crate::tick::spawn_item_entity(&mut state, pos, item);
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.body_mut(slot).pickup_delay = rng.below(40) as u32;
        state.entities.row_mut(slot).age = rng.below(6000) as u32;
        if rng.below(25) == 0 {
            state.entities.body_mut(slot).alive = false;
        }
    }
    state.appearance_changed.clear();
    state
}

type ItemOutcome = (
    EntitySlot,
    bool,
    Option<StackRow>,
    u32,
    u32,
    Option<ItemStack>,
);

fn outcome(state: &TickState) -> (Vec<ItemOutcome>, Vec<EntitySlot>, Vec<crate::boundary::EntityId>) {
    let items = state
        .entities
        .item_slots()
        .map(|slot| {
            let row = state.entities.row(slot);
            (
                slot,
                state.entities.body(slot).alive,
                row.item.as_deref().cloned(),
                row.age,
                state.entities.body(slot).pickup_delay,
                row.appearance.item.clone(),
            )
        })
        .collect();
    let mut consumed: Vec<_> = state.merge_consumed_items.iter().copied().collect();
    consumed.sort_unstable();
    (items, consumed, state.appearance_changed.clone())
}

#[test]
fn cursor_merge_matches_binary_search_merge_on_seeded_item_sets() {
    let registry = Arc::new(CoreRegistry::load(None).expect("registry loads"));
    for seed in 1..=6 {
        let mut reference = seeded(&registry, seed, 1500);
        let mut cursor = seeded(&registry, seed, 1500);
        assert_eq!(outcome(&reference), outcome(&cursor), "seed {seed} setup");
        // Two cadences: the second meets dead receivers and full stacks.
        for tick in [20, 40] {
            reference.tick = tick;
            cursor.tick = tick;
            let live = cursor
                .entities
                .item_slots()
                .filter(|slot| cursor.entities.body(*slot).alive)
                .count();
            reference_merge(&mut reference);
            assert_eq!(merge_neighbours(&mut cursor), live);
            assert_eq!(
                outcome(&cursor),
                outcome(&reference),
                "seed {seed} tick {tick}"
            );
        }
        let (_, consumed, published) = outcome(&cursor);
        assert!(
            consumed.len() > 200 && published.len() > 200,
            "seed {seed}: the set must exercise full and partial merges"
        );
    }
}
