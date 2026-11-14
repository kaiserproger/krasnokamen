//! Tick-owned world borders: vanilla 26.1.2 `WorldBorder` semantics
//! (pinned server-26.1.2.jar SHA256
//! 4723380bd2a0a0206719b50f2e390383afdaf82b0a76a0d573baf788e6aa3e86).
//! `size` is the current extent; a timed `set`/`add` installs a
//! `MovingBorderExtent` (from size, target size, remaining ticks) that the
//! native tick advances once per tick. Damage/warning fields ride the same
//! struct so checkpoints and the init packet observe one owner.
use serde::{Deserialize, Serialize};

pub const MAX_SIZE: f64 = 59_999_968.0;
/// Vanilla `WorldBorder` constructor defaults (`WorldBorder.javap`: ctor
/// stores damagePerBlock 0.2, safeZone 5.0, warningTime 15, warningBlocks 5).
pub const DEFAULT_DAMAGE_PER_BLOCK: f64 = 0.2;
pub const DEFAULT_SAFE_ZONE: f64 = 5.0;
pub const DEFAULT_WARNING_TIME: i32 = 15;
pub const DEFAULT_WARNING_BLOCKS: i32 = 5;
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub center_x: f64,
    pub center_z: f64,
    pub size: f64,
    /// In-flight interpolation target; equals `size` when settled.
    pub lerp_target: f64,
    /// Remaining interpolation ticks; zero when settled.
    pub lerp_remaining: i64,
    /// Vanilla `damagePerBlock` (`worldborder damage amount`).
    pub damage_per_block: f64,
    /// Vanilla `safeZone` (`worldborder damage buffer`).
    pub safe_zone: f64,
    /// Vanilla `warningTime` in seconds (`worldborder warning time`).
    pub warning_time: i32,
    /// Vanilla `warningBlocks` (`worldborder warning distance`).
    pub warning_blocks: i32,
}
impl Default for Border {
    fn default() -> Self {
        Self {
            center_x: 0.0,
            center_z: 0.0,
            size: MAX_SIZE,
            lerp_target: MAX_SIZE,
            lerp_remaining: 0,
            damage_per_block: DEFAULT_DAMAGE_PER_BLOCK,
            safe_zone: DEFAULT_SAFE_ZONE,
            warning_time: DEFAULT_WARNING_TIME,
            warning_blocks: DEFAULT_WARNING_BLOCKS,
        }
    }
}
impl Border {
    pub fn validate(self) -> Result<(), crate::config::CoreError> {
        for (name, value) in [
            ("center_x", self.center_x),
            ("center_z", self.center_z),
            ("size", self.size),
            ("lerp_target", self.lerp_target),
            ("damage_per_block", self.damage_per_block),
            ("safe_zone", self.safe_zone),
        ] {
            if !value.is_finite() {
                return Err(crate::config::CoreError::Storage(format!(
                    "worldborder.{name}: non-finite number"
                )));
            }
        }
        if self.center_x.abs() > 30_000_000.0 || self.center_z.abs() > 30_000_000.0 {
            return Err(crate::config::CoreError::Storage(
                "worldborder.center: outside +/-30000000".into(),
            ));
        }
        if !(1.0..=MAX_SIZE).contains(&self.size) {
            return Err(crate::config::CoreError::Storage(
                "worldborder.size: outside 1..59999968".into(),
            ));
        }
        if !(1.0..=MAX_SIZE).contains(&self.lerp_target) {
            return Err(crate::config::CoreError::Storage(
                "worldborder.lerp_target: outside 1..59999968".into(),
            ));
        }
        if self.lerp_remaining < 0 {
            return Err(crate::config::CoreError::Storage(
                "worldborder.lerp_remaining: negative".into(),
            ));
        }
        if self.damage_per_block < 0.0 {
            return Err(crate::config::CoreError::Storage(
                "worldborder.damage_per_block: negative".into(),
            ));
        }
        if self.safe_zone < 0.0 {
            return Err(crate::config::CoreError::Storage(
                "worldborder.safe_zone: negative".into(),
            ));
        }
        if self.warning_time < 0 || self.warning_blocks < 0 {
            return Err(crate::config::CoreError::Storage(
                "worldborder.warning: negative".into(),
            ));
        }
        Ok(())
    }
    /// Advance one native tick of an in-flight interpolation, matching
    /// vanilla `MovingBorderExtent` linear progress (`size +=
    /// (target - size) / remaining; remaining -= 1`, settling exact).
    pub fn advance_tick(&mut self) {
        if self.lerp_remaining <= 0 {
            self.size = self.lerp_target;
            return;
        }
        self.lerp_remaining -= 1;
        if self.lerp_remaining == 0 {
            self.size = self.lerp_target;
        } else {
            self.size += (self.lerp_target - self.size) / (self.lerp_remaining + 1) as f64;
        }
    }
    /// Vanilla `WorldBorder.isWithinBounds(x, z)`: `minX <= x < maxX` and
    /// `minZ <= z < maxZ`, each edge `centre -/+ size / 2` clamped to the
    /// absolute limit (`MAX_SIZE / 2`).
    #[must_use]
    pub fn is_within_bounds(&self, x: f64, z: f64) -> bool {
        let limit = MAX_SIZE / 2.0;
        let half = self.size / 2.0;
        let edge = |centre: f64, sign: f64| (centre + sign * half).clamp(-limit, limit);
        x >= edge(self.center_x, -1.0)
            && x < edge(self.center_x, 1.0)
            && z >= edge(self.center_z, -1.0)
            && z < edge(self.center_z, 1.0)
    }
}
#[cfg(test)]
use crate::command_execution::execute_border as execute;
#[cfg(test)]
use crate::commands::{BorderCommand, parse_border as parse};

#[cfg(test)]
#[path = "command_border_tests.rs"]
mod tests;
