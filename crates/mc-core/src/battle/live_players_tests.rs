use super::*;
use crate::battle::staging::{self, BattleCommand};
use crate::boundary::{ConnectionId, PlayerInput};
use crate::tick::run_tick;

const ORIGIN: [f64; 3] = [8.5, 64.0, 8.5];

/// The flat fixture with every chunk of the staging ticket resident, a staged
/// 100-per-side battle on its ground, and `names` online.
fn staged(names: &[&str]) -> (TickState, Vec<EntityId>) {
    let registry = std::sync::Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
    let world = crate::horde_bench::flat_world(&registry);
    let mut state = TickState::new(registry, world, 2, 512);
    let want = (-13..=13)
        .flat_map(|x| (-13..=13).map(move |z| (mc_world::ChunkPos { x, z }, 0)))
        .collect();
    state.world.ensure_blocking(&want);
    let mut changes = ChangeSet::default();
    command(
        &mut state,
        &mut changes,
        BattleCommand::Setup {
            seed: 42,
            count: 100,
        },
    )
    .unwrap();
    let desired = state.desired_chunks();
    state.world.ensure_blocking(&desired);
    state.refresh_active_chunks(state.tick);
    let joins: Vec<PlayerInput> = names
        .iter()
        .enumerate()
        .map(|(index, name)| PlayerInput::Join {
            connection: ConnectionId(index as u64 + 1),
            uuid: uuid::Uuid::new_v4(),
            name: (*name).to_owned(),
            permission_level: 0,
        })
        .collect();
    run_tick(&mut state, &joins);
    for _ in 0..4 {
        run_tick(&mut state, &[]);
    }
    assert!(state.hostile.battle.ground_pending.is_empty());
    let ids = names
        .iter()
        .map(|name| player_named(&state, name).expect("joined"))
        .collect();
    (state, ids)
}

fn command(
    state: &mut TickState,
    changes: &mut ChangeSet,
    command: BattleCommand,
) -> Result<String, String> {
    staging::execute(state, changes, ORIGIN, &command)
}

fn parse(text: &str) -> Result<BattleCommand, crate::commands::CommandError> {
    let words: Vec<&str> = text.split(' ').collect();
    staging::parse(&words)
}

fn live_cmd(name: &str) -> BattleCommand {
    BattleCommand::Live {
        name: name.into(),
        side: 1,
    }
}

#[test]
fn commands_parse_names_and_sides_and_reject_the_rest() {
    assert_eq!(
        parse("watch PovRed CamSpec").unwrap(),
        BattleCommand::Watch {
            names: vec!["PovRed".into(), "CamSpec".into()]
        }
    );
    assert_eq!(parse("live PovRed red").unwrap(), live_cmd("PovRed"));
    assert_eq!(
        parse("live PovRed 2").unwrap(),
        BattleCommand::Live {
            name: "PovRed".into(),
            side: 2
        }
    );
    assert_eq!(
        parse("spectate CamSpec PovRed").unwrap(),
        BattleCommand::Spectate {
            name: "CamSpec".into(),
            target: "PovRed".into()
        }
    );
    for bad in [
        "watch",
        "watch a b c d e f g h i",
        "watch Pov-Red",
        "live PovRed green",
        "live PovRed",
        "live ThisNameIsSeventeen red",
        "spectate CamSpec",
    ] {
        assert!(parse(bad).is_err(), "{bad}");
    }
    assert!(crate::commands::parse("battle live PovRed red", false).is_err());
    assert!(crate::commands::parse("battle live PovRed red", true).is_ok());
}

#[test]
fn watch_announces_each_join_and_leave_once() {
    let (mut state, ids) = staged(&["PovRed"]);
    let mut changes = ChangeSet::default();
    let reply = command(
        &mut state,
        &mut changes,
        BattleCommand::Watch {
            names: vec!["PovRed".into(), "CamSpec".into()],
        },
    )
    .unwrap();
    assert_eq!(reply, "Battle watching: PovRed,CamSpec");
    assert_eq!(state.hostile.battle.live.announced, vec![None, None]);
    run_tick(&mut state, &[]);
    assert_eq!(state.hostile.battle.live.announced, vec![Some(ids[0]), None]);
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(7),
            uuid: uuid::Uuid::new_v4(),
            name: "CamSpec".into(),
            permission_level: 0,
        }],
    );
    run_tick(&mut state, &[]);
    let spectator = player_named(&state, "CamSpec").unwrap();
    assert_eq!(
        state.hostile.battle.live.announced,
        vec![Some(ids[0]), Some(spectator)]
    );
}

#[test]
fn live_player_takes_the_centre_infantry_slot_with_its_kit() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    let reply = command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    assert!(reply.starts_with("Battle live placed: PovRed side=1 "), "{reply}");
    let hold = state.hostile.battle.live.holds[0].clone();
    let formation = &state.hostile.battle.formation;
    assert_eq!(formation.held, vec![(hold.regiment, hold.file)]);
    let regiment = &formation.regiments[hold.regiment];
    assert_eq!(regiment.spec.faction, 1);
    assert_eq!(regiment.spec.order, Order::Advance);
    let members = file_members(formation, hold.regiment, hold.file);
    assert!(members.iter().all(|&i| formation.members[i].rank >= 1));
    // Kit: the same item stacks (components included) as the file's soldier.
    let source = state
        .entities
        .slot_of(formation.members[members[0]].id)
        .unwrap();
    let kit = state.entities.row(source).appearance.equipment.clone();
    let sword = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_sword".to_owned()).unwrap())
        .unwrap();
    assert_eq!(kit[0].as_ref().unwrap().item_id, sword, "infantry, not spear");
    assert!(kit[1].is_some(), "off-hand shield");
    let inventory = &state.players[&player].inventory;
    let selected = usize::from(inventory.selected);
    for (equipment, stack) in kit.iter().enumerate() {
        assert_eq!(
            &inventory.slots[player_index(equipment, selected)],
            stack.as_ref().unwrap_or(&ItemStack::EMPTY)
        );
    }
    // Rank 0 where the front soldier stood, facing the enemy (+z for red).
    let body = state.entities.slot_of(player).unwrap();
    assert_eq!(state.entities.pos(body), hold.at);
    assert_eq!(state.entities.rot(body)[0], 0.0);
    let line = slot_position(&regiment.spec, hold.file);
    assert!((hold.at[0] - line[0]).abs() < 1e-9 && (hold.at[2] - line[2]).abs() < 1e-9);
    // Nearest the centre of the red foot line: no foot front file is closer.
    // Nearest the centre of the red foot line (infantry left of the spears,
    // their seam on the origin): the infantry file next to the seam.
    assert!((hold.at[0] - ORIGIN[0]).abs() <= 1.0, "{:?}", hold.at);
    for m in &formation.members {
        let Some(slot) = state.entities.slot_of(m.id) else {
            continue;
        };
        let pos = state.entities.pos(slot);
        let d = ((pos[0] - hold.at[0]).powi(2) + (pos[2] - hold.at[2]).powi(2)).sqrt();
        assert!(d >= 0.99, "soldier {:?} at {d} blocks", m.id);
    }
    assert!(state.hostile.battle.ground_pending.is_empty());
    assert!(state.hostile.battle.ground_refused.is_empty());
    // An ordinary player: hits land.
    assert!(!state.entities.row(body).controls.invulnerable);
    assert_eq!(state.gamerules.get("keep_inventory").map(String::as_str), Some("true"));
}

fn kill(state: &mut TickState, connection: u64) {
    run_tick(
        state,
        &[PlayerInput::Command {
            connection: ConnectionId(connection),
            command: "kill".into(),
            permission_level: 2,
        }],
    );
}

fn assert_in_rank_with_kit(state: &TickState, player: EntityId) {
    let hold = &state.hostile.battle.live.holds[0];
    let formation = &state.hostile.battle.formation;
    let r = &formation.regiments[hold.regiment];
    let slot = position(&r.spec, hold.file, 0, interval(r));
    let body = state.entities.slot_of(player).unwrap();
    let at = state.entities.pos(body);
    // The leader walks at most 0.066 a tick after the placing tick.
    assert!(
        (at[0] - slot[0]).abs() < 0.25 && (at[2] - slot[2]).abs() < 0.25,
        "{at:?} is not rank 0 of file {} at {slot:?}",
        hold.file
    );
    assert_eq!(state.entities.rot(body)[0], yaw_of(r.spec.facing));
    assert!(state.entities.body(body).alive);
    let kit = hold.kit.clone();
    let inventory = &state.players[&player].inventory;
    let selected = usize::from(inventory.selected);
    for (equipment, stack) in kit.iter().enumerate() {
        assert_eq!(
            &inventory.slots[player_index(equipment, selected)],
            stack.as_ref().unwrap_or(&ItemStack::EMPTY)
        );
    }
    assert_eq!(formation.held, vec![(hold.regiment, hold.file)]);
    for &i in &file_members(formation, hold.regiment, hold.file) {
        assert!(formation.members[i].rank >= 1, "rank 0 stays his");
    }
}

#[test]
fn each_respawn_in_battle_puts_the_holder_back_into_the_front_rank_with_the_kit() {
    let (mut state, ids) = staged(&["PovRed", "Late"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    for _ in 0..40 {
        run_tick(&mut state, &[]);
    }
    let late = command(&mut state, &mut changes, live_cmd("Late")).unwrap_err();
    assert!(late.contains("only before start"), "{late}");
    for death in 1..=2 {
        kill(&mut state, 1);
        run_tick(&mut state, &[]);
        let hold = &state.hostile.battle.live.holds[0];
        assert!(hold.dead && hold.deaths == death);
        assert!(command(&mut state, &mut changes, live_cmd("PovRed")).is_err());
        // The kit lost as if looted: the respawn restores it.
        state.players.get_mut(&player).unwrap().inventory.slots[38] = ItemStack::EMPTY;
        run_tick(
            &mut state,
            &[PlayerInput::Respawn {
                connection: ConnectionId(1),
            }],
        );
        run_tick(&mut state, &[]);
        assert!(!state.hostile.battle.live.holds[0].dead);
        assert_in_rank_with_kit(&state, player);
        // Between respawns the battle never moves him.
        let body = state.entities.slot_of(player).unwrap();
        let before = state.entities.pos(body);
        for _ in 0..20 {
            run_tick(&mut state, &[]);
        }
        assert_eq!(state.entities.pos(body), before);
    }
}

#[test]
fn holder_hits_hurt_enemy_soldiers_and_spare_his_own_side() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    let first_of = |state: &TickState, side: u16| {
        let formation = &state.hostile.battle.formation;
        formation
            .members
            .iter()
            .find(|m| {
                formation.regiments[m.regiment].spec.faction == side
                    && formation.regiments[m.regiment].spec.frontage == 1.0
                    && !formation.held.contains(&(m.regiment, m.file))
            })
            .unwrap()
            .id
    };
    for (side, hurt) in [(1_u16, false), (2, true)] {
        let victim = first_of(&state, side);
        let at = state.entities.pos(state.entities.slot_of(victim).unwrap());
        let body = state.entities.slot_of(player).unwrap();
        state.entities.set_pos(body, [at[0], at[1], at[2] - 1.5]);
        for _ in 0..20 {
            run_tick(&mut state, &[]);
        }
        let before = state.entities.row(state.entities.slot_of(victim).unwrap()).health;
        run_tick(
            &mut state,
            &[PlayerInput::Attack {
                connection: ConnectionId(1),
                target: victim,
            }],
        );
        let after = state
            .entities
            .slot_of(victim)
            .map_or(0.0, |slot| state.entities.row(slot).health);
        assert_eq!(after < before, hurt, "side {side}: {before} -> {after}");
    }
}

#[test]
fn a_rejoin_after_start_only_rearms() {
    let (mut state, ids) = staged(&["PovRed"]);
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    let body = state.entities.slot_of(ids[0]).unwrap();
    let before = state.entities.pos(body);
    let reply = command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    assert!(reply.starts_with("Battle live rearmed: PovRed "), "{reply}");
    assert_eq!(state.entities.pos(body), before);
}

#[test]
fn a_respawn_after_his_infantry_broke_takes_a_standing_line_with_the_same_kit() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    let kit = state.hostile.battle.live.holds[0].kit.clone();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    kill(&mut state, 1);
    run_tick(&mut state, &[]);
    // Every red infantry line breaks while he is dead.
    let infantry = state.hostile.battle.live.holds[0].regiment;
    state.hostile.battle.formation.regiments[infantry].morale.broken = true;
    run_tick(
        &mut state,
        &[PlayerInput::Respawn {
            connection: ConnectionId(1),
        }],
    );
    run_tick(&mut state, &[]);
    let hold = state.hostile.battle.live.holds[0].clone();
    assert_ne!(hold.regiment, infantry, "a standing line, not the broken one");
    let r = &state.hostile.battle.formation.regiments[hold.regiment];
    assert_eq!((r.spec.faction, r.spec.frontage), (1, 1.0));
    assert!(!r.morale.broken);
    assert_eq!(hold.kit, kit, "the same infantry kit, not the spear line's");
    assert_in_rank_with_kit(&state, player);
}

#[test]
fn spectator_goes_behind_the_holder_in_spectator_mode() {
    let (mut state, ids) = staged(&["PovRed", "CamSpec"]);
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    let reply = command(
        &mut state,
        &mut changes,
        BattleCommand::Spectate {
            name: "CamSpec".into(),
            target: "PovRed".into(),
        },
    )
    .unwrap();
    assert!(reply.starts_with("Battle spectator placed: CamSpec target=PovRed "), "{reply}");
    assert_eq!(state.players[&ids[1]].game_mode, GameMode::Spectator);
    let hold = &state.hostile.battle.live.holds[0];
    let at = state.entities.pos(state.entities.slot_of(ids[1]).unwrap());
    // Red faces +z: behind is -z, above his head.
    assert!((at[0] - hold.at[0]).abs() < 1e-6);
    assert!((at[2] - (hold.at[2] - SPECTATOR_BACK)).abs() < 1e-6);
    assert!((at[1] - (hold.at[1] + SPECTATOR_UP)).abs() < 1e-6);
    assert!(
        command(
            &mut state,
            &mut changes,
            BattleCommand::Spectate {
                name: "CamSpec".into(),
                target: "Nobody".into(),
            },
        )
        .is_err()
    );
}
