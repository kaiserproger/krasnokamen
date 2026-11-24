//! The derived hot column of one slot: the cold-row scalars a clean row's
//! `ReadRow` needs, refreshed only for dirty slots in
//! [`crate::entities::EntityStore::prepare_motion_hints`]. The cold row stays
//! the authority; `needs_cold` routes every row whose read touches retained
//! state outside this list to the live cold row.
use super::{
    Body, ColdRow, EntityKind, EntitySlot, EntityStore, MOTION_EFFECTS, MotionHint, motion_gate,
};

/// Bits of [`HotRow`].
const HOT_NO_AI: u16 = 1 << 0;
const HOT_NO_GRAVITY: u16 = 1 << 1;
const HOT_PERSISTENT: u16 = 1 << 2;
const HOT_VILLAGER: u16 = 1 << 3;
const HOT_SITTING: u16 = 1 << 4;
const HOT_TAMED: u16 = 1 << 5;
const HOT_LEASH: u16 = 1 << 6;
const HOT_RIDERS: u16 = 1 << 7;
const HOT_SPECIAL: u16 = 1 << 8;
const HOT_NEEDS_COLD: u16 = 1 << 9;

/// Derived cold-row scalars of one slot, refreshed only for dirty slots in
/// [`EntityStore::prepare_motion_hints`]. The cold row stays the authority: the
/// column is never serialized, and `needs_cold` routes every row whose
/// `ReadRow` reads retained state outside this list to the live cold row —
/// `age`, `harvest_ticks`, `no_action_time`, `uuid`, villager internals,
/// `active_effects`, `appearance` and live seat occupancy are never here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HotRow {
    speed: f32,
    sight_range: f32,
    attack_reach: f32,
    attack_damage: f32,
    love: u32,
    breed_cooldown: u32,
    child_ticks: u32,
    slime_size: Option<u8>,
    bits: u16,
}

impl HotRow {
    /// Projection of one cold row. `flags` is the motion column refreshed in the
    /// same pass, so `MOTION_EFFECTS` routes an effect row to the cold row.
    pub(crate) fn of(row: &ColdRow, body: &Body, flags: u16) -> Self {
        let special = row.special_mob.is_some();
        let riders = !row.entity_life.mount.riders.is_empty();
        let tamed = row.entity_life.tamed;
        let mut bits = 0;
        if row.controls.no_ai {
            bits |= HOT_NO_AI;
        }
        if row.controls.no_gravity {
            bits |= HOT_NO_GRAVITY;
        }
        if row.persistent {
            bits |= HOT_PERSISTENT;
        }
        if row.villager.is_some() {
            bits |= HOT_VILLAGER;
        }
        if row.entity_life.sitting {
            bits |= HOT_SITTING;
        }
        if tamed {
            bits |= HOT_TAMED;
        }
        if row.entity_life.leash.is_some() {
            bits |= HOT_LEASH;
        }
        if riders {
            bits |= HOT_RIDERS;
        }
        if special {
            bits |= HOT_SPECIAL;
        }
        if special
            || riders
            || tamed
            || flags & MOTION_EFFECTS != 0
            || matches!(body.kind, EntityKind::Mob { type_id } if crate::piglin::is_piglin_type(type_id))
        {
            bits |= HOT_NEEDS_COLD;
        }
        Self {
            speed: row.speed,
            sight_range: row.sight_range,
            attack_reach: row.attack_reach,
            attack_damage: row.attack_damage,
            love: row.love,
            breed_cooldown: row.breed_cooldown,
            child_ticks: row.entity_life.child_ticks,
            slime_size: row.loot.slime_size,
            bits,
        }
    }

    pub(crate) fn speed(&self) -> f32 {
        self.speed
    }
    pub(crate) fn sight_range(&self) -> f32 {
        self.sight_range
    }
    pub(crate) fn attack_reach(&self) -> f32 {
        self.attack_reach
    }
    pub(crate) fn attack_damage(&self) -> f32 {
        self.attack_damage
    }
    pub(crate) fn love(&self) -> u32 {
        self.love
    }
    pub(crate) fn breed_cooldown(&self) -> u32 {
        self.breed_cooldown
    }
    pub(crate) fn child_ticks(&self) -> u32 {
        self.child_ticks
    }
    pub(crate) fn slime_size(&self) -> Option<u8> {
        self.slime_size
    }
    pub(crate) fn no_ai(&self) -> bool {
        self.bits & HOT_NO_AI != 0
    }
    pub(crate) fn no_gravity(&self) -> bool {
        self.bits & HOT_NO_GRAVITY != 0
    }
    pub(crate) fn persistent(&self) -> bool {
        self.bits & HOT_PERSISTENT != 0
    }
    pub(crate) fn has_villager(&self) -> bool {
        self.bits & HOT_VILLAGER != 0
    }
    pub(crate) fn sitting(&self) -> bool {
        self.bits & HOT_SITTING != 0
    }
    pub(crate) fn has_leash(&self) -> bool {
        self.bits & HOT_LEASH != 0
    }
    /// The row's `ReadRow` reads retained state the column leaves out; its read
    /// takes the live cold row.
    pub(crate) fn needs_cold(&self) -> bool {
        self.bits & HOT_NEEDS_COLD != 0
    }
}

impl EntityStore {
    /// Whether the slot was mutated after `prepare_motion_hints` refreshed the
    /// derived columns; such a row's `flags`/`hints`/`hot` are stale on purpose
    /// and its read goes to the live cold row.
    pub(crate) fn motion_dirty(&self, slot: EntitySlot) -> bool {
        self.dirty_flag[slot.0 as usize]
    }

    /// Derived cold scalars of one slot. A clean slot's column must equal its
    /// cold projection: only `prepare_motion_hints` writes it, and every writer
    /// that can change a projected field marks the slot dirty.
    pub(crate) fn hot_row(&self, slot: EntitySlot) -> &HotRow {
        let index = slot.0 as usize;
        debug_assert!(
            self.dirty_flag[index]
                || self.hot[index]
                    == HotRow::of(&self.cold[index], &self.bodies[index], self.flags[index]),
            "a clean slot's hot row diverges from its cold projection"
        );
        &self.hot[index]
    }

    /// Refresh motion flags for dirty slots only. Resolves each queued
    /// stable id to its current slot through `id_index` (removed ids are
    /// ignored) and retains the queue capacity across ticks. The same pass
    /// rewrites the derived `hot` column for those slots.
    pub(crate) fn prepare_motion_hints(&mut self, registry: &crate::registry::CoreRegistry) {
        if self.dirty_ids.is_empty() {
            return;
        }
        static CHICKEN: std::sync::LazyLock<mc_data::Identifier> = std::sync::LazyLock::new(|| {
            mc_data::Identifier::parse("minecraft:chicken").expect("static identifier parses")
        });
        static TADPOLE: std::sync::LazyLock<mc_data::Identifier> = std::sync::LazyLock::new(|| {
            mc_data::Identifier::parse("minecraft:tadpole").expect("static identifier parses")
        });
        let chicken = registry.entity_types.id_of(&CHICKEN);
        let tadpole = registry.entity_types.id_of(&TADPOLE);
        let dirty = std::mem::take(&mut self.dirty_ids);
        for id in &dirty {
            let Some(slot) = self.id_index.get(id).copied() else {
                continue;
            };
            let index = slot.0 as usize;
            let Some(body) = self.bodies.get(index) else {
                continue;
            };
            if body.id != *id {
                continue;
            }
            self.dirty_flag[index] = false;
            let special = motion_gate::effective_special(registry, body, &self.cold[index]);
            let flags =
                motion_gate::motion_flags_for(registry, self, slot, special, chicken, tadpole);
            let hot = HotRow::of(&self.cold[index], body, flags);
            self.flags[index] = flags;
            self.hints[index] = MotionHint {
                special,
                swell_metadata_neutral: crate::hostile::swell_metadata_neutral(&self.cold[index]),
            };
            self.hot[index] = hot;
        }
        let mut dirty = dirty;
        dirty.clear();
        self.dirty_ids = dirty;
    }
}

/// Test-only: cold rows read through [`EntityStore::row`]. The derived column
/// answers a clean plain row, so a prepare reads none of them.
#[cfg(test)]
pub(crate) static COLD_ROW_READS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Test-only: cold reads since the last call.
#[cfg(test)]
pub(crate) fn take_cold_row_reads() -> usize {
    COLD_ROW_READS.swap(0, std::sync::atomic::Ordering::Relaxed)
}
