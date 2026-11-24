# SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC: patch-mode fix identity

Two commits, one path each:

- `a6a30c6694` (`fix(tick): name the writer when a spawned body already overlaps a collision box`) — the
  diagnostic itself: `TickState::spawn_mob` scans the cells the new AABB covers and emits one WARN (tick,
  slot, type id, blueprint name, position, half width, height, writer) when a resident collision box
  overlaps; the spawn is never refused.
- `6130efff2` (`fix(tick): scan only living bodies for the spawn clearance diagnostic`) — review 6 B2: the
  scan runs only for `EntityArchetype::Living` (hanging entities, items, orbs and other non-living
  archetypes cannot suffocate; a wall frame's 0.25 half width enters its support by 0.21875 and used to
  WARN), and residency is decided by `has_chunk` with `block_at` only reading the block (cold overlay
  included).

`fix_commit = "patch:7ca5ea378d79cf6b…"` names the HEAD equivalent of those two commits' non-test hunks in
`docs/bugs/promotions/SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC.patch` (a diff from the real pre-image
`a6a30c6694^:crates/mc-core/src/tick/entity_lifecycle.rs` to the candidate; `git apply -R` restores the
pre-fix code — verified). Re-audit:
`python3 -m tools.harness ledger promote SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC --fix-patch docs/bugs/promotions/SPAWN-MOB-HAS-NO-BLOCK-CLEARANCE-DIAGNOSTIC.patch --dry-run`.

(Rewritten after review 7 B1: the earlier file still described the superseded `patch:c05869f2…`, a single
hunk and a "re-audit after the style pass". The rustfmt commit `e1547078ec` only reformatted lines inside
these additions, so the digest moved from `c05869f2…` to `7ca5ea37…`; the a/-side never changed.)

## Carried (two hunks, both pure additions)

- `entity_lifecycle.rs @@ -104,6 +104,14 @@` — the `living` filter computed from the entity-type
  contract (`EntityArchetype::Living`). HEAD `crates/mc-core/src/tick/entity_lifecycle.rs:107-114`.
- `entity_lifecycle.rs @@ -175,7 +183,102 @@ impl TickState` — the WARN block (gated by `living`), HEAD
  `:186-203`, and the `body_overlaps_collision` helper, HEAD `:207-...` (the `has_chunk` residency check
  plus the collision-box scan).

Both hunks are additions; there is no a/-side adaptation and no invented code. The patch's a/-blob is
byte-identical to the real pre-image `git show a6a30c6694^:crates/mc-core/src/tick/entity_lifecycle.rs`.

## Not carried

- The three test-module declaration lines added by `a6a30c6694` and moved to `tick.rs` by `a00741b366` —
  (c) a later commit rewrote their location; they stay at `crates/mc-core/src/tick.rs:87-89` so the control
  can fail at the assertion instead of losing the module.
- The test file — (b) kept test API: the registered refs live there.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:tick::entity_lifecycle_tests::a_body_spawned_inside_a_block_is_created_and_named`
  (`crates/mc-core/src/tick/entity_lifecycle_tests.rs:59`, `assert_eq!(lines.len(), 1)`, fields at
  `:61-71`) — RED in the control: without the fix no WARN is emitted, so the captured list is empty.
- `a_body_standing_on_the_collision_box_is_not_reported`, `a_wall_mounted_item_frame_is_not_reported` and
  `a_body_spawned_outside_the_resident_world_is_not_reported` stay **green** with both hunks reversed and
  are **not evidence**: they assert the absence of a report (the frame's case needs only the `living` hunk
  reversed to red [inference]). The row's red is `:59`.
- Promotion `20261124T043400-ledger-promote-oitndrch`: `fix_patch_sha256 = 7ca5ea37…`, the inside-a-block
  ref `FAIL`, the other three green, restore green.
