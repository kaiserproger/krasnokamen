//! PrepLifecycle §2.3 end to end: real builder attempts on terrain cuts whose
//! cells lie outside (or inside) the village territory, under block-log
//! history that still covers the survey or has lapsed past it.
use std::sync::Arc;

use super::tests::{attempt, fixture, fund, open_plan, start};
use super::*;
use crate::block_log::{BlockLog, Change, Delta, Record};
use crate::config::BlockLogSettings;
use crate::entities::EntitySlot;

const ATTEMPTS: usize = 64;

/// A house lot holding three sand cuts, `first` (top of a two-high pillar,
/// cut first because cuts run top-down), `below` it and a lone `later` block
/// in the far corner, plus a grass `plant` cut after `below`.
struct Lot {
    state: TickState,
    id: SettlementId,
    builder: EntitySlot,
    first: [i32; 3],
    below: [i32; 3],
    later: [i32; 3],
    plant: [i32; 3],
    sand: u32,
    item: u32,
}

/// `capture`: block-log capture at admission. `outside`: the territory is
/// moved off the lot after admission (the storage chest stays inside).
fn lot(capture: bool, outside: bool) -> Lot {
    let (mut state, id, builder) = fixture();
    open_plan(&mut state);
    let size = blueprints::get(&state.registry, "house")
        .unwrap()
        .footprint(Rotation::from_quarter_turns(0));
    let origin = [3, 200, 3];
    let sand = super::super::allocator::tests::block(&state, "minecraft:sand", &[]);
    let first = [origin[0], origin[1] + 2, origin[2]];
    let below = [origin[0], origin[1] + 1, origin[2]];
    let later = [
        origin[0] + i32::from(size[0]) - 1,
        origin[1] + 1,
        origin[2] + i32::from(size[2]) - 1,
    ];
    let plant = [origin[0] + 1, origin[1] + 1, origin[2]];
    for at in [first, below, later] {
        state.world.set_block(at[0], at[1], at[2], sand);
    }
    let grass = super::super::allocator::tests::block(&state, "minecraft:short_grass", &[]);
    state.world.set_block(plant[0], plant[1], plant[2], grass);
    state.world.fold_overlay();
    state.block_log = BlockLog::new(BlockLogSettings {
        enabled: capture,
        retention_secs: 0,
        max_records: 1_000,
    });
    start(&mut state, id, "house", 1, None);
    fund(&mut state, "house", 1);
    if outside {
        let record = state.settlements.get_mut(id).unwrap();
        record.anchor = [-6, 201, -6];
        record.radius = 10;
    }
    let record = state.settlements.get(id).unwrap();
    assert!(
        record.contains([1, 201, 1]),
        "storage stays in the territory"
    );
    for at in [first, below, later, plant] {
        assert_eq!(record.contains(at), !outside);
    }
    let item = state
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:sand").unwrap())
        .unwrap();
    Lot {
        state,
        id,
        builder,
        first,
        below,
        later,
        plant,
        sand,
        item,
    }
}

/// A capturing log after the survey (taken at sequence 0): `evicted` older
/// records left by count, then a retained player edit at each `touched`.
fn history(state: &mut TickState, evicted: u64, touched: &[[i32; 3]]) {
    let edit = |seq: u64, pos: [i32; 3]| Record {
        seq,
        tick: Some(1),
        time: 0,
        actor: Arc::from("player"),
        pos,
        change: Change::Block {
            before: state.registry.states.air,
            after: state.registry.states.stone,
            entity: None,
        },
        rolled_back: false,
    };
    let far = [1_000, 64, 1_000];
    let records = (0..=evicted)
        .map(|seq| edit(seq, far))
        .chain(
            touched
                .iter()
                .enumerate()
                .map(|(index, at)| edit(evicted + 1 + index as u64, *at)),
        )
        .collect();
    state.block_log = BlockLog::restore(
        BlockLogSettings {
            enabled: true,
            retention_secs: 0,
            max_records: 1 + touched.len(),
        },
        vec![Delta {
            records,
            marks: Vec::new(),
            coverage: 0,
        }],
    );
}

impl Lot {
    fn is_sand(&self, at: [i32; 3]) -> bool {
        self.state.world.block_at(at[0], at[1], at[2]) == Some(self.sand)
    }

    /// Sand in storage and in the builder's hands.
    fn sand_held(&self) -> i64 {
        let stored: i64 = self.state.block_entities[&(1, 201, 1)]
            .slots
            .iter()
            .filter(|stack| !stack.is_empty() && stack.item_id == self.item)
            .map(|stack| i64::from(stack.count))
            .sum();
        let carried: i64 = self
            .state
            .entities
            .row(self.builder)
            .villager
            .as_ref()
            .unwrap()
            .inventory
            .iter()
            .flatten()
            .filter(|stack| stack.item_id == self.item && stack.count > 0)
            .map(|stack| i64::from(stack.count))
            .sum();
        stored + carried
    }

    fn cut(&self) -> i64 {
        [self.first, self.below, self.later]
            .into_iter()
            .filter(|at| !self.is_sand(*at))
            .count() as i64
    }

    /// Builder attempts until `at` is cut or the attempts run out.
    fn work_until_cut(&mut self, at: [i32; 3]) -> bool {
        for _ in 0..ATTEMPTS {
            if !self.is_sand(at) {
                break;
            }
            attempt(&mut self.state, self.id, self.builder);
        }
        !self.is_sand(at)
    }
}
#[test]
fn covering_history_cuts_an_outside_cell_without_readmission() {
    let mut lot = lot(true, true);
    // Nothing since the survey was evicted, so the later cell's player edit
    // is the debit's own veto for that cell, not a reason to re-admit.
    history(&mut lot.state, 0, &[lot.later]);
    assert!(lot.work_until_cut(lot.first), "covering history proceeds");
    assert_eq!(lot.sand_held(), lot.cut(), "each cut credited once");
}

#[test]
fn lapsed_history_readmits_a_clean_plan_and_keeps_the_new_admission() {
    let mut lot = lot(true, true);
    history(&mut lot.state, 1, &[]);
    assert!(
        lot.work_until_cut(lot.first),
        "a clean remaining plan is admitted again and continues"
    );
    assert_eq!(lot.sand_held(), lot.cut(), "each cut credited once");
    assert!(lot.is_sand(lot.below), "one cut per step: below is next");
    // The re-admission (taken at sequence 2) is stored with that commit: a
    // log that still covers it but no longer covers the original survey
    // keeps working although a remaining cell now carries a player edit.
    history(&mut lot.state, 2, &[lot.later]);
    assert!(
        lot.work_until_cut(lot.below),
        "the stored re-admission still covers the remaining work"
    );
    assert!(lot.is_sand(lot.later), "the player's block keeps its veto");
    assert_eq!(lot.sand_held(), lot.cut());
}

#[test]
fn lapsed_history_refuses_while_a_remaining_outside_cell_has_a_player_edit() {
    let mut lot = lot(true, true);
    history(&mut lot.state, 1, &[lot.later]);
    let handle = lot.state.settlements.get(lot.id).unwrap().projects[0]
        .structure
        .clone();
    assert!(!lot.work_until_cut(lot.first), "the clean first cut waits");
    assert_eq!(
        executor::next_cell(&lot.state, lot.id, &handle).unwrap(),
        Some(lot.first),
        "the refusal stops at the outside cut"
    );
    assert_eq!(lot.sand_held(), 0, "nothing was credited");
    let snapshot = executor::status(&lot.state, lot.id, &handle).unwrap();
    assert_ne!(snapshot.state, ScriptStructureState::Paused);
    assert_eq!(snapshot.pause_reason, None, "no pause is stored");
    assert_eq!(snapshot.completed_work_units, 0, "no cell was worked");
    assert!(snapshot.consumed.is_empty());
    // The ordinary stall path frees the unpaid slot and requeues the order.
    shift_passes(&mut lot, 2_000, 301);
    let record = lot.state.settlements.get(lot.id).unwrap();
    assert!(
        record.projects.is_empty(),
        "the stalled project gave back its slot"
    );
    assert!(
        record.orders.iter().any(|order| order.blueprint == "house"),
        "the owner's order is queued again"
    );
    assert_eq!(
        executor::status(&lot.state, lot.id, &handle).unwrap().state,
        ScriptStructureState::Cancelled
    );
    assert!(lot.is_sand(lot.first) && lot.is_sand(lot.later));
    assert_eq!(lot.sand_held(), 0);
}

#[test]
fn inside_the_territory_lapsed_history_is_not_consulted() {
    let mut lot = lot(true, false);
    history(&mut lot.state, 1, &[lot.later]);
    assert!(
        lot.work_until_cut(lot.first),
        "inside the territory the cut proceeds"
    );
    assert_eq!(lot.sand_held(), lot.cut());
}

#[test]
fn an_uncaptured_survey_keeps_the_documented_residual() {
    let mut lot = lot(false, true);
    history(&mut lot.state, 1, &[lot.later]);
    assert!(
        lot.work_until_cut(lot.first),
        "a manifest surveyed without capture is not held to coverage"
    );
    assert_eq!(lot.sand_held(), lot.cut());
}

/// Opus PP-3: outside the territory a retained player edit refuses a plant
/// cut, which has no debit veto of its own, even while the survey is covered.
#[test]
fn covered_plant_cell_with_retained_player_edit_refuses() {
    let mut lot = lot(true, true);
    history(&mut lot.state, 0, &[lot.plant]);
    assert!(
        lot.work_until_cut(lot.below),
        "the sand cuts before it proceed"
    );
    for _ in 0..ATTEMPTS {
        attempt(&mut lot.state, lot.id, lot.builder);
    }
    let at = lot.plant;
    assert_ne!(
        lot.state.world.block_at(at[0], at[1], at[2]),
        Some(lot.state.registry.states.air),
        "the player's grass stays"
    );
    let handle = lot.state.settlements.get(lot.id).unwrap().projects[0]
        .structure
        .clone();
    assert_eq!(
        executor::next_cell(&lot.state, lot.id, &handle).unwrap(),
        Some(lot.plant)
    );
}

/// Main decision PP-2, an accepted residual for an identical identity only:
/// once coverage lapses, the re-admission is a fresh survey and cannot see a
/// player edit the log has already evicted when the live block still has the
/// survey's identity (sand replaced by sand here). A changed identity under
/// lapsed coverage is refused by the portions' survey check, which consults
/// `provenance::covered`. Only retained edits veto.
#[test]
fn evicted_identical_player_edit_after_lapse() {
    let mut lot = lot(true, true);
    let edit = |seq: u64, pos: [i32; 3]| Record {
        seq,
        tick: Some(1),
        time: 0,
        actor: Arc::from("player"),
        pos,
        change: Change::Block {
            before: lot.state.registry.states.air,
            after: lot.sand,
            entity: None,
        },
        rolled_back: false,
    };
    // The player's identical edit at `first` came after the survey and was
    // evicted by count; one later record remains.
    let records = vec![edit(0, lot.first), edit(1, [1_000, 64, 1_000])];
    lot.state.block_log = BlockLog::restore(
        BlockLogSettings {
            enabled: true,
            retention_secs: 0,
            max_records: 1,
        },
        vec![Delta {
            records,
            marks: Vec::new(),
            coverage: 0,
        }],
    );
    assert!(!lot.state.block_log.player_touched(lot.first));
    assert!(
        lot.work_until_cut(lot.first),
        "the residual: the evicted edit is not protected"
    );
    assert_eq!(lot.sand_held(), lot.cut());
}

/// Text lines queued for `connection`, burst lines included.
fn chat(state: &TickState, connection: mc_domain::ConnectionId) -> Vec<String> {
    fn lines(change: &crate::boundary::PlayerChange, out: &mut Vec<String>) {
        match change {
            crate::boundary::PlayerChange::Chat { message } => out.push(message.clone()),
            crate::boundary::PlayerChange::TextBurst { lines: burst } => {
                for change in burst {
                    lines(change, out);
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for change in state.outbox.get(&connection).into_iter().flatten() {
        lines(change, &mut out);
    }
    out
}

/// Watch passes on the work shift, 20 ticks apart, from `day_time`.
fn shift_passes(lot: &mut Lot, day_time: i64, passes: usize) {
    lot.state.day_time = day_time;
    for _ in 0..passes {
        crate::villager::building::tick(&mut lot.state, lot.id);
        lot.state.tick += 20;
        lot.state.day_time += 20;
        assign_builder(&mut lot.state, lot.id);
    }
}

/// An outside lot whose project has paid materials, its handle and the next
/// cell, with the owner set but offline.
fn paid_lot() -> (Lot, String, [i32; 3]) {
    let mut lot = lot(true, true);
    let handle = lot.state.settlements.get(lot.id).unwrap().projects[0]
        .structure
        .clone();
    let paid = |state: &TickState| {
        !executor::status(state, lot.id, &handle)
            .unwrap()
            .consumed
            .is_empty()
    };
    for _ in 0..ATTEMPTS * 32 {
        if paid(&lot.state) {
            break;
        }
        attempt(&mut lot.state, lot.id, lot.builder);
    }
    assert!(paid(&lot.state), "materials were paid");
    let at = executor::next_cell(&lot.state, lot.id, &handle)
        .unwrap()
        .unwrap();
    lot.state.settlements.get_mut(lot.id).unwrap().owner = Some(OWNER.to_string());
    (lot, handle, at)
}

const OWNER: uuid::Uuid = uuid::Uuid::from_u128(77);
const OWNER_CONNECTION: mc_domain::ConnectionId = mc_domain::ConnectionId(77);

fn log_in(lot: &mut Lot) {
    lot.state.connections.insert(
        OWNER_CONNECTION,
        mc_domain::EntityId(77),
        OWNER,
        "Owner".into(),
    );
}

/// Notices naming `at` the owner's queue holds.
fn told(lot: &Lot, at: [i32; 3]) -> usize {
    let cell = format!("{} {} {}", at[0], at[1], at[2]);
    chat(&lot.state, OWNER_CONNECTION)
        .iter()
        .filter(|line| line.contains(&cell))
        .count()
}

/// Sol S-2 / PV2-1: a paid remainder refused for provenance waits while the
/// player's edit stays in the log, which with time eviction off is forever.
/// The owner is told the blocking cell once per stall episode, also when they
/// were offline at the stall report and come online later, or had their chat
/// hidden, without new progress.
#[test]
fn a_paid_remainder_refused_for_provenance_tells_the_owner_once_after_a_late_login() {
    let (mut lot, handle, at) = paid_lot();
    // The player puts sand into the next cell; the edit stays in the log.
    lot.state.world.set_block(at[0], at[1], at[2], lot.sand);
    history(&mut lot.state, 0, &[at]);
    // Two stall windows while the owner is offline: the episode is reported.
    shift_passes(&mut lot, 2_000, 301);
    let progress = executor::status(&lot.state, lot.id, &handle).unwrap();
    // The login with chat hidden: nothing reaches the owner, nothing settles.
    log_in(&mut lot);
    lot.state
        .connections
        .set_chat_visibility(OWNER_CONNECTION, mc_domain::ChatVisibility::Hidden);
    shift_passes(&mut lot, 2_000, 5);
    assert_eq!(told(&lot, at), 0);
    // Chat shown again, a few passes and no stall window later: told once.
    lot.state
        .connections
        .set_chat_visibility(OWNER_CONNECTION, mc_domain::ChatVisibility::Full);
    shift_passes(&mut lot, 2_000, 5);
    assert_eq!(
        told(&lot, at),
        1,
        "{:?}",
        chat(&lot.state, OWNER_CONNECTION)
    );
    // Two more stall windows of the same episode: not again.
    shift_passes(&mut lot, 2_000, 301);
    assert_eq!(
        told(&lot, at),
        1,
        "{:?}",
        chat(&lot.state, OWNER_CONNECTION)
    );
    let now = executor::status(&lot.state, lot.id, &handle).unwrap();
    assert_eq!(
        (now.current_stage_index, now.completed_work_units),
        (progress.current_stage_index, progress.completed_work_units),
        "no progress started a new episode"
    );
    assert!(lot.is_sand(at), "the player's block stays");
    assert_eq!(
        executor::next_cell(&lot.state, lot.id, &handle).unwrap(),
        Some(at),
        "the paid remainder waits at the blocking cell"
    );
}

/// PV2-1: a stall reported for another cause (a filled container in the next
/// cell) settles nothing; once a retained player edit makes it a provenance
/// refusal, without progress, the online owner is told the cell once.
#[test]
fn a_stall_that_becomes_a_provenance_refusal_tells_the_owner_once() {
    let (mut lot, handle, at) = paid_lot();
    let chest = super::super::allocator::tests::block(&lot.state, "minecraft:chest", &[]);
    lot.state.world.set_block(at[0], at[1], at[2], chest);
    let mut slots = vec![mc_data::ItemStack::EMPTY; 27];
    slots[0] = mc_data::ItemStack::new(lot.item, 1);
    lot.state.block_entities.insert(
        (at[0], at[1], at[2]),
        crate::tick::BlockEntityState {
            slots,
            furnace: None,
        },
    );
    history(&mut lot.state, 0, &[]);
    log_in(&mut lot);
    shift_passes(&mut lot, 2_000, 301);
    let progress = executor::status(&lot.state, lot.id, &handle).unwrap();
    assert_eq!(told(&lot, at), 0, "no provenance refusal yet");
    history(&mut lot.state, 0, &[at]);
    shift_passes(&mut lot, 2_000, 5);
    assert_eq!(
        told(&lot, at),
        1,
        "{:?}",
        chat(&lot.state, OWNER_CONNECTION)
    );
    shift_passes(&mut lot, 2_000, 301);
    assert_eq!(
        told(&lot, at),
        1,
        "{:?}",
        chat(&lot.state, OWNER_CONNECTION)
    );
    let now = executor::status(&lot.state, lot.id, &handle).unwrap();
    assert_eq!(
        (now.current_stage_index, now.completed_work_units),
        (progress.current_stage_index, progress.completed_work_units),
        "one episode throughout"
    );
}

/// Sol S-3: a captured admission's sequence is bounded like every other ledger
/// counter by `MAX_SCRIPT_WORLD_TIME`; a larger one refuses startup.
#[test]
fn a_captured_admission_beyond_script_world_time_refuses_startup() {
    let mut lot = lot(true, true);
    let handle = lot.state.settlements.get(lot.id).unwrap().projects[0]
        .structure
        .clone();
    let set = |state: &mut TickState, since: u64| {
        let key = format!("structure/{handle}");
        let mut record = state
            .plugins
            .get_domain("@native-structures", &key)
            .unwrap()
            .clone();
        let admission = &mut record["admission"];
        assert!(admission.get("captured").is_some(), "{admission}");
        *admission = serde_json::json!({ "captured": { "since": since } });
        state.plugins.set_domain("@native-structures", &key, record);
    };
    set(&mut lot.state, mc_script::MAX_SCRIPT_WORLD_TIME);
    crate::plugin_structures::validate(&lot.state).unwrap();
    set(&mut lot.state, mc_script::MAX_SCRIPT_WORLD_TIME + 1);
    assert!(crate::plugin_structures::validate(&lot.state).is_err());
}
