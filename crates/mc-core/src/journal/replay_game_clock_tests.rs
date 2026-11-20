//! Refusals of the transient game-clock fold (`replay_game_clock.rs`).
//!
//! Each test drives one refusal branch through the public entry points
//! (`new`, `observe_batches`) and asserts the specific message, so a branch
//! that turned into a silent acceptance or a different refusal fails here.

use super::*;

fn running(held_ticks: u64) -> GameClockImage {
    GameClockImage::Running { held_ticks }
}

fn held(game_ticks: u64, since: u64) -> GameClockImage {
    GameClockImage::Held {
        game_ticks,
        held_since: Some(since),
    }
}

fn climate(clock: GameClockImage, anchor: Option<u64>) -> JournalEntry {
    JournalEntry::Climate {
        value: ClimateImage {
            day_time: 0,
            game_clock: clock,
            sim_debt_anchored_since: anchor,
            weather_kind: 0,
            remaining_ticks: 0,
            thunder_remaining_ticks: 0,
            clear_remaining_ticks: 0,
            thundering: false,
            rain_level_bits: 0,
            thunder_level_bits: 0,
            advance_time: false,
            advance_weather: false,
        },
    }
}

fn observe(
    clock: &mut ReplayGameClock,
    hi: u64,
    entry: JournalEntry,
) -> Result<(), crate::config::CoreError> {
    clock.observe_batches(&[(hi.saturating_sub(1), hi, vec![entry])], false)
}

fn refusal(result: Result<(), crate::config::CoreError>, reason: &str) {
    let error = result.expect_err("the fold must refuse this tail");
    let text = format!("{error}");
    assert!(
        text.contains(reason),
        "refusal must name {reason:?}, got {text:?}"
    );
}

/// A writer that changed the simulation-debt epoch cannot be folded onto a
/// clock anchored in another epoch.
#[test]
fn anchor_epoch_change_refuses_the_fold() {
    let mut clock = ReplayGameClock::new(0, Some(running(0))).unwrap();
    observe(&mut clock, 1, climate(running(0), Some(0))).unwrap();
    refusal(
        observe(&mut clock, 2, climate(running(0), Some(5))),
        "simulation-debt anchor epoch changed",
    );
}

/// An older writer that dropped the anchor makes the catch-up debt
/// unknowable; the fold refuses instead of guessing.
#[test]
fn anchor_epoch_removed_by_an_older_writer_refuses_the_fold() {
    let mut clock = ReplayGameClock::new(0, Some(running(0))).unwrap();
    observe(&mut clock, 1, climate(running(0), Some(0))).unwrap();
    refusal(
        observe(&mut clock, 2, climate(running(0), None)),
        "simulation-debt anchor epoch was removed by an older writer",
    );
}

/// Two running images with different held counters cannot say where the
/// transition happened: only running↔held pairs carry enough to place it.
#[test]
fn same_mode_transition_without_a_boundary_refuses_the_fold() {
    let mut clock = ReplayGameClock::new(0, Some(running(0))).unwrap();
    refusal(
        observe(&mut clock, 2, climate(running(1), Some(0))),
        "ambiguous same-mode counter transition",
    );
}

/// Two different images that resolve to the same native boundary contradict
/// each other; the fold must not pick one.
#[test]
fn conflicting_images_at_one_boundary_refuse_the_fold() {
    let mut clock = ReplayGameClock::new(0, Some(running(0))).unwrap();
    observe(&mut clock, 3, climate(held(2, 3), Some(0))).unwrap();
    assert_eq!(clock.game_ticks(3), 2);
    refusal(
        observe(&mut clock, 6, climate(running(0), Some(0))),
        "conflicting images at one transition boundary",
    );
}

/// A boundary behind the ordered native interval (here: behind a batch this
/// fold already passed) would reorder history and is refused.
#[test]
fn boundary_outside_its_ordered_interval_refuses_the_fold() {
    let mut clock = ReplayGameClock::new(0, Some(running(0))).unwrap();
    clock.observe_batches(&[(0, 5, Vec::new())], false).unwrap();
    refusal(
        observe(&mut clock, 6, climate(held(4, 5), Some(0))),
        "transition boundary is outside its ordered native interval",
    );
}
