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

#[test]
fn battle_resume_capture_prunes_only_proven_ground_tombstones() {
    let mut state = crate::battle::tests::staged_flat_fixture(100);
    let dead = state.hostile.battle.formation.members[0].id;
    let alive = state.hostile.battle.formation.members[1].id;
    let unknown = EntityId(i32::MAX - 1);
    let slot = state.entities.slot_of(dead).unwrap();
    state.merge_damage.push((
        slot,
        crate::tick::DamagePiece {
            amount: 1000.0,
            kind: crate::survival::DamageKind::GenericKill,
            source: None,
            physical: None,
            custom_armor: false,
        },
    ));
    state.commit_merge_effects();
    assert!(!state.entities.body(slot).alive);
    state.hostile.battle.ground_pending = vec![dead, alive, unknown];
    state.hostile.battle.ground_refused = vec![dead, alive, unknown];
    let mut entries = Vec::new();
    journal_entries(&mut state, &mut entries).unwrap();
    assert_eq!(state.hostile.battle.ground_pending, [alive, unknown]);
    assert_eq!(state.hostile.battle.ground_refused, [alive, unknown]);

    // Checkpoint uses the same boundary and must not "repair" an unproven ID.
    for pending in [true, false] {
        state.hostile.battle = BattleState::default();
        if pending {
            state.hostile.battle.ground_pending.push(unknown);
        } else {
            state.hostile.battle.ground_refused.push(unknown);
        }
        let tick = state.tick;
        let rows = checkpoint_rows(&mut state, tick).unwrap();
        let replay = Replay {
            battle: checkpoint_image(&rows, state.dimension, tick).unwrap(),
            ..Replay::default()
        };
        assert!(
            validate_replay(&replay, &state.registry)
                .unwrap_err()
                .to_string()
                .contains("missing grounded body"),
            "unknown IDs remain fail-closed at both staging boundaries"
        );
    }
}

/// Recovery's view of the living roster: each member's record with its life,
/// health and side, as a checkpoint carries it. No other entity is known.
fn member_records(state: &crate::tick::TickState) -> BTreeMap<i32, crate::journal::ReplayEntity> {
    let entities = &state.entities;
    let members = &state.hostile.battle.formation.members;
    members
        .iter()
        .filter_map(|member| {
            let slot = entities.slot_of(member.id)?;
            let (body, row) = (entities.body(slot), entities.row(slot));
            let image = crate::journal::EntityStateImage {
                faction: body.faction,
                ..Default::default()
            };
            let record = crate::journal::ReplayEntity {
                uuid: row.uuid.to_string(),
                type_id: match body.kind {
                    crate::entities::EntityKind::Mob { type_id } => type_id,
                    _ => u32::MAX,
                },
                pos: entities.pos(slot),
                vel: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                ground: true,
                health: row.health,
                alive: body.alive,
                age: 0,
                breed_cooldown: 0,
                pickup_delay: 0,
                item: None,
                appearance: Default::default(),
                active_effects: Default::default(),
                villager: None,
                state: image,
                projectile: None,
                tick: state.tick,
            };
            Some((member.id.0, record))
        })
        .collect()
}

/// Only a column that just left clears a reference (combat `prepare`). An
/// identity that never was a column keeps its Windup or shield block through
/// the real tick in which a dead horse leaves the columns, and recovery of
/// the capture still refuses it.
#[test]
fn battle_resume_still_refuses_an_unknown_windup_target_or_block_attacker() {
    use crate::battle::combat::{Action, Weapon};
    let unknown = EntityId(i32::MAX - 1);
    let validate = |state: &mut crate::tick::TickState| {
        let tick = state.tick;
        let rows = checkpoint_rows(state, tick).unwrap();
        let replay = Replay {
            battle: checkpoint_image(&rows, state.dimension, tick).unwrap(),
            entities: member_records(state),
            ..Replay::default()
        };
        validate_replay(&replay, &state.registry)
    };
    for block in [false, true] {
        let mut state = crate::battle::tests::staged_flat_fixture(100);
        execute(
            &mut state,
            &mut ChangeSet::default(),
            [8.5, 64.0, 8.5],
            &BattleCommand::Start,
        )
        .unwrap();
        crate::tick::run_tick(&mut state, &[]);
        validate(&mut state).expect("the capture of a running battle recovers");
        let horse = state
            .entities
            .rider_slots()
            .next()
            .expect("a cavalry horse");
        let horse_id = state.entities.body(horse).id;
        assert!(state.hostile.battle.columns.ids.contains(&horse_id));
        state.merge_damage.push((
            horse,
            crate::tick::DamagePiece {
                amount: 1000.0,
                kind: crate::survival::DamageKind::GenericKill,
                source: None,
                physical: None,
                custom_armor: false,
            },
        ));
        state.commit_merge_effects();
        assert!(!state.entities.body(horse).alive);
        let battle = &mut state.hostile.battle;
        let elapsed = battle.elapsed_ticks;
        let fighter = &mut battle.combat.fighters[0];
        if block {
            crate::battle::combat::tests::set_block(fighter, Some((unknown, elapsed + 5)));
        } else {
            fighter.action = Action::Windup {
                target: unknown,
                start: elapsed,
                weapon: Weapon::Sword,
            };
        }
        crate::tick::run_tick(&mut state, &[]);
        assert!(
            !state.hostile.battle.columns.ids.contains(&horse_id),
            "the dead horse left the execution columns"
        );
        let error = validate(&mut state).unwrap_err().to_string();
        let expected = if block {
            "unknown shield-block attacker".to_string()
        } else {
            format!("unknown windup target {}", unknown.0)
        };
        assert!(error.contains(&expected), "{error}");
    }
}
