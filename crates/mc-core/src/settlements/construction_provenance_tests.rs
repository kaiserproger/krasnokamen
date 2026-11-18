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

/// A house lot holding three sand cuts: `first` (top of a two-high pillar,
/// cut first because cuts run top-down), `below` it, and a lone `later`
/// block in the far corner.
struct Lot {
    state: TickState,
    id: SettlementId,
    builder: EntitySlot,
    first: [i32; 3],
    below: [i32; 3],
    later: [i32; 3],
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
    for at in [first, below, later] {
        state.world.set_block(at[0], at[1], at[2], sand);
    }
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
    for at in [first, below, later] {
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
    lot.state.day_time = 2_000;
    for _ in 0..=300 {
        crate::villager::building::tick(&mut lot.state, lot.id);
        lot.state.tick += 20;
        lot.state.day_time += 20;
        assign_builder(&mut lot.state, lot.id);
    }
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
