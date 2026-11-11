use super::*;

fn fixture() -> TickState {
    crate::hostile_tests::setup(8.0).0
}

fn layout(seed: u64) -> Vec<([u64; 3], u16, String)> {
    let mut state = fixture();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], seed, 100).unwrap();
    state.hostile.battle.formation.members.iter().map(|member| {
        let slot = state.entities.slot_of(member.id).unwrap();
        let row = state.entities.row(slot);
        let weapon = row.appearance.equipment[0].as_ref().unwrap();
        let name = state.registry.items.name_of(weapon.item_id).unwrap().as_str().to_owned();
        (state.entities.pos(slot).map(f64::to_bits), state.entities.body(slot).faction, name)
    }).collect()
}

#[test]
fn seed_reproduces_layout_and_army_mix() {
    let first = layout(123);
    assert_eq!(first.len(), 200);
    assert_eq!(first, layout(123));
    assert_ne!(first, layout(124));
}

#[test]
fn reset_removes_battle_units_and_ticket_not_other_mobs() {
    let mut state = fixture();
    let unrelated = state.entities.slots().map(|slot| state.entities.body(slot).id).collect::<Vec<_>>();
    let mut changes = ChangeSet::default();
    setup(&mut state, &mut changes, [0.0, 80.0, 0.0], 9, 100).unwrap();
    let spawned = changes.entities_spawned.clone();
    assert!(spawned.len() >= 200);
    assert!(spawned.len() <= 400, "at most one horse per soldier");
    assert_eq!(state.hostile.battle.formation.members.len(), 200);
    assert_eq!(state.hostile.battle.formation.regiments.len(), 2);
    let status = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Status).unwrap();
    assert!(status.contains("side1=100 side2=100"), "{status}");
    let roster = execute(&mut state, &mut changes, [0.0; 3], &BattleCommand::Roster).unwrap();
    let listed: std::collections::HashSet<_> = roster.strip_prefix("Battle roster: ").unwrap().split(',').collect();
    assert_eq!(listed.len(), spawned.len());
    for id in &spawned {
        let slot = state.entities.slot_of(*id).unwrap();
        assert!(listed.contains(state.entities.row(slot).uuid.to_string().as_str()));
    }
    assert!(state.tickets.contains_key(OWNER));
    assert_eq!(reset(&mut state, &mut changes), spawned.len());
    assert!(spawned.iter().all(|id| state.entities.slot_of_any(*id).is_none()));
    assert!(unrelated.iter().all(|id| state.entities.slot_of_any(*id).is_some()));
    assert!(!state.tickets.contains_key(OWNER));
    assert!(state.hostile.battle.formation.regiments.is_empty());
}

#[test]
fn commands_require_operator_and_enforce_count_bounds() {
    for text in ["battle setup 1 field-100", "battle start", "battle pause", "battle reset", "battle status", "battle roster"] {
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
        crate::command_execution::execute(&mut state, crate::boundary::ConnectionId(1), EntityId(1), &roster, &mut changes),
        crate::command_execution::CommandResult::Failed
    );
}
