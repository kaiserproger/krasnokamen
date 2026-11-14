//! Derived execution columns. The formation roster is the stable prefix; native
//! faction rows not in that roster (including carried horses) occupy the tail.
//! Native rows remain authoritative. Only preparation resolves identity to slot.

use super::{
    EnemyTarget, Frame, PopulationCache, UnitCounters,
    formation::{FormationRuntime, SoldierSample},
};
use crate::boundary::EntityId;
use crate::entities::{EntityKind, EntitySlot};

/// Cold-row facts, valid only until the next native wire mutation. The slot is
/// merely an identity-checked lookup hint, including for dead retained bodies.
#[derive(Debug, Default)]
struct StaticFacts {
    slot: Option<EntitySlot>,
    valid: bool,
    health: f32,
    no_ai: bool,
    carried: bool,
    targetable: bool,
}

/// Mob type id with main- and off-hand item ids: archer classification inputs.
type ClassificationKey = (u32, Option<u32>, Option<u32>);

#[derive(Debug, Default)]
pub(crate) struct Columns {
    pub ids: Vec<EntityId>,
    pub slots: Vec<Option<EntitySlot>>,
    pub factions: Vec<u16>,
    pub positions: Vec<[f64; 3]>,
    pub targets: Vec<Option<EnemyTarget>>,
    pub drive: Vec<[f64; 3]>,
    /// A horse's way out of a terrain pocket (`ground::deflect_mounted`).
    pub escape: Vec<Option<super::ground::Escape>>,
    pub ranged: Vec<bool>,
    pub reach: Vec<f64>,
    pub active: Vec<bool>,
    pub carried: Vec<bool>,
    pub targetable: Vec<bool>,
    pub index_by_slot: Vec<Option<usize>>,
    pub drive_by_slot: Vec<[f64; 3]>,
    pub look_by_slot: Vec<Option<[f64; 3]>>,
    classification: Vec<Option<ClassificationKey>>,
    pub equipment_dirty: Vec<bool>,
    facts: Vec<StaticFacts>,
}

impl Columns {
    fn append(&mut self) {
        self.ids.push(EntityId(0));
        self.slots.push(None);
        self.factions.push(0);
        self.positions.push([0.0; 3]);
        self.targets.push(None);
        self.drive.push([0.0; 3]);
        self.escape.push(None);
        self.ranged.push(false);
        self.reach.push(0.0);
        self.active.push(false);
        self.carried.push(false);
        self.targetable.push(false);
        self.classification.push(None);
        self.equipment_dirty.push(false);
        self.facts.push(StaticFacts::default());
    }

    /// Slot/identity changes independently force refresh during preparation.
    pub fn invalidate(&mut self, id: EntityId, slot: EntitySlot) {
        if let Some(bi) = self.index(slot)
            && self.ids[bi] == id
        {
            self.facts[bi].valid = false;
        }
    }

    fn native_slot(&self, frame: &Frame<'_>, bi: usize, id: EntityId) -> Option<EntitySlot> {
        self.facts
            .get(bi)
            .and_then(|facts| facts.slot)
            .filter(|slot| {
                (slot.0 as usize) < frame.entities.len() && frame.entities.body(*slot).id == id
            })
            .or_else(|| frame.entities.slot_of_any(id))
    }

    fn prepare_unit(
        &mut self,
        frame: &Frame<'_>,
        bi: usize,
        id: EntityId,
        slot: Option<EntitySlot>,
        expected_faction: Option<u16>,
        population: &mut PopulationCache,
    ) {
        if bi == self.ids.len() {
            self.append();
        }
        if self.ids[bi] != id {
            self.targets[bi] = None;
            self.classification[bi] = None;
            self.facts[bi].valid = false;
            self.escape[bi] = None;
        }
        if self.facts[bi].slot != slot {
            self.facts[bi].valid = false;
        }
        self.facts[bi].slot = slot;
        self.ids[bi] = id;
        self.slots[bi] = None;
        self.drive[bi] = [0.0; 3];
        self.active[bi] = false;
        self.targetable[bi] = false;
        self.carried[bi] = false;
        self.factions[bi] = 0;
        let Some(slot) = slot else {
            self.facts[bi].valid = false;
            self.targets[bi] = None;
            return;
        };
        let body = frame.entities.body(slot);
        if !body.alive {
            self.facts[bi].valid = false;
            self.targets[bi] = None;
            return;
        }
        if !self.facts[bi].valid {
            let row = frame.entities.row(slot);
            let facts = &mut self.facts[bi];
            facts.health = row.health;
            population.remember(slot, id, row.health);
            facts.no_ai = row.controls.no_ai;
            facts.carried = !row.entity_life.mount.riders.is_empty();
            facts.targetable =
                matches!(body.kind, EntityKind::Mob { .. }) && !crate::axolotl::playing_dead(row);
            let type_id = match body.kind {
                EntityKind::Mob { type_id } => type_id,
                _ => u32::MAX,
            };
            let held = row.appearance.equipment[0]
                .as_ref()
                .filter(|stack| !stack.is_empty())
                .map(|stack| stack.item_id);
            let offhand = row.appearance.equipment[1]
                .as_ref()
                .filter(|stack| !stack.is_empty())
                .map(|stack| stack.item_id);
            let key = (type_id, held, offhand);
            if self.classification[bi] != Some(key) {
                self.ranged[bi] = super::combat::is_archer(frame, slot);
                self.classification[bi] = Some(key);
                // Latch until combat consumes it: inactive/carried rows may
                // change equipment several ticks before becoming fighters.
                self.equipment_dirty[bi] = true;
            }
            self.reach[bi] = super::engagement_reach(self.ranged[bi], row.attack_reach);
            facts.valid = true;
        }
        let facts = &mut self.facts[bi];
        let member_visible = expected_faction.is_none_or(|expected| {
            facts.health > 0.0 && super::relation(body.faction, expected) == super::Relation::Same
        });
        if !member_visible || (expected_faction.is_none() && facts.health <= 0.0) {
            facts.valid = false;
            self.targets[bi] = None;
            return;
        }
        self.slots[bi] = Some(slot);
        self.index_by_slot[slot.0 as usize] = Some(bi);
        self.positions[bi] = frame.entities.pos(slot);
        self.factions[bi] = body.faction;
        self.carried[bi] = facts.carried;
        self.targetable[bi] =
            super::battle_row(body.faction) && facts.health > 0.0 && facts.targetable;
        let chunk = frame.entities.chunk_of(slot);
        self.active[bi] =
            self.targetable[bi] && !facts.no_ai && frame.world.is_active_position(chunk.x, chunk.z);
    }

    pub fn prepare(
        &mut self,
        frame: &Frame<'_>,
        formation: &mut FormationRuntime,
        counters: &mut UnitCounters,
        population: &mut PopulationCache,
    ) {
        // Input/pre-hint changes are visible here, before battle and merge.
        population.prepare(frame.entities);
        super::invalidate_facts(self, population, frame.entities);
        self.index_by_slot.resize(frame.entities.len(), None);
        self.index_by_slot.fill(None);
        counters.clear();
        formation.begin_preparation(frame.tick);
        let mut len = 0;
        for bi in 0..formation.members.len() {
            let member = &formation.members[bi];
            let id = member.id;
            let expected = formation.regiments[member.regiment].spec.faction;
            let slot = self.native_slot(frame, bi, id);
            self.prepare_unit(frame, bi, id, slot, Some(expected), population);
            let sample = slot.and_then(|slot| {
                let body = frame.entities.body(slot);
                counters.record(body.faction, body.alive && self.facts[bi].health > 0.0);
                body.alive.then(|| SoldierSample {
                    slot: Some(slot),
                    pos: frame.entities.pos(slot),
                    health: self.facts[bi].health,
                    faction: body.faction,
                })
            });
            formation.sample_member(bi, sample);
            len += 1;
        }
        for slot in frame.entities.battle_slots() {
            if self.index_by_slot[slot.0 as usize].is_some() {
                continue;
            }
            let body = frame.entities.body(slot);
            let counted = formation.has_member(body.id);
            if !body.alive || !matches!(body.kind, EntityKind::Mob { .. }) {
                if !counted {
                    counters.record(
                        body.faction,
                        body.alive && population.health(frame.entities, slot) > 0.0,
                    );
                }
                continue;
            }
            self.prepare_unit(frame, len, body.id, Some(slot), None, population);
            if !counted {
                counters.record(body.faction, self.facts[len].health > 0.0);
            }
            if self.slots[len].is_some() {
                len += 1;
            }
        }
        counters.finish();
        self.ids.truncate(len);
        self.slots.truncate(len);
        self.factions.truncate(len);
        self.positions.truncate(len);
        self.targets.truncate(len);
        self.drive.truncate(len);
        self.ranged.truncate(len);
        self.reach.truncate(len);
        self.active.truncate(len);
        self.carried.truncate(len);
        self.targetable.truncate(len);
        self.classification.truncate(len);
        self.equipment_dirty.truncate(len);
        self.facts.truncate(len);
        self.clear_projection(frame.entities.len());
    }

    pub fn index(&self, slot: EntitySlot) -> Option<usize> {
        self.index_by_slot.get(slot.0 as usize).copied().flatten()
    }

    #[cfg(test)]
    pub fn target(&self, slot: EntitySlot) -> Option<&EnemyTarget> {
        self.targets[self.index(slot)?].as_ref()
    }

    pub fn clear_projection(&mut self, len: usize) {
        self.drive_by_slot.resize(len, [0.0; 3]);
        self.drive_by_slot.fill([0.0; 3]);
        self.look_by_slot.resize(len, None);
        self.look_by_slot.fill(None);
    }

    pub fn project(&mut self) {
        for (bi, slot) in self.slots.iter().enumerate() {
            if let Some(slot) = slot {
                self.drive_by_slot[slot.0 as usize] = self.drive[bi];
                self.look_by_slot[slot.0 as usize] = self.targets[bi].map(|target| target.pos);
            }
        }
    }
}
