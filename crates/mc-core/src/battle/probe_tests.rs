//! The phase-log facing probe (probe.rs).

use super::*;
use crate::battle::staging;

const ORIGIN: [f64; 3] = [0.5, 100.0, 0.5];

/// receipts/BattleTurnCapD36/phase_facing.py reads these keys in this order.
#[test]
fn phase_log_counters_keep_the_analyzer_keys() {
    assert_eq!(
        Counts::default().to_string(),
        "standing=0 switches=0 transitions=0 transitions_yaw20=0 transitions_yaw45=0 transitions_head20=0 windups=0 windups_head20=0 windups_head45=0 windups_body20=0 windups_body45=0"
    );
}

/// Every windup a foot soldier starts is seen on its own tick, and rows with
/// a target are classified standing or driven.
#[test]
fn a_staged_battle_counts_every_foot_windup_on_its_tick() {
    let mut state = crate::battle::tests::staged_flat_fixture(30);
    let mut changes = crate::boundary::ChangeSet::default();
    staging::execute(
        &mut state,
        &mut changes,
        ORIGIN,
        &staging::BattleCommand::Start,
    )
    .unwrap();
    let mut probe = Probe::default();
    let (mut total, mut swings) = (Counts::default(), 0);
    for _ in 0..600 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick: battle.elapsed_ticks - 1,
        };
        probe.observe(&frame, battle);
        let counts = probe.counts;
        total.standing += counts.standing;
        total.transitions += counts.transitions;
        total.windups += counts.windups;
        total.windups_head[0] += counts.windups_head[0];
        swings += battle.combat.work.swings;
    }
    assert!(total.standing > 0, "{total}");
    assert!(total.windups > 0, "{total}");
    assert_eq!(total.windups, swings, "{total}");
    assert!(total.windups_head[0] <= total.windups, "{total}");
}
