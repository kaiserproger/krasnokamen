use super::*;
use crate::battle::combat::{Action, Weapon};
use crate::battle::staging::{self, BattleCommand};
use crate::boundary::{ConnectionId, EntityDataValue, EntityEvent, Hand, PlayerInput};
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
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name.to_owned()).unwrap())
            .unwrap()
    };
    // Foot carries the spear in; the infantry arm wears the iron chestplate.
    assert_eq!(
        kit[0].as_ref().unwrap().item_id,
        item("minecraft:iron_spear")
    );
    assert_eq!(
        kit[4].as_ref().unwrap().item_id,
        item("minecraft:iron_chestplate"),
        "infantry, not spear"
    );
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
    // Nearest the centre of the red foot line (infantry left of the spears,
    // their seam on the origin): the infantry file next to the seam once the
    // staged files open to the fighting interval.
    let opened = crate::battle::formation::position(
        &regiment.spec,
        hold.file,
        0,
        crate::battle::formation::file_interval(regiment),
    );
    assert!((opened[0] - ORIGIN[0]).abs() <= 1.0, "{opened:?} {:?}", hold.at);
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

/// Where his file's front living soldier stands now.
fn file_front(state: &TickState, regiment: usize, file: usize) -> [f64; 3] {
    let formation = &state.hostile.battle.formation;
    let front = file_members(formation, regiment, file)
        .into_iter()
        .map(|m| &formation.members[m])
        .filter(|m| m.alive && !m.wounded)
        .min_by_key(|m| m.rank)
        .expect("a living soldier in his file");
    state.entities.pos(state.entities.slot_of(front.id).unwrap())
}

fn assert_in_rank_with_kit(state: &TickState, player: EntityId) {
    let hold = &state.hostile.battle.live.holds[0];
    let formation = &state.hostile.battle.formation;
    let r = &formation.regiments[hold.regiment];
    let front = file_front(state, hold.regiment, hold.file);
    let body = state.entities.slot_of(player).unwrap();
    let at = state.entities.pos(body);
    // A block ahead of the front soldier, not on his centre; one tick of the
    // soldier's walk since.
    let offset = [at[0] - front[0], at[2] - front[2]];
    let distance = offset[0].hypot(offset[1]);
    let ahead = offset[0] * r.spec.facing[0] + offset[1] * r.spec.facing[1];
    assert!(
        (0.5..1.6).contains(&distance) && ahead > 0.0,
        "{at:?} is not just ahead of file {}'s front soldier at {front:?}",
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

/// His whole inventory, cursor and crafting grid hold exactly the kit, once.
fn assert_exactly_the_kit(state: &TickState, player: EntityId) {
    let kit = &state.hostile.battle.live.holds[0].kit;
    let player_state = &state.players[&player];
    let inventory = &player_state.inventory;
    let selected = usize::from(inventory.selected);
    let mut expected = vec![ItemStack::EMPTY; inventory.slots.len()];
    for (equipment, stack) in kit.iter().enumerate() {
        if let Some(stack) = stack {
            expected[player_index(equipment, selected)] = stack.clone();
        }
    }
    assert_eq!(inventory.slots, expected, "inventory is not exactly the kit");
    let menu = &player_state.inventory_menu;
    assert!(menu.carried.is_empty(), "cursor {:?}", menu.carried);
    assert!(menu.grid.iter().all(ItemStack::is_empty), "grid {:?}", menu.grid);
    let sword = kit[0].as_ref().unwrap().item_id;
    let swords: i32 = inventory
        .slots
        .iter()
        .filter(|stack| !stack.is_empty() && stack.item_id == sword)
        .map(|stack| stack.count)
        .sum();
    assert_eq!(swords, 1, "exactly one sword");
}

/// Before dying he moves the sword out of his hand to `to` and picks up
/// other things (a dirt stack, a stick in the crafting grid, an arrow on the
/// cursor), then he dies and respawns.
fn shuffle_die_and_respawn(state: &mut TickState, player: EntityId, to: usize) {
    let item = |name: &str| {
        state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse(name.to_owned()).unwrap())
            .unwrap()
    };
    let (dirt, stick, arrow) = (
        item("minecraft:dirt"),
        item("minecraft:stick"),
        item("minecraft:arrow"),
    );
    let player_state = state.players.get_mut(&player).unwrap();
    let selected = usize::from(player_state.inventory.selected);
    player_state.inventory.slots.swap(selected, to);
    player_state.inventory.slots[12] = ItemStack::new(dirt, 16);
    player_state.inventory_menu.grid[0] = ItemStack::new(stick, 2);
    player_state.inventory_menu.carried = ItemStack::new(arrow, 3);
    let deaths = state.hostile.battle.live.holds[0].deaths;
    kill(state, 1);
    run_tick(state, &[]);
    assert_eq!(state.hostile.battle.live.holds[0].deaths, deaths + 1);
    run_tick(
        state,
        &[PlayerInput::Respawn {
            connection: ConnectionId(1),
        }],
    );
    run_tick(state, &[]);
    assert!(!state.hostile.battle.live.holds[0].dead);
}

#[test]
fn a_respawn_gives_exactly_the_kit_after_he_moved_his_sword() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    assert_exactly_the_kit(&state, player);
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    // Sword to another hotbar slot.
    let selected = usize::from(state.players[&player].inventory.selected);
    shuffle_die_and_respawn(&mut state, player, (selected + 3) % 9);
    assert_exactly_the_kit(&state, player);
    assert_in_rank_with_kit(&state, player);
}

#[test]
fn two_respawns_in_a_row_give_no_duplicates() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    // Sword to the main inventory, then to the hotbar.
    for to in [20, 7] {
        shuffle_die_and_respawn(&mut state, player, to);
        assert_exactly_the_kit(&state, player);
    }
    // A rejoin's re-arm is the same custody.
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    assert_exactly_the_kit(&state, player);
}

/// A started battle with PovRed stood a pace in front of a blue front-rank
/// foot soldier (blue faces -z); returns PovRed, that soldier and his place.
fn facing_blue() -> (TickState, EntityId, EntityId, [f64; 3]) {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    let formation = &state.hostile.battle.formation;
    let blue = formation
        .members
        .iter()
        .find(|m| {
            let spec = &formation.regiments[m.regiment].spec;
            spec.faction == 2 && spec.frontage == 1.0 && spec.order != Order::Hold && m.rank == 0
        })
        .unwrap()
        .id;
    let at = state.entities.pos(state.entities.slot_of(blue).unwrap());
    (state, player, blue, at)
}

/// Hold the two in contact, as at an engaged front: the soldier at `at`
/// (his line would otherwise march on past), PovRed one block in front of
/// him, inside the vanilla mob reach (centres under 1.43 apart).
fn in_contact(state: &mut TickState, player: EntityId, blue: EntityId, at: [f64; 3]) {
    let body = state.entities.slot_of(blue).unwrap();
    state.entities.set_pos(body, at);
    let body = state.entities.slot_of(player).unwrap();
    state.entities.set_pos(body, [at[0], at[1], at[2] - 1.0]);
}

/// For the next tick every blue fighter but `except` is held in `blue`, and
/// every red one is out of action (red archers shoot at the blue line he
/// stands in front of): only the soldier under test can hurt him.
fn hold(state: &mut TickState, blue: Action, except: Option<EntityId>) {
    let battle = &mut state.hostile.battle;
    for bi in 0..battle.columns.ids.len() {
        if Some(battle.columns.ids[bi]) == except {
            continue;
        }
        battle.combat.fighters[bi].action = match battle.columns.factions[bi] {
            2 => blue,
            _ => Action::Wounded,
        };
    }
}

fn swung(output: &crate::boundary::TickOutput, soldier: EntityId) -> bool {
    output.changes.entity_events.iter().any(|event| {
        matches!(event, EntityEvent::Swing { entity, hand: Hand::Main, .. } if *entity == soldier)
    })
}

#[test]
fn a_spearman_draws_his_sword_on_a_holder_inside_the_spear_bound() {
    let (mut state, player, blue, at) = facing_blue();
    let sword = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:iron_sword".to_owned()).unwrap())
        .unwrap();
    let mut drawn = None;
    for tick in 0..60_u64 {
        // PovRed a block in front: box gap 0.4, inside the spear's 1.0.
        in_contact(&mut state, player, blue, at);
        hold(&mut state, Action::Wounded, Some(blue));
        run_tick(&mut state, &[]);
        let slot = state.entities.slot_of(blue).unwrap();
        let held = state.entities.row(slot).appearance.equipment[0].as_ref();
        if held.is_some_and(|stack| stack.item_id == sword) {
            drawn = Some(tick);
            break;
        }
    }
    assert!(
        drawn.is_some(),
        "the blue spearman never drew his sword on the holder"
    );
}

#[test]
fn wounded_staggered_or_recovering_soldiers_in_reach_never_hit_the_holder() {
    for action in [
        Action::Recover { until: u64::MAX },
        Action::Stagger { until: u64::MAX },
        Action::Wounded,
    ] {
        let (mut state, player, blue, at) = facing_blue();
        for _ in 0..120 {
            in_contact(&mut state, player, blue, at);
            hold(&mut state, action, None);
            let (output, ..) = run_tick(&mut state, &[]);
            assert!(!swung(&output, blue), "{action:?} soldier swung");
        }
        assert_eq!(
            state.players[&player].vitals.health,
            20.0,
            "{action:?} soldiers hit him"
        );
    }
}

#[test]
fn a_ready_enemy_soldier_hits_the_holder_once_per_swing_cycle() {
    let (mut state, player, blue, at) = facing_blue();
    // (tick, windup) of each swing the ready soldier starts, and hit ticks.
    let mut swings: Vec<(u64, u64)> = Vec::new();
    let mut hits = Vec::new();
    let mut health = state.players[&player].vitals.health;
    let main_hand = |state: &TickState| {
        let slot = state.entities.slot_of(blue).unwrap();
        state.entities.row(slot).appearance.equipment[0].clone()
    };
    for tick in 0..600_u64 {
        in_contact(&mut state, player, blue, at);
        // Every other blue man out of action too, archers included (Recover
        // does not stop a bow).
        hold(&mut state, Action::Wounded, Some(blue));
        let before = main_hand(&state);
        let (output, ..) = run_tick(&mut state, &[]);
        // A weapon swap's main-hand swing comes with his main-hand change.
        if swung(&output, blue) && main_hand(&state) == before {
            let battle = &state.hostile.battle;
            let slot = state.entities.slot_of(blue).unwrap();
            let bi = battle.columns.index(slot).unwrap();
            let Action::Windup { target, weapon, .. } = battle.combat.fighters[bi].action else {
                panic!("swing without a windup: {:?}", battle.combat.fighters[bi].action);
            };
            assert_eq!(target, player, "his swing is at the holder");
            assert_ne!(weapon, Weapon::Bow);
            let raised = state.entities.row(slot).appearance.extra.iter().any(|(index, value)| {
                *index == 8 && matches!(value, EntityDataValue::Byte(flags) if *flags != 0)
            });
            assert!(!raised, "the shield is lowered for the swing");
            swings.push((tick, u64::from(weapon.windup())));
        }
        let now = state.players[&player].vitals.health;
        if now < health {
            hits.push(tick);
        }
        health = now;
        if hits.len() >= 2 || now <= 0.0 {
            break;
        }
    }
    assert!(!hits.is_empty(), "the ready soldier never hit him; swings {swings:?}");
    // Every blow is the impact of his own swing, one per cycle.
    for &hit in &hits {
        assert!(
            swings.iter().any(|&(swing, windup)| swing + windup == hit),
            "hit at {hit} is no swing's impact: hits {hits:?} swings {swings:?}"
        );
    }
    assert!(hits.len() <= swings.len(), "hits {hits:?} swings {swings:?}");
    // His own side never swings: a red holder beside red soldiers only.
    let (mut state, ids) = staged(&["PovRed"]);
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    for _ in 0..60 {
        run_tick(&mut state, &[]);
    }
    assert_eq!(state.players[&ids[0]].vitals.health, 20.0, "red soldiers swung at him");
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
    // Every red infantry line breaks while he is dead, out of contact: no
    // soldier of his line has an enemy within reach.
    let hold = state.hostile.battle.live.holds[0].clone();
    let infantry = hold.regiment;
    let line = &mut state.hostile.battle.formation.regiments[infantry];
    line.morale.broken = true;
    let keep = (hold.regiment, hold.file);
    assert!(nearest_file(&state, infantry, keep, hold.alive_at, true).is_none());
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

/// The virtual leader (`spec.origin`) walks on along the facing while men
/// stay in melee or at the field bounds (trial-5: the holder respawned at
/// least 1,500 blocks from every soldier the recorder saw): a respawn goes
/// to his file's men, not to the leader's slot.
#[test]
fn a_respawn_goes_to_his_files_men_when_the_leader_walked_far_ahead() {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    kill(&mut state, 1);
    run_tick(&mut state, &[]);
    for r in &mut state.hostile.battle.formation.regiments {
        r.spec.origin[0] += r.spec.facing[0] * 150.0;
        r.spec.origin[2] += r.spec.facing[1] * 150.0;
    }
    run_tick(
        &mut state,
        &[PlayerInput::Respawn {
            connection: ConnectionId(1),
        }],
    );
    run_tick(&mut state, &[]);
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

fn respawn(state: &mut TickState, connection: u64) {
    run_tick(
        state,
        &[PlayerInput::Respawn {
            connection: ConnectionId(connection),
        }],
    );
    run_tick(state, &[]);
}

/// As the last battle tick left it for the next respawn: the front soldier
/// of `file` retains a soldier of the other side as his opponent, standing
/// within his melee range or a block beyond it (box gap along +z). Each call
/// takes an opponent no soldier retains yet, so several files can be in
/// contact at once.
fn engage(state: &mut TickState, regiment: usize, file: usize, in_reach: bool) {
    let battle = &mut state.hostile.battle;
    let formation = &battle.formation;
    let faction = formation.regiments[regiment].spec.faction;
    let taken: Vec<EntityId> = battle.columns.targets.iter().flatten().map(|t| t.id).collect();
    let enemy = formation
        .members
        .iter()
        .find(|m| formation.regiments[m.regiment].spec.faction != faction && !taken.contains(&m.id))
        .unwrap()
        .id;
    let bi = file_members(formation, regiment, file)[0];
    let reach = battle.combat.fighters[bi].range.max;
    assert!(
        battle.columns.active[bi],
        "file {file}'s soldier stands in the battle"
    );
    let actor = battle.columns.slots[bi].unwrap();
    battle.columns.targets[bi] = Some(EnemyTarget {
        id: enemy,
        pos: battle.columns.positions[bi],
        distance: 1.0,
    });
    // Two 0.6-wide soldiers: centre distance = box gap + 0.6.
    let gap = if in_reach { reach * 0.5 } else { reach + 1.0 };
    let at = state.entities.pos(actor);
    let body = state.entities.slot_of(enemy).unwrap();
    state
        .entities
        .set_pos(body, [at[0], at[1], at[2] + 0.6 + gap]);
}

/// A started battle in which PovRed, holding the centre file, fell a block
/// ahead of the front soldier of the middle file of his line and is dead;
/// returns PovRed, his regiment, that file and the centre file the old rule
/// takes.
fn fallen() -> (TickState, EntityId, usize, usize, usize) {
    let (mut state, ids) = staged(&["PovRed"]);
    let player = ids[0];
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    let hold = state.hostile.battle.live.holds[0].clone();
    let line = &state.hostile.battle.formation.regiments[hold.regiment];
    let files = usize::from(line.spec.files);
    let centre = choose_slot(&state, 1, Some((hold.regiment, hold.file))).unwrap();
    assert_eq!(centre, (hold.regiment, hold.file));
    let fell = files / 2;
    // Red faces +z: a block ahead of that file's front soldier.
    let front = file_front(&state, hold.regiment, fell);
    let body = state.entities.slot_of(player).unwrap();
    state
        .entities
        .set_pos(body, [front[0], front[1], front[2] + 1.0]);
    run_tick(&mut state, &[]);
    kill(&mut state, 1);
    run_tick(&mut state, &[]);
    assert!(state.hostile.battle.live.holds[0].dead);
    (state, player, hold.regiment, fell, hold.file)
}

/// Trial-7: PovRed died at the west end of the line and the centre rule put
/// him 230 blocks away, with no enemy near for four minutes. A respawn goes
/// to the file of his own line in contact nearest where he fell.
#[test]
fn a_respawn_goes_to_the_file_of_his_line_in_contact_nearest_where_he_fell() {
    let (mut state, player, regiment, near, centre) = fallen();
    // The centre file is at the seam with the spears, an end of his line;
    // the far file is the other end.
    let line = &state.hostile.battle.formation.regiments[regiment];
    let files = usize::from(line.spec.files);
    let far = if centre == 0 { files - 1 } else { 0 };
    assert!(
        near != centre && far != centre && near != far,
        "{near} {far} {centre}"
    );
    engage(&mut state, regiment, far, true);
    engage(&mut state, regiment, near, true);
    for file in [far, near] {
        let battle = &state.hostile.battle;
        assert!(
            file_members(&battle.formation, regiment, file)
                .into_iter()
                .any(|bi| in_contact(&state.entities, battle, bi)),
            "file {file} is in contact before the respawn"
        );
    }
    respawn(&mut state, 1);
    let hold = &state.hostile.battle.live.holds[0];
    assert_eq!((hold.regiment, hold.file), (regiment, near));
    assert_in_rank_with_kit(&state, player);
}

/// A standing red foot line other than `own`.
fn other_line(state: &TickState, own: usize) -> usize {
    let formation = &state.hostile.battle.formation;
    (0..formation.regiments.len())
        .find(|&i| i != own && eligible(state, &formation.regiments[i], 1, 1))
        .expect("a second red foot line")
}

/// No file of his own regiment in contact (a retained opponent beyond weapon
/// reach is none), while another red line and the blue side fight: he stays
/// in his own regiment, at its living file nearest where he fell.
#[test]
fn a_respawn_with_no_contact_in_his_regiment_takes_its_file_nearest_where_he_fell() {
    let (mut state, player, regiment, fell, centre) = fallen();
    assert_ne!(fell, centre);
    engage(&mut state, regiment, fell, false);
    let spears = other_line(&state, regiment);
    engage(&mut state, spears, 0, true);
    let formation = &state.hostile.battle.formation;
    let blue = (0..formation.regiments.len())
        .find(|&i| formation.regiments[i].spec.faction == 2)
        .unwrap();
    engage(&mut state, blue, 0, true);
    respawn(&mut state, 1);
    let hold = &state.hostile.battle.live.holds[0];
    assert_eq!((hold.regiment, hold.file), (regiment, fell));
    assert_in_rank_with_kit(&state, player);
}

/// His regiment broke but a file of it is still in contact: he respawns at
/// that file, though a standing red line fights too.
#[test]
fn a_respawn_goes_to_his_broken_regiments_file_in_contact() {
    let (mut state, player, regiment, fell, centre) = fallen();
    assert_ne!(fell, centre);
    state.hostile.battle.formation.regiments[regiment]
        .morale
        .broken = true;
    let spears = other_line(&state, regiment);
    engage(&mut state, spears, 0, true);
    engage(&mut state, regiment, fell, true);
    respawn(&mut state, 1);
    let hold = &state.hostile.battle.live.holds[0];
    assert_eq!((hold.regiment, hold.file), (regiment, fell));
    assert_in_rank_with_kit(&state, player);
}

/// His regiment has no living file left: he goes to the file of another
/// standing red line in contact nearest where he fell, not to the centre file
/// of the old rule.
#[test]
fn a_respawn_after_his_regiment_is_gone_takes_the_sides_contact_nearest_where_he_fell() {
    let (mut state, player, regiment, _, _) = fallen();
    let kit = state.hostile.battle.live.holds[0].kit.clone();
    // Every man of his regiment fell while he was dead.
    let formation = &mut state.hostile.battle.formation;
    let line = &mut formation.regiments[regiment];
    let (start, len) = (line.start, line.len);
    line.live_count = 0;
    for member in &mut formation.members[start..start + len] {
        member.alive = false;
    }
    let spears = other_line(&state, regiment);
    let keep = {
        let hold = &state.hostile.battle.live.holds[0];
        (hold.regiment, hold.file)
    };
    let centre = choose_slot(&state, 1, Some(keep)).unwrap();
    let files = usize::from(state.hostile.battle.formation.regiments[spears].spec.files);
    // The end of the spear line away from the old rule's file; he fell there.
    let far = match centre {
        (line, 0) if line == spears => files - 1,
        _ => 0,
    };
    let mid = files / 2;
    assert!(
        far != mid && centre != (spears, far),
        "{far} {mid} {centre:?}"
    );
    state.hostile.battle.live.holds[0].alive_at = file_front(&state, spears, far);
    engage(&mut state, spears, mid, true);
    engage(&mut state, spears, far, true);
    respawn(&mut state, 1);
    let hold = &state.hostile.battle.live.holds[0];
    assert_eq!((hold.regiment, hold.file), (spears, far));
    assert_eq!(hold.kit, kit, "his own kit, not the spear line's");
    assert_in_rank_with_kit(&state, player);
}

fn assert_behind(state: &TickState, viewer: EntityId, player: EntityId) {
    assert_eq!(state.players[&viewer].game_mode, GameMode::Spectator);
    let hold = &state.hostile.battle.live.holds[0];
    let facing = state.hostile.battle.formation.regiments[hold.regiment]
        .spec
        .facing;
    let pos = state.entities.pos(state.entities.slot_of(player).unwrap());
    let at = state.entities.pos(state.entities.slot_of(viewer).unwrap());
    let behind = [
        pos[0] - facing[0] * SPECTATOR_BACK,
        pos[1] + SPECTATOR_UP,
        pos[2] - facing[1] * SPECTATOR_BACK,
    ];
    assert!(
        (0..3).all(|axis| (at[axis] - behind[axis]).abs() < 1e-3),
        "spectator at {at:?}, not behind the holder at {pos:?}"
    );
}

/// Trial-7: after PovRed's death and respawn CamSpec stayed in the cave where
/// he died, 230 blocks away, and saw nothing for four minutes. The spectator
/// bound by `battle spectate` goes behind him again on each respawn and
/// re-arm.
#[test]
fn the_bound_spectator_goes_behind_the_holder_again_after_a_respawn_and_a_rearm() {
    let (mut state, ids) = staged(&["PovRed", "CamSpec"]);
    let (player, viewer) = (ids[0], ids[1]);
    let mut changes = ChangeSet::default();
    command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    command(
        &mut state,
        &mut changes,
        BattleCommand::Spectate {
            name: "CamSpec".into(),
            target: "PovRed".into(),
        },
    )
    .unwrap();
    command(&mut state, &mut changes, BattleCommand::Start).unwrap();
    run_tick(&mut state, &[]);
    // He flew down after the holder into a cave far from the line.
    let away = |state: &mut TickState| {
        let body = state.entities.slot_of(viewer).unwrap();
        state
            .entities
            .set_pos(body, [ORIGIN[0] + 40.0, 20.0, ORIGIN[2] - 40.0]);
    };
    away(&mut state);
    kill(&mut state, 1);
    run_tick(&mut state, &[]);
    respawn(&mut state, 1);
    assert!(!state.hostile.battle.live.holds[0].dead);
    assert_behind(&state, viewer, player);
    away(&mut state);
    let reply = command(&mut state, &mut changes, live_cmd("PovRed")).unwrap();
    assert!(reply.starts_with("Battle live rearmed: PovRed "), "{reply}");
    assert_behind(&state, viewer, player);
}
