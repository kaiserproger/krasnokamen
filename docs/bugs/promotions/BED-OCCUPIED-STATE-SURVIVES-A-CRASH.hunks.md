# BED-OCCUPIED-STATE-SURVIVES-A-CRASH: patch-mode fix identity (re-promotion)

`fix_commit = "patch:b299f880de3250e234f5d527be8c19bacab85af3cbe32f2ec158f74ac4ff6e34"`
(`docs/bugs/fix-patches/BED-OCCUPIED-STATE-SURVIVES-A-CRASH.patch`, a diff from the
row's base `bfab7c1327` to head `febee2d55b` over the fix's three production
files).

This supersedes the first promotion (`20261125T184507-ledger-promote-kxgntxp9`,
`fix_commit = 59f68649f1`): the Opus review's R1 found that seeding **both** bed
halves rewrites a live sleeper's bed, and the corrected fix is a second commit on
top (`27feed72df`) rather than a rewrite of the promoted `59f68649f1`.

## The fix, in two commits

| commit | what it adds |
|---|---|
| `59f68649f1` | `sync_chunk_block_entities` calls `sleep::seed_occupied_beds`; the seed puts every `*_bed` with `occupied=true` into `TickState::occupied_beds`, so the existing mirror clears a property a mid-sleep checkpoint persisted across a restart. No persistence-format change. |
| `27feed72df` | The seed names only the **head** cell (`Sleeper::bed`, a resident's `home`), normalising either half by `facing`, and the docs say the pass runs on **every hydration** (every storage-revision bump), not only on admission. Both-halves seeding had put the foot in `previous` against a `wanted` naming only the head, so a sleeping player's bed was cleared and set again after every revision of its chunk (four `BlockChange`s to clients and the WAL). Also adds the no-rewrite guard. |

The patch is the union of the two commits' non-test hunks as one base→head diff,
so its `a/`-side is the real pre-image (the base blob) and it reverse-applies on
the candidate (`git apply -R --check` rc 0). Test files
(`crates/mc-core/src/core_checkpoint_custody_tests.rs`,
`crates/mc-core/src/sleep_tests.rs`) are excluded and stay on both sides.

## Proving assertion

`cargo:mc-core:core::checkpoint_custody_tests::checkpoint_mid_sleep_leaves_no_stale_occupied_bed_after_reopen`
— `crates/mc-core/src/core_checkpoint_custody_tests.rs:4084` on this head (it was
`:4081` before the `style:` commit; the receipt records the line at its own
tree). Reversed (no seed), the reopen keeps `occupied=true` on both halves and
the first tick cannot clear it, so the test fails exactly there
(`.analysis/validation/20261125T214523-ledger-promote-gnmt1dj7/ledger-1-run.log:575-576`).

`cargo:mc-core:sleep::tests::a_sleepers_bed_is_not_rewritten_on_the_next_tick` is
the row's second ref and is named out of `proving_refs`: with the whole seed
removed the mirror writes nothing, so it is green in the control by design
(recorded as green in `promote.json`). On the candidate it is the guard against
the both-halves churn; it was red on `59f68649f1` with four `BlockChange`s on the
bed cells (`20261125T203230-test-bc928l6a`).

## Candidate

`tree_head = febee2d55b`, `tree_dirty = false`; both refs green in the restore
(`.analysis/validation/20261125T214523-ledger-promote-gnmt1dj7/ledger-3-run.log`).
Handoff note: the receipt's `tree_head` predates the later docs-only commits of
this lane; the code it ran is the code the patch reverses.
