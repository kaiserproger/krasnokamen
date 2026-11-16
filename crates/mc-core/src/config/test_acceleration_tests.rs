use super::*;
use crate::config::{CoreConfig, CoreError};

const CHILD: &str = "config::test_acceleration::tests::test_acceleration_startup_child";
const CHILD_VALUES: &str = "KRASNOKAMEN_TEST_ACCELERATION_CHILD";

/// Child process of the test below: opens a core under a warn-only subscriber,
/// the filter `RUST_LOG=warn` gives the server binary.
#[test]
#[ignore = "run only as the child of test_acceleration_startup_line_survives_a_warn_log_filter"]
fn test_acceleration_startup_child() {
    let values: Vec<f64> = std::env::var(CHILD_VALUES)
        .unwrap()
        .split(',')
        .map(|value| value.parse().unwrap())
        .collect();
    let root = tempfile::tempdir().unwrap();
    let mut config = CoreConfig::new(root.path().join(".krasnokamen-conversion-log-probe"), 0);
    config.test_acceleration = TestAcceleration {
        gathering_yield: values[0],
        construction_speed: values[1],
        birth_chance: values[2],
        migration_rate: values[3],
    };
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .with_writer(std::io::stderr)
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        // Refuse staging before opening a world or spawning any threads.
        assert!(matches!(
            crate::Core::open(config),
            Err(CoreError::Storage(_))
        ));
    });
}

#[test]
fn test_acceleration_startup_line_survives_a_warn_log_filter() {
    for acceleration in [
        TestAcceleration::default(),
        TestAcceleration {
            gathering_yield: 4.0,
            construction_speed: 3.0,
            birth_chance: 2.0,
            migration_rate: 5.0,
        },
    ] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CHILD, "--ignored", "--nocapture"])
            .env(
                CHILD_VALUES,
                format!(
                    "{},{},{},{}",
                    acceleration.gathering_yield,
                    acceleration.construction_speed,
                    acceleration.birth_chance,
                    acceleration.migration_rate,
                ),
            )
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{text}");
        let lines: Vec<_> = text
            .lines()
            .filter(|line| line.contains("test_acceleration "))
            .collect();
        assert_eq!(
            lines,
            [format!(
                "test_acceleration gathering_yield={} construction_speed={} birth_chance={} migration_rate={}",
                acceleration.gathering_yield,
                acceleration.construction_speed,
                acceleration.birth_chance,
                acceleration.migration_rate,
            )],
            "{text}"
        );
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
