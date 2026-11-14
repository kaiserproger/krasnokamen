use super::*;

#[test]
fn faction_relations_distinguish_neutral_enemy_and_roster_ownership() {
    for a in [0, 1, 2, u16::MAX] {
        assert_eq!(battle_row(a), a != 0);
        for b in [0, 1, 2, u16::MAX] {
            let expected = match (a, b) {
                (0, _) | (_, 0) => Relation::Neutral,
                _ if a == b => Relation::Same,
                _ => Relation::Enemy,
            };
            assert_eq!(relation(a, b), expected);
            assert_eq!(enemies(a, b), expected == Relation::Enemy);
        }
    }
}

#[test]
fn faction_snapshot_publishes_holds_without_enrolling_player_bodies() {
    use crate::battle::live_players::tests::{command, staged};
    use crate::battle::staging::BattleCommand;
    use crate::boundary::ChangeSet;
    use crate::tick::run_tick;
    use std::sync::Arc;

    let (mut state, ids) = staged(&["PovRed", "PovBlue"]);
    let mut changes = ChangeSet::default();
    for (name, side) in [("PovRed", 1), ("PovBlue", 2)] {
        command(
            &mut state,
            &mut changes,
            BattleCommand::Live {
                name: name.into(),
                side,
            },
        )
        .unwrap();
    }
    let first = run_tick(&mut state, &[]).0.snapshot;
    assert_eq!(
        &*first.player_factions,
        &[("PovBlue".into(), 2), ("PovRed".into(), 1)]
    );
    for id in ids {
        let slot = state.entities.slot_of(id).unwrap();
        assert_eq!(state.entities.body(slot).faction, 0);
        assert!(!state.entities.battle_slots().any(|member| member == slot));
    }
    state.hostile.battle.live.holds.reverse();
    state.hostile.battle.live.holds[0].deaths += 1;
    let unchanged = run_tick(&mut state, &[]).0.snapshot;
    assert!(Arc::ptr_eq(
        &first.player_factions,
        &unchanged.player_factions
    ));
    state.hostile.battle.live.holds[0].side = 1;
    let changed = run_tick(&mut state, &[]).0.snapshot;
    assert!(!Arc::ptr_eq(
        &first.player_factions,
        &changed.player_factions
    ));
    assert_eq!(
        &*changed.player_factions,
        &[("PovBlue".into(), 1), ("PovRed".into(), 1)]
    );
    command(&mut state, &mut changes, BattleCommand::Reset).unwrap();
    let reset = run_tick(&mut state, &[]).0.snapshot;
    assert!(reset.player_factions.is_empty());
    assert!(!Arc::ptr_eq(
        &changed.player_factions,
        &reset.player_factions
    ));
    assert!(Arc::ptr_eq(
        &reset.player_factions,
        &run_tick(&mut state, &[]).0.snapshot.player_factions
    ));
}

fn team_command(state: &mut crate::tick::TickState, actor: EntityId, args: &[&str]) {
    let command = crate::scoreboard::ScoreboardCommand::parse("team", args).unwrap();
    crate::scoreboard::execute(state, actor, &command);
}

#[test]
fn faction_live_refuses_a_core_team_member_before_taking_a_slot() {
    use crate::battle::live_players::tests::{command, live_cmd, staged};
    let (mut state, ids) = staged(&["PovRed"]);
    team_command(&mut state, ids[0], &["add", "allies"]);
    team_command(&mut state, ids[0], &["join", "allies", "PovRed"]);
    let error = command(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        live_cmd("PovRed"),
    )
    .unwrap_err();
    assert!(error.contains("PovRed is in team allies"), "{error}");
    assert!(state.hostile.battle.live.holds.is_empty());
    assert!(state.hostile.battle.formation.held.is_empty());
    team_command(&mut state, ids[0], &["leave", "PovRed"]);
    command(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        live_cmd("PovRed"),
    )
    .unwrap();
    assert_eq!(state.hostile.battle.live.holds.len(), 1);
}

#[test]
fn faction_mount_refuses_a_core_team_member_before_changing_custody() {
    use crate::battle::live_players::tests::staged;
    let (mut state, ids) = staged(&["PovRed"]);
    let rider_type = state.registry.entity_type_id("minecraft:husk").unwrap();
    let rider = state.spawn_mob(rider_type, [8.0, 64.0, 8.0], 20.0);
    state
        .entities
        .set_faction(state.entities.slot_of(rider).unwrap(), 1);
    let horse_type = mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:horse")
        .unwrap()
        .protocol_id;
    let horse = state.spawn_mob(horse_type, [8.0, 64.0, 8.0], 20.0);
    let slot = state.entities.slot_of(horse).unwrap();
    let entry = state.entities.row(slot).uuid.to_string();
    team_command(&mut state, ids[0], &["add", "allies"]);
    team_command(&mut state, ids[0], &["join", "allies", &entry]);
    let error = crate::battle::cavalry::mount(&mut state, rider, horse).unwrap_err();
    assert!(
        error
            .to_string()
            .contains(&format!("{entry} is in team allies")),
        "{error}"
    );
    assert_eq!(state.entities.body(slot).faction, 0);
    assert!(state.entities.row(slot).entity_life.mount.riders.is_empty());
    team_command(&mut state, ids[0], &["leave", &entry]);
    crate::battle::cavalry::mount(&mut state, rider, horse).unwrap();
    assert_ne!(state.entities.body(slot).faction, 0);
}
