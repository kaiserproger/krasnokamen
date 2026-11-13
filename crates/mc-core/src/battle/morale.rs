//! Regiment morale. Loss and pressure follow research-combat-behavior.md §4.8.
//! Film casualty-rate factor 1 (Bannerlord 2) keeps a steady front in melee
//! to about 20% removed before it breaks; most killing then follows in pursuit.
//! Film nerve thresholds delay cascading rear flight, not human action clocks.
//! Contact modifiers are levels, not an endlessly accumulating tick penalty.

use super::formation::PULSE_MORALE_FLOOR;
use super::{BattleState, Frame, Output};

// Film nerve: front 3..18, rear 13..28 (the +10 rear bias of
// research-combat-behavior.md §4.8). The rear top must clear the pulse morale
// floor, or a worn regiment stops fighting before anyone can flee: in the
// native 20k film a 23 top never broke within three contact minutes. A 40 top
// (+22) broke the regiment losing its exchange at 11-18 % removed, its rear
// half, which does not fight, panicking first: at 1 500 a side both foot
// regiments of a side were gone within two minutes of contact.
const FRONT_NERVE_BASE: f32 = 3.0;
const NERVE_VARIATIONS: u32 = 16;
const CASUALTY_RATE: f32 = 1.0;
const REAR_NERVE_BONUS: f32 = 10.0;
// A regiment worn near its pulse floor while still in contact gives way within
// seconds instead of standing frozen with neither losses nor flight.
const WORN_MARGIN: f32 = 10.0;
const WORN_EROSION: f32 = 1.0;
// Watching the opposing regiment run steadies the victors: a symmetric melee
// tips into one rout and one pursuit instead of two simultaneous collapses.
const VICTORY_RECOVERY: f32 = 1.0;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Pressure {
    pub missiles: bool,
    pub shield_wall: bool,
    pub flank: bool,
    pub rear: bool,
    pub flanks_secure: bool,
    pub adjacent_rout: bool,
    /// The regiment's front touches an unbroken enemy this decision.
    pub engaged: bool,
    /// The nearest enemy regiment has broken this decision.
    pub enemy_broken: bool,
    pub panicking: u16,
}

#[derive(Debug, Clone)]
pub(crate) struct Morale {
    pub value: f32,
    cohesion: f32,
    pub broken: bool,
    pub removed: usize,
    pub kills: u16,
    pub missile_losses: usize,
    missile_removed: usize,
    pub pressure: Pressure,
    losses: [usize; 20],
    loss_epoch: u64,
}

impl Default for Morale {
    fn default() -> Self {
        Self {
            value: 75.0,
            cohesion: 75.0,
            broken: false,
            removed: 0,
            kills: 0,
            missile_losses: 0,
            missile_removed: 0,
            pressure: Pressure::default(),
            losses: [0; 20],
            loss_epoch: 0,
        }
    }
}

impl Morale {
    pub fn update(
        &mut self,
        tick: u64,
        strength: usize,
        removed: usize,
        stamina: f32,
        charging: bool,
    ) {
        let epoch = tick / 10;
        for step in self.loss_epoch + 1..=epoch.min(self.loss_epoch + 20) {
            self.losses[(step % 20) as usize] = 0;
        }
        if epoch.saturating_sub(self.loss_epoch) >= 20 {
            self.losses.fill(0);
        }
        self.loss_epoch = epoch;
        let new_losses = removed.saturating_sub(self.removed);
        self.losses[(epoch % 20) as usize] += new_losses;
        let scaling = 100.0 / strength.max(1) as f32;
        // Bannerlord: loss 4, melee .75, ranged .5, casualty-rate factor 2 (film: 1);
        // gains 3. Events are aggregate per regiment, normalized to 100 men.
        let ranged = self
            .missile_losses
            .saturating_sub(self.missile_removed)
            .min(new_losses);
        self.cohesion -= (4.0 * 0.75 * (new_losses - ranged) as f32 + 4.0 * 0.5 * ranged as f32)
            * CASUALTY_RATE
            * scaling;
        self.cohesion += 3.0 * f32::from(self.kills.min(10)) * scaling;
        self.cohesion -= (f32::from(self.pressure.panicking) * 1.1).min(10.0);
        if self.pressure.adjacent_rout {
            self.cohesion -= 10.0;
        }
        if self.pressure.engaged && self.value < PULSE_MORALE_FLOOR + WORN_MARGIN {
            self.cohesion -= WORN_EROSION;
        }
        if self.pressure.enemy_broken {
            self.cohesion += VICTORY_RECOVERY;
        }
        let loss_fraction = removed as f32 / strength.max(1) as f32;
        let cap = (100.0 - 300.0 * loss_fraction).max(0.0);
        self.cohesion = self.cohesion.clamp(0.0, cap);
        let p = self.pressure;
        let mut modifier = if p.flanks_secure { 5.0 } else { 0.0 };
        modifier -= if p.flank { 6.0 } else { 0.0 };
        modifier -= if p.rear { 14.0 } else { 0.0 };
        modifier -= if p.missiles {
            if p.shield_wall { 4.0 } else { 8.0 }
        } else {
            0.0
        };
        modifier -= if stamina < 20.0 {
            15.0
        } else if stamina < 45.0 {
            5.0
        } else {
            0.0
        };
        if self.losses.iter().sum::<usize>() as f32 > strength as f32 * 0.02 {
            modifier -= 5.0;
        }
        if charging {
            modifier += 15.0;
        }
        self.value = (self.cohesion + modifier).clamp(0.0, cap);
        self.broken |= self.value < 17.0;
        self.removed = removed;
        self.kills = 0;
        self.missile_removed = self.missile_losses;
        self.pressure = Pressure::default();
    }
}

/// Rear ranks have the higher flight threshold. Stable per-soldier variation
/// produces a trickle before the regiment breaks, not one simultaneous switch.
pub(crate) fn nerve(id: i32, rank: u16, ranks: u16) -> f32 {
    let variation = (id as u32).wrapping_mul(2_654_435_761) % NERVE_VARIATIONS;
    FRONT_NERVE_BASE
        + variation as f32
        + if rank * 2 >= ranks {
            REAR_NERVE_BONUS
        } else {
            0.0
        }
}

pub(crate) fn step(frame: &Frame<'_>, battle: &mut BattleState, _out: &mut Output) {
    battle.formation.update_morale(frame.tick);
}

#[cfg(test)]
#[path = "morale_tests.rs"]
mod tests;
