//! Explicit test-only settlement acceleration, independent of gameplay state.
//! All-one settings preserve normal gameplay; accelerated runs are not acceptance.

/// Test-only native settlement multipliers, parsed as one configuration section.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
#[serde(default)]
pub struct TestAcceleration {
    pub gathering_yield: f64,
    pub construction_speed: f64,
    pub birth_chance: f64,
    pub migration_rate: f64,
}

impl Default for TestAcceleration {
    fn default() -> Self {
        Self {
            gathering_yield: 1.0,
            construction_speed: 1.0,
            birth_chance: 1.0,
            migration_rate: 1.0,
        }
    }
}

impl TestAcceleration {
    /// The host parser uses this list for unknown-leaf diagnostics.
    pub const KEYS: &[&str] = &[
        "gathering_yield",
        "construction_speed",
        "birth_chance",
        "migration_rate",
    ];

    /// Reject invalid operator input before any world is opened.
    pub fn validate(&self) -> Result<(), String> {
        for (key, value) in Self::KEYS.iter().zip([
            self.gathering_yield,
            self.construction_speed,
            self.birth_chance,
            self.migration_rate,
        ]) {
            if !value.is_finite() || value <= 0.0 {
                return Err(format!(
                    "test_acceleration.{key} must be finite and positive"
                ));
            }
        }
        Ok(())
    }

    /// Validates, then prints all four values on stderr. Not a `tracing` event,
    /// so no log filter (`RUST_LOG=warn`) can hide the disclosure.
    pub(crate) fn startup(&self) -> Result<(), String> {
        self.validate()?;
        eprintln!(
            "test_acceleration gathering_yield={} construction_speed={} birth_chance={} migration_rate={}",
            self.gathering_yield, self.construction_speed, self.birth_chance, self.migration_rate,
        );
        Ok(())
    }

    pub(crate) fn gathered_count(&self, count: i32, seed: u64) -> i32 {
        if self.gathering_yield == 1.0 {
            return count;
        }
        scaled_units(f64::from(count) * self.gathering_yield, seed).min(i32::MAX as u32) as i32
    }

    pub(crate) fn construction_steps(&self, seed: u64) -> u32 {
        if self.construction_speed == 1.0 {
            return 1;
        }
        scaled_units(self.construction_speed, seed)
    }

    pub(crate) fn birth_ticks(&self, ticks: u64) -> u64 {
        if self.birth_chance == 1.0 {
            return ticks;
        }
        ((ticks as f64 / self.birth_chance).ceil() as u64).max(1)
    }
}

/// Integer units plus a deterministic fractional draw. SplitMix64 is a pure
/// seed finalizer here, not another gameplay RNG or a dependency back into it.
fn scaled_units(value: f64, seed: u64) -> u32 {
    let whole = value as u32;
    if whole == u32::MAX || value.fract() == 0.0 {
        return whole;
    }
    let mut mixed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
    mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    let draw = ((mixed ^ (mixed >> 31)) >> 11) as f64 / (1u64 << 53) as f64;
    whole + u32::from(draw < value.fract())
}

#[cfg(test)]
#[path = "test_acceleration_tests.rs"]
mod tests;
