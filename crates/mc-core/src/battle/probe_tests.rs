//! The phase-log facing probe (probe.rs).

use super::*;
use crate::battle::columns::Columns;
use crate::battle::combat::Weapon;
use crate::battle::{EnemyTarget, staging};
use crate::boundary::ChunkPos;
use crate::registry::CoreRegistry;
use crate::tick::TickState;
use std::sync::Arc;

const ORIGIN: [f64; 3] = [0.5, 100.0, 0.5];
/// 8 blocks toward +x: bearing -90 degrees from `ORIGIN`.
const EAST: [f64; 3] = [8.5, 100.0, 0.5];

fn fixture() -> TickState {
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let mut world = crate::horde_bench::flat_world(&registry);
    let chunks = [ChunkPos { x: 0, z: 0 }];
    world.ensure_blocking(&chunks.into_iter().map(|chunk| (chunk, 0)).collect());
    world.set_active_chunks(chunks.into_iter().collect(), 1);
    TickState::new(registry, world, 2, 512)
}

fn husk(state: &mut TickState, at: [f64; 3]) -> EntityId {
    let husk = state.registry.entity_type_id("minecraft:husk").unwrap();
    let id = state.spawn_mob(husk, at, 20.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_faction(slot, 1);
    id
}

/// receipts/BattleTurnCapD36/phase_facing.py reads these keys in this order.
#[test]
fn phase_log_counters_keep_the_analyzer_keys() {
    assert_eq!(
        Counts::default().to_string(),
        "standing=0 switches=0 transitions=0 transitions_yaw20=0 transitions_yaw45=0 transitions_head20=0 windups=0 windups_head20=0 windups_head45=0 windups_body20=0 windups_body45=0"
    );
}

/// A windup is measured once, at the end of the tick it starts in: head and
/// body as written that tick, against the bearing to the windup's target.
#[test]
fn a_windup_is_measured_from_the_head_and_body_written_in_its_tick() {
    let mut state = fixture();
    let attacker = husk(&mut state, ORIGIN);
    let victim = husk(&mut state, EAST);
    let slot = state.entities.slot_of(attacker).unwrap();
    let battle = &mut state.hostile.battle;
    battle.running = true;
    battle.elapsed_ticks = 11;
    battle.combat.prepare(&[attacker, victim]);
    battle.combat.fighters[0].action = Action::Windup {
        target: victim,
        start: 10,
        weapon: Weapon::Sword,
    };
    let measured = |state: &TickState| {
        let mut probe = Probe::default();
        probe.windups(state);
        let first = probe.counts;
        probe.windups(state);
        assert_eq!(probe.counts.windups, first.windups, "counted once per tick");
        (first.windups, first.windups_head, first.windups_body)
    };
    // Body (yaw) 50 degrees off the bearing, head 30 off.
    state.entities.set_facing(slot, [-40.0, 0.0, -60.0]);
    assert_eq!(measured(&state), (1, [1, 0], [1, 1]));
    // Head 50 off, body 10 off.
    state.entities.set_facing(slot, [-80.0, 0.0, -140.0]);
    assert_eq!(measured(&state), (1, [1, 1], [0, 0]));
    // On the bearing.
    state.entities.set_facing(slot, [-90.0, 0.0, -90.0]);
    assert_eq!(measured(&state), (1, [0, 0], [0, 0]));
    // A windup started on another tick is not this tick's.
    state.hostile.battle.elapsed_ticks = 12;
    assert_eq!(measured(&state), (0, [0, 0], [0, 0]));
}

/// A standing->driven transition is measured at the battle step of the driven
/// tick, from the yaw the driven step starts from and the stored head.
#[test]
fn a_transition_is_measured_from_the_yaw_its_driven_step_starts_from() {
    let mut state = fixture();
    let row = husk(&mut state, ORIGIN);
    let enemy = husk(&mut state, EAST);
    let slot = state.entities.slot_of(row).unwrap();
    // Yaw 50 degrees off the bearing -90, head 10 off.
    state.entities.set_facing(slot, [-40.0, 0.0, -80.0]);
    let mut columns = Columns::default();
    columns.ids.push(row);
    columns.slots.push(Some(slot));
    columns.positions.push(ORIGIN);
    columns.targets.push(Some(EnemyTarget {
        id: enemy,
        pos: EAST,
        distance: 8.0,
    }));
    columns.drive.push([0.0; 3]);
    state.hostile.battle.columns = columns;
    let mut probe = Probe::default();
    let mut observe = |state: &TickState| {
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick: 0,
        };
        probe.observe(&frame, &state.hostile.battle);
        probe.counts
    };
    let standing = observe(&state);
    assert_eq!((standing.standing, standing.transitions), (1, 0));
    state.hostile.battle.columns.drive[0] = [0.1, 0.0, 0.0];
    let driven = observe(&state);
    assert_eq!(
        (
            driven.standing,
            driven.transitions,
            driven.transitions_yaw,
            driven.transitions_head
        ),
        (0, 1, [1, 1], 0)
    );
    assert_eq!(
        observe(&state).transitions,
        0,
        "still driven: no new transition"
    );
}

/// Every windup a foot soldier starts is seen once, after its tick, and rows
/// with a target are classified standing or driven.
#[test]
fn a_staged_battle_counts_every_foot_windup_once() {
    let mut state = crate::battle::tests::staged_flat_fixture(30);
    let mut changes = crate::boundary::ChangeSet::default();
    staging::execute(
        &mut state,
        &mut changes,
        ORIGIN,
        &staging::BattleCommand::Start,
    )
    .unwrap();
    let mut probe = Probe::default();
    let (mut standing, mut windups, mut swings) = (0, 0, 0);
    for _ in 0..600 {
        crate::tick::run_tick(&mut state, &[]);
        let battle = &state.hostile.battle;
        let frame = Frame {
            entities: &state.entities,
            registry: &state.registry,
            world: &state.world,
            tick: battle.elapsed_ticks - 1,
        };
        probe.observe(&frame, battle);
        probe.windups(&state);
        probe.windups(&state);
        standing += probe.counts.standing;
        windups += probe.counts.windups;
        swings += battle.combat.work.swings;
    }
    assert!(standing > 0);
    assert!(windups > 0);
    assert_eq!(windups, swings);
}
