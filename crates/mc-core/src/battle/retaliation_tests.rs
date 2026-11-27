use super::*;
use crate::battle::RETARGET_TICKS;
use crate::battle::combat::Action;
use crate::battle::formation::Order;
use crate::battle::live_players::tests::{facing_blue, hold};
use crate::boundary::{EntityEvent, PlayerInput};
use crate::tick::run_tick;
use mc_domain::ConnectionId;

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
    ((a[0] - b[0]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
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
    // The calibrated removal rate is below one percent per landed blow.
    // Exercise a full front, not a tiny skirmish with no nonfatal removals.
    let mut state = crate::battle::tests::staged_flat_fixture(1_000);
    crate::battle::staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &crate::battle::staging::BattleCommand::Start,
    )
    .unwrap();
    let (mut remembered, mut contact) = (0, 0);
    let (mut wounded, mut killed, mut admitted) = (0, 0, 0);
    for _ in 0..2400 {
        run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        contact += usize::from(battle.combat.work.in_weapon_reach > 0);
        remembered = remembered.max(battle.retaliation.work.remembered);
        wounded += battle.combat.work.wounded;
        killed += battle.combat.work.killed;
        admitted += battle.retaliation.work.blows;
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
    assert!(
        remembered > 0,
        "no soldier's blow reached retaliation; wounded={wounded}, killed={killed}, admitted={admitted}"
    );
}

/// Soldiers answering `attacker` as helpers.
fn helpers_of(state: &TickState, attacker: EntityId) -> Vec<EntityId> {
    let battle = &state.hostile.battle;
    battle
        .columns
        .ids
        .iter()
        .copied()
        .filter(|&id| battle.retaliation.answering(id) == Some((attacker, true)))
        .collect()
}

/// PovRed at `spot` jabs `victim`, held at `at`, every 20 ticks for `ticks`.
fn jab_held(
    state: &mut TickState,
    player: EntityId,
    victim: EntityId,
    at: [f64; 3],
    spot: [f64; 3],
    ticks: u64,
) {
    for tick in 0..ticks {
        ready(state, player, victim, spot);
        put(state, victim, at);
        let input = if tick % 20 == 0 {
            vec![jab()]
        } else {
            Vec::new()
        };
        run_tick(state, &input);
    }
}

/// Fable F1: PovRed leaves the crew he drew (he goes 18+ blocks along the
/// line to jab another soldier who cannot reach him). The helpers left behind
/// can no longer answer him, so they leave his crew at once, and idle men near
/// his new place take it over, still at most two.
#[test]
fn helpers_left_behind_give_the_crew_to_friends_near_his_new_place() {
    let (mut state, player, blue, at) = facing_blue();
    stand_fast(&mut state, blue);
    let spot = [at[0], at[1], at[2] - 3.5];
    recharge(&mut state, player, blue, spot, at);
    jab_held(&mut state, player, blue, at, spot, 100);
    let left = helpers_of(&state, player);
    assert_eq!(left.len(), HELPERS, "no crew at his first place");
    let (other, other_at) = {
        let formation = &state.hostile.battle.formation;
        formation
            .members
            .iter()
            .filter(|m| {
                let spec = &formation.regiments[m.regiment].spec;
                spec.faction == 2 && spec.frontage == 1.0 && m.rank == 0
            })
            .map(|m| {
                (
                    m.id,
                    state.entities.pos(state.entities.slot_of(m.id).unwrap()),
                )
            })
            .find(|(_, p)| {
                let squared = (p[0] - at[0]).powi(2) + (p[2] - at[2]).powi(2);
                (18.0_f64.powi(2)..40.0_f64.powi(2)).contains(&squared)
            })
            .expect("a blue front-rank soldier 18-40 blocks along the line")
    };
    stand_fast(&mut state, other);
    let other_spot = [other_at[0], other_at[1], other_at[2] - 3.5];
    jab_held(&mut state, player, other, other_at, other_spot, 100);
    for id in &left {
        assert_eq!(
            state.hostile.battle.retaliation.answering(*id),
            None,
            "{id:?} still helps against PovRed 18+ blocks away"
        );
    }
    let crew = helpers_of(&state, player);
    assert_eq!(
        crew.len(),
        HELPERS,
        "no new crew at his new place: {crew:?}"
    );
    assert!(
        crew.iter().all(|id| !left.contains(id)),
        "{crew:?} vs {left:?}"
    );
}

/// Fable F5 / H5: a jabbed rider answers through his horse. PovRed is no
/// battle column, so `cavalry` steers the targeted rider at him as at a flank.
/// The jab's knockback can carry the horse past 6 blocks (from 4 to about
/// 6.2 here), so the rider takes PovRed exactly at his first re-pick that
/// finds him within [`MELEE_ENGAGEMENT_REACH`], not before and not later; the
/// horse closes on him, and within the three seconds PovRed is inside the
/// rider's weapon reach (combat strikes an answered holder in reach first)
/// or, when the horses' spacing keeps him out of it, is handed on.
#[test]
fn a_jabbed_rider_turns_his_horse_on_the_holder() {
    let (mut state, player, _, _) = facing_blue();
    let (rider, horse) = {
        let formation = &state.hostile.battle.formation;
        formation
            .members
            .iter()
            .filter(|m| {
                let spec = &formation.regiments[m.regiment].spec;
                spec.faction == 2 && spec.frontage > 1.0 && m.rank == 0
            })
            .find_map(|m| {
                let slot = state.entities.slot_of(m.id)?;
                Some((m.id, state.entities.row(slot).entity_life.mount.vehicle?))
            })
            .expect("a blue rider in the front rank")
    };
    stand_fast(&mut state, rider);
    let horse_at = state.entities.pos(state.entities.slot_of(horse).unwrap());
    let spot = [horse_at[0], horse_at[1], horse_at[2] - 4.0];
    // The spear's charge comes back while the horse (the rider sits on it)
    // stands where it stood.
    for _ in 0..20 {
        ready(&mut state, player, rider, spot);
        put(&mut state, horse, horse_at);
        run_tick(&mut state, &[]);
    }
    ready(&mut state, player, rider, spot);
    put(&mut state, horse, horse_at);
    let (output, ..) = run_tick(&mut state, &[jab()]);
    assert_jab_hurt(&state, &output, player, rider);
    let start = distance(&state, player, horse);
    let (mut turned, mut nearest, mut reached) = (None, start, false);
    for tick in 1..=60_u64 {
        ready(&mut state, player, rider, spot);
        // What this tick's re-pick sees: the rider where the last tick left
        // him, PovRed at `spot`.
        let away = {
            let rider_at = state.entities.pos(state.entities.slot_of(rider).unwrap());
            crate::target_grid::distance(rider_at, spot)
        };
        let due = crate::battle::retarget_due(rider, state.hostile.battle.elapsed_ticks);
        run_tick(&mut state, &[]);
        nearest = nearest.min(distance(&state, player, horse));
        if turned.is_none() {
            let takes = column_target(&state, rider) == Some(player);
            assert_eq!(
                takes,
                due && away <= MELEE_ENGAGEMENT_REACH,
                "tick {tick}: re-pick due {due}, PovRed {away:.2} blocks off, taken {takes}"
            );
            if takes {
                turned = Some(tick);
            }
            continue;
        }
        let battle = &state.hostile.battle;
        let slot = state.entities.slot_of(rider).unwrap();
        let bi = battle.columns.index(slot).unwrap();
        let range = battle.combat.fighters[bi].range;
        reached |= crate::battle::live_players::in_reach(
            battle,
            &state.entities,
            slot,
            range,
            Some(player),
        )
        .is_some();
    }
    turned.expect("the rider never took PovRed as his target");
    assert!(
        nearest < start - 0.5,
        "his horse never closed: {start:.2} -> {nearest:.2}"
    );
    let handed = !helpers_of(&state, player).is_empty();
    assert!(
        reached || handed,
        "PovRed never came into the rider's reach and was not handed on"
    );
}

/// Fable F5: PovRed dies while a soldier answers him. The soldier lets him go
/// on the next battle tick (no target, no step toward him) and never swings at
/// the body.
#[test]
fn a_soldier_lets_a_dead_holder_go_at_once() {
    let (mut state, player, blue, at) = facing_blue();
    stand_fast(&mut state, blue);
    let spot = [at[0], at[1], at[2] - 3.5];
    recharge(&mut state, player, blue, spot, at);
    ready(&mut state, player, blue, spot);
    put(&mut state, blue, at);
    let (output, ..) = run_tick(&mut state, &[jab()]);
    assert_jab_hurt(&state, &output, player, blue);
    let mut answering = false;
    for _ in 0..=RETARGET_TICKS {
        ready(&mut state, player, blue, spot);
        run_tick(&mut state, &[]);
        answering |= column_target(&state, blue) == Some(player);
    }
    assert!(answering, "the soldier never answered PovRed");
    run_tick(
        &mut state,
        &[PlayerInput::Command {
            connection: ConnectionId(1),
            command: "kill".into(),
            permission_level: 2,
        }],
    );
    assert!(
        state.players[&player].vitals.is_dead(),
        "PovRed survived the kill"
    );
    run_tick(&mut state, &[]);
    assert_ne!(
        column_target(&state, blue),
        Some(player),
        "he still targets the dead PovRed"
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
        run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let bi = battle
            .columns
            .index(state.entities.slot_of(blue).unwrap())
            .unwrap();
        assert!(
            !matches!(battle.combat.fighters[bi].action, Action::Windup { target, .. } if target == player),
            "he swings at the dead PovRed"
        );
    }
}

/// Fable F3: only a blow that can lead to an answer or a hand-off is
/// remembered. An enemy beyond [`HELP_RADIUS`] (a far archer's arrow) can be
/// neither answered nor handed on, and a horse is no soldier: neither blow
/// leaves a grudge, so the 20k pass stays small.
#[test]
fn blows_that_can_lead_nowhere_are_not_remembered() {
    let (mut state, player, blue, at) = facing_blue();
    let far = {
        let formation = &state.hostile.battle.formation;
        formation
            .members
            .iter()
            .filter(|m| formation.regiments[m.regiment].spec.faction == 1)
            .map(|m| m.id)
            .find(|&id| distance(&state, id, blue) > HELP_RADIUS + 1.0)
            .expect("a red soldier beyond the help radius")
    };
    let horse = state
        .entities
        .rider_slots()
        .find(|&slot| state.entities.body(slot).faction == 2)
        .expect("a blue horse");
    let horse_at = state.entities.pos(horse);
    let spot = [at[0], at[1], at[2] - 3.5];
    ready(&mut state, player, blue, spot);
    put(
        &mut state,
        player,
        [horse_at[0], horse_at[1], horse_at[2] - 3.0],
    );
    let slot = state.entities.slot_of(blue).unwrap();
    hurt(&mut state, slot, far);
    hurt(&mut state, horse, player);
    run_tick(&mut state, &[]);
    let retaliation = &state.hostile.battle.retaliation;
    assert!(
        retaliation.work.blows >= 2,
        "blows {}",
        retaliation.work.blows
    );
    assert!(
        !retaliation.grudges.contains_key(&blue),
        "the far blow was remembered"
    );
    let horse_id = state.entities.body(horse).id;
    assert!(
        !retaliation.grudges.contains_key(&horse_id),
        "the horse's blow was remembered"
    );
}

/// Capture keeps a pending blow only by an attacker the image carries: a held
/// player and a living native unit. An ordinary player and an attacker without
/// a row are blows the next pass discards; replay still refuses an image that
/// names an unknown identity.
#[test]
fn battle_resume_capture_keeps_only_attackers_the_image_carries() {
    let (mut state, holder, blue, at) = facing_blue();
    run_tick(
        &mut state,
        &[PlayerInput::Join {
            connection: ConnectionId(78),
            uuid: uuid::Uuid::new_v4(),
            name: "Visitor".into(),
            permission_level: 0,
        }],
    );
    let visitor = state.connections.entity_of(ConnectionId(78)).unwrap();
    assert!(opponent(&state.entities, &state.hostile.battle.live, 2, at, visitor).is_none());
    let native = state.hostile.battle.formation.members[0].id;
    let unknown = EntityId(i32::MAX - 1);
    let carried = [(blue, holder), (blue, native)];
    state.hostile.battle.retaliation.pending =
        [carried[0], (blue, unknown), carried[1], (blue, visitor)].to_vec();
    let battle = &mut state.hostile.battle;
    battle
        .retaliation
        .reconcile_capture(&state.entities, &battle.live, &state.connections);
    assert_eq!(battle.retaliation.pending, carried);
    battle.retaliation.pending.push((blue, unknown));
    assert!(
        battle
            .retaliation
            .validate_references(&|id| id != unknown)
            .is_err(),
        "unknown attacker identity must still refuse replay"
    );
}

/// A grudge forgets its attacker, or the enemy who hurt him before, only when
/// that identity just left the execution columns (a horse extra that died):
/// the next image could not name it. An identity that never was a column
/// stays, and recovery still refuses it.
#[test]
fn a_grudge_forgets_only_an_attacker_who_left_the_columns() {
    let mut state = crate::battle::tests::staged_flat_fixture(100);
    crate::battle::staging::execute(
        &mut state,
        &mut crate::boundary::ChangeSet::default(),
        [8.5, 64.0, 8.5],
        &crate::battle::staging::BattleCommand::Start,
    )
    .unwrap();
    run_tick(&mut state, &[]);
    let horse = state
        .entities
        .rider_slots()
        .next()
        .expect("a cavalry horse");
    let horse_id = state.entities.body(horse).id;
    let unknown = EntityId(i32::MAX - 1);
    let battle = &mut state.hostile.battle;
    assert!(battle.columns.ids.contains(&horse_id));
    let members = &battle.formation.members;
    let (a, b, c) = (members[0].id, members[1].id, members[2].id);
    let soldier = members.last().unwrap().id;
    let at = battle.elapsed_ticks;
    let grudge = |attacker, other| Grudge {
        attacker,
        at,
        since: at,
        other,
        helping: false,
        answering: false,
    };
    battle.retaliation.grudges.extend([
        (a, grudge(horse_id, None)),
        (b, grudge(unknown, Some((horse_id, at)))),
        (c, grudge(soldier, Some((unknown, at)))),
    ]);
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
    run_tick(&mut state, &[]);
    let grudges = &state.hostile.battle.retaliation.grudges;
    assert!(
        !state.hostile.battle.columns.ids.contains(&horse_id),
        "the horse left"
    );
    assert!(!grudges.contains_key(&a), "a grudge on the departed horse");
    assert_eq!(grudges[&b].attacker, unknown);
    assert_eq!(grudges[&b].other, None, "the departed horse as other");
    assert_eq!(grudges[&c].attacker, soldier);
    assert_eq!(grudges[&c].other.map(|(id, _)| id), Some(unknown));
    // Recovery knows the roster and the execution columns (no holders here).
    let replay = |state: &TickState| {
        let battle = &state.hostile.battle;
        battle.retaliation.validate_references(&|id| {
            battle.formation.has_member(id) || battle.columns.ids.contains(&id)
        })
    };
    assert!(replay(&state).is_err(), "an unknown attacker must refuse");
    state.hostile.battle.retaliation.grudges.remove(&b);
    assert!(replay(&state).is_err(), "an unknown other must refuse");
    state.hostile.battle.retaliation.grudges.remove(&c);
    replay(&state).expect("the departed horse left no reference");
}
