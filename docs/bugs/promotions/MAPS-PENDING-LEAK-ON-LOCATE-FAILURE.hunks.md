# MAPS-PENDING-LEAK-ON-LOCATE-FAILURE — fix-hunk map

The production-only patch `MAPS-PENDING-LEAK-ON-LOCATE-FAILURE.patch` has the non-test hunk identity
`patch:a064694411dbc13f1fadf9d2baef581413e3994bfb96e81c5441511059f223a2` (SHA-256 of its non-test
hunks as `ledger promote` splits them). It reverse-applies on the promotion candidate
(`git apply -R --check`, clean tree) and its forward direction restores the candidate byte-for-byte.

## The two original commits and the part of the row's path each closes

- `8499f457a` (the behaviour fix) adds the failure continuation: the `StructureLookupFailure` alias,
  `StructureLookupRequest.on_failure`, the `LocateCompletion::Callback { reply, on_failure }` struct
  variant, the `submit_structure_lookup` pass-through, the three terminal invocations in `deliver`
  (metadata error), `terminal_wrong_dimension` (wrong dimension) and `fail_worker` (worker loss), and
  the two caller closures in `maps/loot.rs` / `maps/trades.rs` that remove `pending_loot` /
  `pending_trades`. It also rustfmt-normalizes unrelated lines in `command_locate.rs`.
- `94410e6d4` (test reachability) widens `LocateCompletion` to `pub(crate)`, adds the
  `#[cfg(test)] take_pending_callback_release` hook, the `maps/tests.rs` release test, and the row's
  first field set. It closes no production path of its own.

## Hunk map: everything the patch carries, and every hunk left out

`8499f457a` — `crates/mc-core/src/command_locate.rs`:

| original hunk | content | disposition |
|---|---|---|
| `@@ -695,9` (`LocateState::busy`) | rustfmt: joins the `Entity` arm | carried; the a/-side is the pre-image's two-line arm |
| `@@ -1168,6` | adds the `StructureLookupFailure` alias | **(b)** left out: the kept `StructureLookupRequest.on_failure` field and the kept test hook name the alias; reversing leaves the type unresolved |
| `@@ -1179,12` | adds `on_failure` to the request and turns `Callback(reply)` into the struct variant | **(b)** left out: the kept tests construct `Callback { reply, on_failure }` and the kept `take_pending_callback_release` matches the struct variant |
| `@@ -1220,7` (`submit_structure_lookup`) | forwards `request.on_failure` into the struct variant | **(b)** left out: reversing it builds the removed tuple variant |
| `@@ -1265,9`, `@@ -1287,14`, `@@ -1303,9`, `@@ -1335,9`, `@@ -1365,9`, `@@ -1379,9` (`submit` error arms) | rustfmt only | carried; the a/-side is the pre-image's wrapping |
| `@@ -1439,12` (`deliver`) | metadata error releases the plan | carried; the a/-side is the pre-image text (comment, warn line) with the struct pattern `Callback { reply, .. }` |
| `@@ -1498,7` (`deliver`, `Command` arm) | rustfmt only | carried |
| `@@ -1521,7` (`command_result`) | rustfmt only | carried |
| `@@ -1579,8` (`terminal_wrong_dimension`) | wrong dimension releases the plan | carried; the a/-side is the pre-image text with the struct pattern `Callback { .. }` |
| `@@ -1614,9` (`fail_worker`) | worker loss releases the plan | carried; the a/-side is the pre-image text with the struct pattern `Callback { .. }` |

`8499f457a` — other files:

- `crates/mc-core/src/command_locate_tests.rs`: test file, kept in the candidate (the splitter never
  carries test hunks).
- `crates/mc-core/src/maps/loot.rs` and `crates/mc-core/src/maps/trades.rs`: **(b)** left out. Their
  one production change each is `on_failure: Some(Box::new(move |state| { state.maps.pending_* .remove(&key); }))`
  in the `StructureLookupRequest` literal. Reversing them omits a field the kept struct still
  requires, so the control would not compile; the kept closures are stored, never invoked, on the
  a/-side.

`94410e6d4` — `crates/mc-core/src/command_locate.rs`:

| original hunk | content | disposition |
|---|---|---|
| `@@ -1187,7` | `enum LocateCompletion` → `pub(crate) enum LocateCompletion` | carried; the pre-image's private enum, no behaviour change (nothing outside the module names the type; the hook beside it still compiles) |
| `@@ -1608,6` | adds `#[cfg(test)] take_pending_callback_release` | **(b)** left out: the kept `maps/tests.rs` ref calls it; reversing it stops the control from compiling |

`94410e6d4` — `crates/mc-core/src/maps/tests.rs` (kept test file) and
`docs/bugs/rows/MAPS-PENDING-LEAK-ON-LOCATE-FAILURE.toml` (docs, skipped by the splitter).

## The a/-side against the real pre-image `8499f457a^`

The a/-side is `8499f457a^`'s text for every region the two commits touched, including the pre-image
comments, warn lines, line wrapping and private enum. The complete list of differences:

1. `deliver`: the pattern is `Callback { reply, .. }` instead of `Callback(reply)`.
2. `terminal_wrong_dimension`: the pattern is `Callback { .. }` instead of `Callback(_)`.
3. `fail_worker`: the pattern is `Callback { .. }` instead of `Callback(_)`.
4. The (b) API the kept tests, hook and callers need is HEAD's, not the pre-image's: the
   `StructureLookupFailure` alias, `StructureLookupRequest.on_failure`, the
   `LocateCompletion::Callback { reply, on_failure }` variant, the `submit_structure_lookup`
   forward, the `maps/loot.rs` and `maps/trades.rs` closures, and `take_pending_callback_release`
   (whose own `match` uses `Callback { on_failure, .. }`).

None of these changes behaviour: the three pattern lines only stop binding the kept `on_failure`
field, and on the a/-side nothing ever calls a stored closure — the three release calls are absent,
which is the row's bug. The unused field produces one dead-code warning in the control build; no
test reads it there.

## Why this control is honest: the proving refs

Every claimed terminal path has its own registered ref, and each red is that test's own release-flag
assertion:

| ref | assertion |
|---|---|
| `cargo:mc-core:command_locate::tests::a_metadata_error_releases_a_map_loot_callback_so_the_container_opens_again` | `command_locate_tests.rs:1020` — `a metadata error must release the pending loot/trade plan` |
| `cargo:mc-core:command_locate::tests::a_wrong_dimension_reply_releases_a_map_loot_callback_so_the_container_opens_again` | `command_locate_tests.rs:1068` — `a wrong-dimension reply must release the pending loot/trade plan` |
| `cargo:mc-core:command_locate::tests::worker_loss_releases_a_map_loot_callback_so_the_container_opens_again` | `command_locate_tests.rs:951` — `worker loss must release the pending loot/trade plan` |

The metadata-error ref sends `LocateDone { error: Some(..) }` through `drain` (the worker's own
answer), the wrong-dimension ref queues a callback plan whose dimension differs from
`state.dimension`, and the worker-loss ref drops the done channel. The fourth ref,
`cargo:mc-core:tick_tests::maps_tests::a_released_locate_plan_lets_the_container_open_again`, is
**not evidence**: it drives `take_pending_callback_release` directly, so it is green on both sides;
it guards that a released plan leaves `pending_loot` and a reopen re-plans.

## Observed control and restore

`python3 -m tools.harness ledger promote MAPS-PENDING-LEAK-ON-LOCATE-FAILURE --fix-patch
docs/bugs/promotions/MAPS-PENDING-LEAK-ON-LOCATE-FAILURE.patch`, receipt
`20261124T053858-ledger-promote-yl4bphf3` on `6930c32408` (`tree_dirty: false`,
`control_tree: head`, `reversed_files` = `crates/mc-core/src/command_locate.rs`); the receipt is
`MAPS-PENDING-LEAK-ON-LOCATE-FAILURE.json` beside this file.

- Control (`ledger-1-run.log`) — all three claimed terminal paths fail at their own release
  assertion, so the red covers the whole claim, not only worker loss:
  - `command_locate_tests.rs:1020` — `a metadata error must release the pending loot/trade plan`;
  - `command_locate_tests.rs:1068` — `a wrong-dimension reply must release the pending loot/trade plan`;
  - `command_locate_tests.rs:951` — `worker loss must release the pending loot/trade plan`;
  the maps ref is green in the control as expected (it is not evidence).
- Restore (`ledger-3-run.log`) — all four refs green on `6930c32408`.

## What stays out of the row

No wall-clock or counter claim is made; the patch changes only which terminal paths invoke the
caller's stored release. `maps/loot.rs`, `maps/trades.rs` and the test hook are kept as API, not
reversed.
