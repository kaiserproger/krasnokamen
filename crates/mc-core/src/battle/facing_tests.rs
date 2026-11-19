//! D36 facing of standing soldiers (design R2 section 7.1; row
//! BATTLE-TARGET-LOOK-TURNS-WHOLE-BODY). Thresholds: the trigger reads the
//! smoothed bearing, the release the current one.

use super::*;
use crate::battle::{EnemyTarget, staging};
use crate::boundary::ChunkPos;
use crate::registry::CoreRegistry;
use crate::tick::TickState;
use std::sync::Arc;

const ORIGIN: [f64; 3] = [0.5, 100.0, 0.5];
const TARGET: EntityId = EntityId(7);

/// A point 8 blocks from `ORIGIN` at bearing `degrees` (0 = +z, -90 = +x).
fn aim(degrees: f64) -> [f64; 3] {
    let radians = degrees.to_radians();
    [
        ORIGIN[0] - f64::from(mth::sin(radians)) * 8.0,
        ORIGIN[1],
        ORIGIN[2] + f64::from(mth::cos(radians)) * 8.0,
    ]
}

fn aim_up(degrees: f64, rise: f64) -> [f64; 3] {
    let [x, y, z] = aim(degrees);
    [x, y + rise, z]
}

/// One battle tick of a standing row whose step writes the published facing.
struct Standing {
    row: Row,
    stored: [f32; 3],
}
impl Standing {
    /// An established standing row: its head, pitch and body hold `stored`.
    fn new(stored: [f32; 3]) -> Self {
        Self {
            row: Row {
                id: Some(EntityId(1)),
                law: Law::Split,
                target: Some(TARGET),
                smooth: stored[2],
                pitch_smooth: stored[1],
                body: stored[0],
                last_stable: stored[2],
                position: ORIGIN,
                ..Row::default()
            },
            stored,
        }
    }

    fn tick_at(&mut self, target: EntityId, aim: [f64; 3]) -> Facing {
        let facing = next(&mut self.row, Law::Split, self.stored, ORIGIN, target, aim);
        if let Facing::Set { rotation, .. } = facing {
            self.stored = rotation;
        }
        self.row.position = ORIGIN;
        facing
    }

    fn tick(&mut self, bearing: f64) -> Facing {
        self.tick_at(TARGET, aim(bearing))
    }

    fn head(&self) -> f32 {
        self.stored[2]
    }
}

fn bits(rotation: [f32; 3]) -> [u32; 3] {
    rotation.map(f32::to_bits)
}

fn off(a: f32, b: f32) -> f32 {
    wrap(a - b).abs()
}

#[test]
fn head_turns_while_the_body_holds_then_the_body_aligns_after_ten_stable_ticks_in_ten() {
    let mut row = Standing::new([0.0; 3]);
    // Sustained 30 degrees: the smoothed error reaches 20 on the fourth tick.
    let mut ticks = 0;
    while row.head() == 0.0 {
        row.tick(30.0);
        ticks += 1;
        assert!(ticks <= 4, "the turn starts on the fourth tick");
        assert_eq!(row.stored[0].to_bits(), 0.0_f32.to_bits(), "the body holds");
    }
    assert_eq!(ticks, 4);
    row.tick(30.0);
    assert!(
        off(row.head(), 30.0) < 0.05,
        "the head lands: {}",
        row.head()
    );
    // Ten stable ticks: the body holds at 0 (inside the cone).
    for _ in 0..10 {
        row.tick(30.0);
        assert_eq!(row.stored[0].to_bits(), 0.0_f32.to_bits());
    }
    // Then it aligns within ten more ticks and ends on the head, bit for bit.
    let mut moved = false;
    for _ in 0..10 {
        row.tick(30.0);
        moved |= row.row.body != 0.0;
        assert_eq!(
            row.stored[0].to_bits(),
            0.0_f32.to_bits(),
            "the stored yaw never follows the standing body"
        );
    }
    assert!(moved);
    assert_eq!(row.row.body.to_bits(), row.head().to_bits());
}

#[test]
fn virtual_body_trails_the_head_at_the_cone() {
    let mut row = Standing::new([0.0; 3]);
    let mut at_cone = false;
    for _ in 0..30 {
        row.tick(150.0);
        let gap = off(row.head(), row.row.body);
        assert!(
            gap <= CONE + 1.0e-3,
            "the head never leaves the cone: {gap}"
        );
        at_cone |= (gap - CONE).abs() < 1.0e-3;
    }
    assert!(at_cone, "the body trails at the cone edge during the turn");
    assert!(off(row.head(), 150.0) <= RELEASE);
}

#[test]
fn dead_zone_holds_at_19_degrees_and_turns_at_21() {
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..60 {
        let facing = row.tick(19.0);
        assert!(
            facing.rests([0.0; 3]),
            "19 degrees stays inside the dead zone"
        );
    }
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..60 {
        row.tick(21.0);
    }
    assert!(
        off(row.head(), 21.0) <= RELEASE,
        "21 degrees turns: {}",
        row.head()
    );
}

#[test]
fn a_turn_releases_within_5_degrees_of_the_current_bearing() {
    for (bearing, turning) in [(19.5, false), (20.5, true)] {
        let mut row = Standing::new([0.0; 3]);
        row.row.law = Law::Split;
        row.row.target = Some(TARGET);
        row.row.turning = true;
        row.row.smooth = bearing as f32;
        row.tick(bearing);
        assert_eq!(row.head(), 15.0, "one rate step");
        assert_eq!(
            row.row.turning,
            turning,
            "error {} after the step",
            bearing - 15.0
        );
    }
}

#[test]
fn trigger_reads_the_smoothed_bearing() {
    // A one-tick raw swing of 66 degrees moves the filter by 19.8: no turn.
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick(66.0);
    row.tick(0.0);
    row.tick(0.0);
    assert_eq!(row.head().to_bits(), 0.0_f32.to_bits());
    // A one-tick swap of 100 degrees moves it by 30: the turn starts at once,
    // toward the current bearing.
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick(100.0);
    assert_eq!(row.head(), 15.0);
    // A sustained 45-degree offset starts a turn on its second tick.
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick(45.0);
    assert_eq!(row.head(), 0.0);
    row.tick(45.0);
    assert_eq!(row.head(), 15.0);
    // A 120-degree jump turns at once.
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick(120.0);
    assert_eq!(row.head(), 15.0);
}

#[test]
fn a_turn_lands_on_the_current_bearing_not_the_smoothed_one() {
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    for _ in 0..5 {
        row.tick(60.0);
    }
    assert!(off(row.head(), 60.0) < 0.05, "head {}", row.head());
    assert!(!row.row.turning);
}

#[test]
fn the_hold_is_bit_identical_through_jitter_inside_the_dead_zone() {
    let mut row = Standing::new([10.0, 0.0, 10.0]);
    for tick in 0..200 {
        let before = row.stored;
        let jitter = [3.0, -2.5, 1.0, -3.0][tick % 4];
        let facing = row.tick(10.0 + jitter);
        assert!(
            facing.rests(before),
            "tick {tick}: {facing:?} vs {before:?}"
        );
        assert_eq!(bits(row.stored), bits(before));
    }
}

#[test]
fn pitch_has_its_own_dead_zone_and_lands_on_the_current_elevation() {
    // 0.98 blocks up over 8 is 7 degrees: held. 1.27 is 9 degrees: turned.
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..40 {
        let facing = row.tick_at(TARGET, aim_up(0.0, 0.98));
        assert!(facing.rests([0.0; 3]));
    }
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..40 {
        row.tick_at(TARGET, aim_up(0.0, 1.27));
    }
    let wanted = -(mth::atan2(1.27, 8.0) as f32) * mth::RAD_TO_DEG;
    assert!(
        (row.stored[1] - wanted).abs() <= PITCH_RELEASE,
        "pitch {}",
        row.stored[1]
    );
    assert_eq!(row.head().to_bits(), 0.0_f32.to_bits(), "no yaw turn");
}

#[test]
fn a_standing_row_keeps_its_stored_yaw_and_publishes_the_drawn_body() {
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..40 {
        let Facing::Set {
            rotation,
            body: drawn,
        } = row.tick(50.0)
        else {
            panic!("a projected row is set");
        };
        assert_eq!(
            rotation[0].to_bits(),
            0.0_f32.to_bits(),
            "a standing row writes no yaw"
        );
        assert_eq!(
            drawn.to_bits(),
            row.row.body.to_bits(),
            "the wire carries the row's drawn body"
        );
        assert_eq!(rotation[2], row.stored[2], "the head turns on the wire");
    }
    assert_eq!(
        row.row.body.to_bits(),
        row.stored[2].to_bits(),
        "the drawn body aligned on the head"
    );
    assert!(
        off(row.stored[2], 50.0) <= RELEASE,
        "head {}",
        row.stored[2]
    );
}

#[test]
fn leaving_the_projection_starts_from_the_drawn_body() {
    let mut row = Standing::new([0.0; 3]);
    for _ in 0..40 {
        row.tick(50.0);
    }
    let drawn = row.row.body;
    // Handed until a step takes it: a row whose chunk is not simulated (or
    // whose battle paused before its step) keeps every stored bit.
    for _ in 0..3 {
        assert_eq!(
            leave(&mut row.row, row.stored),
            Some(Facing::From { yaw: drawn })
        );
    }
    // Its step ran: any stored change consumed it.
    let stepped = [drawn - 30.0, row.stored[1], row.stored[2]];
    assert_eq!(leave(&mut row.row, stepped), None);
    assert_eq!(leave(&mut row.row, stepped), None, "once");
    // A row whose drawn body is its stored yaw leaves without a hand-off.
    let mut still = Standing::new([0.0; 3]);
    assert_eq!(leave(&mut still.row, still.stored), None);
}

/// A row handed its body and projected again before any step took it keeps
/// its drawn body (no re-seed from the stale stored yaw).
#[test]
fn a_handed_row_projected_again_before_its_step_keeps_its_drawn_body() {
    let mut lazy = Standing::new([0.0; 3]);
    for _ in 0..40 {
        lazy.tick(50.0);
    }
    let drawn = lazy.row.body;
    assert_ne!(drawn.to_bits(), lazy.stored[0].to_bits());
    leave(&mut lazy.row, lazy.stored);
    let Facing::Set { body, .. } = lazy.tick(50.0) else {
        panic!("projected again")
    };
    assert_eq!(body.to_bits(), drawn.to_bits());
}

#[test]
fn back_from_a_driven_step_the_head_is_not_snapped() {
    // A driven step left yaw = head = 40 and the bearing is now 70: the first
    // standing tick turns the head one rate step, it does not snap it.
    let mut row = Standing::new([40.0, 0.0, 40.0]);
    row.row.law = Law::Off;
    let facing = row.tick(70.0);
    let Facing::Set { rotation, .. } = facing else {
        panic!("set")
    };
    assert_eq!(
        bits(rotation),
        bits([40.0, 0.0, 55.0]),
        "one rate step, body held"
    );
    // At zero error the first standing tick holds.
    let mut row = Standing::new([70.0, 0.0, 70.0]);
    assert!(row.tick(70.0).rests([70.0, 0.0, 70.0]));
}

#[test]
fn a_target_switch_reseeds_the_filter_only_for_a_jump_of_45_or_more() {
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick_at(EntityId(8), aim(30.0));
    assert!(
        (row.row.smooth - 9.0).abs() < 0.05,
        "filtered: {}",
        row.row.smooth
    );
    assert_eq!(row.head(), 0.0);
    let mut row = Standing::new([0.0; 3]);
    row.tick(0.0);
    row.tick_at(EntityId(8), aim(60.0));
    assert!(
        (row.row.smooth - 60.0).abs() < 0.05,
        "re-seeded: {}",
        row.row.smooth
    );
    assert_eq!(row.head(), 15.0, "the turn starts at once");
}

#[test]
fn old_look_keeps_the_previous_step_and_rest_guard() {
    let mut row = Row::default();
    let position = ORIGIN;
    let target = aim(40.0);
    // 40 degrees off: the old guard refuses the rest; the step turns it.
    let stored = [0.0, 0.0, 0.0];
    let look = next(&mut row, Law::Old, stored, position, TARGET, target);
    assert_eq!(look, Facing::Look { rest: None });
    assert!(!look.rests(stored));
    // 0.3 degrees off with the head on the body: held.
    let stored = [40.3, 0.0, 40.3];
    let held = next(&mut row, Law::Old, stored, position, TARGET, target);
    assert!(held.rests(stored));
    // The same, with the head off the body: the old guard refused it.
    let stored = [40.3, 0.0, 70.0];
    let off_body = next(&mut row, Law::Old, stored, position, TARGET, target);
    assert!(!off_body.rests(stored), "{off_body:?}");
    // A row coming off the split law first hands its drawn body.
    let mut split = Standing::new([0.0; 3]);
    for _ in 0..40 {
        split.tick(50.0);
    }
    let drawn = split.row.body;
    assert_eq!(
        next(
            &mut split.row,
            Law::Old,
            split.stored,
            ORIGIN,
            TARGET,
            aim(50.0)
        ),
        Facing::From { yaw: drawn }
    );
}

fn fixture() -> TickState {
    let registry = Arc::new(CoreRegistry::load(None).unwrap());
    let mut world = crate::horde_bench::flat_world(&registry);
    let chunks = [ChunkPos { x: 0, z: 0 }];
    world.ensure_blocking(&chunks.into_iter().map(|chunk| (chunk, 0)).collect());
    world.set_active_chunks(chunks.into_iter().collect(), 1);
    TickState::new(registry, world, 2, 512)
}

fn husk(state: &mut TickState, x: f64) -> EntityId {
    let husk = state.registry.entity_type_id("minecraft:husk").unwrap();
    let id = state.spawn_mob(husk, [x, 100.0, 0.5], 20.0);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_faction(slot, 1);
    id
}

fn columns(state: &TickState, rows: &[(EntityId, [f64; 3])]) -> Columns {
    let mut columns = Columns::default();
    for &(id, target) in rows {
        let slot = state.entities.slot_of(id);
        columns.ids.push(id);
        columns.slots.push(slot);
        columns.positions.push(state.entities.pos(slot.unwrap()));
        columns.targets.push(Some(EnemyTarget {
            id: TARGET,
            pos: target,
            distance: 8.0,
        }));
        columns.drive.push([0.0; 3]);
        columns.active.push(true);
    }
    columns
}

fn project_once(state: &mut TickState, columns: &Columns, combat: &CombatRuntime) {
    let mut facing = std::mem::take(&mut state.hostile.battle.facing);
    let frame = Frame {
        entities: &state.entities,
        registry: &state.registry,
        world: &state.world,
        tick: 0,
    };
    project(&frame, columns, combat, &mut facing);
    state.hostile.battle.facing = facing;
}

/// A dead tail row shifts its successor into its column index; the successor
/// must not inherit its filter (a false snap turn).
#[test]
fn a_dead_tail_row_does_not_move_a_neighbours_head() {
    let mut state = fixture();
    let a = husk(&mut state, 0.5);
    let b = husk(&mut state, 2.5);
    let c = husk(&mut state, 4.5);
    let slot_c = state.entities.slot_of(c).unwrap();
    for id in [a, b, c] {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_facing(slot, [0.0; 3]);
    }
    // a and c face +z exactly; b's target is behind it.
    let ahead = |x: f64| [x, 100.0, 20.5];
    let behind = |x: f64| [x, 100.0, -20.5];
    let combat = CombatRuntime::default();
    let all = columns(
        &state,
        &[(a, ahead(0.5)), (b, behind(2.5)), (c, ahead(4.5))],
    );
    for _ in 0..3 {
        project_once(&mut state, &all, &combat);
    }
    let before = state.entities.rot(slot_c);
    let shifted = columns(&state, &[(a, ahead(0.5)), (c, ahead(4.5))]);
    for _ in 0..3 {
        project_once(&mut state, &shifted, &combat);
        let facing = state.hostile.battle.facing.of(slot_c).unwrap();
        assert!(facing.rests(before), "{facing:?}");
    }
}

#[test]
fn riders_head_on_body_mobs_and_charged_fighters_keep_the_old_look() {
    let mut state = fixture();
    let foot = husk(&mut state, 0.5);
    let charged = husk(&mut state, 2.5);
    let rider = husk(&mut state, 4.5);
    let horse_type = state.registry.entity_type_id("minecraft:horse").unwrap();
    let horse = state.spawn_mob(horse_type, [4.5, 100.0, 0.5], 20.0);
    let horse_slot = state.entities.slot_of(horse).unwrap();
    state.entities.set_faction(horse_slot, 1);
    crate::battle::cavalry::mount(&mut state, rider, horse).unwrap();
    crate::mounts::tick(&mut state, &mut crate::boundary::ChangeSet::default());
    for id in [foot, charged, rider] {
        let slot = state.entities.slot_of(id).unwrap();
        state.entities.set_facing(slot, [0.0; 3]);
    }
    // Every target is 20 blocks toward +x (bearing -90).
    let rows = [
        (foot, [20.5, 100.0, 0.5]),
        (charged, [22.5, 100.0, 0.5]),
        (rider, [24.5, 100.0, 0.5]),
    ];
    let columns = columns(&state, &rows);
    let mut combat = CombatRuntime::default();
    combat.prepare(&columns.ids);
    combat.fighters[1].charge = serde_json::from_value(
        serde_json::json!({"impact": 0, "refused": false, "reform_until": 0}),
    )
    .unwrap();
    project_once(&mut state, &columns, &combat);
    let set = |id: EntityId| {
        let slot = state.entities.slot_of(id).unwrap();
        match state.hostile.battle.facing.of(slot) {
            Some(Facing::Set { rotation, .. }) => (rotation, state.entities.rot(slot)),
            other => panic!("{other:?}"),
        }
    };
    let (foot_rotation, foot_stored) = set(foot);
    assert_eq!(
        foot_rotation[0].to_bits(),
        foot_stored[0].to_bits(),
        "the split body holds"
    );
    assert_eq!(
        foot_rotation[2],
        foot_stored[2] - 15.0,
        "the split head turns at the rate toward -90"
    );
    for id in [charged, rider] {
        let slot = state.entities.slot_of(id).unwrap();
        assert_eq!(
            state.hostile.battle.facing.of(slot),
            Some(Facing::Look { rest: None }),
            "{id:?}: the old look turns the whole body in its step"
        );
    }
}

/// The registration in `battle::step`: in a running battle every standing row
/// with a target is projected, and after the tick its stored rotation is the
/// published one (written by its step, or kept by its rest), with the drawn
/// body as the yaw on a tick it moved. Half way, every projected row's head
/// and yaw are turned 60 degrees off: the head turns back alone while the
/// stored yaw stands.
#[test]
fn a_running_battle_writes_the_published_facing_of_every_standing_row() {
    let mut state = crate::battle::tests::staged_flat_fixture(30);
    let mut changes = crate::boundary::ChangeSet::default();
    staging::execute(
        &mut state,
        &mut changes,
        ORIGIN,
        &staging::BattleCommand::Start,
    )
    .unwrap();
    let (mut projected, mut split, mut held_yaw) = (0, 0, 0);
    for tick in 0..600 {
        crate::tick::run_tick(&mut state, &[]);
        let slots: Vec<_> = state.entities.battle_slots().collect();
        for slot in slots {
            let Some(Facing::Set { rotation, body }) = state.hostile.battle.facing.of(slot) else {
                continue;
            };
            if !state.entities.body(slot).alive {
                continue;
            }
            projected += 1;
            let stored = state.entities.rot(slot);
            let on_a_move = bits([body, rotation[1], rotation[2]]);
            assert!(
                bits(stored) == bits(rotation) || bits(stored) == on_a_move,
                "stored {stored:?} published {rotation:?} body {body}"
            );
            split += usize::from(stored[2].to_bits() != stored[0].to_bits());
            held_yaw += usize::from(body.to_bits() != stored[0].to_bits());
            if tick == 300 {
                let [yaw, pitch, head] = stored;
                state
                    .entities
                    .set_facing(slot, [yaw + 60.0, pitch, head + 60.0]);
            }
        }
    }
    assert!(
        projected > 0,
        "standing soldiers with targets are projected"
    );
    assert!(
        split > 0,
        "a standing soldier turns its head alone ({projected} projected)"
    );
    assert!(
        held_yaw > 0,
        "a standing soldier draws a body off its stored yaw ({projected} projected)"
    );
}

/// D36 3.4.3 and 3.2(ii): after `battle pause` (or stop) nothing is projected.
/// The drawn body becomes the yaw on the pause tick, so no head jumps toward a
/// stale yaw; a head left off its body returns to it in the step, every row on
/// its body within 8 ticks. Each standing row's body is first turned 180
/// degrees from its stored yaw (a standing row stores it only on a move) with
/// the head 60 degrees off the body.
#[test]
fn a_paused_battle_rests_every_row_within_8_ticks() {
    let mut state = crate::battle::tests::staged_flat_fixture(20);
    let mut changes = crate::boundary::ChangeSet::default();
    staging::execute(
        &mut state,
        &mut changes,
        ORIGIN,
        &staging::BattleCommand::Start,
    )
    .unwrap();
    let split = |state: &TickState| {
        state
            .hostile
            .battle
            .facing
            .rows
            .iter()
            .filter(|row| row.law == Law::Split)
            .count()
    };
    for _ in 0..1000 {
        crate::tick::run_tick(&mut state, &[]);
        if split(&state) >= 3 {
            break;
        }
    }
    let mut turned = Vec::new();
    for row in &mut state.hostile.battle.facing.rows {
        let Some(slot) = row.id.and_then(|id| state.entities.slot_of(id)) else {
            continue;
        };
        if row.law != Law::Split || !state.entities.body(slot).alive {
            continue;
        }
        let [yaw, pitch, _] = state.entities.rot(slot);
        row.body = wrap(yaw + 180.0);
        let head = row.body + 60.0;
        row.last_stable = head;
        state.entities.set_facing(slot, [yaw, pitch, head]);
        state.entities.set_vel(slot, [0.0; 3]);
        turned.push((slot, row.body, head));
    }
    assert!(turned.len() >= 3, "standing rows to pause");
    let soldiers: Vec<_> = state.entities.battle_slots().collect();
    staging::execute(
        &mut state,
        &mut changes,
        ORIGIN,
        &staging::BattleCommand::Pause,
    )
    .unwrap();
    crate::tick::run_tick(&mut state, &[]);
    for &(slot, body, head) in &turned {
        if !state.entities.body(slot).alive {
            continue;
        }
        let [yaw, _, after] = state.entities.rot(slot);
        // follow_head: 10 degrees per tick toward the body.
        assert!(
            off(after, head) <= 10.0 + 1.0e-3,
            "head jumped {head} -> {after} (yaw {yaw}, body {body})"
        );
        assert_eq!(yaw.to_bits(), body.to_bits(), "the drawn body is the yaw");
    }
    let mut settled_after = 0;
    for tick in 2..=8 {
        crate::tick::run_tick(&mut state, &[]);
        if soldiers.iter().all(|&slot| {
            !state.entities.body(slot).alive || {
                let [yaw, _, head] = state.entities.rot(slot);
                yaw.to_bits() == head.to_bits()
            }
        }) {
            settled_after = tick;
            break;
        }
    }
    assert!(
        settled_after > 0,
        "every head is on its body within 8 ticks"
    );
    assert!(state.hostile.battle.facing.of(soldiers[0]).is_none());
}

/// A row whose chunk is not simulated (`columns.active` false) keeps handing
/// its drawn body until a step has taken it: the lazy facing never loses the
/// virtual yaw because the row was skipped on the tick it left the projection.
#[test]
fn an_inactive_row_keeps_handing_its_body_until_a_step_takes_it() {
    let mut state = fixture();
    let id = husk(&mut state, 0.5);
    let slot = state.entities.slot_of(id).unwrap();
    state.entities.set_facing(slot, [0.0; 3]);
    let ahead = [40.5, 100.0, 0.5];
    let mut standing = columns(&state, &[(id, ahead)]);
    let combat = CombatRuntime::default();
    for _ in 0..40 {
        project_once(&mut state, &standing, &combat);
        if let Some(Facing::Set { rotation, .. }) = state.hostile.battle.facing.of(slot) {
            state.entities.set_facing(slot, rotation);
        }
    }
    let drawn = match state.hostile.battle.facing.of(slot) {
        Some(Facing::Set { body, .. }) => body,
        other => panic!("{other:?}"),
    };
    assert_ne!(drawn.to_bits(), state.entities.rot(slot)[0].to_bits());
    standing.active[0] = false;
    for _ in 0..3 {
        project_once(&mut state, &standing, &combat);
        assert_eq!(
            state.hostile.battle.facing.of(slot),
            Some(Facing::From { yaw: drawn }),
            "handed while not simulated"
        );
    }
    let [_, pitch, head] = state.entities.rot(slot);
    state.entities.set_facing(slot, [drawn, pitch, head]);
    project_once(&mut state, &standing, &combat);
    assert_eq!(state.hostile.battle.facing.of(slot), None, "taken");
}

/// Facing state is runtime-only: a tick that changes nothing but the
/// projection journals no battle record.
#[test]
fn the_battle_patch_is_empty_when_only_facing_changes() {
    let mut state = crate::battle::tests::staged_flat_fixture(10);
    let mut entries = Vec::new();
    crate::battle::persistence::journal_entries(&mut state, &mut entries).unwrap();
    entries.clear();
    crate::battle::persistence::journal_entries(&mut state, &mut entries).unwrap();
    assert!(entries.is_empty(), "nothing changed: {entries:?}");
    let soldier = state.entities.battle_slots().next().unwrap();
    let id = state.entities.body(soldier).id;
    let rows = [(id, [0.5, 100.0, 40.5])];
    let columns = columns(&state, &rows);
    project_once(&mut state, &columns, &CombatRuntime::default());
    assert!(state.hostile.battle.facing.of(soldier).is_some());
    crate::battle::persistence::journal_entries(&mut state, &mut entries).unwrap();
    assert!(
        entries.is_empty(),
        "facing-only change journals nothing: {entries:?}"
    );
}

/// D36 3.4a: the facing state is not persisted, so the contract is the same
/// result from the same start state and inputs. A restored row has its head
/// on its stored yaw; one turn brings it onto the target, then it rests.
#[test]
fn from_the_same_start_state_the_same_facing_and_a_restored_row_reaches_rest() {
    let run = || {
        let mut row = Standing::new([170.0, 5.0, 170.0]);
        row.row.law = Law::Off;
        (0..60)
            .map(|tick| {
                bits(match row.tick(-30.0 + (tick % 5) as f64) {
                    Facing::Set { rotation, .. } => rotation,
                    other => unreachable!("{other:?}"),
                })
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
    let mut row = Standing::new([170.0, 5.0, 170.0]);
    row.row.law = Law::Off;
    let mut rested = false;
    for _ in 0..60 {
        let before = row.stored;
        rested = row.tick(-30.0).rests(before);
    }
    assert!(rested, "the restored row rests again");
    assert!(off(row.head(), -30.0) <= RELEASE);
}
