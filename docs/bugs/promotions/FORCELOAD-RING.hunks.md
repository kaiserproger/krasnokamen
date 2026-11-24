# FORCELOAD-RING: patch-mode fix identity

Original fix: `5d86de847` (`fix(core): forceload ring ticks blocks but not entities`) — a level-31 forced ticket
ticks entities only in its centre chunk; the level-32 ring ticks blocks but not entities, matching vanilla
`ChunkMap` ticket levels. The fix set `ENTITY_RADIUS` to 0 and rewrote the two doc blocks that claimed the ring
simulates entities.

The committed fix no longer reverses on HEAD (the module's doc heading and the `Boundary`/`command_args` imports
were rewritten by later commits), so the row is a scoped patch:
`fix_commit = "patch:d512890b4d8300361ce7ad940766621c143bb9ced864837c2a9da1c646e2eb6b"` in
`docs/bugs/promotions/FORCELOAD-RING.patch`. Both a/-sides are the real pre-image: the three-line doc block and
the comment plus `pub(crate) const ENTITY_RADIUS: i32 = 1;` as they stood before `5d86de847`. Re-audit:
`python3 -m tools.harness ledger promote FORCELOAD-RING --fix-patch docs/bugs/promotions/FORCELOAD-RING.patch --dry-run`.

## Carried

- `crates/mc-core/src/forceload.rs` `@@ -8,9 +8,8 @@` — the module doc block: the pre-image "loads the chunk
  plus a two-chunk ring and simulates the chunk with its one-chunk ring" becomes "ticks entities only in the
  centre, blocks through distance one, and loads chunks through distance two".
- `crates/mc-core/src/forceload.rs` `@@ -25,11 +24,10 @@` — `ENTITY_RADIUS` 1 -> 0 with the two doc comments
  above `BLOCK_RADIUS`/`ENTITY_RADIUS`.

## Not carried

- Nothing else: `5d86de847`'s non-test hunks are exactly these two files' doc/const changes. `LOAD_RADIUS` (2)
  and `BLOCK_RADIUS` (1) are unchanged by the fix and stay in both trees.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:forceload::tests::forced_neighbour_ticks_blocks_but_freezes_entity_motion_and_age` — with the
  patch reversed the ring chunk simulates the entity again, so the test fails at its own
  `assert_eq!(state.entities.pos(neighbour_slot), before)` / `assert_eq!(state.entities.row(neighbour_slot).age,
  age)` (`crates/mc-core/src/forceload_tests.rs:303-304`), while `assert!(state.world.is_block_active(11, 10),
  "level 32 still ticks blocks")` at `:305` stays green: the block half is untouched, the entity half is the
  row's symptom. The recorded reversal control is in the row's `repro_check` (`control-A.log`).
- **No real-client ref is possible.** The symptom lives in the ring of a *remote* forced chunk: with a player
  present the ring is normally simulated anyway (masking the bug), and with no player there the client neither
  loads the chunk nor sees its entities, so no graphical scenario can distinguish the two trees. The row is
  `repro = manual`; promotion needs Main's decision on whether the cargo ref through the real tick path is
  enough for a symptom no client can observe.
