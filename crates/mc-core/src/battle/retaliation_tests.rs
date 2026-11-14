use super::*;
use crate::battle::RETARGET_TICKS;
use crate::battle::combat::Action;
use crate::battle::formation::Order;
use crate::battle::live_players::tests::{facing_blue, hold};
use crate::boundary::{ConnectionId, EntityEvent, PlayerInput};
use crate::tick::run_tick;

/// His regiment holds its ground, so nothing but an answer moves him toward
/// PovRed, and he carries no shield: a guard's raised shield blocks the jab.
fn stand_fast(state: &mut TickState, soldier: EntityId) {
    let formation = &mut state.hostile.battle.formation;
    let member = formation.members.iter().find(|m| m.id == soldier).unwrap();
    let id = formation.regiments[member.regiment].spec.id;
    assert!(formation.set_order(id, Order::Hold));
    let slot = state.entities.slot_of(soldier).unwrap();
    std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance).equipment[1] = None;
    state.appearance_changed.push(soldier);
}

fn put(state: &mut TickState, id: EntityId, at: [f64; 3]) {
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_pos(slot, at);
    state.entities.set_vel(slot, [0.0; 3]);
}

/// PovRed at `spot`, facing the blue line (+z), level; every blue fighter but
/// `blue` and every red one out of action.
fn ready(state: &mut TickState, player: EntityId, blue: EntityId, spot: [f64; 3]) {
    put(state, player, spot);
    let slot = state.entities.slot_of(player).unwrap();
    state.entities.set_rot(slot, 0.0, 0.0);
    hold(state, Action::Wounded, Some(blue));
}

/// The spear's charge (19-tick delay) back after the kit was put in his hand;
/// the soldier stands at `at` meanwhile.
fn recharge(state: &mut TickState, player: EntityId, blue: EntityId, spot: [f64; 3], at: [f64; 3]) {
    for _ in 0..20 {
        ready(state, player, blue, spot);
        put(state, blue, at);
        run_tick(state, &[]);
    }
}

fn hurt_by(output: &crate::boundary::TickOutput, victim: EntityId, attacker: EntityId) -> bool {
    output.changes.entity_events.iter().any(|event| {
        matches!(event, EntityEvent::Damage { entity, cause, .. }
            if *entity == victim && *cause == Some(attacker))
    })
}

fn jab() -> PlayerInput {
    PlayerInput::Stab {
        connection: ConnectionId(1),
    }
}

fn distance(state: &TickState, a: EntityId, b: EntityId) -> f64 {
    let [a, b] = [a, b].map(|id| state.entities.pos(state.entities.slot_of(id).unwrap()));
    (a[0] - b[0]).hypot(a[2] - b[2])
}

fn column_target(state: &TickState, soldier: EntityId) -> Option<EntityId> {
    let battle = &state.hostile.battle;
    let bi = battle.columns.index(state.entities.slot_of(soldier)?)?;
    battle.columns.targets[bi].map(|target| target.id)
}

fn assert_jab_hurt(
    state: &TickState,
    output: &crate::boundary::TickOutput,
    player: EntityId,
    blue: EntityId,
) {
    let damaged: Vec<_> = output
        .changes
        .entity_events
        .iter()
        .filter_map(|event| match event {
            EntityEvent::Damage { entity, cause, .. } => Some((*entity, *cause)),
            _ => None,
        })
        .collect();
    assert!(
        hurt_by(output, blue, player),
        "the jab missed the soldier {blue:?} {:.2} blocks away; damage events {damaged:?}",
        distance(state, player, blue)
    );
}

/// G2L5 (owner questions 1 and 3): PovRed jabbed blue soldiers from 2.4–4.3
/// blocks and none of them ever answered him. Jabbed once from 3.5 blocks,
/// beyond his own spear's mob reach (box gap 2.25), a blue soldier whose line
/// holds its ground takes PovRed as his target (where he looks), steps in
/// and swings at him within the three seconds, and the blow lands. Three
/// seconds after that jab he lets him go and walks back to his slot.
#[test]
fn a_soldier_jabbed_from_beyond_his_reach_turns_steps_in_and_strikes_back() {
    let (mut state, player, blue, at) = facing_blue();
    stand_fast(&mut state, blue);
    let spot = [at[0], at[1], at[2] - 3.5];
    put(&mut state, blue, at);
    recharge(&mut state, player, blue, spot, at);
    ready(&mut state, player, blue, spot);
    put(&mut state, blue, at);
    let (output, ..) = run_tick(&mut state, &[jab()]);
    assert_jab_hurt(&state, &output, player, blue);
    let start = distance(&state, player, blue);
    let (mut turned, mut swung_at, mut struck) = (None, None, None);
    let mut nearest = start;
    for tick in 1..=60_u64 {
        ready(&mut state, player, blue, spot);
        let (output, ..) = run_tick(&mut state, &[]);
        if turned.is_none() && column_target(&state, blue) == Some(player) {
            turned = Some(tick);
        }
        nearest = nearest.min(distance(&state, player, blue));
        if swung_at.is_none() && crate::battle::live_players::tests::swung(&output, blue) {
            let battle = &state.hostile.battle;
            let bi = battle
                .columns
                .index(state.entities.slot_of(blue).unwrap())
                .unwrap();
            if let Action::Windup { target, .. } = battle.combat.fighters[bi].action {
                assert_eq!(target, player, "his swing is at PovRed");
                swung_at = Some(tick);
            }
        }
        if struck.is_none() && hurt_by(&output, player, blue) {
            struck = Some(tick);
        }
    }
    let turned = turned.expect("the soldier never took PovRed as his target");
    assert!(turned <= RETARGET_TICKS, "turned only after {turned} ticks");
    assert!(
        nearest < start - 0.5,
        "he never stepped in: {start:.2} -> {nearest:.2}"
    );
    let swung_at = swung_at.expect("he never swung at PovRed within three seconds");
    let struck = struck.expect("his blow never landed on PovRed");
    assert!(
        struck > swung_at,
        "hit {struck} before the swing {swung_at}"
    );
    // No more jabs: the grudge lapses 60 ticks after the jab, at his next
    // re-pick at the latest, and he walks back to his slot out of reach.
    for _ in 61..=(GRUDGE_TICKS + RETARGET_TICKS + 1) {
        ready(&mut state, player, blue, spot);
        run_tick(&mut state, &[]);
    }
    assert_ne!(
        column_target(&state, blue),
        Some(player),
        "the grudge never lapsed"
    );
    assert_eq!(state.hostile.battle.retaliation.answering(blue), None);
    let index = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .position(|m| m.id == blue)
        .unwrap();
    assert_eq!(state.hostile.battle.formation.members[index].close_on, None);
    for _ in 0..40 {
        ready(&mut state, player, blue, spot);
        run_tick(&mut state, &[]);
    }
    assert!(
        distance(&state, player, blue) > 3.0,
        "he stayed on PovRed after the grudge lapsed"
    );
}

/// The jabbed soldier cannot step in (held where he stands, as when his own
/// rank or PovRed's blocks him), so PovRed stays beyond his reach: after the
/// step-in time he hands PovRed to the idle blue soldiers nearest PovRed,
/// never more than two however long the jabs go on, and they answer him.
/// Nobody else targets PovRed but soldiers his jabs hurt (a jab pierces).
#[test]
fn an_attacker_out_of_reach_goes_to_at_most_two_idle_friends() {
    let (mut state, player, blue, at) = facing_blue();
    stand_fast(&mut state, blue);
    let spot = [at[0], at[1], at[2] - 3.5];
    put(&mut state, blue, at);
    recharge(&mut state, player, blue, spot, at);
    let mut helpers_max = 0;
    let mut helped_from = None;
    let mut first = None;
    for tick in 0..200_u64 {
        ready(&mut state, player, blue, spot);
        put(&mut state, blue, at);
        let input = if tick % 20 == 0 {
            vec![jab()]
        } else {
            Vec::new()
        };
        let (output, ..) = run_tick(&mut state, &input);
        if hurt_by(&output, blue, player) {
            first.get_or_insert(tick);
        }
        let battle = &state.hostile.battle;
        let mut helpers = Vec::new();
        for bi in 0..battle.columns.ids.len() {
            let id = battle.columns.ids[bi];
            if id == blue || battle.columns.targets[bi].map(|t| t.id) != Some(player) {
                continue;
            }
            assert_eq!(
                battle.columns.factions[bi], 2,
                "a red soldier targets PovRed"
            );
            let Some((attacker, helping)) = battle.retaliation.answering(id) else {
                panic!("{id:?} targets PovRed neither hurt by him nor handed him");
            };
            assert_eq!(attacker, player);
            if helping {
                helpers.push(id);
            }
        }
        if !helpers.is_empty() {
            helped_from.get_or_insert(tick);
        }
        helpers_max = helpers_max.max(helpers.len());
    }
    let first = first.expect("no jab hurt the soldier");
    assert_eq!(
        state.hostile.battle.retaliation.answering(blue),
        Some((player, false)),
        "the jabbed soldier does not answer PovRed"
    );
    let helped_from = helped_from.expect("nobody took PovRed over from the jabbed soldier");
    assert!(
        helped_from >= first + STEP_IN_TICKS,
        "handed on at {helped_from}, before the step-in time from {first}"
    );
    assert_eq!(helpers_max, HELPERS, "helpers per attacker");
}

/// Two enemies hurt one soldier inside the window (PovRed's jab, then a red
/// soldier's blow): at his next re-pick, long before the step-in time, both
/// attackers go to idle friends, at most two each.
#[test]
fn a_soldier_hurt_by_two_enemies_hands_both_on() {
    let (mut state, player, blue, at) = facing_blue();
    stand_fast(&mut state, blue);
    let red = state
        .hostile
        .battle
        .formation
        .members
        .iter()
        .find(|m| {
            state.hostile.battle.formation.regiments[m.regiment]
                .spec
                .faction
                == 1
                && m.rank == 0
        })
        .unwrap()
        .id;
    let red_at = [at[0] + 1.5, at[1], at[2] - 1.0];
    let spot = [at[0], at[1], at[2] - 3.5];
    put(&mut state, blue, at);
    recharge(&mut state, player, blue, spot, at);
    ready(&mut state, player, blue, spot);
    put(&mut state, blue, at);
    put(&mut state, red, red_at);
    let (output, ..) = run_tick(&mut state, &[jab()]);
    assert_jab_hurt(&state, &output, player, blue);
    // The red soldier's blow as the damage merge reports it.
    let slot = state.entities.slot_of(blue).unwrap();
    hurt(&mut state, slot, red);
    let mut most = [0_usize; 2];
    for _ in 0..(RETARGET_TICKS + 1) {
        ready(&mut state, player, blue, spot);
        put(&mut state, blue, at);
        put(&mut state, red, red_at);
        run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let mut count = [0; 2];
        for &id in &battle.columns.ids {
            match battle.retaliation.answering(id) {
                Some((attacker, true)) if attacker == player => count[0] += 1,
                Some((attacker, true)) if attacker == red => count[1] += 1,
                _ => {}
            }
        }
        most = [most[0].max(count[0]), most[1].max(count[1])];
    }
    assert_eq!(
        most,
        [HELPERS, HELPERS],
        "helpers for PovRed and for the red soldier"
    );
}

/// Between soldiers a landed blow wounds or kills, so a soldier-only contact
/// turns nobody from his nearest enemy: the soldiers' blows reach
/// retaliation, yet no soldier ever answers a soldier and nobody is handed an
/// attacker.
#[test]
fn soldier_blows_between_soldiers_turn_nobody_from_the_nearest_enemy() {
    let mut state = crate::battle::tests::staged_flat_fixture(100);
    crate::battle::staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &crate::battle::staging::BattleCommand::Start,
    )
    .unwrap();
    let (mut remembered, mut contact) = (0, 0);
    for _ in 0..2400 {
        run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        contact += usize::from(battle.combat.work.in_weapon_reach > 0);
        remembered = remembered.max(battle.retaliation.work.remembered);
        for &id in &battle.columns.ids {
            assert_eq!(
                battle.retaliation.answering(id),
                None,
                "{id:?} answers a soldier"
            );
        }
        if contact >= 600 {
            break;
        }
    }
    assert!(contact >= 600, "only {contact} ticks of contact");
    let (dead, removed, _) = crate::battle::tests::native_battle_losses(&state);
    assert!(
        removed.iter().sum::<usize>() > 0,
        "no blow landed: dead {dead:?}"
    );
    assert!(remembered > 0, "no soldier's blow reached retaliation");
}
