//! PrepLifecycle §2.3: the commit gate for cells outside village territory.
use std::sync::Arc;

use super::*;
use crate::block_log::{BlockLog, Change, Delta, Record};
use crate::config::BlockLogSettings;

/// A capturing log whose coverage floor is `coverage` (that many older records
/// were evicted by count) and which retains a player edit at each `touched`.
fn history(state: &TickState, coverage: u64, touched: &[[i32; 3]]) -> BlockLog {
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
    let records = (0..=coverage)
        .map(|seq| edit(seq, far))
        .chain(
            touched
                .iter()
                .enumerate()
                .map(|(index, at)| edit(coverage + 1 + index as u64, *at)),
        )
        .collect();
    BlockLog::restore(
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
    )
}

#[test]
fn outside_cells_need_covering_history_or_a_clean_readmission() {
    let (mut state, id, _) = crate::settlements::construction::tests::fixture();
    state.settlements.get_mut(id).unwrap().radius = 4;
    let (stone, air) = (state.registry.states.stone, state.registry.states.air);
    let (inside, outside, later) = ([8, 202, 8], [14, 202, 14], [13, 202, 14]);
    for at in [inside, outside, later] {
        state.world.set_block(at[0], at[1], at[2], stone);
    }
    let record = state.settlements.get(id).unwrap();
    assert!(record.contains(inside) && !record.contains(outside) && !record.contains(later));
    let cut = |at| Cell {
        at,
        state: air,
        resource: None,
        quantity: 0,
    };
    let cells = vec![vec![cut(outside), cut(later), cut(inside)]];
    let first = &cells[0][..1];
    let surveyed = Admission::Captured { since: 0 };

    state.block_log = history(&state, 0, &[]);
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 0], first),
        Ok(None),
        "covering history keeps the admission"
    );

    state.block_log = history(&state, 0, &[outside]);
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 0], first),
        Err(Failure::Blocked),
        "a retained player edit refuses an outside cell even under coverage"
    );

    state.block_log = history(&state, 1, &[]);
    assert!(!cell_ok(&state, id, surveyed, outside));
    let fresh = admit(&state, id, surveyed, &cells, [0, 0], first)
        .unwrap()
        .expect("a clean remaining plan is admitted again");
    assert_eq!(fresh, Admission::Captured { since: 2 });
    assert!(cell_ok(&state, id, fresh, outside));

    state.block_log = history(&state, 1, &[later]);
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 0], first),
        Err(Failure::Blocked),
        "a retained player edit on a remaining outside cell refuses re-admission"
    );

    state.block_log = history(&state, 1, &[inside]);
    assert!(cell_ok(&state, id, surveyed, inside));
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 2], &cells[0][2..]),
        Ok(None),
        "inside the territory history is not consulted"
    );
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 0], first),
        Ok(Some(Admission::Captured { since: 3 })),
        "a player edit inside the territory never refuses the outside plan"
    );

    assert_eq!(
        admit(&state, id, Admission::Uncaptured, &cells, [0, 0], first),
        Ok(None),
        "a manifest surveyed without capture keeps the documented residual"
    );

    state.block_log = BlockLog::restore(
        BlockLogSettings {
            enabled: false,
            ..BlockLogSettings::default()
        },
        Vec::new(),
    );
    assert!(!cell_ok(&state, id, surveyed, outside));
    assert_eq!(
        admit(&state, id, surveyed, &cells, [0, 0], first),
        Ok(None),
        "a re-admission without capture keeps the captured survey"
    );
}

#[test]
fn placing_into_air_destroys_nothing_and_needs_no_history() {
    let (mut state, id, _) = crate::settlements::construction::tests::fixture();
    state.settlements.get_mut(id).unwrap().radius = 4;
    let outside = [14, 202, 14];
    let cells = vec![vec![Cell {
        at: outside,
        state: state.registry.states.stone,
        resource: None,
        quantity: 0,
    }]];
    state.block_log = history(&state, 1, &[]);
    assert_eq!(
        admit(
            &state,
            id,
            Admission::Captured { since: 0 },
            &cells,
            [0, 0],
            &cells[0]
        ),
        Ok(None)
    );
}
