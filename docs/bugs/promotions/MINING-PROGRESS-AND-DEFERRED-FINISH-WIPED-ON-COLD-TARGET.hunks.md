# MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET: patch-mode fix identity

Original fix, three commits on this branch:

1. `c7bd44dd06` (`fix(residency): keep cold menu, mining, brush, copper and egg writes off the
   overlay`) introduced the freeze (`cold_target_or_eye`, `range_holds` before `valid_target`) and the
   `cold_delivery_frame` helper plus its call in `take_finished`; it closed the row's progress-wipe half.
2. `b460f313cc` (`fix(residency): address the control review — mining frame, uniform menu guards, egg
   unit guard`) re-ordered `range_holds` before the freeze and added the cold-neighbour regression.
3. `ae56fe9f3` (`fix(residency): deliver a queued mining finish only on a resident frame`) completes the
   hold: the 100 % + queued-finish test runs before any residency lookup, the frame is the target's ±2
   chunk span plus the actor's own chunk (at most four chunk lookups), and the test re-admits the cold
   column and asserts the finish is delivered exactly once.

`fix_commit = "patch:38621c26bae3396f…"` names the HEAD equivalents of those fix's non-test hunks in
`docs/bugs/promotions/MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET.patch` (a diff from the
pre-fix form `c7bd44dd06^:crates/mc-core/src/tick/mining.rs` to the candidate; `git apply -R` restores the
pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET --fix-patch docs/bugs/promotions/MINING-PROGRESS-AND-DEFERRED-FINISH-WIPED-ON-COLD-TARGET.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried (the whole non-test path of this row)

- `mining.rs @@ -38,8 +38,67 @@ fn valid_target` — the cold helpers `cold_cell`, `eye_cell`,
  `cold_target_or_eye`, `cold_delivery_frame` (chunk span) and `range_holds`, and the two guards in
  `advanced` (life/reach drop first, then the cold freeze before `valid_target` reads `block_at`). HEAD
  `crates/mc-core/src/tick/mining.rs:39-99`.
- `mining.rs @@ -95,6 +154,23 @@ pub(super) fn take_finished` — the hoisted
  `progress >= 1.0 && finish.is_some()` test and the delivery-frame hold. HEAD
  `crates/mc-core/src/tick/mining.rs:165-182`.
- `mining.rs @@ -102,9 +178,6 @@ pub(super) fn take_finished` — the old post-fetch `progress < 1.0`
  check the hoist replaced (removed at HEAD by the same commit).

No non-test hunk of the three commits is left out; the patch is the complete pre-fix → HEAD diff of
`crates/mc-core/src/tick/mining.rs`.

## Not carried

- `c7bd44dd06`'s product hunks in `brushing.rs`, `copper.rs`, `tick/containers.rs`,
  `tick/input/inventory.rs`, `tick/stations.rs`, `tick.rs`, `vanilla_interact.rs` — (a) the behaviour of
  BRUSH-RESET-WRITES-DUSTED-INTO-COLD-CHUNK, COPPER-HONEYCOMB-WAXES-ACROSS-COLD-CHUNK-EDGE,
  ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE, ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY and
  DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY; their reversals would red their own tests, not this row's.
- `b460f313cc`'s `inventory.rs`/`stations.rs`/`vanilla_interact.rs` hunks — (a) the same other rows.
- `c7bd44dd06`, `b460f313cc` and `ae56fe9f3` test hunks in `tick/mining_tests.rs` — (b) kept so the
  control compiles and runs the refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:tick::mining_tests::mining_progress_and_a_queued_finish_pause_while_the_target_chunk_is_cold`
  — `crates/mc-core/src/tick/mining_tests.rs:646`
  `"a cold target pauses mining progress instead of finishing"` (`mining::allows(Finish)` turns true once
  the reverse makes progress accrue from the cold overlay) and `:642` `"a queued finish waits for its
  target chunk"` for the extraction.
- `cargo:mc-core:tick::mining_tests::a_queued_finish_waits_while_a_neighbour_inside_the_delivery_frame_is_cold`
  — `crates/mc-core/src/tick/mining_tests.rs:706-709` `"a cold +-2 neighbour holds the queued finish"`;
  the same test's `:723` `"the admitted delivery frame releases the queued finish"` is the positive
  control (green at HEAD, red never asserted there).
