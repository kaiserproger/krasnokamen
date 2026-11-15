use super::*;

#[test]
fn chunk_batch_initial_limit_ack_resume_and_ten_batch_ceiling() {
    let mut state = ChunkBatchState::default();
    assert_eq!(state.allowance(), 9);
    state.sent(9);
    for _ in 0..100 {
        assert_eq!(state.allowance(), 0);
    }
    state.acknowledge(64.0);
    for _ in 0..10 {
        assert_eq!(state.allowance(), 16);
        state.sent(16);
    }
    for _ in 0..100 {
        assert_eq!(state.allowance(), 0);
    }
    state.acknowledge(4.0);
    assert_eq!(state.allowance(), 4);
    state.sent(4);
    assert_eq!(state.allowance(), 0);
}

#[test]
fn chunk_batch_clamps_nan_negative_infinite_and_huge_rates() {
    for (input, expected) in [
        (f32::NAN, 0.01),
        (-1.0, 0.01),
        (f32::NEG_INFINITY, 0.01),
        (0.0, 0.01),
        (f32::INFINITY, 64.0),
        (f32::MAX, 64.0),
        (4.0, 4.0),
    ] {
        let mut state = ChunkBatchState::default();
        state.acknowledge(input);
        assert_eq!(state.desired, expected);
        assert!(state.allowance() <= 16);
    }
}

#[test]
fn chunk_batch_fractional_quota_accumulates_without_idle_burst_or_limit_credit() {
    let mut state = ChunkBatchState::default();
    state.acknowledge(0.25);
    assert_eq!(state.allowance(), 1); // ACK with no outstanding batch resets quota to one.
    state.sent(1);
    assert_eq!(state.allowance(), 0);
    assert_eq!(state.allowance(), 0);
    assert_eq!(state.allowance(), 0);
    assert_eq!(state.allowance(), 1);
    state.sent(1);
    for _ in 0..100 {
        state.allowance();
    }
    assert_eq!(state.quota, 1.0);
    state.unacknowledged.resize(state.limit, (u64::MAX, 0));
    let quota = state.quota;
    for _ in 0..100 {
        assert_eq!(state.allowance(), 0);
    }
    assert_eq!(state.quota, quota);
}

#[test]
fn chunk_batch_duplicate_ack_never_underflows_and_resets_quota() {
    let mut state = ChunkBatchState::default();
    for _ in 0..20 {
        state.acknowledge(0.01);
    }
    assert!(state.unacknowledged.is_empty());
    assert_eq!(state.quota, 1.0);
    assert_eq!(state.limit, 10);
}

#[test]
fn chunk_batch_unsolicited_ack_matches_vanilla_zero_debt_semantics() {
    let mut state = ChunkBatchState::default();
    state.acknowledge(0.01);
    assert_eq!(state.limit, 10);
    assert_eq!(state.quota, 1.0);
    assert!(state.unacknowledged.is_empty());
    assert_eq!(state.allowance(), 1);
    let batch = state.sent(1);
    state.acknowledge(4.0);
    assert_eq!(state.quota, 1.0);
    state.discarded(batch);
    assert_eq!(state.quota, 1.0, "acknowledged work is not refunded");
}
