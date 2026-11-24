# SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC: patch-mode fix identity

Original fix: `a6a30c6694` (`fix(tick): name the writer when a spawned body already overlaps a
collision box`) — `TickState::spawn_mob` gained the clearance scan and the warn line.
`fix_commit = "patch:bf1a0ca535bf1096…"` names the HEAD equivalent of this row's non-test hunk in
`docs/bugs/promotions/SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC.patch` (a diff from the pre-fix form to
the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC --fix-patch docs/bugs/promotions/SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Why patch mode

`a6a30c6694` also declared the test module at the end of the production file (`#[cfg(test)] #[path =
"entity_lifecycle_tests.rs"] mod tests;`). Reversing that hunk in the control deletes the declaration, so
the row's refs become **unrunnable** instead of failing at their assertion — the first control attempt was
refused for exactly that (`20261124T024843-ledger-promote-mutmjuuj`: `cargo:mc-core:tick::entity_lifecycle::tests::…=unrunnable
(unknown or ignored test)`). `a00741b366` then moved the declaration to `tick.rs` beside the tick host's
other `#[path]` test modules and registered the refs under `tick::entity_lifecycle_tests`. Because HEAD no
longer has the `mod tests;` lines, `git show a6a30c6694 -- crates/mc-core/src/tick/entity_lifecycle.rs |
git apply -R --check` fails at HEAD, and the `a6a30c6694` tree cannot produce a control for the moved ref
names: its hunks no longer reverse-apply on the candidate and its tree cannot run the kept tests. The
patch below is the production half of that hunk, ported to the candidate.

## Carried

- `entity_lifecycle.rs @@ -175,7 +175,95 @@ impl TickState` — the whole warn block plus the
  `body_overlaps_collision` helper. HEAD `crates/mc-core/src/tick/entity_lifecycle.rs:178-194` (the call
  and the `tracing::warn!`) and `:199-268` (the helper). The a/-side is the exact pre-image
  (`git show a6a30c6694^:crates/mc-core/src/tick/entity_lifecycle.rs`); there is **no** a/-side adaptation.

  Verified: the patch's a/-blob is byte-identical to that real pre-image blob
  (`diff /tmp/real_pre_el.rs /tmp/pre_el.rs` → no output), so the only difference between the pre-image and
  HEAD is the reversed fix. No counter, oracle, helper text or test-visible API exists on the a/- side:
  the kept test's only observable is the real `tracing::warn!` the fix adds, and the pre-fix code emits
  none. Reversing the patch therefore removes behaviour the pre-fix tree genuinely lacked, not a probe
  invented for the control.

## Original non-test hunks (complete)

```
## a6a30c6694 non-test hunks (kept test file: crates/mc-core/src/tick/entity_lifecycle_tests.rs)
- crates/mc-core/src/tick/entity_lifecycle.rs: -175,7 +175,99 (warn block, helper, and the test-module declaration)
```

## Not carried

- The three trailing declaration lines of that hunk (`#[cfg(test)]`, `#[path = "entity_lifecycle_tests.rs"]`,
  `mod tests;`) — (c) `a00741b366` rewrote them into `crates/mc-core/src/tick.rs:87-89`, where they must
  stay for the control to compile and fail at the assertion; reversing them again would only make the refs
  unrunnable. They are not behaviour lines.
- The test file — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:tick::entity_lifecycle_tests::a_body_spawned_inside_a_block_is_created_and_named`
  (`crates/mc-core/src/tick/entity_lifecycle_tests.rs:59`, `assert_eq!(lines.len(), 1)`, and the field
  loop at `:61-71`): with the fix reversed no warn is emitted, so the captured list is empty.
- `cargo:mc-core:tick::entity_lifecycle_tests::a_body_standing_on_the_collision_box_is_not_reported`
  (`:85`) and `…a_body_spawned_outside_the_resident_world_is_not_reported` (`:103`): they are the
  no-false-positive halves. They stay **green** with the fix reversed (an empty list is what they assert),
  so they are **not evidence** for the control; the row's red is `:59`.
