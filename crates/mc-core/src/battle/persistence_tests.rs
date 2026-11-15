use super::*;
use crate::battle::staging::{BattleCommand, execute};
use crate::boundary::ChangeSet;

#[test]
fn battle_history_roundtrip_preserves_decisions_and_rebuilds_native_slots() {
    let mut state = crate::battle::tests::staged_flat_fixture(100);
    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Start,
    )
    .unwrap();
    for _ in 0..20 {
        crate::tick::run_tick(&mut state, &[]);
    }
    let image = encode(&state).unwrap();
    let restored = decode(&image).unwrap();
    assert!(restored.running);
    assert_eq!(restored.elapsed_ticks, state.hostile.battle.elapsed_ticks);
    assert_eq!(restored.formation.members.len(), 200);
    assert_eq!(
        serde_json::to_value(&restored).unwrap(),
        serde_json::to_value(&state.hostile.battle).unwrap()
    );
    state.hostile.battle = restored;
    restore(&mut state).unwrap();
    assert!(
        state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .all(|m| m.slot.is_some())
    );
    let status = execute(
        &mut state,
        &mut ChangeSet::default(),
        [0.0; 3],
        &BattleCommand::Setup {
            seed: 99,
            count: 100,
        },
    )
    .unwrap();
    assert!(status.starts_with("Battle restored:"), "{status}");
    crate::tick::run_tick(&mut state, &[]);
    assert_eq!(state.hostile.battle.elapsed_ticks, 21);
}

#[test]
fn battle_history_refuses_unknown_version_and_inconsistent_roster() {
    let state = crate::battle::tests::staged_flat_fixture(100);
    let original: serde_json::Value = serde_json::from_str(&encode(&state).unwrap()).unwrap();
    let mut image = original.clone();
    image["version"] = 2.into();
    assert!(
        decode(&image.to_string())
            .unwrap_err()
            .to_string()
            .contains("unsupported version")
    );
    let mut image = original;
    image["battle"]["formation"]["members"][0]["regiment"] = 999.into();
    assert!(
        decode(&image.to_string())
            .unwrap_err()
            .to_string()
            .contains("invalid roster member")
    );
}

#[test]
fn battle_history_maximum_roster_fits_native_record_budget() {
    let mut state = crate::battle::tests::staged_flat_fixture(10_000);
    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Start,
    )
    .unwrap();
    crate::tick::run_tick(&mut state, &[]);
    let image = encode(&state).unwrap();
    eprintln!(
        "battle history: 20k roster native payload={} bytes",
        image.len()
    );
    assert!(
        image.len() <= 16 * 1024 * 1024,
        "20k is supported by staging and must fit the native record envelope"
    );
    decode(&image).unwrap();
}

#[test]
fn battle_resume_refuses_missing_live_entities_and_bad_checkpoint_metadata() {
    let state = crate::battle::tests::staged_flat_fixture(100);
    let image = encode(&state).unwrap();
    let replay = Replay {
        battle: Some(Arc::new(image.clone())),
        ..Replay::default()
    };
    assert!(
        validate_replay(&replay, &state.registry)
            .unwrap_err()
            .to_string()
            .contains("missing live member")
    );
    let mut rows = BTreeMap::from([(
        row_key(Dimension::Overworld),
        crate::native_checkpoint::record(40, image.into_bytes()),
    )]);
    assert!(
        checkpoint_image(&rows, Dimension::Overworld, 41)
            .unwrap_err()
            .to_string()
            .contains("invalid checkpoint row")
    );
    rows.insert(
        "battle/99".into(),
        crate::native_checkpoint::record(40, vec![]),
    );
    assert!(
        checkpoint_image(&rows, Dimension::Overworld, 40)
            .unwrap_err()
            .to_string()
            .contains("unsupported checkpoint row")
    );
}
