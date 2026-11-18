//! The per-record structure ledger: what a tick journals, how the WAL replays
//! it, which formats refuse, and how the watermark replaces stored receipts.

use super::*;
use crate::plugin_structures::Replica;

fn keys(entries: &[(&str, bool)]) -> BTreeMap<String, bool> {
    entries
        .iter()
        .map(|(key, present)| ((*key).to_owned(), *present))
        .collect()
}

fn surveyed(state: &mut TickState, at: [i32; 3]) -> ScriptSurveySnapshot {
    let bounds = ScriptWorldBounds::new(at, at).unwrap();
    let (_, result) = survey(state, "demo", "minecraft:overworld", bounds).unwrap();
    let ScriptWorldStructureResult::Survey { survey } = result else {
        panic!("survey")
    };
    *survey
}

fn lifecycle(id: &str, revision: u64, kind: &str) -> ScriptWorldStructureOperation {
    let (operation_id, structure_id) = (format!("{kind}-{revision}"), id.to_owned());
    match kind {
        "pause" => ScriptWorldStructureOperation::PauseStructure {
            operation_id,
            structure_id,
            expected_revision: revision,
        },
        "resume" => ScriptWorldStructureOperation::ResumeStructure {
            operation_id,
            structure_id,
            expected_revision: revision,
        },
        _ => ScriptWorldStructureOperation::CancelStructure {
            operation_id,
            structure_id,
            expected_revision: revision,
        },
    }
}

fn record(state: &TickState, id: &str) -> ScriptStructureSnapshot {
    ledger(state).unwrap().structures()[id].snapshot.clone()
}

/// Every writer of the plugin path, one WAL boundary each: the WAL replica
/// equals the live cache after each, and each boundary carries exactly the
/// records it wrote plus the meta key.
#[test]
fn every_ledger_write_replays_from_the_wal_to_the_live_cache() {
    let mut state = state();
    let mut replica = Replica::capture(&mut state);
    replica.assert_matches(&state);
    assert!(
        replica.step(&mut state).keys.is_empty(),
        "a fresh world journals no ledger key"
    );

    let id = stored_structure(&mut state);
    let structure = ledger::structure_key(&id);
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), (&structure, true)])
    );
    replica.assert_matches(&state);

    let at = anchor(&state);
    let stone = state.registry.states.stone;
    state.world.set_block(at[0] + 3, at[1], at[2] + 3, stone);
    // A floor beside the worksite block, so a worker can stand next to it.
    state
        .world
        .set_block(at[0] + 4, at[1] - 1, at[2] + 3, stone);
    let first = surveyed(&mut state, [at[0] + 3, at[1], at[2] + 3]);
    let first_key = format!("survey/{}", first.survey_token);
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), (&first_key, true)])
    );
    replica.assert_matches(&state);

    // Several writes of one record inside one boundary journal its final image once.
    let revision = record(&state, &id).revision;
    execute_operation(&mut state, "demo", &lifecycle(&id, revision, "pause")).unwrap();
    let paused = record(&state, &id).revision;
    execute_operation(&mut state, "demo", &lifecycle(&id, paused, "resume")).unwrap();
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), (&structure, true)])
    );
    replica.assert_matches(&state);
    assert_eq!(record(&state, &id).state, ScriptStructureState::Running);

    // A record inserted and removed inside one boundary leaves a removal.
    let mut local = ledger_mut(&mut state).unwrap();
    let mut temporary = local.structures()[&id].clone();
    temporary.snapshot.structure_id = "temporary".into();
    local.insert_structure("temporary".into(), temporary);
    save(&mut state, local);
    let mut local = ledger_mut(&mut state).unwrap();
    assert!(local.take_structure("temporary").is_some());
    save(&mut state, local);
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), ("structure/temporary", false)])
    );
    replica.assert_matches(&state);

    // A writer that fails before save publishes nothing.
    let before = ledger_domain(&state);
    let current = record(&state, &id).revision;
    assert_eq!(
        execute_operation(&mut state, "demo", &lifecycle(&id, current + 7, "cancel")).err(),
        Some(Failure::StaleRevision)
    );
    assert!(replica.step(&mut state).keys.is_empty());
    assert_eq!(ledger_domain(&state), before);
    let _ = ledger(&state).unwrap();
    replica.assert_matches(&state);

    // An expired survey is removed by the next survey's write.
    for index in 0..65 {
        state.plugins.apply_storage(
            "other-package",
            &[ScriptStorageMutation::CompareAndSwap {
                key: format!("transaction-{index}"),
                expected_version: None,
                value: "committed".into(),
            }],
        );
    }
    let second = surveyed(&mut state, [at[0] + 3, at[1], at[2] + 3]);
    let second_key = format!("survey/{}", second.survey_token);
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), (&first_key, false), (&second_key, true)])
    );
    replica.assert_matches(&state);

    let request = ScriptWorksiteRequest::new(
        ScriptWorldBounds::new([at[0] + 3, at[1], at[2] + 3], [at[0] + 3, at[1], at[2] + 3])
            .unwrap(),
        vec![ScriptWorksiteInteractionPoint::Block {
            at: [at[0] + 3, at[1], at[2] + 3],
            block: "minecraft:stone".into(),
        }],
        second.survey_token.clone(),
        second.world_revision,
    );
    let (_, result) = execute_operation(
        &mut state,
        "demo",
        &ScriptWorldStructureOperation::RegisterWorksite {
            operation_id: "register".into(),
            request: Box::new(request),
        },
    )
    .unwrap();
    let ScriptWorldStructureResult::Worksite { worksite } = result else {
        panic!("worksite")
    };
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[
            ("ledger", true),
            (&format!("worksite/{}", worksite.worksite_id), true)
        ])
    );
    replica.assert_matches(&state);

    let current = record(&state, &id).revision;
    execute_operation(&mut state, "demo", &lifecycle(&id, current, "cancel")).unwrap();
    assert_eq!(
        replica.step(&mut state).keys,
        keys(&[("ledger", true), (&structure, true)])
    );
    replica.assert_matches(&state);
    validate(&state).unwrap();
}

/// The replica check is not two copies of the same omission: a mutation that
/// skips its write mark stays in the live cache but never reaches the WAL.
#[test]
fn a_missed_write_mark_is_caught_by_the_wal_replica() {
    let mut state = state();
    let id = stored_structure(&mut state);
    let mut replica = Replica::capture(&mut state);
    let mut local = ledger_mut(&mut state).unwrap();
    local.revision += 1;
    local
        .structure_mut_unmarked(&id)
        .unwrap()
        .snapshot
        .pause_reason = Some("missed".into());
    save(&mut state, local);
    assert_eq!(replica.step(&mut state).keys, keys(&[("ledger", true)]));
    let error = replica.check(&state).unwrap_err();
    assert!(error.contains(&ledger::structure_key(&id)), "{error}");
}

/// The journaled bytes of one write depend on the record written, not on
/// how many other structures the world has built.
#[test]
fn a_write_journals_the_same_bytes_beside_one_or_ten_committed_structures() {
    fn cancel_beside(committed: usize) -> (BTreeMap<String, bool>, usize, usize) {
        let mut state = state();
        let id = stored_structure(&mut state);
        let mut local = ledger_mut(&mut state).unwrap();
        let template = local.structures()[&id].clone();
        for index in 0..committed {
            let mut other = template.clone();
            other.snapshot.structure_id = format!("committed-{index:02}");
            other.snapshot.state = ScriptStructureState::Committed;
            local.insert_structure(other.snapshot.structure_id.clone(), other);
        }
        save(&mut state, local);
        let mut replica = Replica::capture(&mut state);
        let whole: usize = ledger_domain(&state)
            .values()
            .map(|value| serde_json::to_vec(value).unwrap().len())
            .sum();
        execute_operation(&mut state, "demo", &lifecycle(&id, 1, "cancel")).unwrap();
        let boundary = replica.step(&mut state);
        replica.assert_matches(&state);
        (boundary.keys, boundary.bytes.len(), whole)
    }
    let (one_keys, one_bytes, one_whole) = cancel_beside(1);
    let (ten_keys, ten_bytes, ten_whole) = cancel_beside(10);
    assert_eq!(
        one_keys,
        keys(&[("ledger", true), ("structure/saved-house", true)])
    );
    assert_eq!(one_keys, ten_keys);
    assert_eq!(
        one_bytes, ten_bytes,
        "the cancel journals the cancelled record and the meta key only"
    );
    assert!(
        ten_whole > 4 * one_whole,
        "the ledger itself grew: {one_whole} -> {ten_whole} B"
    );
}

fn refusal(change: impl FnOnce(&mut TickState, &str)) -> String {
    let mut state = state();
    let id = stored_structure(&mut state);
    validate(&state).unwrap();
    change(&mut state, &id);
    match validate(&state) {
        Err(crate::config::CoreError::Recovery(message)) => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

const MALFORMED: &str = "invalid native plugin structure ledger";

/// What the single-image format and its decoder looked like before the split.
#[derive(Deserialize)]
#[allow(dead_code)]
struct OldLedger {
    revision: u64,
    surveys: BTreeMap<String, serde_json::Value>,
    structures: BTreeMap<String, serde_json::Value>,
    worksites: BTreeMap<String, serde_json::Value>,
}

/// One in-place edit of a valid split ledger, applied before `validate`.
type LedgerChange = Box<dyn FnOnce(&mut TickState, &str)>;

#[test]
fn the_new_binary_refuses_every_foreign_ledger_shape() {
    let legacy = |surveys_only: bool| {
        move |state: &mut TickState, id: &str| {
            let key = ledger::structure_key(id);
            let mut record = state.plugins.get_domain(OWNER, &key).unwrap().clone();
            record["receipts"] = serde_json::json!([]);
            let structures = if surveys_only {
                serde_json::json!({})
            } else {
                serde_json::json!({ id: record })
            };
            state.plugins.delete_domain(OWNER, &key);
            state.plugins.set_domain(
                OWNER,
                ledger::META,
                serde_json::json!({"revision": 1, "surveys": {}, "structures": structures, "worksites": {}}),
            );
        }
    };
    for surveys_only in [false, true] {
        let message = refusal(legacy(surveys_only));
        assert!(message.contains("single-image format"), "{message}");
    }
    let cases: [(&str, LedgerChange); 7] = [
        (
            "schema 3",
            Box::new(|state, _| {
                state.plugins.set_domain(
                    OWNER,
                    ledger::META,
                    serde_json::json!({"schema": 3, "revision": 1}),
                );
            }),
        ),
        (
            "unknown meta field",
            Box::new(|state, _| {
                state.plugins.set_domain(
                    OWNER,
                    ledger::META,
                    serde_json::json!({"schema": 2, "revision": 1, "x": 0}),
                );
            }),
        ),
        (
            "records without meta",
            Box::new(|state, _| {
                state.plugins.delete_domain(OWNER, ledger::META);
            }),
        ),
        (
            "unknown key",
            Box::new(|state, _| {
                state
                    .plugins
                    .set_domain(OWNER, "bogus/1", serde_json::json!({}));
            }),
        ),
        (
            "key and record ID differ",
            Box::new(|state, id| {
                let key = ledger::structure_key(id);
                let record = state.plugins.get_domain(OWNER, &key).unwrap().clone();
                state.plugins.delete_domain(OWNER, &key);
                state
                    .plugins
                    .set_domain(OWNER, &ledger::structure_key("elsewhere"), record);
            }),
        ),
        (
            "record keeps receipts",
            Box::new(|state, id| {
                let key = ledger::structure_key(id);
                state.plugins.get_domain_mut(OWNER, &key).unwrap()["receipts"] =
                    serde_json::json!([]);
            }),
        ),
        (
            "watermark above its revision",
            Box::new(|state, id| {
                let key = ledger::structure_key(id);
                let record = state.plugins.get_domain_mut(OWNER, &key).unwrap();
                record["snapshot"]["watermark"] = serde_json::json!(2);
            }),
        ),
    ];
    for (case, change) in cases {
        assert_eq!(refusal(change), MALFORMED, "{case}");
    }

    // No key at all is the only fresh ledger; a survey alone is not empty.
    let mut fresh = state();
    validate(&fresh).unwrap();
    let at = anchor(&fresh);
    surveyed(&mut fresh, at);
    validate(&fresh).unwrap();
    let meta = fresh
        .plugins
        .get_domain(OWNER, ledger::META)
        .unwrap()
        .clone();
    assert_eq!(meta["schema"], 2);
    fresh.plugins.delete_domain(OWNER, ledger::META);
    assert!(
        validate(&fresh).is_err(),
        "a survey without meta is not a fresh world"
    );
    // The older binary's decoder needs the three maps the meta key no longer has.
    assert!(serde_json::from_value::<OldLedger>(meta).is_err());
}

/// The watermark counts advances; every other transition moves only the
/// revision, so `watermark <= revision` holds for each stored record.
#[test]
fn the_watermark_counts_advances_and_stays_within_its_revision() {
    let mut state = state();
    let id = stored_structure(&mut state);
    let created = record(&state, &id);
    assert_eq!(created.watermark, 0);
    for kind in ["pause", "resume"] {
        let before = record(&state, &id);
        execute_operation(&mut state, "demo", &lifecycle(&id, before.revision, kind)).unwrap();
        let after = record(&state, &id);
        assert!(after.revision > before.revision, "{kind}");
        assert_eq!(after.watermark, before.watermark, "{kind}");
    }
    // The startup check is coarse: a wrong watermark within range (here equal
    // to the record's revision) is accepted, one above the revision refused.
    let mut local = ledger_mut(&mut state).unwrap();
    let revision = local.structures()[&id].snapshot.revision;
    local.structure_mut(&id).unwrap().snapshot.watermark = revision;
    save(&mut state, local);
    validate(&state).unwrap();
    let mut local = ledger_mut(&mut state).unwrap();
    local.structure_mut(&id).unwrap().snapshot.watermark = revision + 1;
    save(&mut state, local);
    assert!(validate(&state).is_err());

    let mut fixture = StructureBuildFixture::new(false);
    let structure = "saved-house";
    let before = record(&fixture.state, structure);
    let outcome = crate::plugin::operation(&mut fixture.state, "demo", &fixture.request);
    assert_eq!(outcome.state(), ScriptOperationState::Committed);
    let after = record(&fixture.state, structure);
    assert_eq!(after.watermark, before.watermark + 1);
    assert!(after.revision > before.revision);
    validate(&fixture.state).unwrap();
}

fn stone_in(state: &TickState, endpoint: &ScriptInventoryEndpoint) -> i32 {
    inventories::load(state, "demo", endpoint)
        .unwrap()
        .slots
        .values()
        .map(|stack| stack.count)
        .sum()
}

/// A repeated plugin advance returns its first result, before and after the
/// WAL recovery of the plugin state, with one debit and one placement; a new
/// operation with the old revision is refused. Folding the same absolute
/// delta twice changes nothing more.
#[test]
fn a_repeated_advance_is_exactly_once_across_wal_recovery() {
    let mut fixture = StructureBuildFixture::new(false);
    let mut replica = Replica::capture(&mut fixture.state);
    let checkpoint = replica.plugins();
    let stock = stone_in(&fixture.state, &fixture.endpoint);
    let first = crate::plugin::operation(&mut fixture.state, "demo", &fixture.request);
    assert_eq!(first.state(), ScriptOperationState::Committed);
    let outcome = serde_json::to_value(&first).unwrap();
    assert_eq!(
        find(&outcome, "sequence"),
        Some(&serde_json::json!(1)),
        "{outcome}"
    );
    let placed = fixture.state.plugin_block_writes.clone();
    assert_eq!(placed.len(), 2);
    assert_eq!(stone_in(&fixture.state, &fixture.endpoint), stock - 2);
    let boundary = replica.step(&mut fixture.state);
    assert!(boundary.keys[&ledger::structure_key("saved-house")]);
    replica.assert_matches(&fixture.state);

    let repeat = crate::plugin::operation(&mut fixture.state, "demo", &fixture.request);
    assert_eq!(repeat, first);
    assert_eq!(
        fixture.state.plugin_block_writes, placed,
        "no second placement"
    );
    assert_eq!(
        stone_in(&fixture.state, &fixture.endpoint),
        stock - 2,
        "no second debit"
    );
    assert!(replica.step(&mut fixture.state).keys.is_empty());

    // The same absolute delta folded twice equals folding it once.
    let entries: Vec<crate::journal::JournalEntry> =
        serde_json::from_slice(&boundary.bytes).unwrap();
    let replay = |plugins, tick| {
        crate::journal::replay_batches(
            crate::journal::Replay {
                plugins,
                ..Default::default()
            },
            &[(tick, tick, entries.clone())],
        )
        .plugins
    };
    let once = replay(checkpoint, 1);
    let twice = replay(once.clone(), 2);
    assert_eq!(
        serde_json::to_value(&twice).unwrap(),
        serde_json::to_value(&once).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&once).unwrap(),
        serde_json::to_value(replica.plugins()).unwrap()
    );

    // Recovery hands the WAL-folded state to a new owner of the same world.
    fixture.state.plugins = once;
    let after_recovery = crate::plugin::operation(&mut fixture.state, "demo", &fixture.request);
    assert_eq!(after_recovery, first);
    assert_eq!(fixture.state.plugin_block_writes, placed);
    assert_eq!(stone_in(&fixture.state, &fixture.endpoint), stock - 2);
    assert_eq!(record(&fixture.state, "saved-house").watermark, 1);

    let mut stale = fixture.request.operation().clone();
    let ScriptOperation::WorldStructure {
        operation: ScriptWorldStructureOperation::AdvanceStructure { operation_id, .. },
    } = &mut stale
    else {
        unreachable!()
    };
    *operation_id = "advance-again".into();
    let stale = ScriptOperationRequest::try_new("again", stale).unwrap();
    let refused = crate::plugin::operation(&mut fixture.state, "demo", &stale);
    assert_eq!(refused.failure(), Some(Failure::StaleRevision));
    assert_eq!(fixture.state.plugin_block_writes, placed);
    assert_eq!(stone_in(&fixture.state, &fixture.endpoint), stock - 2);
    assert_eq!(record(&fixture.state, "saved-house").watermark, 1);
    validate(&fixture.state).unwrap();
}

fn find<'a>(value: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map
            .get(key)
            .or_else(|| map.values().find_map(|value| find(value, key))),
        serde_json::Value::Array(items) => items.iter().find_map(|value| find(value, key)),
        _ => None,
    }
}

#[test]
fn plugin_structure_requires_explicit_empty_portions() {
    let mut state = state();
    let handle = stored_structure(&mut state);
    let key = ledger::structure_key(&handle);
    let valid = state.plugins.get_domain(OWNER, &key).unwrap().clone();
    assert_eq!(valid["portions"], serde_json::json!([]));
    validate(&state).unwrap();
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("portions");
    state.plugins.set_domain(OWNER, &key, missing);
    assert!(validate(&state).is_err());
    let mut extra = valid.clone();
    extra["portions"] = serde_json::json!([{"expected": [], "roots": []}]);
    state.plugins.set_domain(OWNER, &key, extra);
    assert!(validate(&state).is_err());
    state.plugins.set_domain(OWNER, &key, valid);
    validate(&state).unwrap();
}
