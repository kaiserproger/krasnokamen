# CONTAINER-CUSTODY-LOST-ON-UNLOADED-REPLAY: patch-mode fix identity

Original fix: aa3e10f6. `fix_commit = "patch:449d7c4619a7acd234f38c056b01cae551efe32514d4728e9b03bf00481942f8"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/CONTAINER-CUSTODY-LOST-ON-UNLOADED-REPLAY.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote CONTAINER-CUSTODY-LOST-ON-UNLOADED-REPLAY --fix-patch docs/bugs/promotions/CONTAINER-CUSTODY-LOST-ON-UNLOADED-REPLAY.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried: journal.rs -72,6 (decline the checkpoint unless container preparation succeeds) = HEAD journal.rs:110-120; recovery.rs -278,6 (mark a replayed container chunk dirty) = HEAD recovery.rs:281-289.

Not carried: journal.rs -64,6 `fn prepare_container_checkpoint` (c): its body was rewritten by 7c21c708 (refused-chunk counting/logging, journal.rs:70-99); without the carried call it is inert in the control. capture.rs -4,6 `materialize_container_chunks` (c): only that function calls it; deleting it would force deleting 7c21c708's lines.

## Original non-test hunks

```
## aa3e10f6 non-test hunks (kept test files: none)
- crates/mc-core/src/core/journal.rs: -64,6 +64,28 pub(super) fn start_checkpoint(; -72,6 +94,17 pub(super) fn start_checkpoint_with_reply(
- crates/mc-core/src/core/recovery.rs: -278,6 +278,15 pub(super) fn load_dimension_state(
- crates/mc-core/src/world/capture.rs: -4,6 +4,30
```
