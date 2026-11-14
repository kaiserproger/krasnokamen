use super::*;

fn fixture() -> TickState {
    crate::hostile_tests::setup(8.0).0
}

fn layout(seed: u64) -> Vec<([u64; 3], u16, String)> {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], seed, 100).unwrap();
    state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .map(|member| {
            let slot = state.entities.slot_of(member.id).unwrap();
            let row = state.entities.row(slot);
            let weapon = row.appearance.equipment[0].as_ref().unwrap();
            let name = state
                .registry
                .items
                .name_of(weapon.item_id)
                .unwrap()
                .as_str()
                .to_owned();
            (
                state.entities.pos(slot).map(f64::to_bits),
                state.entities.body(slot).faction,
                name,
            )
        })
        .collect()
}

#[test]
fn seed_reproduces_layout_and_army_mix() {
    let first = layout(123);
    assert_eq!(first.len(), 200);
    assert_eq!(first, layout(123));
    assert_ne!(first, layout(124));
}

#[test]
fn preset_has_exact_homogeneous_mix_and_spaced_horse_flanks() {
    let planned = plan(42, 10_000, [0.0, 80.0, 0.0], 1);
    for (kind, expected) in [
        (Kind::Infantry, 5_000),
        (Kind::Spear, 2_000),
        (Kind::Archer, 2_000),
        (Kind::Cavalry, 1_000),
    ] {
        assert_eq!(
            planned
                .iter()
                .filter(|(_, k, _)| *k == kind)
                .map(|(_, _, n)| *n)
                .sum::<usize>(),
            expected
        );
    }
    for (spec, kind, count) in &planned {
        assert!(*count <= 1000);
        if *kind == Kind::Cavalry {
            assert!(spec.origin[0].abs() > 100.0);
            assert_eq!(
                (slot_position(spec, 1)[0] - slot_position(spec, 0)[0]).abs(),
                2.0
            );
        }
        if *kind == Kind::Archer {
            assert_eq!(spec.origin[2], -125.0);
            assert_eq!(spec.order, Order::Hold);
        }
    }
}

#[test]
fn staged_equipment_exposes_authored_assets_for_both_sides_and_all_kinds() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 42, 100).unwrap();
    let mut seen = [[false; 5]; 2];
    for id in changes.entities_spawned {
        let slot = state.entities.slot_of(id).unwrap();
        let body = state.entities.body(slot);
        let side = usize::from(body.faction - 1);
        let row = state.entities.row(slot);
        let equippable = |stack: &mc_data::ItemStack, expected: &str, expected_slot: &str| {
            let fields = mc_data::item_nbt::stack_to_fields(&state.registry.items, stack).unwrap();
            let Some(Tag::Compound(components)) = mc_data::item_nbt::field(&fields, "components")
            else {
                panic!("missing components")
            };
            let Some(Tag::Compound(value)) =
                mc_data::item_nbt::field(components, "minecraft:equippable")
            else {
                panic!("missing equippable")
            };
            assert_eq!(
                mc_data::item_nbt::field(value, "slot"),
                Some(&Tag::String(expected_slot.into()))
            );
            assert_eq!(
                mc_data::item_nbt::field(value, "asset_id"),
                Some(&Tag::String(expected.into()))
            );
        };
        let Some(weapon) = row.appearance.equipment[0].as_ref() else {
            let armor = row.loot.body_armor.as_ref().unwrap();
            equippable(
                armor,
                if side == 0 {
                    "krasnokamen_battle:red_caparison"
                } else {
                    "krasnokamen_battle:blue_caparison"
                },
                "body",
            );
            seen[side][4] = true;
            continue;
        };
        let item = state
            .registry
            .items
            .name_of(weapon.item_id)
            .unwrap()
            .as_str();
        let (kind, model) = match (item, row.entity_life.mount.vehicle.is_some()) {
            ("minecraft:trident", _) => (1, "krasnokamen_battle:iron_spear"),
            ("minecraft:bow", _) => (2, "krasnokamen_battle:iron_longbow_standby"),
            ("minecraft:iron_sword", true) => (3, "krasnokamen_battle:iron_lance"),
            ("minecraft:iron_sword", false) => (0, "krasnokamen_battle:iron_longsword"),
            _ => panic!("unexpected battle weapon {item}"),
        };
        seen[side][kind] = true;
        assert_eq!(weapon.item_model.as_ref().unwrap().as_str(), model);
        equippable(
            row.appearance.equipment[5].as_ref().unwrap(),
            if kind == 3 {
                "krasnokamen_battle:crested_helm"
            } else {
                "krasnokamen_battle:iron_plate"
            },
            "head",
        );
        equippable(
            row.appearance.equipment[4].as_ref().unwrap(),
            if side == 0 {
                "krasnokamen_battle:red_surcoat"
            } else {
                "krasnokamen_battle:blue_surcoat"
            },
            "chest",
        );
        if kind != 2 {
            let shield = row.appearance.equipment[1].as_ref().unwrap();
            assert_eq!(
                shield.item_model.as_ref().unwrap().as_str(),
                if side == 0 {
                    "krasnokamen_battle:red_heater_shield"
                } else {
                    "krasnokamen_battle:blue_heater_shield"
                }
            );
        }
    }
    assert_eq!(seen, [[true; 5]; 2]);
}

#[test]
fn reset_removes_battle_units_and_ticket_not_other_mobs() {
    let mut state = fixture();
    let unrelated = state
        .entities
        .slots()
        .map(|slot| state.entities.body(slot).id)
        .collect::<Vec<_>>();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap();
    let spawned = changes.entities_spawned.clone();
    assert!(spawned.len() >= 200);
    assert!(spawned.len() <= 400, "at most one horse per soldier");
    assert_eq!(state.hostile.battle.formation.members.len(), 200);
    assert_eq!(state.hostile.battle.formation.regiments.len(), 10);
    for name in ["mob_drops", "mob_griefing", "spawn_mobs"] {
        assert_eq!(state.gamerules.get(name).map(String::as_str), Some("false"));
    }
    let husk = state
        .registry
        .entity_types
        .id_of(&mc_data::Identifier::parse("minecraft:husk").unwrap())
        .unwrap();
    for regiment in &state.hostile.battle.formation.regiments {
        let members =
            &state.hostile.battle.formation.members[regiment.start..regiment.start + regiment.len];
        let first_slot = state.entities.slot_of(members[0].id).unwrap();
        let first = state.entities.row(first_slot);
        let weapon = first.appearance.equipment[0].as_ref().unwrap().item_id;
        let mounted = first.entity_life.mount.vehicle.is_some();
        for member in members {
            let slot = state.entities.slot_of(member.id).unwrap();
            let row = state.entities.row(slot);
            assert!(
                matches!(state.entities.body(slot).kind, crate::entities::EntityKind::Mob { type_id } if type_id == husk)
            );
            assert_eq!(
                row.appearance.equipment[0].as_ref().unwrap().item_id,
                weapon
            );
            assert_eq!(row.entity_life.mount.vehicle.is_some(), mounted);
        }
    }
    let error = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Start).unwrap_err();
    assert!(error.contains("outside loaded simulation"), "{error}");
    assert!(!state.hostile.battle.running);
    let status = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(status.contains("side1=100 side2=100"), "{status}");
    assert!(
        status.contains(&format!("horses={} ", spawned.len() - 200)),
        "{status}"
    );
    let roster = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Roster).unwrap();
    let listed: std::collections::HashSet<_> = roster
        .strip_prefix("Battle roster: ")
        .unwrap()
        .split(',')
        .collect();
    assert_eq!(listed.len(), spawned.len());
    for id in &spawned {
        let slot = state.entities.slot_of(*id).unwrap();
        assert!(
            state.entities.row(slot).persistent,
            "battle entity can naturally despawn"
        );
        assert!(listed.contains(state.entities.row(slot).uuid.to_string().as_str()));
    }
    let want: rustc_hash::FxHashMap<_, _> = spawned
        .iter()
        .map(|id| {
            let slot = state.entities.slot_of(*id).unwrap();
            let pos = state.entities.pos(slot);
            (
                ChunkPos {
                    x: (pos[0].floor() as i32).div_euclid(16),
                    z: (pos[2].floor() as i32).div_euclid(16),
                },
                0,
            )
        })
        .collect();
    state.world.ensure_blocking(&want);
    state.refresh_active_chunks(state.tick);
    execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Start).unwrap();
    assert!(state.hostile.battle.running);
    execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Pause).unwrap();
    assert!(!state.hostile.battle.running);
    assert!(state.tickets.contains_key(OWNER));
    assert_eq!(reset(&mut state, &mut changes), spawned.len());
    assert!(
        spawned
            .iter()
            .all(|id| state.entities.slot_of_any(*id).is_none())
    );
    assert!(
        unrelated
            .iter()
            .all(|id| state.entities.slot_of_any(*id).is_some())
    );
    assert!(!state.tickets.contains_key(OWNER));
    assert!(state.hostile.battle.formation.regiments.is_empty());
}

#[test]
fn status_tick_matches_command_commit_in_local_and_shared_dispatch() {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    state.tick = 20;
    let local = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(local.contains("tick=21 "), "{local}");
    state.command_context.shared = true;
    let shared = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(shared.contains("tick=20 "), "{shared}");
}

#[test]
fn peaceful_world_rejects_setup_without_partial_changes() {
    let mut state = fixture();
    state.difficulty = crate::survival::Difficulty::Peaceful;
    let mut changes = ChangeSet::default();
    let error = setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap_err();
    assert!(error.contains("peaceful"));
    assert!(changes.entities_spawned.is_empty());
    assert!(!state.tickets.contains_key(OWNER));
}

#[test]
fn commands_require_operator_and_enforce_count_bounds() {
    for text in [
        "battle setup 1 field-100",
        "battle start",
        "battle pause",
        "battle reset",
        "battle status",
        "battle roster",
    ] {
        assert!(crate::commands::parse(text, false).is_err(), "{text}");
        assert!(crate::commands::parse(text, true).is_ok(), "{text}");
    }
    assert!(crate::commands::root_permitted("battle", 2));
    assert!(!crate::commands::root_permitted("battle", 1));
    assert!(crate::commands::parse("battle setup 1 field-100 10001", true).is_err());
    assert!(crate::commands::parse("battle setup 1 field-100 0", true).is_err());
    let roster = crate::commands::parse("battle roster", true).unwrap();
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    assert_eq!(
        crate::command_execution::execute(
            &mut state,
            crate::boundary::ConnectionId(1),
            EntityId(1),
            &roster,
            &mut changes
        ),
        crate::command_execution::CommandResult::Failed
    );
}

#[test]
fn failed_setup_rolls_back_owned_rows_changes_ticket_rules_and_journal() {
    use crate::journal::JournalEntry;
    use std::collections::BTreeSet;
    let mut state = fixture();
    state.gamerules.insert("spawn_mobs".into(), "false".into());
    state.gamerules.insert("mob_drops".into(), "true".into());
    state.gamerules.insert("mob_griefing".into(), "true".into());
    state.gamerules_dirty = false;
    let kind = state.registry.entity_type_id("minecraft:cow").unwrap();
    let unrelated = state.spawn_mob(kind, [0.0, 80.0, 0.0], 20.0);
    let removed = state.spawn_mob(kind, [1.0, 80.0, 0.0], 20.0);
    state.remove_entity(state.entities.slot_of(removed).unwrap());
    let mut changes = ChangeSet::default();
    changes.entities_spawned.push(unrelated);
    changes.entities_removed.push(removed);
    let spawned_before = changes.entities_spawned.clone();
    let removed_before = changes.entities_removed.clone();
    let ids_before: BTreeSet<_> = state
        .entities
        .slots()
        .map(|slot| state.entities.body(slot).id)
        .collect();
    let rules_before = state.gamerules.clone();
    let origin = [0.0, 80.0, 0.0];
    let mut collision = plan(9, 100, origin, 1)[0].0;
    collision.files = 1;
    assert_eq!(collision.id, 0);
    state
        .hostile
        .battle
        .formation
        .add_regiment(collision, &[unrelated])
        .unwrap();
    assert!(setup(&mut state, &mut changes, origin, 9, 100).is_err());
    assert!(
        tagged_ids(&state).is_empty(),
        "failed setup left battle-owned rows"
    );
    assert_eq!(
        state
            .entities
            .slots()
            .map(|slot| state.entities.body(slot).id)
            .collect::<BTreeSet<_>>(),
        ids_before
    );
    assert_eq!(changes.entities_spawned, spawned_before);
    assert_eq!(changes.entities_removed, removed_before);
    assert!(!state.tickets.contains_key(OWNER));
    assert_eq!(state.gamerules, rules_before);
    assert!(!state.gamerules_dirty);

    let (mut output, _, _) = crate::tick::run_tick(&mut state, &[]);
    let published = std::sync::Arc::get_mut(&mut output.changes).unwrap();
    published.entities_spawned.extend(changes.entities_spawned);
    published.entities_removed.extend(changes.entities_removed);
    let entries = crate::core::journal_entries_for_tick(&mut state, &output);
    let allowed: BTreeSet<_> = ids_before
        .into_iter()
        .chain([removed])
        .map(|id| id.0)
        .collect();
    for entry in entries {
        if let JournalEntry::Entity { id, .. } | JournalEntry::RemoveEntity { id, .. } = entry {
            assert!(
                allowed.contains(&id),
                "failed setup leaked entity {id} into WAL"
            );
        }
    }
}
