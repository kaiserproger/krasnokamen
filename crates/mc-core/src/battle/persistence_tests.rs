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

fn started(side: usize) -> crate::tick::TickState {
    let mut state = crate::battle::tests::staged_flat_fixture(side);
    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Start,
    )
    .unwrap();
    state
}

fn parsed(image: &str) -> serde_json::Value {
    serde_json::from_str(image).unwrap()
}

/// A running battle journals its whole image once after a checkpoint capture
/// and then only the parts that changed: a soldier standing guard does not
/// re-journal his fighter record every tick (his fight clock moves only when
/// he enters or leaves the fight loop). Folding the records over the
/// checkpoint row reproduces the live image after every tick, also while the
/// lists shrink, shift and grow: a horse leaves the columns, a roster member
/// dies, the battle resets and a new one is staged and started.
#[test]
fn battle_wal_patches_changed_parts_and_replays_the_live_image() {
    let mut state = started(100);
    for _ in 0..20 {
        crate::tick::run_tick(&mut state, &[]);
    }
    // An established baseline, so the capture below must force the full image.
    journal_entries(&mut state, &mut Vec::new()).unwrap();
    let tick = state.tick;
    let rows = checkpoint_rows(&mut state, tick).unwrap();
    let mut replay = Replay {
        battle: checkpoint_image(&rows, state.dimension, tick).unwrap(),
        ..Replay::default()
    };
    let fighters = state.hostile.battle.combat.fighters.len();
    let full = encode(&state).unwrap().len();
    let (mut patches, mut patch_bytes, mut patched_fighters) = (0, 0, 0);
    for step in 0..30 {
        crate::tick::run_tick(&mut state, &[]);
        let mut entries = Vec::new();
        journal_entries(&mut state, &mut entries).unwrap();
        assert_eq!(entries.len(), 1, "a running battle changes every tick");
        match &entries[0] {
            JournalEntry::Battle { .. } => {
                assert_eq!(step, 0, "only the first record after a capture is full");
            }
            JournalEntry::BattlePatch { value } => {
                assert_ne!(
                    step, 0,
                    "the first record after a capture is the full image"
                );
                patches += 1;
                patch_bytes += value.to_string().len();
                patched_fighters += value["lists"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|list| list["list"] == "fighters")
                    .map(|list| list["set"].as_array().map_or(0, Vec::len))
                    .sum::<usize>();
            }
            other => panic!("unexpected battle record {other:?}"),
        }
        replay = crate::journal::apply_batch_images(replay, &[(state.tick, state.tick, entries)]);
        assert!(replay.battle_fault.is_none(), "{:?}", replay.battle_fault);
        assert!(
            parsed(&image(&replay).unwrap()) == parsed(&encode(&state).unwrap()),
            "tick {}: the folded records reproduce the live image",
            state.tick
        );
    }
    eprintln!(
        "battle WAL: {fighters} fighters, full image {full} B, {patches} patches \
         {} B/tick, {} fighter records/tick",
        patch_bytes / patches,
        patched_fighters / patches
    );
    assert!(
        patched_fighters * 4 <= patches * fighters,
        "a steady fight must not re-journal every fighter: {patched_fighters} records in {patches} ticks of {fighters}"
    );
    assert!(
        patch_bytes * 4 <= patches * full,
        "a tick's patch must stay far below the full image: {patch_bytes} B in {patches} ticks, full {full} B"
    );

    let horse = state
        .entities
        .rider_slots()
        .next()
        .expect("a cavalry horse");
    let horse_id = state.entities.body(horse).id;
    assert!(state.hostile.battle.columns.ids.contains(&horse_id));
    kill(&mut state, horse);
    crate::tick::run_tick(&mut state, &[]);
    assert!(
        !state.hostile.battle.columns.ids.contains(&horse_id),
        "the dead horse left the execution columns"
    );
    replay = fold_and_compare(&mut state, replay, "horse left the columns");

    let member = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .find_map(|member| state.entities.slot_of(member.id))
        .expect("a living roster member");
    let member_id = state.entities.body(member).id;
    kill(&mut state, member);
    crate::tick::run_tick(&mut state, &[]);
    assert!(
        parsed(&encode(&state).unwrap())["removed"]
            .as_array()
            .unwrap()
            .contains(&member_id.0.into()),
        "the dead member is a removed roster ID"
    );
    replay = fold_and_compare(&mut state, replay, "roster member died");

    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Reset,
    )
    .unwrap();
    assert!(state.hostile.battle.formation.members.is_empty());
    replay = fold_and_compare(&mut state, replay, "battle reset");

    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Setup { seed: 7, count: 20 },
    )
    .unwrap();
    let desired = state.desired_chunks();
    state.world.ensure_blocking(&desired);
    state.refresh_active_chunks(state.tick);
    replay = fold_and_compare(&mut state, replay, "new battle staged");
    for _ in 0..8 {
        crate::tick::run_tick(&mut state, &[]);
        replay = fold_and_compare(&mut state, replay, "new battle grounding");
    }
    execute(
        &mut state,
        &mut ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &BattleCommand::Start,
    )
    .unwrap();
    for _ in 0..3 {
        crate::tick::run_tick(&mut state, &[]);
        replay = fold_and_compare(&mut state, replay, "new battle running");
    }
    assert_eq!(state.hostile.battle.formation.members.len(), 40);
}

fn kill(state: &mut crate::tick::TickState, slot: crate::entities::EntitySlot) {
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
}

/// Journal one boundary inside the same WAL tail (patches only, no full image),
/// fold it and compare the folded image with the live one.
fn fold_and_compare(state: &mut crate::tick::TickState, replay: Replay, step: &str) -> Replay {
    let mut entries = Vec::new();
    journal_entries(state, &mut entries).unwrap();
    assert!(
        entries
            .iter()
            .all(|entry| matches!(entry, JournalEntry::BattlePatch { .. })),
        "{step}: a tail continues with patches: {entries:?}"
    );
    let replay = crate::journal::apply_batch_images(replay, &[(state.tick, state.tick, entries)]);
    assert!(
        replay.battle_fault.is_none(),
        "{step}: {:?}",
        replay.battle_fault
    );
    assert!(
        parsed(&image(&replay).unwrap()) == parsed(&encode(state).unwrap()),
        "{step}: the folded records reproduce the live image"
    );
    replay
}

/// A patch builds only on a full image earlier in the WAL tail; a bad index,
/// an unset grown element, an unknown list or a length past capacity refuses
/// recovery instead of installing a partial history.
#[test]
fn battle_wal_patch_refuses_without_a_full_image_or_with_bad_elements() {
    let mut state = started(100);
    crate::tick::run_tick(&mut state, &[]);
    let tick = state.tick;
    let rows = checkpoint_rows(&mut state, tick).unwrap();
    let base = checkpoint_image(&rows, state.dimension, tick)
        .unwrap()
        .unwrap();
    let ids = state.hostile.battle.combat.ids.len();
    assert!(ids > 0, "a running battle has execution columns");
    let fold = |full: bool, patch: serde_json::Value| {
        let mut entries = Vec::new();
        if full {
            entries.push(JournalEntry::Battle {
                value: Arc::clone(&base),
            });
        }
        entries.push(JournalEntry::BattlePatch { value: patch });
        let replay = crate::journal::apply_batch_images(
            Replay {
                battle: Some(Arc::clone(&base)),
                entities: member_records(&state),
                ..Replay::default()
            },
            &[(tick + 1, tick + 1, entries)],
        );
        validate_replay(&replay, &state.registry).map_err(|error| error.to_string())
    };
    fold(true, serde_json::json!({})).expect("an empty patch on a full image recovers");
    let refused = |base, patch, expected: &str| {
        let error = fold(base, patch).unwrap_err();
        assert!(error.contains(expected), "{error}");
    };
    refused(
        false,
        serde_json::json!({}),
        "without a full image earlier in the WAL tail",
    );
    refused(
        true,
        serde_json::json!({"lists": [{"list": "ids", "len": 1, "set": [[5, 7]]}]}),
        "index 5 past length 1",
    );
    refused(
        true,
        serde_json::json!({"lists": [{"list": "ids", "len": ids + 1, "set": []}]}),
        "invalid type: null",
    );
    refused(
        true,
        serde_json::json!({"lists": [{"list": "horses", "len": 0, "set": []}]}),
        "malformed battle patch",
    );
    refused(
        true,
        serde_json::json!({"lists": [{"list": "members", "len": 1_000_000, "set": []}]}),
        "exceeds capacity",
    );
}

/// The element with its guard deadline blanked: a change that moves nothing but
/// `action.until` is a guard re-stamp, not a decision.
fn without_hold(element: &serde_json::Value) -> serde_json::Value {
    let mut stripped = element.clone();
    if let Some(until) = stripped
        .get_mut("action")
        .and_then(|action| action.get_mut("until"))
    {
        *until = serde_json::Value::from(0);
    }
    stripped
}

/// Guard churn (D20, 2026-11-17): a routed man's guard deadline is dead state
/// while he flees — the second pass skips him on `routing` before reading it —
/// so a mass rout must not re-journal every fugitive's fighter record each
/// tick. One stamp per flight, resolved when a pass can read it, and the
/// folded records still reproduce the live image.
#[test]
fn battle_wal_does_not_rejournal_a_routing_fighter_every_tick() {
    use crate::battle::combat::Action;
    let mut state = started(100);
    for _ in 0..20 {
        crate::tick::run_tick(&mut state, &[]);
    }
    journal_entries(&mut state, &mut Vec::new()).unwrap();
    let tick = state.tick;
    let rows = checkpoint_rows(&mut state, tick).unwrap();
    let mut replay = Replay {
        battle: checkpoint_image(&rows, state.dimension, tick).unwrap(),
        ..Replay::default()
    };
    // A mass rout: break every regiment of one side, as `cavalry_tests` does.
    for regiment in &mut state.hostile.battle.formation.regiments {
        if regiment.spec.faction == 2 {
            regiment.morale.broken = true;
        }
    }
    let mut routed = 0;
    for _ in 0..120 {
        crate::tick::run_tick(&mut state, &[]);
        routed = state
            .hostile
            .battle
            .formation
            .members
            .iter()
            .filter(|member| member.routing)
            .count();
        if routed > 8 {
            break;
        }
    }
    assert!(
        routed > 8,
        "the forced break must put men to flight: {routed}"
    );
    let mut mirror: Vec<serde_json::Value> = Vec::new();
    let (mut elements, mut guard_only, mut held) = (0_usize, 0_usize, 0_usize);
    for _ in 0..30 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        // Invariant of the change: a routed man outside the bounds' bay keeps a
        // guard, so flight still suppresses his attack.
        for bi in 0..battle.formation.members.len() {
            let context = battle.formation.context_at(bi);
            if context.routing && !battle.formation.cornered_at(bi) {
                assert!(
                    matches!(battle.combat.fighters[bi].action, Action::Guard { .. }),
                    "a routing man {bi} holds a guard"
                );
                held += 1;
            }
        }
        // Which fighters cannot act (`context.routing`) and are not at bay.
        let holding: Vec<bool> = (0..battle.formation.members.len())
            .map(|bi| {
                let member = &battle.formation.members[bi];
                (member.routing || !member.alive || member.wounded)
                    && !battle.formation.cornered_at(bi)
            })
            .collect();
        let mut entries = Vec::new();
        journal_entries(&mut state, &mut entries).unwrap();
        for entry in &entries {
            match entry {
                JournalEntry::Battle { value } => {
                    let image: serde_json::Value = serde_json::from_str(value).unwrap();
                    mirror = image["battle"]["combat"]["fighters"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();
                }
                JournalEntry::BattlePatch { value } => {
                    for list in value["lists"].as_array().into_iter().flatten() {
                        if list["list"] != "fighters" {
                            continue;
                        }
                        let len = list["len"].as_u64().unwrap() as usize;
                        mirror.resize(len, serde_json::Value::Null);
                        for pair in list["set"].as_array().unwrap() {
                            let index = pair[0].as_u64().unwrap() as usize;
                            let element = &pair[1];
                            elements += 1;
                            let previous = mirror.get(index);
                            let routing = holding.get(index).copied().unwrap_or(false);
                            if routing
                                && previous.is_some_and(|previous| {
                                    !previous.is_null()
                                        && without_hold(previous) == without_hold(element)
                                })
                            {
                                guard_only += 1;
                            }
                            mirror[index] = element.clone();
                        }
                    }
                }
                other => panic!("unexpected battle record {other:?}"),
            }
        }
        replay = crate::journal::apply_batch_images(replay, &[(state.tick, state.tick, entries)]);
        assert!(
            replay.battle_fault.is_none(),
            "{:?}",
            replay.battle_fault
        );
        assert!(
            parsed(&image(&replay).unwrap()) == parsed(&encode(&state).unwrap()),
            "tick {}: the folded records reproduce the live image",
            state.tick
        );
    }
    assert!(held > 8, "the rout must hold men in flight: {held}");
    assert!(elements > 0, "the rout must journal fighter records");
    assert_eq!(
        guard_only, 0,
        "{guard_only} of {elements} fighter elements changed nothing but `action.until` during a mass rout"
    );
}
