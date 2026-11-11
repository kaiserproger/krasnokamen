//! Derived execution columns. The formation roster is the stable prefix; native
//! faction rows not in that roster (including carried horses) occupy the tail.
//! Native rows remain authoritative. Only preparation resolves identity to slot.

use super::{EnemyTarget, Frame, formation::FormationRuntime};
use crate::boundary::EntityId;
use crate::entities::{EntityKind, EntitySlot};

#[derive(Debug, Default)]
pub(crate) struct Columns {
    pub ids: Vec<EntityId>,
    pub slots: Vec<Option<EntitySlot>>,
    pub factions: Vec<u16>,
    pub positions: Vec<[f64; 3]>,
    pub targets: Vec<Option<EnemyTarget>>,
    pub drive: Vec<[f64; 3]>,
    pub ranged: Vec<bool>,
    pub reach: Vec<f64>,
    pub active: Vec<bool>,
    pub carried: Vec<bool>,
    pub targetable: Vec<bool>,
    pub index_by_slot: Vec<Option<usize>>,
    pub drive_by_slot: Vec<[f64; 3]>,
    pub look_by_slot: Vec<Option<[f64; 3]>>,
    classification: Vec<Option<(u32, Option<u32>)>>,
}

impl Columns {
    fn append(&mut self) {
        self.ids.push(EntityId(0));
        self.slots.push(None);
        self.factions.push(0);
        self.positions.push([0.0; 3]);
        self.targets.push(None);
        self.drive.push([0.0; 3]);
        self.ranged.push(false);
        self.reach.push(0.0);
        self.active.push(false);
        self.carried.push(false);
        self.targetable.push(false);
        self.classification.push(None);
    }

    fn prepare_unit(
        &mut self,
        frame: &Frame<'_>,
        bi: usize,
        id: EntityId,
        slot: Option<EntitySlot>,
    ) {
        if bi == self.ids.len() {
            self.append();
        }
        if self.ids[bi] != id {
            self.targets[bi] = None;
            self.classification[bi] = None;
        }
        self.ids[bi] = id;
        self.slots[bi] = slot;
        self.drive[bi] = [0.0; 3];
        self.active[bi] = false;
        self.targetable[bi] = false;
        self.carried[bi] = false;
        self.factions[bi] = 0;
        let Some(slot) = slot else {
            self.targets[bi] = None;
            return;
        };
        self.index_by_slot[slot.0 as usize] = Some(bi);
        let body = frame.entities.body(slot);
        let row = frame.entities.row(slot);
        self.positions[bi] = frame.entities.pos(slot);
        self.factions[bi] = body.faction;
        self.carried[bi] = row.entity_life.mount.rider.is_some();
        self.targetable[bi] = body.faction != 0
            && body.alive
            && row.health > 0.0
            && matches!(body.kind, EntityKind::Mob { .. })
            && !crate::axolotl::playing_dead(row);
        let chunk = frame.entities.chunk_of(slot);
        self.active[bi] = self.targetable[bi]
            && !row.controls.no_ai
            && frame.world.is_active_position(chunk.x, chunk.z);
        let type_id = match body.kind {
            EntityKind::Mob { type_id } => type_id,
            _ => u32::MAX,
        };
        let held = row.appearance.equipment[0]
            .as_ref()
            .filter(|stack| !stack.is_empty())
            .map(|stack| stack.item_id);
        let key = (type_id, held);
        if self.classification[bi] != Some(key) {
            self.ranged[bi] = super::combat::is_archer(frame, slot);
            self.classification[bi] = Some(key);
        }
        self.reach[bi] = super::engagement_reach(self.ranged[bi], row.attack_reach);
    }

    pub fn prepare(&mut self, frame: &Frame<'_>, formation: &FormationRuntime) {
        self.index_by_slot.resize(frame.entities.len(), None);
        self.index_by_slot.fill(None);
        let mut len = 0;
        for member in &formation.members {
            self.prepare_unit(frame, len, member.id, member.slot);
            len += 1;
        }
        for slot in frame.entities.battle_slots() {
            if self.index_by_slot[slot.0 as usize].is_some() {
                continue;
            }
            let body = frame.entities.body(slot);
            if !body.alive
                || frame.entities.row(slot).health <= 0.0
                || !matches!(body.kind, EntityKind::Mob { .. })
            {
                continue;
            }
            self.prepare_unit(frame, len, frame.entities.body(slot).id, Some(slot));
            len += 1;
        }
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
        self.clear_projection(frame.entities.len());
    }

    pub fn index(&self, slot: EntitySlot) -> Option<usize> {
        self.index_by_slot.get(slot.0 as usize).copied().flatten()
    }

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
