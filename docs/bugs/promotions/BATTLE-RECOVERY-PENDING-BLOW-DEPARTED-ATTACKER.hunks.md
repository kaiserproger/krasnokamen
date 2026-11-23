# BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER — fix-hunk map

The forward-ported production-only patch `BATTLE-RECOVERY-PENDING-BLOW-DEPARTED-ATTACKER.patch` is byte-identical to the row's `docs/bugs/fix-patches/` patch, SHA-256 `5c6803ffc5ea93d667c5f00fabb120c6a9a59c7b4866e7df2af245cea4b0a87f`. It maps two original commits onto the current `battle/retaliation.rs::Retaliation::reconcile_capture`:

| Original production change | Current patch equivalent |
|---|---|
| `1ef97c325ea57f4a7c072b1245c4219bc44dbf4e`: absent attacker `true → false` | Patch lines 7–9: absent attacker `true → live.historical_players().any(...)`. The final value incorporates the held-player follow-up below. |
| `1ef97c325ea57f4a7c072b1245c4219bc44dbf4e`: non-player attacker `true → body.alive` | Patch lines 11–14: `true → body.alive`, preserving the later source layout. |
| `ae0f2ec11bf509b798074ef16e5a95664fb5968e`: absent attacker `false → live.historical_players().any(...)` | Same patch lines 7–9. This is required to preserve a departed held player's pending blow. |

Original non-test exclusions: `ae0f2ec11` changes only the explanatory comment in `battle/persistence.rs`; `1ef97c325` and `ae0f2ec11` replace explanatory comments in `battle/retaliation.rs`. These comments do not alter behavior and the first comment was superseded by the later commit. `1ef97c325` also rewrites the non-player kind lookup into a `body` binding so `body.alive` can be read; the adapted control retains that later binding without changing the old `true` outcome. No other non-test behavioral hunks occur in either commit. Test hunks in `battle/retaliation_tests.rs` and `native_battle_persistence_drill_tests.rs`, and both commits' `docs/architecture/persistence.md` hunks, remain in the candidate and are not part of the production-only inverse.

Proving refs, all in `mc-core::core::native_checkpoint_crash_tests::persistence_drills::battle_resume`:

- `battle_resume_clean_checkpoint_with_a_pending_blow_by_a_departed_horse_reopens`
- `battle_resume_clean_checkpoint_after_a_foreign_attacker_dies_in_its_blow_merge_reopens`
- `battle_resume_clean_checkpoint_after_a_creeper_blast_on_soldiers_reopens`

The retained combined inverse `.analysis/validation/20261123T073128-ledger-promote-3h7v3kno/ledger-1-run.log` makes all three fail in `recover_offline` with `unknown retaliation identity` at `native_battle_persistence_drill_tests.rs:1156`; `ledger-3-run.log` restores all four registered refs green on HEAD `17b4abdd` and dirty fingerprint `3c2223f681e036c28c90322898f0e0f02d4b94e10ef754b8a0d646863acb5310`. The separate held-player ref stays green under this combined inverse. An earlier control reversing only `ae0f2ec11` made only the held-player assertion red and is **not** proof of the departed-attacker startup failure. These receipts are historical candidate evidence, not a current-tree promotion or real-client acceptance.
