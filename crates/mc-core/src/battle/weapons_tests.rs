use super::*;
use crate::battle::combat::tests::{fixture, index, prepare};
use crate::battle::formation::{Order, RegimentSpec};
use crate::mounts::tests::{slot, spawn};
use mc_nbt::Tag;

const SPEAR: (&str, &str) = ("minecraft:iron_spear", "krasnokamen_battle:iron_spear");
const SWORD: (&str, &str) = ("minecraft:iron_sword", "krasnokamen_battle:iron_longsword");
const BOW: (&str, &str) = ("minecraft:bow", "krasnokamen_battle:iron_longbow_standby");

fn stack(state: &TickState, (item, model): (&str, &str)) -> ItemStack {
    mc_data::item_nbt::stack_from_component_patch(
        &state.registry.items,
        &mc_data::Identifier::parse(item.to_owned()).unwrap(),
        &[(
            "minecraft:item_model".to_owned(),
            Tag::String(model.to_owned()),
        )],
    )
    .unwrap()
}

fn held(state: &TickState, id: EntityId) -> String {
    let stack = state.entities.row(slot(state, id)).appearance.equipment[0]
        .clone()
        .unwrap();
    state
        .registry
        .items
        .name_of(stack.item_id)
        .unwrap()
        .as_str()
        .to_owned()
}

/// Arm `id` as staging does: `first` in the main hand, `second` stowed.
fn arm(
    state: &mut TickState,
    battle: &mut BattleState,
    id: EntityId,
    first: (&str, &str),
    second: (&str, &str),
    pair: Pair,
) {
    let first = stack(state, first);
    let second = stack(state, second);
    let s = slot(state, id);
    std::sync::Arc::make_mut(&mut state.entities.row_mut(s).appearance).equipment[0] = Some(first);
    battle.combat.arsenal.stow(id, second, pair);
}

/// An enemy that stands still and never fights back: targetable, not active.
fn dummy(state: &mut TickState, pos: [f64; 3]) -> EntityId {
    let enemy = spawn(state, "husk", pos);
    let s = slot(state, enemy);
    state.entities.set_faction(s, 2);
    state.entities.row_mut(s).controls.no_ai = true;
    enemy
}

/// A soldier of side 1 with `first` in hand and `second` stowed, and a still
/// enemy `distance` blocks (centre to centre) along +x: husk boxes are 0.6
/// wide, so the box gap is `distance - 0.6`.
fn duel(
    first: (&str, &str),
    second: (&str, &str),
    pair: Pair,
    distance: f64,
) -> (TickState, BattleState, EntityId, EntityId) {
    let (mut state, _, pos) = fixture();
    let actor = spawn(&mut state, "husk", pos);
    let s = slot(&state, actor);
    state.entities.set_faction(s, 1);
    let enemy = dummy(&mut state, [pos[0] + distance, pos[1], pos[2]]);
    let mut battle = BattleState::default();
    arm(&mut state, &mut battle, actor, first, second, pair);
    (state, battle, actor, enemy)
}

fn place(state: &mut TickState, id: EntityId, from: EntityId, distance: f64) {
    let mut pos = state.entities.pos(slot(state, from));
    pos[0] += distance;
    let s = slot(state, id);
    state.entities.set_pos(s, pos);
}

/// One battle tick of targeting and combat, with the drawn weapons put in
/// hand exactly as `battle::step` does after the frozen pass.
fn run(state: &mut TickState, battle: &mut BattleState, tick: u64) -> Output {
    prepare(state, battle, tick);
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick,
    };
    super::super::targeting::update(&frame, battle);
    let mut out = Output::default();
    super::super::combat::step(&frame, battle, &mut out);
    for (id, stack) in out.weapons.clone() {
        hold(state, id, stack);
    }
    out
}

/// Ticks `*tick..*tick + ticks`: the first swap of `id`, if any, and its tick.
fn swap_within(
    state: &mut TickState,
    battle: &mut BattleState,
    tick: &mut u64,
    id: EntityId,
    ticks: u64,
) -> Option<(u64, ItemStack)> {
    let end = *tick + ticks;
    while *tick < end {
        let now = *tick;
        let out = run(state, battle, now);
        *tick += 1;
        if let Some((_, drawn)) = out.weapons.into_iter().find(|(who, _)| *who == id) {
            return Some((now, drawn));
        }
    }
    None
}

#[test]
fn spearman_draws_the_sword_inside_the_near_bound_and_retakes_the_spear_beyond_its_reach() {
    // Iron spear: near bound 2.0 * mob factor 0.5 = 1.0, reach 4.5 * 0.5 = 2.25
    // blocks of box growth.
    let (mut state, mut battle, actor, enemy) = duel(SPEAR, SWORD, Pair::SpearSword, 2.4);
    let mut tick = 0;
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, actor, 60),
        None,
        "on the approach (gap 1.8) the spear stays up"
    );
    let bi = index(&state, &battle, actor);
    assert_eq!(
        battle.combat.fighters[bi].arms.spear,
        MeleeReach {
            min: 1.0,
            max: 2.25
        }
    );
    // Gap 0.9: inside the near bound.
    place(&mut state, enemy, actor, 1.5);
    let (_, drawn) = swap_within(&mut state, &mut battle, &mut tick, actor, 60).expect("sword");
    assert_eq!(
        drawn,
        stack(&state, SWORD),
        "the kit's sword, item model and all"
    );
    assert_eq!(held(&state, actor), "minecraft:iron_sword");
    // Gap 2.0: back out past the bound but within the spear's reach.
    place(&mut state, enemy, actor, 2.6);
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, actor, 80),
        None,
        "within the spear's reach the sword stays out"
    );
    // Gap 2.4: beyond the spear's reach, the spear again.
    place(&mut state, enemy, actor, 3.0);
    let (_, drawn) = swap_within(&mut state, &mut battle, &mut tick, actor, 80).expect("spear");
    assert_eq!(drawn, stack(&state, SPEAR));
    assert_eq!(held(&state, actor), "minecraft:iron_spear");
}

#[test]
fn the_sword_stays_out_when_his_man_falls_and_another_enemy_is_within_spear_reach() {
    let (mut state, mut battle, actor, enemy) = duel(SPEAR, SWORD, Pair::SpearSword, 1.5);
    let mut tick = 0;
    let (swap, _) = swap_within(&mut state, &mut battle, &mut tick, actor, 60).expect("sword");
    // A second enemy at gap 1.4, farther than his man (gap 0.9).
    let pos = state.entities.pos(slot(&state, actor));
    let second = dummy(&mut state, [pos[0], pos[1], pos[2] + 2.0]);
    // Past the swap pause, and on a tick with no retarget due: his man leaves
    // the fight and his retained target is gone until the next retarget.
    while tick < swap + 20 || super::super::retarget_due(actor, tick) {
        assert!(run(&mut state, &mut battle, tick).weapons.is_empty());
        tick += 1;
    }
    let s = slot(&state, enemy);
    state.entities.set_faction(s, 0);
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, actor, 80),
        None,
        "the sword stays out against the next man"
    );
    let bi = index(&state, &battle, actor);
    assert_eq!(
        battle.columns.targets[bi].map(|target| target.id),
        Some(second)
    );
}

#[test]
fn an_enemy_pulsing_in_and_out_of_the_near_bound_does_not_flap_the_weapon() {
    let (mut state, mut battle, actor, enemy) = duel(SPEAR, SWORD, Pair::SpearSword, 3.0);
    let mut swaps = 0;
    for tick in 0..400 {
        // Gap 0.4 and 2.15 by turns (a 1.75-block surge): across the near
        // bound, within the spear's reach.
        place(
            &mut state,
            enemy,
            actor,
            if tick % 40 < 20 { 1.0 } else { 2.75 },
        );
        swaps += run(&mut state, &mut battle, tick)
            .weapons
            .iter()
            .filter(|(who, _)| *who == actor)
            .count();
    }
    assert_eq!(swaps, 1, "one draw of the sword, then no flapping");
    assert_eq!(held(&state, actor), "minecraft:iron_sword");
}

#[test]
fn no_windup_until_the_drawn_sword_is_up() {
    let (mut state, mut battle, actor, enemy) = duel(SPEAR, SWORD, Pair::SpearSword, 3.0);
    // Pick a decision tick whose roll starts a swing, and close the enemy in
    // (gap 0.4: inside the near bound and the sword's reach) the tick before.
    let roll = crate::battle::combat::roll;
    let probability = if roll(actor, 0, 1) < 2500 { 8000 } else { 2000 };
    let decision = (60..2000_u64)
        .find(|t| (t + actor.0 as u64).is_multiple_of(20) && roll(actor, t / 20, 2) < probability)
        .unwrap();
    for tick in 0..decision - 1 {
        let out = run(&mut state, &mut battle, tick);
        assert!(
            out.weapons.is_empty(),
            "no swap with the enemy out of reach"
        );
    }
    place(&mut state, enemy, actor, 1.0);
    let swap = decision - 1;
    let out = run(&mut state, &mut battle, swap);
    let pause = 13; // ceil(20 / iron sword attack speed 1.6)
    assert_eq!(out.weapons.len(), 1);
    assert!(out.events.contains(&EntityEvent::Swing {
        entity: actor,
        hand: Hand::Main,
        animation: SwingKind::Whack,
        duration_ticks: pause,
    }));
    let bi = index(&state, &battle, actor);
    let mut first_windup = None;
    for tick in decision..decision + 1000 {
        run(&mut state, &mut battle, tick);
        if matches!(battle.combat.fighters[bi].action, Action::Windup { .. }) {
            first_windup = Some(tick);
            break;
        }
    }
    let first_windup = first_windup.expect("the sword strikes once it is up");
    assert_eq!(battle.combat.fighters[bi].weapon, Weapon::Sword);
    assert!(
        first_windup >= swap + u64::from(pause),
        "windup at {first_windup}, swap at {swap}"
    );
}

#[test]
fn archer_draws_the_sword_when_an_enemy_closes_and_the_bow_when_he_is_far() {
    let (mut state, mut battle, archer, enemy) = duel(BOW, SWORD, Pair::BowSword, 12.0);
    let mut tick = 0;
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, archer, 40),
        None
    );
    place(&mut state, enemy, archer, ARCHER_SWORD - 1.0);
    let (_, drawn) = swap_within(&mut state, &mut battle, &mut tick, archer, 20).expect("sword");
    assert_eq!(drawn, stack(&state, SWORD));
    // The sword re-derives the archer's classification on the next tick.
    run(&mut state, &mut battle, tick);
    tick += 1;
    let bi = index(&state, &battle, archer);
    assert_eq!(battle.combat.fighters[bi].weapon, Weapon::Sword);
    assert!(!battle.columns.ranged[bi]);
    // Between the two distances the sword stays out.
    place(&mut state, enemy, archer, ARCHER_BOW - 2.0);
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, archer, 60),
        None
    );
    place(&mut state, enemy, archer, ARCHER_BOW + 2.0);
    let (_, drawn) = swap_within(&mut state, &mut battle, &mut tick, archer, 20).expect("bow");
    assert_eq!(drawn, stack(&state, BOW));
    run(&mut state, &mut battle, tick);
    let bi = index(&state, &battle, archer);
    assert_eq!(battle.combat.fighters[bi].weapon, Weapon::Bow);
    assert!(battle.columns.ranged[bi]);
}

#[test]
fn second_rank_and_horsemen_draw_the_sword_inside_the_near_bound_too() {
    let at = |gap: f64| move |grow: f64| gap < grow;
    let spear = MeleeReach {
        min: 1.0,
        max: 2.25,
    };
    // The sword inside the near bound, the spear again beyond its reach.
    assert!(wants_sword(false, spear, at(0.75)));
    assert!(!wants_sword(false, spear, at(1.1)));
    assert!(wants_sword(true, spear, at(2.2)));
    assert!(!wants_sword(true, spear, at(2.3)));

    // A horseman at gap 1.2: the spear stays (it strikes there); at gap 0.75,
    // inside its 1.0 near bound where it cannot strike, the sword comes out.
    let (mut state, mut battle, actor, rider) = duel(SPEAR, SWORD, Pair::SpearSword, 4.0);
    let pos = state.entities.pos(slot(&state, rider));
    let horse = spawn(&mut state, "horse", pos);
    crate::battle::cavalry::mount(&mut state, rider, horse).unwrap();
    let mut tick = 0;
    place(&mut state, rider, actor, 1.8);
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, actor, 60),
        None,
        "against a horseman beyond the near bound the spear stays up"
    );
    place(&mut state, rider, actor, 1.35);
    assert!(
        swap_within(&mut state, &mut battle, &mut tick, actor, 60).is_some(),
        "a horseman inside the spear's near bound draws the sword"
    );

    // The second rank against foot: the same bound.
    let (mut state, _, pos) = fixture();
    let front = spawn(&mut state, "husk", pos);
    let rear = spawn(&mut state, "husk", [pos[0], pos[1], pos[2] - 2.0]);
    let mut battle = BattleState::default();
    for id in [front, rear] {
        let s = slot(&state, id);
        state.entities.set_faction(s, 1);
        arm(&mut state, &mut battle, id, SPEAR, SWORD, Pair::SpearSword);
    }
    battle
        .formation
        .add_regiment(
            RegimentSpec {
                id: 1,
                faction: 1,
                origin: pos,
                facing: [0.0, 1.0],
                files: 1,
                frontage: 1.0,
                order: Order::Hold,
            },
            &[front, rear],
        )
        .unwrap();
    let enemy = dummy(&mut state, [pos[0] + 1.8, pos[1], pos[2] - 2.0]);
    let mut tick = 0;
    assert_eq!(
        swap_within(&mut state, &mut battle, &mut tick, rear, 60),
        None,
        "the second rank keeps the spear beyond the near bound"
    );
    place(&mut state, enemy, rear, 1.35);
    assert!(
        swap_within(&mut state, &mut battle, &mut tick, rear, 60).is_some(),
        "the second rank draws the sword inside the spear's near bound"
    );
}

#[test]
fn a_swap_reaches_the_published_appearance_and_reclassifies_the_fighter() {
    let (mut state, _, pos) = fixture();
    // Clear of the fixture player, on its prepared terrain.
    let actor = spawn(&mut state, "husk", [pos[0] + 3.0, pos[1], pos[2]]);
    let s = slot(&state, actor);
    state.entities.set_faction(s, 1);
    let enemy = dummy(&mut state, [pos[0] + 4.0, pos[1], pos[2]]);
    for id in [actor, enemy] {
        let s = slot(&state, id);
        state.entities.row_mut(s).persistent = true;
    }
    let mut battle = std::mem::take(&mut state.hostile.battle);
    arm(
        &mut state,
        &mut battle,
        actor,
        SPEAR,
        SWORD,
        Pair::SpearSword,
    );
    battle.running = true;
    state.hostile.battle = battle;
    let mut published = None;
    for _ in 0..40 {
        let (output, _, _) = crate::tick::run_tick(&mut state, &[]);
        if output.changes.entities_appearance_changed.contains(&actor) {
            published = Some(output);
            break;
        }
    }
    let output = published.expect("the swap is published as an appearance change");
    assert!(output.changes.entity_events.iter().any(|event| matches!(
        event,
        EntityEvent::Swing { entity, hand: Hand::Main, animation: SwingKind::Whack, duration_ticks: 13 }
            if *entity == actor
    )));
    // The native main hand (what `SetEquipment` carries) holds the kit sword.
    let s = slot(&state, actor);
    assert_eq!(
        state.entities.row(s).appearance.equipment[0],
        Some(stack(&state, SWORD))
    );
    crate::tick::run_tick(&mut state, &[]);
    let battle = &state.hostile.battle;
    let bi = battle.columns.index(s).unwrap();
    let fighter = &battle.combat.fighters[bi];
    assert_eq!(fighter.weapon, Weapon::Sword);
    assert_eq!(fighter.range, MeleeReach::mob(None));
    assert_eq!(fighter.arms.stowed(), Some(&stack(&state, SPEAR)));
}
