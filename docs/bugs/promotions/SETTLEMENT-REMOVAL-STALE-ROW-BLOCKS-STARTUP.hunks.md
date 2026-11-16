# Settlement removal stale-row control

- Original fix: `0794f3da96706a730fb672390406ce77909e311b` (cherry-pick of `3f15ef1f1`).
- Production hunk in `crates/mc-world/src/native/checkpoint.rs` is included unchanged: the checkpoint transaction deletes omitted `settlement/` rows. Reversing it restores the original stale-row behavior.
- The `#[cfg(test)]` module registration in `crates/mc-core/src/core.rs` is kept. Although the generic non-test splitter includes it, it is test infrastructure: removing it would make the registered control test unavailable rather than reproduce the symptom (omission reason b).
- The new test file and documentation/ledger hunks are not production fix hunks. No production hunk is omitted and no a-side adaptation is needed.
- Proving ref: `cargo:mc-core:core::settlement_removal_persistence_tests::removed_settlement_row_is_deleted_by_the_next_checkpoint_and_the_world_reopens`. The symptom assertion is the recovery expectation in `crates/mc-core/src/core/settlement_removal_persistence_tests.rs:107`; without deletion it fails with `native settlement/1: capture tick mismatch`. The original failing receipt is `receipts/SettlementRowDelete/WRITEUP.md` and `wt/SettlementRowDelete/.analysis/validation/20261116T145232-test-igi5nlbk/test.log` on the owner host.
- Candidate control: `wt/SettlementRowPromote/.analysis/validation/20261116T152434-ledger-promote-qbpur72t/ledger-1-run.log` fails at that same recovery assertion; `ledger-3-run.log` passes after restoration.
- The independently reviewed scope is the real command, synchronous production checkpoint writer and recovery helper, not a journal-thread loop, crash injection or full `Core::open` after removal; see `receipts/SettlementRowDelete/REVIEW-astra.md`.
