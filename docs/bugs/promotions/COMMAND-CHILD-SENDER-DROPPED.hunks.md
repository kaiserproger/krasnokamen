# COMMAND-CHILD-SENDER-DROPPED: patch-mode fix identity

Original fix: `334c3f686` (`fix(core): run queued commands in the world they were queued from`) — it
replaced the root-state check in `execute_job` with a per-job check. `fix_commit =
"patch:b8374c6a589f4544…"` names the HEAD equivalent in
`docs/bugs/promotions/COMMAND-CHILD-SENDER-DROPPED.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote COMMAND-CHILD-SENDER-DROPPED --fix-patch docs/bugs/promotions/COMMAND-CHILD-SENDER-DROPPED.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Mechanism (HEAD, corrected 2026-11-24 after the final review)

`execute_job` (`crates/mc-core/src/command_context_deferred.rs:137-142`) reads the permission observed
during the job's dimension phase, resolves that world, and runs the job only when
`job_source_is_current(world, observed, job)` holds. `job_source_is_current` (`:164-190`) checks the source
entity in that world's own connection table and consults **either** the job world's plugin-player
permission **or** the root table's observed value (`.or(observed)`), because `swap_globals` moves the
permission table between dimensions. The pre-fix form checked the *root* `TickState` only:
`if !source_is_current(state, job.context, job.source) { return; }`. With the reverse applied, the root
table has the sender's connection but the observed permission belongs to the child phase, so the check
fails and the queued `/kill`, `/summon` and `/gamerule` job is dropped — the row's symptom.

## Carried

- `command_context_deferred.rs @@ -135,9 +135,50 @@` (the `execute_job` rewrite) — HEAD equivalent
  `crates/mc-core/src/command_context_deferred.rs:137-142`: the reverse restores the root-context
  `source_is_current` check and then calls `run_job`. The a/-side is the pre-fix `execute_job` body from
  `334c3f686^` (same in old trees, e.g. `wt/XCoreItems/.../command_context_deferred.rs:137-150` and
  `wt/LedgerCommands:109-118`); the only difference is this tree's extracted `run_job`, which holds the
  identical block-chain/`super::run` body.

## Original non-test hunks (complete)

```
## 334c3f686 non-test hunks (kept test files: crates/mc-core/src/dimensions_tests.rs)
- crates/mc-core/src/command_context_deferred.rs: -135,9 +135,50 fn source_is_current; -150,6 +191,28 pub(crate) fn execute_job
```

## Not carried

- The `source_is_current` sweep/changed-world code inside `@@ -135,9 +135,50` — **(a)-not-a-hunk of this
  fix**: it predates `334c3f686` (present beside `execute_job` in old trees,
  `wt/XCoreItems/.../command_context_deferred.rs:126-139`), so there is nothing of it to reverse; it stays
  because the reversed body calls it. Labelling it (a) means "other row/pre-fix code", not "another hunk".
- `command_context_deferred.rs @@ -150,6 +191,28 @@ pub(crate) fn execute_job` — this fix's second hunk:
  the `job_source_is_current` helper that the new `execute_job` body calls. Carried inside the single
  reversed hunk region: the patch's b/+ side keeps it, the a/- side (its reverse) leaves it in place
  unused. It is not reversed because no test calls it directly (only `execute_job` does) and the patch
  must stay a minimal one-hunk reverse; (b) applies to the *kept* lines, not to a behaviour reverse.
- Test files — kept (b).

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:dimensions::tests::data_edits_an_entity_for_senders_in_the_nether_and_the_end`
  — `crates/mc-core/src/dimensions_tests.rs:2823` sends `summon` from the Nether/End and `:2839-2843`
  asserts `world.entities.row(slot).health == 4.0` after `data merge entity … {Health:4f}`: with the
  root-context check the job is dropped and the cow keeps `Health:9f` (the merge never runs).
- `cargo:mc-core:dimensions::tests::send_command_feedback_set_from_the_nether_is_published_for_the_edge`
  — `:441` and `:443` assert the published flag follows the Nether sender's `gamerule
  send_command_feedback`; both commands are dropped by the root check, so the first assertion stays true
  when it must become false.
- `cargo:mc-core:dimensions::tests::kill_from_the_nether_clears_the_live_nether_cow_and_spares_the_root`
  — `:2792-2795` `"the child kill left a live Nether cow"` and `:2807-2810` `"the child kill cleared the
  live root sentinel"`: with the root check the `/kill` is dropped and the Nether cow stays alive.
- Not evidence: `anchor_respawn_can_stand_on_top_when_surrounded` (`:1699`) and
  `anchor_respawn_uses_carpet_collision_height` (`:1737`). They are also registered, and they went red in
  the control, but their red depends on the test's own setup (respawn anchor geometry, not the deferred
  job path); the review marks them setup-dependent and this mapping does not claim them.
