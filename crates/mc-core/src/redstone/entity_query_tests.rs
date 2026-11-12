//! The local chunk-index census must count exactly what the former per-pass
//! AABB tree over every live body counted. The tree is kept here as the oracle.
use super::*;
use crate::entities::{EntityKind, EntitySlot};
use crate::redstone::entity_query::EntityQuery;

#[derive(Clone, Copy)]
struct Body {
    slot: EntitySlot,
    min: [f64; 3],
    max: [f64; 3],
    subtree_min: [f64; 3],
    subtree_max: [f64; 3],
}

/// The former `EntityQuery`, verbatim apart from the test counter.
#[derive(Default)]
struct ReferenceTree {
    bodies: Vec<Body>,
}

impl ReferenceTree {
    fn rebuild(&mut self, state: &TickState) {
        self.bodies.clear();
        for slot in state.entities.slots() {
            let body = state.entities.body(slot);
            if !body.alive
                || state.entity_session_locked(body.id)
                || state
                    .players
                    .get(&body.id)
                    .is_some_and(|player| player.spectator())
            {
                continue;
            }
            let pos = state.entities.pos(slot);
            let min = [pos[0] - body.half_width, pos[1], pos[2] - body.half_width];
            let max = [
                pos[0] + body.half_width,
                pos[1] + body.height,
                pos[2] + body.half_width,
            ];
            if (0..3).any(|axis| {
                !min[axis].is_finite() || !max[axis].is_finite() || min[axis] > max[axis]
            }) {
                continue;
            }
            self.bodies.push(Body {
                slot,
                min,
                max,
                subtree_min: min,
                subtree_max: max,
            });
        }
        Self::partition(&mut self.bodies, 0);
    }

    fn partition(bodies: &mut [Body], axis: usize) -> Option<([f64; 3], [f64; 3])> {
        if bodies.is_empty() {
            return None;
        }
        let middle = bodies.len() / 2;
        let (left, root, right) = bodies.select_nth_unstable_by(middle, |a, b| {
            a.min[axis]
                .total_cmp(&b.min[axis])
                .then_with(|| a.slot.cmp(&b.slot))
        });
        let mut min = root.min;
        let mut max = root.max;
        for bounds in [
            Self::partition(left, (axis + 1) % 3),
            Self::partition(right, (axis + 1) % 3),
        ]
        .into_iter()
        .flatten()
        {
            for axis in 0..3 {
                min[axis] = min[axis].min(bounds.0[axis]);
                max[axis] = max[axis].max(bounds.1[axis]);
            }
        }
        root.subtree_min = min;
        root.subtree_max = max;
        Some((min, max))
    }

    fn count(
        &self,
        min: [f64; 3],
        max: [f64; 3],
        limit: usize,
        accepts: &impl Fn(EntitySlot) -> bool,
    ) -> usize {
        Self::count_in(&self.bodies, min, max, limit, accepts)
    }

    fn count_in(
        bodies: &[Body],
        min: [f64; 3],
        max: [f64; 3],
        limit: usize,
        accepts: &impl Fn(EntitySlot) -> bool,
    ) -> usize {
        if bodies.is_empty() || limit == 0 {
            return 0;
        }
        let middle = bodies.len() / 2;
        let root = &bodies[middle];
        let overlaps = |lo: [f64; 3], hi: [f64; 3]| {
            (0..3).all(|axis| hi[axis] > min[axis] && lo[axis] < max[axis])
        };
        if !overlaps(root.subtree_min, root.subtree_max) {
            return 0;
        }
        let mut count = usize::from(overlaps(root.min, root.max) && accepts(root.slot));
        count += Self::count_in(&bodies[..middle], min, max, limit - count, accepts);
        count += Self::count_in(&bodies[middle + 1..], min, max, limit - count, accepts);
        count
    }
}

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn type_id(s: &TickState, name: &str) -> u32 {
    s.registry
        .entity_types
        .id_of(&mc_data::Identifier::parse(format!("minecraft:{name}")).unwrap())
        .unwrap()
}

fn populate(s: &mut TickState, rng: &mut Rng, count: usize) -> Vec<EntitySlot> {
    let kinds = [
        "pig",
        "arrow",
        "spectral_arrow",
        "minecart",
        "chest_minecart",
        "villager",
        "zombie",
    ];
    let mut slots = Vec::new();
    for index in 0..count {
        let pos = [
            rng.range(-40.0, 40.0),
            rng.range(198.0, 202.0),
            rng.range(-40.0, 40.0),
        ];
        let slot = if index % 5 == 0 {
            let id = crate::tick::spawn_item_entity(s, pos, mc_data::ItemStack::new(1, 1));
            s.entities.slot_of(id).unwrap()
        } else {
            let t = type_id(s, kinds[rng.below(kinds.len())]);
            let id = s.spawn_mob(t, pos, 20.0);
            s.entities.slot_of(id).unwrap()
        };
        slots.push(slot);
    }
    slots
}

/// Players (some spectating or session-locked), giant and malformed bodies, dead rows.
fn decorate(s: &mut TickState, rng: &mut Rng, slots: &[EntitySlot]) {
    s.require_plugin_login = true;
    for (index, &slot) in slots.iter().enumerate() {
        match index % 23 {
            0..=2 => {
                let connection = crate::boundary::ConnectionId(index as _);
                s.entities.set_kind(slot, EntityKind::Player { connection });
                let mode = if index % 23 == 1 {
                    crate::commands::GameMode::Spectator
                } else {
                    crate::commands::GameMode::Survival
                };
                let id = s.entities.body(slot).id;
                s.players
                    .insert(id, crate::player::PlayerState::new(connection, mode));
                if index % 23 == 2 && index % 2 == 0 {
                    s.locked_sessions.insert(connection);
                }
            }
            3 => s.entities.body_mut(slot).alive = false,
            4 => {
                s.entities.set_half_width(slot, rng.range(1.0, 4.0));
                s.entities.body_mut(slot).height = rng.range(1.0, 5.0);
            }
            5 => s.entities.body_mut(slot).height = -1.0,
            6 => {
                let mut pos = s.entities.pos(slot);
                pos[1] = f64::NAN;
                s.entities.set_pos(slot, pos);
            }
            _ => {}
        }
    }
}

/// Chunk-crossing moves, and a kind change that leaves a stale membership
/// which a later move can turn into a duplicate listing in one chunk.
fn churn(s: &mut TickState, rng: &mut Rng, slots: &[EntitySlot]) {
    let pig = type_id(s, "pig");
    for &slot in slots {
        match rng.below(4) {
            0 => {
                let mut pos = s.entities.pos(slot);
                if pos.iter().all(|v| v.is_finite()) {
                    pos[0] += rng.range(-30.0, 30.0);
                    pos[2] += rng.range(-30.0, 30.0);
                    s.entities.set_pos(slot, pos);
                }
            }
            1 if matches!(s.entities.body(slot).kind, EntityKind::Mob { .. }) => {
                let origin = s.entities.pos(slot);
                s.entities.set_kind(slot, EntityKind::Mob { type_id: pig });
                if origin.iter().all(|v| v.is_finite()) {
                    s.entities
                        .set_pos(slot, [origin[0] + 20.0, origin[1], origin[2]]);
                    s.entities.set_pos(slot, origin);
                }
            }
            _ => {}
        }
    }
}

fn compare(s: &TickState, rng: &mut Rng, queries: usize) -> usize {
    let mut reference = ReferenceTree::default();
    reference.rebuild(s);
    let mut local = EntityQuery::default();
    let mut nonzero = 0;
    let mobs_only = |slot: EntitySlot| matches!(s.entities.body(slot).kind, EntityKind::Mob { .. });
    let minecarts = |slot: EntitySlot| {
        let EntityKind::Mob { type_id } = s.entities.body(slot).kind else {
            return false;
        };
        mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)
            .is_some_and(|entity| entity.name.ends_with("minecart"))
    };
    for _ in 0..queries {
        let x = f64::from(rng.range(-50.0, 50.0).floor() as i32);
        let z = f64::from(rng.range(-50.0, 50.0).floor() as i32);
        let y = f64::from(rng.range(197.0, 202.0).floor() as i32);
        // Plate, detector rail, wooden button and a chunk-straddling wide box.
        let boxes = [
            ([x + 0.125, y, z + 0.125], [x + 0.875, y + 0.25, z + 0.875]),
            ([x + 0.125, y, z + 0.125], [x + 0.875, y + 0.8, z + 0.875]),
            ([x, y, z], [x + 1.0, y + 1.0, z + 1.0]),
            ([x - 9.0, y - 2.0, z - 9.0], [x + 9.0, y + 3.0, z + 9.0]),
        ];
        for (min, max) in boxes {
            for limit in [1, 15, 150] {
                let all = |_: EntitySlot| true;
                let expected = [
                    reference.count(min, max, limit, &all),
                    reference.count(min, max, limit, &mobs_only),
                    reference.count(min, max, limit, &minecarts),
                ];
                let actual = [
                    local.count(s, min, max, limit, all),
                    local.count(s, min, max, limit, mobs_only),
                    local.count(s, min, max, limit, minecarts),
                ];
                assert_eq!(actual, expected, "box {min:?}..{max:?} limit {limit}");
                nonzero += expected.iter().filter(|count| **count > 0).count();
            }
        }
    }
    nonzero
}

#[test]
fn local_census_matches_the_full_population_tree_on_seeded_worlds() {
    let mut nonzero = 0;
    for seed in 1..=6u64 {
        let mut s = state();
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15 ^ seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
        let slots = populate(&mut s, &mut rng, 700);
        nonzero += compare(&s, &mut rng, 40);
        decorate(&mut s, &mut rng, &slots);
        nonzero += compare(&s, &mut rng, 40);
        churn(&mut s, &mut rng, &slots);
        nonzero += compare(&s, &mut rng, 40);
    }
    assert!(nonzero > 2000, "comparison must exercise hits: {nonzero}");
}

#[test]
fn duplicate_membership_after_kind_change_counts_once() {
    let mut s = state();
    let villager = type_id(&s, "villager");
    let pig = type_id(&s, "pig");
    let id = s.spawn_mob(villager, [0.5, 200.0, 0.5], 20.0);
    let slot = s.entities.slot_of(id).unwrap();
    s.entities.set_kind(slot, EntityKind::Mob { type_id: pig });
    s.entities.set_pos(slot, [40.5, 200.0, 0.5]);
    s.entities.set_pos(slot, [0.5, 200.0, 0.5]);
    let mut listed = 0;
    s.entities
        .for_each_slot_in_chunk(ChunkPos { x: 0, z: 0 }, |_| listed += 1);
    assert_eq!(listed, 2, "fixture must produce a duplicate listing");
    let mut query = EntityQuery::default();
    assert_eq!(
        query.count(&s, [0.0, 200.0, 0.0], [1.0, 201.0, 1.0], 15, |_| true),
        1
    );
}
