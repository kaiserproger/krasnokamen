//! Virtual regiment leaders and stable rank-major rosters for the staged battle.
//! Sources/tuning: receipts/BattleVideo/research-combat-behavior.md §§4.1–4.10.
//! Decisions are staggered at 10 ticks; motion and membership liveness are not.

use super::morale::{Morale, Pressure, nerve};
use crate::boundary::EntityId;
use crate::entities::EntitySlot;
use mc_physics::mth;
use rustc_hash::FxHashMap;

pub(crate) const DECISION_TICKS: u64 = 10;
pub(crate) const SECTION_SIZE: usize = 20;
pub(crate) const STANDOFF: f64 = 4.0;
// Film lulls compress to a five-second base wait; action clocks stay human-speed.
pub(crate) const SURGE_PROBABILITY: f64 = 0.1;
pub(crate) const SURGE_TICKS: u64 = 200;
pub(crate) const SURGE_FATIGUE: f32 = 0.75;
pub(crate) const STANDING_RECOVERY: f32 = 0.25;
pub(crate) const EXHAUSTED_STAMINA: f32 = 30.0;
/// Below this morale sections neither start nor keep a pulse: losses stall.
pub(crate) const PULSE_MORALE_FLOOR: f32 = 30.0;
const MAX_REGIMENTS: usize = 128;
const MAX_SOLDIERS: usize = 20_000;
/// Window of the operator's stall census: ten seconds of advancing.
pub(crate) const STALL_TICKS: u64 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Order {
    Advance,
    Hold,
    Charge,
    #[cfg(test)]
    Withdraw,
    Pursue,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RegimentSpec {
    pub id: u32,
    pub faction: u16,
    /// Centre of the FRONT rank, not the centroid of the whole block.
    pub origin: [f64; 3],
    /// Unit horizontal vector; front is +facing, right is (z,-x).
    pub facing: [f64; 2],
    pub files: u16,
    /// One block for foot, two for mounted files; shared by spawn and motion.
    pub frontage: f64,
    pub order: Order,
}

pub(crate) fn slot_position(spec: &RegimentSpec, slot: usize) -> [f64; 3] {
    position(
        spec,
        slot % usize::from(spec.files),
        (slot / usize::from(spec.files)) as u16,
        spec.frontage,
    )
}

fn position(spec: &RegimentSpec, file: usize, rank: u16, interval: f64) -> [f64; 3] {
    let lateral = (file as f64 - f64::from(spec.files - 1) * 0.5) * interval;
    [
        spec.origin[0] + spec.facing[1] * lateral - spec.facing[0] * f64::from(rank) * 2.0,
        spec.origin[1],
        spec.origin[2] - spec.facing[0] * lateral - spec.facing[1] * f64::from(rank) * 2.0,
    ]
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct CombatContext {
    pub regiment: u32,
    #[cfg(test)]
    pub rank: u16,
    pub front_rank: bool,
    pub pulsing: bool,
    pub routing: bool,
    pub wavering: bool,
    pub morale: f32,
    pub facing: [f64; 2],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SoldierSample {
    pub slot: Option<EntitySlot>,
    pub pos: [f64; 3],
    pub health: f32,
    pub faction: u16,
}

#[derive(Debug)]
pub(crate) struct Member {
    pub id: EntityId,
    pub slot: Option<EntitySlot>,
    pub regiment: usize,
    pub file: usize,
    pub original_rank: u16,
    pub rank: u16,
    pub pos: [f64; 3],
    pub alive: bool,
    pub wounded: bool,
    pub missile_loss: bool,
    pub routing: bool,
    pub wavering: bool,
    pub nerve: f32,
    /// Position at the start of the current stall window, (x, z).
    pub anchor: [f64; 2],
}

#[derive(Debug)]
pub(crate) struct Section {
    pub phase: u64,
    pub pulse_until: u64,
    pub cooldown_until: u64,
    pub starts: u64,
    pub exchange: i16,
}

#[derive(Debug)]
pub(crate) struct Regiment {
    pub spec: RegimentSpec,
    pub start: usize,
    pub len: usize,
    pub ranks: u16,
    pub sections: Vec<Section>,
    pub morale: Morale,
    pub stamina: f32,
    pub enemy: Option<[f64; 3]>,
    pub enemy_regiment: Option<usize>,
    pub enemy_routing: bool,
    pub gap: f64,
    pub speed: f64,
    pub interval: f64,
    pub removed: usize,
    pub live_sum: [f64; 3],
    pub live_count: usize,
    pub charge_started: Option<u64>,
    pub broke_now: bool,
    pub evacuating: Vec<bool>,
    pub live_members: Vec<usize>,
    /// Virtual leader at the start of the current stall window, (x, z).
    pub anchor: [f64; 2],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Work {
    pub soldiers: u64,
    pub regiments: u64,
    pub sections: u64,
    pub section_checks: u64,
    pub rotated: u64,
    pub morale_members: u64,
    pub morale_neighbours: u64,
    pub distance_samples: u64,
    pub separation_candidates: u64,
    pub push_calls: u64,
}

#[derive(Debug, Default)]
pub(crate) struct FormationRuntime {
    pub members: Vec<Member>,
    pub regiments: Vec<Regiment>,
    index: FxHashMap<EntityId, usize>,
    pub work: Work,
    pub tick: u64,
    pub neighbours: super::steering::SeparationGrid,
    pub bounds: Option<([f64; 2], [f64; 2])>,
    /// Last stall census: live, steady members of regiments whose leader
    /// advanced at least four blocks in the window and is not in melee reach
    /// (gap > 6), and those of them that moved less than one block (blocked
    /// by terrain or crowd).
    pub advancing: usize,
    pub stalled: usize,
}

impl FormationRuntime {
    #[cfg(test)]
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Loaded staging-ticket interior, in block coordinates (x,z). Routers
    /// reach this perimeter and halt; they must never enter inactive chunks.
    pub fn set_bounds(&mut self, min: [f64; 2], max: [f64; 2]) -> Result<(), &'static str> {
        if !(0..2).all(|i| min[i].is_finite() && max[i].is_finite() && min[i] < max[i]) {
            return Err("battle movement bounds require finite min < max");
        }
        if self.members.iter().any(|m| {
            m.pos[0] < min[0] || m.pos[0] > max[0] || m.pos[2] < min[1] || m.pos[2] > max[1]
        }) {
            return Err("battle movement bounds exclude a registered soldier");
        }
        self.bounds = Some((min, max));
        Ok(())
    }

    /// Apply after mounted-speed scaling too: otherwise normalizing a tiny
    /// edge-clipped foot drive would send the horse outside the loaded ticket.
    pub fn constrain_velocity(&self, pos: [f64; 3], mut velocity: [f64; 3]) -> [f64; 3] {
        if let Some((min, max)) = self.bounds {
            // Knockback is integrated by ordinary physics after battle steering.
            // Recover within the existing step-speed budget, never teleport the
            // authoritative pose back or let a routing drive keep going out.
            let back = [
                pos[0].clamp(min[0], max[0]) - pos[0],
                pos[2].clamp(min[1], max[1]) - pos[2],
            ];
            if back[0] != 0.0 || back[1] != 0.0 {
                let distance = mth::length2(back[0], back[1]);
                let speed =
                    mth::length2(velocity[0], velocity[2]).max(0.079 / mc_physics::TICK_SECONDS);
                let scale = speed.min(distance / mc_physics::TICK_SECONDS) / distance;
                velocity[0] = back[0] * scale;
                velocity[2] = back[1] * scale;
                return velocity;
            }
            for axis in 0..2 {
                let component = axis * 2;
                let p = pos[component];
                if (min[axis]..=max[axis]).contains(&p) {
                    let next = p + velocity[component] * mc_physics::TICK_SECONDS;
                    velocity[component] =
                        (next.clamp(min[axis], max[axis]) - p) / mc_physics::TICK_SECONDS;
                }
            }
        }
        velocity
    }

    /// Staging admission is atomic. Caller order is rank-major and remains
    /// stable across entity-store dense compaction and worker scheduling.
    pub fn add_regiment(
        &mut self,
        spec: RegimentSpec,
        ids: &[EntityId],
    ) -> Result<(), &'static str> {
        if spec.faction == 0
            || spec.files == 0
            || ids.is_empty()
            || ids.len() > 1000
            || usize::from(spec.files) > ids.len()
        {
            return Err("battle regiment requires faction, 1..1000 members and valid files");
        }
        if !spec.origin.iter().all(|x| x.is_finite())
            || !spec.facing.iter().all(|x| x.is_finite())
            || (mth::length2(spec.facing[0], spec.facing[1]) - 1.0).abs() > 1e-6
            || !spec.frontage.is_finite()
            || !(1.0..=2.0).contains(&spec.frontage)
        {
            return Err(
                "battle regiment requires finite origin, unit facing and 1..2 block frontage",
            );
        }
        if self.regiments.len() == MAX_REGIMENTS || self.members.len() + ids.len() > MAX_SOLDIERS {
            return Err("battle formation capacity exceeded (128 regiments, 20000 soldiers)");
        }
        if self.regiments.iter().any(|r| r.spec.id == spec.id) {
            return Err("duplicate battle regiment id");
        }
        let mut unique = rustc_hash::FxHashSet::default();
        if ids
            .iter()
            .any(|id| self.index.contains_key(id) || !unique.insert(*id))
        {
            return Err("duplicate battle regiment member");
        }
        let regiment = self.regiments.len();
        let start = self.members.len();
        let files = usize::from(spec.files);
        let ranks = ids.len().div_ceil(files) as u16;
        for (slot, &id) in ids.iter().enumerate() {
            let rank = (slot / files) as u16;
            let pos = slot_position(&spec, slot);
            self.index.insert(id, self.members.len());
            self.members.push(Member {
                id,
                regiment,
                file: slot % files,
                original_rank: rank,
                rank,
                pos,
                slot: None,
                alive: true,
                wounded: false,
                missile_loss: false,
                routing: false,
                wavering: false,
                nerve: nerve(id.0, rank, ranks),
                anchor: [pos[0], pos[2]],
            });
        }
        self.regiments.push(Regiment {
            spec,
            start,
            len: ids.len(),
            ranks,
            sections: (0..ids.len().div_ceil(SECTION_SIZE))
                .map(|s| Section {
                    phase: (u64::from(spec.id) + s as u64) % DECISION_TICKS,
                    pulse_until: 0,
                    cooldown_until: 0,
                    starts: 0,
                    exchange: 0,
                })
                .collect(),
            morale: Morale::default(),
            stamina: 100.0,
            enemy: None,
            enemy_regiment: None,
            enemy_routing: false,
            gap: f64::INFINITY,
            speed: 0.0,
            interval: spec.frontage,
            removed: 0,
            charge_started: None,
            live_sum: [0.0; 3],
            live_count: 0,
            broke_now: false,
            evacuating: vec![false; files],
            live_members: Vec::with_capacity(ids.len()),
            anchor: [spec.origin[0], spec.origin[2]],
        });
        Ok(())
    }

    pub fn set_order(&mut self, regiment: u32, order: Order) -> bool {
        let Some(r) = self.regiments.iter_mut().find(|r| r.spec.id == regiment) else {
            return false;
        };
        if r.spec.order != order {
            r.charge_started = None;
        }
        r.spec.order = order;
        let stops_pulses = match order {
            #[cfg(test)]
            Order::Withdraw => true,
            Order::Pursue => true,
            _ => false,
        };
        if stops_pulses {
            for section in &mut r.sections {
                section.pulse_until = 0;
            }
        }
        true
    }

    #[cfg(test)]
    /// Polybius fighting interval: 1..2 blocks per man. Staging must leave the
    /// corresponding regiment intervals; this never changes rank spacing.
    pub fn set_fighting_interval(
        &mut self,
        regiment: u32,
        blocks: f64,
    ) -> Result<(), &'static str> {
        if !blocks.is_finite() || !(1.0..=2.0).contains(&blocks) {
            return Err("battle fighting interval must be 1..2 blocks");
        }
        let r = self
            .regiments
            .iter_mut()
            .find(|r| r.spec.id == regiment)
            .ok_or("unknown battle regiment")?;
        if blocks < r.spec.frontage {
            return Err("battle fighting interval cannot be narrower than marching frontage");
        }
        r.interval = blocks;
        Ok(())
    }

    #[cfg(test)]
    pub fn context(&self, id: EntityId) -> Option<CombatContext> {
        Some(self.context_at(*self.index.get(&id)?))
    }

    pub(crate) fn context_at(&self, index: usize) -> CombatContext {
        let m = &self.members[index];
        let r = &self.regiments[m.regiment];
        // Slot rotation also rotates section responsibility to the relieved file.
        let section = (usize::from(m.rank) * usize::from(r.spec.files) + m.file) / SECTION_SIZE;
        let pulsing = r
            .sections
            .get(section)
            .is_some_and(|s| s.pulse_until > self.tick);
        CombatContext {
            regiment: r.spec.id,
            #[cfg(test)]
            rank: m.rank,
            front_rank: m.rank == 0 && m.alive && !m.wounded,
            pulsing,
            routing: m.routing || !m.alive || m.wounded,
            wavering: m.wavering,
            morale: r.morale.value,
            facing: r.spec.facing,
        }
    }

    /// Shared nearest-regiment choice, sampled every ten ticks. The roster is
    /// compacted from the current frozen snapshot, so choosing a live target
    /// uses two indexed reads and no per-archer spatial query.
    pub fn archer_target(&self, bi: usize, roll: u64) -> Option<EntityId> {
        let member = self.members.get(bi)?;
        let enemy = self.regiments[member.regiment].enemy_regiment?;
        let roster = &self.regiments[enemy].live_members;
        if roster.is_empty() {
            return None;
        }
        Some(self.members[roster[(roll % roster.len() as u64) as usize]].id)
    }

    #[cfg(test)]
    pub fn can_engage(&self, id: EntityId) -> bool {
        self.context(id).is_none_or(|c| c.front_rank && !c.routing)
    }

    #[cfg(test)]
    pub fn can_attack(&self, id: EntityId) -> bool {
        let Some(&index) = self.index.get(&id) else {
            return true;
        };
        self.can_attack_at(index)
    }

    pub(crate) fn can_attack_at(&self, index: usize) -> bool {
        let c = self.context_at(index);
        let r = &self.regiments[self.members[index].regiment];
        let pursuing = r.spec.order == Order::Pursue && r.enemy_routing;
        c.front_rank && (c.pulsing || pursuing) && !c.routing && !c.wavering
    }

    pub fn mark_wounded(&mut self, id: EntityId) {
        if let Some(&i) = self.index.get(&id) {
            self.members[i].wounded = true;
        }
    }

    pub fn report_kill(&mut self, attacker: EntityId, victim: EntityId, ranged: bool) {
        if let Some(&i) = self.index.get(&attacker) {
            let r = &mut self.regiments[self.members[i].regiment];
            r.morale.kills = r.morale.kills.saturating_add(1);
        }
        if let Some(&i) = self.index.get(&victim) {
            let r = &mut self.regiments[self.members[i].regiment];
            self.members[i].missile_loss = ranged;
            let s = (usize::from(self.members[i].rank) * usize::from(r.spec.files)
                + self.members[i].file)
                / SECTION_SIZE;
            if let Some(section) = r.sections.get_mut(s) {
                section.exchange -= 1;
            }
        }
    }

    pub fn report_pressure(&mut self, id: EntityId, pressure: Pressure) {
        let Some(&i) = self.index.get(&id) else {
            return;
        };
        let p = &mut self.regiments[self.members[i].regiment].morale.pressure;
        p.missiles |= pressure.missiles;
        p.shield_wall |= pressure.shield_wall;
        p.flank |= pressure.flank;
        p.rear |= pressure.rear;
        p.flanks_secure |= pressure.flanks_secure;
        p.adjacent_rout |= pressure.adjacent_rout;
        p.panicking = p.panicking.saturating_add(pressure.panicking);
    }

    /// Frozen snapshots and native columns use the same ordered fold.
    #[cfg(test)]
    pub fn prepare(
        &mut self,
        tick: u64,
        mut lookup: impl FnMut(EntityId) -> Option<SoldierSample>,
    ) {
        self.begin_preparation(tick);
        for bi in 0..self.members.len() {
            let sample = lookup(self.members[bi].id);
            self.sample_member(bi, sample);
        }
        self.finish_preparation();
    }

    pub fn has_member(&self, id: EntityId) -> bool {
        self.index.contains_key(&id)
    }

    pub fn begin_preparation(&mut self, tick: u64) {
        self.tick = tick;
        self.work = Work::default();
        for r in &mut self.regiments {
            r.live_sum = [0.0; 3];
            r.live_count = 0;
            r.live_members.clear();
        }
    }

    pub fn sample_member(&mut self, bi: usize, sample: Option<SoldierSample>) {
        self.work.soldiers += 1;
        let m = &mut self.members[bi];
        let expected = self.regiments[m.regiment].spec.faction;
        m.slot = None;
        m.alive = sample.is_some_and(|sample| {
            m.pos = sample.pos;
            m.slot = sample
                .slot
                .filter(|_| sample.health > 0.0 && sample.faction == expected);
            sample.health > 0.0 && sample.faction == expected
        });
        if m.alive {
            let r = &mut self.regiments[m.regiment];
            r.live_count += 1;
            if !m.wounded {
                r.live_members.push(bi);
            }
            for axis in 0..3 {
                r.live_sum[axis] += m.pos[axis];
            }
        }
    }

    pub fn finish_preparation(&mut self) {
        for r in &mut self.regiments {
            if r.morale.broken && r.live_count > 0 {
                r.spec.origin = r.live_sum.map(|x| x / r.live_count as f64);
            }
        }
        for i in 0..self.regiments.len() {
            if self.tick % DECISION_TICKS == u64::from(self.regiments[i].spec.id) % DECISION_TICKS {
                self.work.regiments += 1;
                self.rotate_wounded(i);
                let (enemy_regiment, enemy, routing, examined) =
                    super::steering::nearest_front(&self.regiments, i);
                self.regiments[i].enemy_regiment = enemy_regiment;
                self.work.distance_samples += examined;
                self.regiments[i].enemy = enemy;
                self.regiments[i].enemy_routing = routing;
            }
            self.update_sections(i);
            self.move_leader(i);
        }
        if self.tick % STALL_TICKS == 0 {
            self.census_stalls();
        }
    }

    /// Ten-second stall census for operators and relief takes. O(N) once
    /// per window; motion never reads it.
    fn census_stalls(&mut self) {
        self.advancing = 0;
        self.stalled = 0;
        for r in &mut self.regiments {
            // A line in melee reach stops by design: it is not advancing.
            let advanced = mth::length2(
                r.spec.origin[0] - r.anchor[0],
                r.spec.origin[2] - r.anchor[1],
            ) >= 4.0
                && !r.morale.broken
                && r.gap > 6.0;
            r.anchor = [r.spec.origin[0], r.spec.origin[2]];
            for m in &mut self.members[r.start..r.start + r.len] {
                if advanced && m.alive && !m.wounded && !m.routing {
                    self.advancing += 1;
                    self.stalled += usize::from(
                        mth::length2(m.pos[0] - m.anchor[0], m.pos[2] - m.anchor[1]) < 1.0,
                    );
                }
                m.anchor = [m.pos[0], m.pos[2]];
            }
        }
    }

    fn rotate_wounded(&mut self, i: usize) {
        let r = &mut self.regiments[i];
        let files = usize::from(r.spec.files);
        let members = &mut self.members[r.start..r.start + r.len];
        r.removed = 0;
        r.evacuating.fill(false);
        self.work.rotated += r.len as u64;
        for file in 0..files {
            let mut next_rank = 0;
            for index in (file..members.len()).step_by(files) {
                let m = &mut members[index];
                if m.alive && !m.wounded {
                    m.rank = next_rank;
                    next_rank += 1;
                } else {
                    r.removed += 1;
                    r.evacuating[file] |= m.alive && m.wounded;
                    m.rank = r.ranks + m.original_rank;
                }
            }
        }
    }

    fn update_sections(&mut self, i: usize) {
        let r = &mut self.regiments[i];
        let contact = r.gap <= 6.0 && r.enemy.is_some();
        for s in 0..r.sections.len() {
            self.work.section_checks += 1;
            if self.tick % DECISION_TICKS != r.sections[s].phase {
                continue;
            }
            self.work.sections += 1;
            let neighbour_pulsing = (s > 0 && r.sections[s - 1].pulse_until > self.tick)
                || (s + 1 < r.sections.len() && r.sections[s + 1].pulse_until > self.tick);
            let leader = &self.members[r.start + s * SECTION_SIZE];
            let section = &mut r.sections[s];
            let active = section.pulse_until > self.tick;
            if active
                && (section.exchange <= -3
                    || r.stamina < EXHAUSTED_STAMINA
                    || r.morale.value < PULSE_MORALE_FLOOR)
            {
                section.pulse_until = self.tick;
                section.cooldown_until = self.tick + 40;
            }
            if !contact
                || section.pulse_until > self.tick
                || section.cooldown_until > self.tick
                || r.morale.value < PULSE_MORALE_FLOOR
                || r.stamina < 20.0
                || r.morale.broken
                || s * SECTION_SIZE >= usize::from(r.spec.files)
                || match r.spec.order {
                    #[cfg(test)]
                    Order::Withdraw => true,
                    Order::Pursue => true,
                    _ => false,
                }
            {
                continue;
            }
            // Stateless seeded decisions: no shared RNG or scheduling dependence.
            let probability = if neighbour_pulsing {
                0.3
            } else {
                SURGE_PROBABILITY
            } * f64::from(
                ((r.morale.value - PULSE_MORALE_FLOOR) / 50.0).clamp(0.0, 1.0),
            ) * f64::from(r.stamina / 100.0)
                * if leader.alive && !leader.wounded {
                    1.0
                } else {
                    0.3
                };
            if r.spec.order == Order::Charge || roll(r.spec.id, s, self.tick) < probability {
                section.pulse_until = self.tick + SURGE_TICKS;
                section.cooldown_until = section.pulse_until + 40;
                section.exchange = 0;
                section.starts += 1;
            }
        }
        let fighting = r.sections.iter().any(|s| s.pulse_until > self.tick);
        if self.tick % DECISION_TICKS == u64::from(r.spec.id) % DECISION_TICKS {
            let delta = if fighting {
                -SURGE_FATIGUE
            } else if matches!(r.spec.order, Order::Charge | Order::Pursue) {
                -0.4
            } else if r.speed == 0.0 {
                STANDING_RECOVERY
            } else {
                -0.025
            };
            r.stamina = (r.stamina + delta).clamp(0.0, 100.0);
        }
    }

    fn move_leader(&mut self, i: usize) {
        let r = &mut self.regiments[i];
        // A virtual leader moves along its assigned facing: lateral convergence
        // on one enemy centroid must never collapse a line into a blob.
        r.gap = r.enemy.map_or(f64::INFINITY, |p| {
            (p[0] - r.spec.origin[0]) * r.spec.facing[0]
                + (p[2] - r.spec.origin[2]) * r.spec.facing[1]
        });
        let fat = if r.stamina < 20.0 {
            0.85
        } else if r.stamina < 45.0 {
            0.9
        } else {
            1.0
        };
        let broken = r.morale.broken;
        r.speed = if broken {
            0.0
        } else {
            match r.spec.order {
                Order::Hold => {
                    if r.gap < 3.0 {
                        -0.04
                    } else {
                        0.0
                    }
                }
                #[cfg(test)]
                Order::Withdraw => -0.05,
                Order::Pursue => {
                    if r.enemy.is_some() {
                        0.12
                    } else {
                        0.0
                    }
                }
                Order::Advance | Order::Charge => {
                    if r.enemy.is_none() {
                        0.0
                    } else if r.gap > STANDOFF {
                        (r.gap - STANDOFF).min(0.066)
                    } else if r.gap < 3.0 {
                        -0.04
                    } else {
                        0.0
                    }
                }
            }
        } * fat;
        if r.spec.order == Order::Charge && r.charge_started.is_none() {
            r.charge_started = Some(self.tick);
        }
        r.spec.origin[0] += r.spec.facing[0] * r.speed;
        r.spec.origin[2] += r.spec.facing[1] * r.speed;
    }

    pub fn update_morale(&mut self, tick: u64) {
        for r in &mut self.regiments {
            r.broke_now = false;
            if tick % DECISION_TICKS != u64::from(r.spec.id) % DECISION_TICKS {
                continue;
            }
            let charging = r
                .charge_started
                .is_some_and(|start| tick.saturating_sub(start) < 200)
                && r.spec.order == Order::Charge;
            if let Some(enemy) = r.enemy {
                let direction = super::steering::unit(
                    [enemy[0] - r.spec.origin[0], enemy[2] - r.spec.origin[2]],
                    r.spec.facing,
                );
                let dot = direction[0] * r.spec.facing[0] + direction[1] * r.spec.facing[1];
                let distance =
                    mth::length2(enemy[0] - r.spec.origin[0], enemy[2] - r.spec.origin[2]);
                if distance <= 6.0 {
                    r.morale.pressure.flank |= dot.abs() < 0.5;
                    r.morale.pressure.rear |= dot < -0.5;
                }
            }
            let (removed, ranged) = self.members[r.start..r.start + r.len].iter().fold(
                (0, 0),
                |(removed, ranged), m| {
                    let casualty = !m.alive || m.wounded;
                    (
                        removed + usize::from(casualty),
                        ranged + usize::from(casualty && m.missile_loss),
                    )
                },
            );
            r.removed = removed;
            r.morale.missile_losses = ranged;
            self.work.morale_members += 2 * r.len as u64;
            let was_broken = r.morale.broken;
            r.morale.update(tick, r.len, r.removed, r.stamina, charging);
            r.broke_now = !was_broken && r.morale.broken;
            let mut new_panics = 0_u16;
            for m in &mut self.members[r.start..r.start + r.len] {
                let was_routing = m.routing;
                m.wavering = r.morale.value < m.nerve + 15.0;
                m.routing |= r.morale.value < m.nerve || r.morale.broken;
                new_panics += u16::from(!was_routing && m.routing);
            }
            r.morale.pressure.panicking = new_panics;
        }
        // Contagion is an event on the transition, not -10 every half-second
        // forever. Front widths make adjacent blocks count despite distant
        // regiment centroids. Work is bounded by 128² only on break ticks.
        for i in 0..self.regiments.len() {
            if !self.regiments[i].broke_now {
                continue;
            }
            for j in 0..self.regiments.len() {
                if i == j || self.regiments[i].spec.faction != self.regiments[j].spec.faction {
                    continue;
                }
                self.work.morale_neighbours += 1;
                let a = self.regiments[i].spec;
                let b = self.regiments[j].spec;
                let radius = 16.0
                    + (f64::from(a.files) * a.frontage + f64::from(b.files) * b.frontage) * 0.5;
                if mth::length2(a.origin[0] - b.origin[0], a.origin[2] - b.origin[2]) <= radius {
                    self.regiments[j].morale.pressure.adjacent_rout = true;
                }
            }
        }
    }

    pub fn destination(&self, index: usize) -> ([f64; 3], f64) {
        let m = &self.members[index];
        let r = &self.regiments[m.regiment];
        if m.routing && !m.wounded {
            let source = r.enemy.unwrap_or([
                m.pos[0] + r.spec.facing[0],
                m.pos[1],
                m.pos[2] + r.spec.facing[1],
            ]);
            let away = super::steering::unit(
                [m.pos[0] - source[0], m.pos[2] - source[2]],
                [-r.spec.facing[0], -r.spec.facing[1]],
            );
            return (
                [
                    m.pos[0] + away[0] * 20.0,
                    m.pos[1],
                    m.pos[2] + away[1] * 20.0,
                ],
                0.25,
            );
        }
        if r.spec.order == Order::Pursue
            && r.enemy_routing
            && !m.wounded
            && let Some(enemy) = r.enemy
        {
            return (enemy, 0.28);
        }
        let engaged = r.gap <= 6.0;
        let interval = if engaged { r.interval } else { r.spec.frontage };
        let mut target = position(&r.spec, m.file, m.rank, interval);
        // The leader's height is meaningless on relief; a slot is on the
        // ground the soldier walks on.
        target[1] = m.pos[1];
        if m.wounded {
            // Opposite .35-block lanes give .7 passing distance within a
            // one-block file. Dress sideways FIRST; aiming at a far rear slot
            // would give almost no lateral speed and deadlock the replacement.
            return (passing_lane(m, &r.spec, target, 0.35), 0.04);
        }
        if r.evacuating[m.file] {
            target = passing_lane(m, &r.spec, target, -0.35);
        }
        let context = self.context_at(index);
        if context.front_rank && context.pulsing && !context.wavering {
            target[0] += r.spec.facing[0] * 2.25;
            target[2] += r.spec.facing[1] * 2.25;
        } else if m.wavering {
            target[0] -= r.spec.facing[0];
            target[2] -= r.spec.facing[1];
        }
        (target, if context.pulsing { 0.10 } else { 0.079 })
    }

    /// Where terrain detours aim: the formation slot without lanes, pulses or
    /// wavering, so a body dressing sideways is still steered past a wall;
    /// routing, wounded and pursuing members aim at their destination.
    pub fn heading_target(&self, index: usize) -> [f64; 3] {
        let m = &self.members[index];
        let r = &self.regiments[m.regiment];
        if m.routing || m.wounded || (r.spec.order == Order::Pursue && r.enemy_routing) {
            return self.destination(index).0;
        }
        let interval = if r.gap <= 6.0 { r.interval } else { r.spec.frontage };
        position(&r.spec, m.file, m.rank, interval)
    }
}

fn passing_lane(m: &Member, spec: &RegimentSpec, mut target: [f64; 3], offset: f64) -> [f64; 3] {
    let right = [spec.facing[1], -spec.facing[0]];
    target[0] += right[0] * offset;
    target[2] += right[1] * offset;
    let sideways = (target[0] - m.pos[0]) * right[0] + (target[2] - m.pos[2]) * right[1];
    if sideways.abs() > 0.02 {
        target[0] = m.pos[0] + right[0] * sideways;
        target[2] = m.pos[2] + right[1] * sideways;
    }
    target
}

pub(crate) fn roll(regiment: u32, section: usize, tick: u64) -> f64 {
    let mut x = u64::from(regiment).wrapping_mul(0x9e37_79b9_7f4a_7c15)
        ^ (section as u64).wrapping_mul(0xbf58_476d_1ce4_e5b9)
        ^ tick;
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (x ^ (x >> 31)) as f64 / u64::MAX as f64
}

#[cfg(test)]
#[path = "formation_tests.rs"]
mod tests;
