//! Vanilla `LivingEntity.tickDeath`: a killed mob keeps its body for 20 ticks
//! (clients play the death tilt), then poofs (entity event 60) and leaves.
use super::*;

fn spawn(state: &mut TickState, name: &str, pos: [f64; 3]) -> EntityId {
    let type_id = state.registry.entity_type_id(name).unwrap();
    state.spawn_mob(type_id, pos, 10.0)
}

/// Stage one hit through the ordinary merge-commit damage path.
fn hit(
    state: &mut TickState,
    victim: EntityId,
    amount: f32,
    source: Option<crate::intent::DamageSource>,
) {
    let slot = state.entities.slot_of_any(victim).unwrap();
    state.merge_damage.push((
        slot,
        DamagePiece {
            physical: None,
            custom_armor: false,
            amount,
            kind: if source.is_some() {
                mc_survival::DamageKind::Melee
            } else {
                mc_survival::DamageKind::GenericKill
            },
            source,
        },
    ));
}

fn melee(attacker: EntityId, position: [f64; 3]) -> crate::intent::DamageSource {
    crate::intent::DamageSource::Melee {
        attacker,
        position,
        yaw: 0.0,
        bonus_knockback: 0.0,
        sweep: false,
        fire_ticks: 0,
        bane_ticks: 0,
    }
}

fn tick(state: &mut TickState) -> TickOutput {
    run_tick(state, &[]).0
}

fn poofed(output: &TickOutput, id: EntityId) -> bool {
    output
        .changes
        .entity_events
        .iter()
        .any(|event| matches!(event, EntityEvent::Status { entity, event: 60 } if *entity == id))
}

fn about(output: &TickOutput, id: EntityId) -> bool {
    output.changes.entity_events.iter().any(|event| {
        matches!(event, EntityEvent::Hurt { entity }
            | EntityEvent::Damage { entity, .. }
            | EntityEvent::Death { entity } if *entity == id)
    })
}

/// Entities of `kind` among `ids`.
fn count(state: &TickState, ids: &[EntityId], kind: fn(EntityKind) -> bool) -> usize {
    ids.iter()
        .filter_map(|id| state.entities.slot_of_any(*id))
        .filter(|&slot| kind(state.entities.body(slot).kind))
        .count()
}

fn item(kind: EntityKind) -> bool {
    matches!(kind, EntityKind::Item { .. })
}

fn orb(kind: EntityKind) -> bool {
    matches!(kind, EntityKind::ExperienceOrb { .. })
}

#[test]
fn killed_mob_keeps_its_body_twenty_ticks_then_poofs_and_leaves() {
    let (mut state, player, centre) = crate::hostile_tests::setup(8.0);
    let cow = spawn(&mut state, "minecraft:cow", centre);
    hit(&mut state, cow, 100.0, Some(melee(player, centre)));
    let death = tick(&mut state);
    assert!(
        death
            .changes
            .entity_events
            .iter()
            .any(|event| matches!(event, EntityEvent::Death { entity } if *entity == cow)),
        "entity event 3 at death"
    );
    assert!(!death.changes.entities_removed.contains(&cow));
    let corpse = state.entities.slot_of_any(cow).expect("the body stays");
    assert!(!state.entities.body(corpse).alive);
    assert_eq!(
        state.entities.row(corpse).appearance.health,
        0.0,
        "vanilla setHealth clamps the published health at 0"
    );
    assert!(
        death.snapshot.entities.get(cow).is_some(),
        "clients keep seeing the body"
    );
    for elapsed in 1..20 {
        let output = tick(&mut state);
        assert!(
            !output.changes.entities_removed.contains(&cow) && !poofed(&output, cow),
            "body left early, {elapsed} ticks after death"
        );
        assert!(output.snapshot.entities.get(cow).is_some());
    }
    let end = tick(&mut state);
    assert!(poofed(&end, cow), "entity event 60 at the end");
    assert!(end.changes.entities_removed.contains(&cow));
    assert!(state.entities.slot_of_any(cow).is_none());
    assert!(end.snapshot.entities.get(cow).is_none());
}

#[test]
fn corpse_takes_no_damage_interaction_ai_or_mob_cap() {
    let (mut state, player, centre) = crate::hostile_tests::setup(8.0);
    let zombie = spawn(&mut state, "minecraft:zombie", centre);
    let observer = state.entities.pos(state.entities.slot_of(player).unwrap());
    let living: usize = crate::natural_spawn::census(&mut state, &[observer])
        .iter()
        .sum();
    // No source: no knockback, so only AI could move the body sideways.
    hit(&mut state, zombie, 100.0, None);
    tick(&mut state);
    let corpse = state.entities.slot_of_any(zombie).expect("the body stays");
    assert!(
        state.entities.slot_of(zombie).is_none(),
        "no interaction target"
    );
    let health = state.entities.row(corpse).health;
    let at = state.entities.pos(corpse);
    hit(&mut state, zombie, 5.0, Some(melee(player, observer)));
    let struck = tick(&mut state);
    assert!(!about(&struck, zombie), "a corpse takes no further hits");
    let corpse = state.entities.slot_of_any(zombie).unwrap();
    assert_eq!(state.entities.row(corpse).health, health);
    assert_eq!(
        state.entities.motion_flags(corpse) & MOTION_OWNED,
        0,
        "no AI step"
    );
    assert_eq!(
        crate::natural_spawn::census(&mut state, &[observer])
            .iter()
            .sum::<usize>(),
        living - 1,
        "no mob cap share"
    );
    for _ in 2..20 {
        tick(&mut state);
        let corpse = state.entities.slot_of_any(zombie).unwrap();
        let now = state.entities.pos(corpse);
        assert_eq!([now[0], now[2]], [at[0], at[2]], "the body walked");
    }
}

#[test]
fn death_drops_once_and_removal_drops_nothing() {
    let (mut state, player, centre) = crate::hostile_tests::setup(8.0);
    let cow = spawn(&mut state, "minecraft:cow", centre);
    hit(&mut state, cow, 100.0, Some(melee(player, centre)));
    let death = tick(&mut state);
    let spawned = &death.changes.entities_spawned;
    assert!(count(&state, spawned, item) > 0, "loot drops at death");
    assert!(count(&state, spawned, orb) > 0, "experience drops at death");
    assert!(state.entities.slot_of_any(cow).is_some(), "the body stays");
    let mut later = 0;
    for elapsed in 1..=20 {
        if elapsed == 5 {
            hit(&mut state, cow, 100.0, Some(melee(player, centre)));
        }
        let output = tick(&mut state);
        later += count(&state, &output.changes.entities_spawned, item);
    }
    assert!(state.entities.slot_of_any(cow).is_none());
    assert_eq!(later, 0, "the corpse dropped loot again");
}

#[test]
fn armor_stand_still_breaks_at_once() {
    let (mut state, _, centre) = crate::hostile_tests::setup(8.0);
    let stand = spawn(&mut state, "minecraft:armor_stand", centre);
    hit(&mut state, stand, 100.0, None);
    let output = tick(&mut state);
    assert!(output.changes.entities_removed.contains(&stand));
    assert!(state.entities.slot_of_any(stand).is_none());
}

#[test]
fn corpse_leaves_the_special_mob_index() {
    let (mut state, _, centre) = crate::hostile_tests::setup(8.0);
    let registry = std::sync::Arc::clone(&state.registry);
    let skeleton = spawn(&mut state, "minecraft:skeleton", centre);
    state.entities.prepare_motion_hints(&registry);
    let slot = state.entities.slot_of(skeleton).unwrap();
    assert!(state.entities.motion_hint(slot).special.is_some());
    assert_ne!(state.entities.motion_flags(slot) & MOTION_SPECIAL, 0);
    hit(&mut state, skeleton, 100.0, None);
    tick(&mut state);
    // The death marks the row dirty; the next refresh drops its special.
    state.entities.prepare_motion_hints(&registry);
    let corpse = state
        .entities
        .slot_of_any(skeleton)
        .expect("the body stays");
    assert!(state.entities.motion_hint(corpse).special.is_none());
    assert_eq!(state.entities.motion_flags(corpse) & MOTION_SPECIAL, 0);
}
