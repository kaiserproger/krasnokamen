//! Vanilla experience orb lifecycle, denominations, collection and Mending.

use std::sync::Arc;

use crate::boundary::{ChangeSet, EntityAppearance, EntityEvent, EntityId};
use crate::entities::{ColdFields, EntityKind};
use crate::tick::{TickSampler, TickState};

/// Vanilla `ExperienceOrb.getExperienceValue`.
#[must_use]
pub fn experience_value(amount: i32) -> i32 {
    [2477, 1237, 617, 307, 149, 73, 37, 17, 7, 3, 1]
        .into_iter()
        .find(|value| amount >= *value)
        .unwrap_or(1)
}

fn random(seed: u64) -> u64 {
    let mut value = seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// Shared indexed orb allocation; producers supply native value/count and motion.
pub(crate) fn spawn(
    state: &mut TickState,
    type_id: u32,
    pos: [f64; 3],
    value_count: (i32, i32),
    vel: [f64; 3],
    yaw: f32,
) -> EntityId {
    let id = state.entities.alloc_id();
    state.entities.spawn(
        id,
        state.entity_uuid(id),
        EntityKind::ExperienceOrb { type_id },
        ColdFields {
            pos,
            vel,
            yaw,
            pitch: 0.0,
            ground: false,
            health: 5.0,
            breed_cooldown: 0,
            pickup_delay: 0,
            item: None,
            appearance: Arc::new(EntityAppearance {
                experience_orb: Some(value_count),
                health: 5.0,
                ..Default::default()
            }),
            movement: crate::blueprint::MovementClass::Item,
            half_width: 0.25,
            height: 0.5,
            speed: 0.0,
            attack_damage: 0.0,
            sight_range: 0.0,
            attack_reach: 0.0,
        },
    );
    id
}

/// Spawn denominations; the vanilla randomized 40-group merge bounds orb density.
pub(crate) fn award(
    state: &mut TickState,
    pos: [f64; 3],
    mut amount: i32,
    spawned: &mut Vec<EntityId>,
) {
    let Some(type_id) = state.registry.entity_types.id_of(
        &mc_data::Identifier::parse("minecraft:experience_orb").expect("built-in identifier"),
    ) else {
        return;
    };
    while amount > 0 {
        let value = experience_value(amount);
        amount -= value;
        let seed =
            random(state.tick ^ (state.entities.len() as u64).rotate_left(32) ^ amount as u64);
        let group = (seed % 40) as i32;
        let existing = state
            .entities
            .experience_orbs()
            .filter_map(|id| state.entities.slot_of(id))
            .find(|slot| {
                let body = state.entities.body(*slot);
                let row = state.entities.row(*slot);
                row.appearance
                    .experience_orb
                    .is_some_and(|(v, _)| v == value)
                    && (body.id.0 - group) % 40 == 0
                    && state
                        .entities
                        .pos(*slot)
                        .into_iter()
                        .zip(pos)
                        .all(|(a, b)| (a - b).abs() <= 0.75)
            });
        if let Some(slot) = existing {
            let (body, row) = state.entities.parts_mut(slot);
            let data = Arc::make_mut(&mut row.appearance)
                .experience_orb
                .as_mut()
                .expect("orb");
            data.1 = data.1.saturating_add(1);
            row.age = 0;
            state.appearance_changed.push(body.id);
            continue;
        }
        let unit = |salt| random(seed ^ salt) as f64 / u64::MAX as f64;
        let id = spawn(
            state,
            type_id,
            pos,
            (value, 1),
            // Core velocities are blocks/second; vanilla uses blocks/tick.
            [
                (unit(1) * 0.4 - 0.2) * 20.0,
                unit(2) * 8.0,
                (unit(3) * 0.4 - 0.2) * 20.0,
            ],
            (unit(4) * 360.0) as f32,
        );
        spawned.push(id);
    }
}

fn distance_sq(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| (a - b).powi(2)).sum()
}

fn merge_cell(pos: [f64; 3]) -> [i32; 3] {
    pos.map(|component| (component / 4.0).floor() as i32)
}

/// Player-attributed mob XP. Animals roll 1–3; baby animals drop none.
#[must_use]
pub(crate) fn mob_experience(name: &str, baby: bool, seed: u64) -> i32 {
    match name.strip_prefix("minecraft:").unwrap_or(name) {
        "cow" | "mooshroom" | "pig" | "sheep" | "chicken" | "rabbit" | "horse" | "donkey"
        | "mule" | "llama" | "trader_llama" | "wolf" | "cat" | "ocelot" | "fox" | "panda"
        | "polar_bear" | "goat" | "camel" | "armadillo" | "bee" | "turtle" | "axolotl" | "frog"
        | "sniffer" | "dolphin" | "nautilus" => {
            if baby {
                0
            } else {
                1 + (random(seed) % 3) as i32
            }
        }
        "cod" | "salmon" | "pufferfish" | "tropical_fish" => 1 + (random(seed) % 3) as i32,
        "blaze" | "breeze" | "elder_guardian" | "guardian" => 10,
        "ravager" | "piglin_brute" => 20,
        "wither" => 50,
        "ender_dragon" => 12000,
        "slime" | "magma_cube" => 1,
        "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin" if baby => 12,
        "zombie" | "husk" | "drowned" | "zombie_villager" | "zombified_piglin" | "skeleton"
        | "stray" | "bogged" | "wither_skeleton" | "creeper" | "spider" | "cave_spider"
        | "enderman" | "endermite" | "silverfish" | "witch" | "ghast" | "phantom" | "shulker"
        | "pillager" | "vindicator" | "evoker" | "vex" | "piglin" | "hoglin" | "zoglin"
        | "warden" | "parched" | "camel_husk" | "zombie_nautilus" => 5,
        _ => 0,
    }
}

/// Mending selects a damaged equipped item, repairs two durability per XP,
/// then repeats with the remainder before crediting experience.
fn mend(player: &mut crate::player::PlayerState, mut amount: i32, mut seed: u64) -> (i32, bool) {
    let slots = [
        usize::from(player.inventory.selected.min(8)),
        crate::inventory::OFF_HAND,
        36,
        37,
        38,
        39,
    ];
    let mut changed = false;
    while amount > 0 {
        let mut candidates = [0; 6];
        let mut count = 0;
        for slot in slots {
            let stack = &player.inventory.slots[slot];
            if !stack.is_empty()
                && stack.damage.is_some_and(|damage| damage > 0)
                && stack.enchantments.iter().any(|enchantment| {
                    enchantment.id.as_str() == "minecraft:mending" && enchantment.level > 0
                })
            {
                candidates[count] = slot;
                count += 1;
            }
        }
        if count == 0 {
            break;
        }
        seed = random(seed);
        let stack = &mut player.inventory.slots[candidates[(seed % count as u64) as usize]];
        let damage = stack.damage.unwrap_or(0);
        let repaired = damage.min(amount.saturating_mul(2));
        stack.damage = Some(damage - repaired);
        changed = true;
        amount -= repaired / 2;
    }
    (amount, changed)
}

/// Logical body visits, including rejected/deleted candidates and index construction.
/// Compiles away outside tests; unlike admitted-orb telemetry this measures search work.
#[derive(Default)]
struct OrbWork {
    #[cfg(test)]
    orbs: std::cell::Cell<u64>,
    #[cfg(test)]
    players: std::cell::Cell<u64>,
}

impl OrbWork {
    #[inline]
    fn orb(&self) {
        #[cfg(test)]
        self.orbs.set(self.orbs.get() + 1);
    }

    #[inline]
    fn player(&self) {
        #[cfg(test)]
        self.players.set(self.players.get() + 1);
    }
}

type PlayerChunks = std::collections::HashMap<crate::boundary::ChunkPos, Vec<(EntityId, [f64; 3])>>;

/// Query the existing entity chunk index once per visited chunk, not once per
/// orb. Dense XP piles must not become a quadratic scan of non-player bodies.
fn local_players(
    state: &TickState,
    pos: [f64; 3],
    radius: f64,
    cache: &mut PlayerChunks,
    out: &mut Vec<(EntityId, [f64; 3])>,
    work: &OrbWork,
) {
    out.clear();
    let chunk = |v: f64| (v.floor() as i32).div_euclid(16);
    for x in chunk(pos[0] - radius)..=chunk(pos[0] + radius) {
        for z in chunk(pos[2] - radius)..=chunk(pos[2] + radius) {
            let players = cache
                .entry(crate::boundary::ChunkPos { x, z })
                .or_insert_with(|| {
                    let mut players = Vec::new();
                    state.entities.for_each_slot_in_chunk(
                        crate::boundary::ChunkPos { x, z },
                        |slot| {
                            let body = state.entities.body(slot);
                            if matches!(body.kind, EntityKind::Player { .. }) {
                                work.player();
                                if body.alive
                                    && state.players.get(&body.id).is_some_and(|p| !p.spectator())
                                    && !state.entity_session_locked(body.id)
                                {
                                    players.push((body.id, state.entities.pos(slot)));
                                }
                            } else {
                                work.orb();
                            }
                        },
                    );
                    players
                });
            for player in players.iter() {
                work.player();
                out.push(*player);
            }
        }
    }
    out.sort_unstable_by_key(|(id, _)| *id);
}

type MergeKey = (i32, i32, i32, i32, i32);

#[derive(Default)]
struct MergeCells {
    cells: std::collections::HashMap<MergeKey, std::collections::BTreeSet<EntityId>>,
    // The old 4-block admission cell and its ordering remain part of the oracle.
    origins: std::collections::HashMap<EntityId, [i32; 3]>,
}

impl MergeCells {
    fn key(id: EntityId, value: i32, pos: [f64; 3]) -> MergeKey {
        let [x, y, z] = pos.map(|v| v.floor() as i32);
        (value, id.0.rem_euclid(40), x, y, z)
    }

    fn insert(&mut self, id: EntityId, value: i32, pos: [f64; 3]) {
        self.origins.insert(id, merge_cell(pos));
        self.cells
            .entry(Self::key(id, value, pos))
            .or_default()
            .insert(id);
    }

    fn remove(&mut self, id: EntityId, value: i32, pos: [f64; 3]) {
        let key = Self::key(id, value, pos);
        if let Some(ids) = self.cells.get_mut(&key) {
            ids.remove(&id);
            if ids.is_empty() {
                self.cells.remove(&key);
            }
        }
    }

    fn moved(&mut self, id: EntityId, value: i32, before: [f64; 3], after: [f64; 3]) {
        if Self::key(id, value, before) != Self::key(id, value, after) {
            self.remove(id, value, before);
            self.cells
                .entry(Self::key(id, value, after))
                .or_default()
                .insert(id);
        }
    }

    fn nearby(
        &self,
        state: &TickState,
        id: EntityId,
        value: i32,
        pos: [f64; 3],
        out: &mut Vec<EntityId>,
        work: &OrbWork,
    ) {
        out.clear();
        let coarse = merge_cell(pos);
        let (v, group, x, y, z) = Self::key(id, value, pos);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = self.cells.get(&(v, group, x + dx, y + dy, z + dz)) {
                        for other in ids {
                            work.orb();
                            if *other == id
                                || !self.origins[other]
                                    .into_iter()
                                    .zip(coarse)
                                    .all(|(a, b)| (i64::from(a) - i64::from(b)).abs() <= 1)
                            {
                                continue;
                            }
                            let slot = state.entities.slot_of(*other).expect("live merge index");
                            if state
                                .entities
                                .pos(slot)
                                .into_iter()
                                .zip(pos)
                                .all(|(a, b)| (a - b).abs() <= 1.0)
                            {
                                out.push(*other);
                            }
                        }
                    }
                }
            }
        }
        out.sort_unstable_by_key(|id| (self.origins[id], *id));
    }
}

/// Runs after entity merge: physics, periodic local merges, expiry and collection.
pub(crate) fn run(state: &mut TickState, changes: &mut ChangeSet, events: &mut Vec<EntityEvent>) {
    run_inner(state, changes, events, None, &OrbWork::default());
}

/// Count only bodies actually admitted to the ordinary active-orb owner.
pub(crate) fn run_counted(
    state: &mut TickState,
    changes: &mut ChangeSet,
    events: &mut Vec<EntityEvent>,
    sample: &mut crate::boundary::CorePerfTick,
) {
    run_inner(state, changes, events, Some(sample), &OrbWork::default());
}

fn run_inner(
    state: &mut TickState,
    changes: &mut ChangeSet,
    events: &mut Vec<EntityEvent>,
    mut sample: Option<&mut crate::boundary::CorePerfTick>,
    work: &OrbWork,
) {
    if state.entities.experience_orbs().next().is_none() {
        return;
    }
    let spawned: std::collections::HashSet<_> = changes.entities_spawned.iter().copied().collect();
    let orbs: Vec<_> = state
        .entities
        .experience_orbs()
        .filter(|id| {
            work.orb();
            !spawned.contains(id)
        })
        .collect();
    if orbs.is_empty() {
        return;
    }
    let mut player_chunks = PlayerChunks::new();
    let mut players = Vec::new();
    let mut merge_cells = MergeCells::default();
    let mut nearby = Vec::new();
    if state.tick.is_multiple_of(20) {
        for id in &orbs {
            work.orb();
            let slot = state.entities.slot_of(*id).expect("orb index");
            let pos = state.entities.pos(slot);
            if !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            ) {
                continue;
            }
            let value = state
                .entities
                .row(slot)
                .appearance
                .experience_orb
                .expect("orb")
                .0;
            merge_cells.insert(*id, value, pos);
        }
    }
    for id in orbs {
        work.orb();
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        if let Some(sample) = sample.as_deref_mut() {
            sample.xp_orbs_processed += 1;
            if let Some(region) =
                crate::tick::coreperf_block_region(pos[0].floor() as i32, pos[2].floor() as i32)
            {
                *sample.xp_orbs_by_region.entry(region).or_default() += 1;
                let (value, count) = state
                    .entities
                    .row(slot)
                    .appearance
                    .experience_orb
                    .expect("orb");
                *sample.xp_value_by_region.entry(region).or_default() +=
                    i64::from(value) * i64::from(count);
            }
        }
        let mut velocity = state.entities.vel(slot).map(|component| component / 20.0);
        if !state.entities.row(slot).controls.no_gravity {
            velocity[1] -= 0.03;
        }
        local_players(state, pos, 8.0, &mut player_chunks, &mut players, work);
        let nearest = players
            .iter()
            .filter(|(_, at)| {
                work.player();
                distance_sq(*at, pos) <= 64.0
            })
            .min_by(|a, b| distance_sq(a.1, pos).total_cmp(&distance_sq(b.1, pos)));
        if let Some((_, target)) = nearest {
            let delta = [
                target[0] - pos[0],
                target[1] + 0.81 - pos[1],
                target[2] - pos[2],
            ];
            let distance = distance_sq(delta, [0.0; 3]).sqrt();
            if distance > 0.0 {
                let power = (1.0 - distance / 8.0).max(0.0).powi(2) * 0.1;
                for axis in 0..3 {
                    velocity[axis] += delta[axis] / distance * power;
                }
            }
        }
        let sampler = TickSampler {
            world: &state.world,
            registry: &state.registry,
        };
        if let Some(resolved) = mc_physics::resolve_collision_displacement(
            &sampler,
            mc_physics::Vec3::new(pos[0], pos[1], pos[2]),
            mc_physics::Aabb {
                half_width: 0.25,
                height: 0.5,
            },
            mc_physics::Vec3::new(velocity[0], velocity[1], velocity[2]),
            state.entities.ground(slot),
            0.0,
        ) {
            state.entities.set_pos(
                slot,
                [
                    pos[0] + resolved.delta.x,
                    pos[1] + resolved.delta.y,
                    pos[2] + resolved.delta.z,
                ],
            );
            if (resolved.delta.x - velocity[0]).abs() > 1e-8 {
                velocity[0] = 0.0;
            }
            if (resolved.delta.z - velocity[2]).abs() > 1e-8 {
                velocity[2] = 0.0;
            }
            if resolved.grounded && velocity[1] < -0.03 {
                velocity[1] *= -0.4;
            }
            let friction = if resolved.grounded { 0.6 * 0.98 } else { 0.98 };
            state
                .entities
                .set_vel(slot, velocity.map(|component| component * friction * 20.0));
            state.entities.set_ground(slot, resolved.grounded);
        }
        let value = state
            .entities
            .row(slot)
            .appearance
            .experience_orb
            .expect("orb")
            .0;
        if state.tick.is_multiple_of(20) {
            merge_cells.moved(id, value, pos, state.entities.pos(slot));
        }
        if state.entities.row(slot).age >= 6000 {
            merge_cells.remove(id, value, state.entities.pos(slot));
            changes.entities_removed.push(id);
            state.remove_entity(slot);
            continue;
        }
        if state.tick.is_multiple_of(20) {
            let pos = state.entities.pos(slot);
            merge_cells.nearby(state, id, value, pos, &mut nearby, work);
            for other in nearby.iter().copied() {
                work.orb();
                let other_slot = state.entities.slot_of(other).expect("merge target");
                let count = state
                    .entities
                    .row(other_slot)
                    .appearance
                    .experience_orb
                    .expect("orb")
                    .1;
                let age = state.entities.row(other_slot).age;
                merge_cells.remove(other, value, state.entities.pos(other_slot));
                state.remove_entity(other_slot);
                changes.entities_removed.push(other);
                let slot = state.entities.slot_of(id).expect("merge owner");
                let row = state.entities.row_mut(slot);
                row.age = row.age.min(age);
                let orb = Arc::make_mut(&mut row.appearance)
                    .experience_orb
                    .as_mut()
                    .expect("orb");
                orb.1 = orb.1.saturating_add(count);
                state.appearance_changed.push(id);
            }
        }
        let slot = state.entities.slot_of(id).expect("orb owner");
        let pos = state.entities.pos(slot);
        local_players(state, pos, 0.8, &mut player_chunks, &mut players, work);
        let collector = players.iter().find(|(player, at)| {
            work.player();
            (at[0] - pos[0]).abs() <= 0.8
                && (at[2] - pos[2]).abs() <= 0.8
                && pos[1] >= at[1] - 0.5
                && pos[1] <= at[1] + 2.0
                && state
                    .entities
                    .slot_of(*player)
                    .is_some_and(|slot| state.entities.body(slot).pickup_delay == 0)
        });
        let Some((collector, _)) = collector else {
            continue;
        };
        let collector = *collector;
        let (value, count) = state
            .entities
            .row(slot)
            .appearance
            .experience_orb
            .expect("orb");
        let player = state.players.get_mut(&collector).expect("player");
        let (remaining, repaired) = mend(player, value, state.tick ^ id.0 as u64);
        // Native playerTouch credits only a positive Mending remainder.
        if remaining > 0 {
            crate::commands::add_experience(&mut player.vitals.experience, remaining, false);
        }
        let connection = player.connection;
        let xp = player.experience_change();
        let inventory = repaired.then(|| player.inventory_contents());
        state.push_player(connection, xp);
        if let Some(inventory) = inventory {
            state.push_player(connection, inventory);
        }
        let collector_slot = state.entities.slot_of(collector).expect("collector");
        state.entities.body_mut(collector_slot).pickup_delay = 2;
        events.push(EntityEvent::Pickup {
            item: id,
            collector,
            count: 1,
        });
        if count == 1 {
            merge_cells.remove(id, value, pos);
            changes.entities_removed.push(id);
            state.remove_entity(slot);
        } else {
            Arc::make_mut(&mut state.entities.row_mut(slot).appearance).experience_orb =
                Some((value, count - 1));
            state.appearance_changed.push(id);
        }
    }
}

#[cfg(test)]
#[path = "experience_orbs_top100_h08_tests.rs"]
mod top100_h08_tests;

#[cfg(test)]
mod tests {
    use super::experience_value;

    #[test]
    fn coreperf_counts_only_active_orbs_admitted_to_the_real_owner() {
        let (mut state, _, pos) = crate::hostile_tests::setup(8.0);
        let mut spawned = Vec::new();
        super::award(&mut state, pos, 3, &mut spawned);
        assert_eq!(spawned.len(), 1);
        let mut just_spawned = crate::boundary::ChangeSet::default();
        just_spawned.entities_spawned.clone_from(&spawned);
        let mut sample = crate::boundary::CorePerfTick::default();
        super::run_counted(&mut state, &mut just_spawned, &mut Vec::new(), &mut sample);
        assert_eq!(sample.xp_orbs_processed, 0);
        let mut changes = crate::boundary::ChangeSet::default();
        super::run_counted(&mut state, &mut changes, &mut Vec::new(), &mut sample);
        assert_eq!(sample.xp_orbs_processed, 1);
    }

    #[test]
    fn vanilla_experience_denominations_and_boundaries() {
        for (amount, expected) in [
            (1, 1),
            (2, 1),
            (3, 3),
            (5, 3),
            (7, 7),
            (16, 7),
            (17, 17),
            (37, 37),
            (73, 73),
            (149, 149),
            (307, 307),
            (617, 617),
            (1237, 1237),
            (2477, 2477),
            (12000, 2477),
        ] {
            assert_eq!(experience_value(amount), expected);
        }
    }
}
