use super::*;
use crate::config::{CoreConfig, CoreError};
use std::io::Write;

#[derive(Clone, Default)]
struct Log(std::sync::Arc<parking_lot::Mutex<Vec<u8>>>);

impl Write for Log {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn test_acceleration_startup_always_logs_all_four_values_once() {
    for acceleration in [
        TestAcceleration::default(),
        TestAcceleration {
            gathering_yield: 4.0,
            construction_speed: 3.0,
            birth_chance: 2.0,
            migration_rate: 5.0,
        },
    ] {
        let root = tempfile::tempdir().unwrap();
        let mut config = CoreConfig::new(root.path().join(".krasnokamen-conversion-log-probe"), 0);
        config.test_acceleration = acceleration;
        let log = Log::default();
        let writer = log.clone();
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            // Refuse staging before opening a world or spawning any threads.
            assert!(matches!(
                crate::Core::open(config),
                Err(CoreError::Storage(_))
            ));
        });
        let text = String::from_utf8(log.0.lock().clone()).unwrap();
        let lines: Vec<_> = text
            .lines()
            .filter(|line| line.contains("test_acceleration "))
            .collect();
        assert_eq!(lines.len(), 1, "{text}");
        assert!(lines[0].ends_with(&format!(
            "test_acceleration gathering_yield={} construction_speed={} birth_chance={} migration_rate={}",
            acceleration.gathering_yield, acceleration.construction_speed,
            acceleration.birth_chance, acceleration.migration_rate,
        )), "{text}");
    }
}

#[test]
fn test_acceleration_invalid_core_input_never_opens_world() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("world");
    let mut config = CoreConfig::new(path.clone(), 0);
    config.test_acceleration.gathering_yield = f64::NAN;
    assert!(matches!(
        crate::Core::open(config),
        Err(CoreError::Config(_))
    ));
    assert!(!path.exists());
}

#[test]
fn test_acceleration_integer_fractional_and_default_units_are_deterministic() {
    let normal = TestAcceleration::default();
    for seed in 0..100 {
        assert_eq!(normal.gathered_count(3, seed), 3);
        assert_eq!(normal.construction_steps(seed), 1);
        assert_eq!(normal.birth_ticks(6_000), 6_000);
        assert_eq!(scaled_units(4.0, seed), 4);
    }
    assert_eq!(scaled_units(0.5, 0), 0);
    assert_eq!(scaled_units(0.5, 3), 1);
    let outcomes: std::collections::BTreeSet<_> =
        (0..100).map(|seed| scaled_units(0.5, seed)).collect();
    assert_eq!(outcomes, std::collections::BTreeSet::from([0, 1]));
    let fast = TestAcceleration {
        birth_chance: f64::MAX,
        ..normal
    };
    assert_eq!(fast.birth_ticks(6_000), 1);
    let slow = TestAcceleration {
        birth_chance: f64::MIN_POSITIVE,
        ..normal
    };
    assert_eq!(slow.birth_ticks(6_000), u64::MAX);
}
