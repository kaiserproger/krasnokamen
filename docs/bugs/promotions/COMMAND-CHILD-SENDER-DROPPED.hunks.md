# COMMAND-CHILD-SENDER-DROPPED: patch-mode fix identity

Original fix: `334c3f686` (`fix(core): run queued commands in the world they were queued from`) — it
scoped `execute_job` to the job's own dimension and re-read the permission from that world, instead of
running every queued job against the root `TickState`/root context. A later rewrite of
`command_context_deferred.rs` (the `swap_globals` comment and `job_source_is_current` helper) means the
commit's diff no longer reverse-applies, so the patch is the HEAD equivalent of the fix's behaviour:
the root-context `source_is_current` check followed by the job run. `fix_commit =
"patch:b8374c6a589f4544…"` names it in
`docs/bugs/promotions/COMMAND-CHILD-SENDER-DROPPED.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote COMMAND-CHILD-SENDER-DROPPED --fix-patch docs/bugs/promotions/COMMAND-CHILD-SENDER-DROPPED.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_context_deferred.rs @@ -135,9 +135,50 @@` (the `execute_job` rewrite) — the HEAD equivalent is
  the `execute_job` body at `crates/mc-core/src/command_context_deferred.rs:137-142`: reversing it
  restores `if !source_is_current(state, job.context, job.source) { return; }` against the root world and
  runs the job there, exactly the pre-fix behaviour (the a/-side is prior code, not an invented stub).
  HEAD's `run_job` keeps the block-chain/`super::run` body; the `job_source_is_current` helper and the
  `swap_globals` comment (from `76a539ea9` and the later rewrite) stay, unused by the reversed path —
  (c) for that helper's own text.

## Original non-test hunks (complete)

```
## 334c3f686 non-test hunks (kept test files: crates/mc-core/src/dimensions_tests.rs)
- crates/mc-core/src/command_context_deferred.rs: -135,9 +135,50 fn source_is_current; -150,6 +191,28 pub(crate) fn execute_job
```

## Not carried

- The remainder of those two hunks (`source_is_current`'s world sweep and the `job_source_is_current`
  helper) — (b) the reversed `execute_job` calls `source_is_current` and the kept dimensions tests drive
  the same helper; the helper is what makes the pre-fix control compile.
- Test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:dimensions::tests::data_edits_an_entity_for_senders_in_the_nether_and_the_end`
- `cargo:mc-core:dimensions::tests::send_command_feedback_set_from_the_nether_is_published_for_the_edge`
- `cargo:mc-core:dimensions::tests::anchor_respawn_can_stand_on_top_when_surrounded`
- `cargo:mc-core:dimensions::tests::anchor_respawn_uses_carpet_collision_height`
- `cargo:mc-core:dimensions::tests::kill_from_the_nether_clears_the_live_nether_cow_and_spares_the_root`

With the root-context check restored, a sender standing in the Nether/End is absent from the root
connection table, so `source_is_current` fails and the job is dropped: the first two refs fail on their
own nether-sender assertions and `kill_from_the_nether_clears_the_live_nether_cow_and_spares_the_root`
leaves the nether cow alive. No client ref is registered for this row.
