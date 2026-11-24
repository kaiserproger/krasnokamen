# DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY: patch-mode fix identity

Original fix: the residency guard added to the dragon-egg target search by this lane (`b460f313c`,
`fix(residency): address the control review — mining frame, uniform menu guards, egg unit guard`). Vanilla
draws its teleport target inside a +-15 horizontal / +-7 vertical frame; `block_at` answers from the cold
overlay for a non-resident chunk, so without the guard the egg writes itself into a chunk that is not in
memory. `fix_commit = "patch:2015e4cf380b0bfd…"` names the HEAD equivalent in
`docs/bugs/promotions/DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY.patch` (a diff from the pre-fix
`plugin_player::allows_block`-only condition to the candidate; `git apply -R` restores it). Re-audit:
`python3 -m tools.harness ledger promote DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY --fix-patch docs/bugs/promotions/DRAGON-EGG-TELEPORTS-INTO-COLD-CHUNK-OVERLAY.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `vanilla_interact.rs @@ -123,7 +123,15 @@ fn dragon_egg` — the `has_chunk(target)` + resident-air
  terms ahead of `plugin_player::allows_block`; HEAD `crates/mc-core/src/vanilla_interact.rs:126-133`.
  The a/-side is the same condition without the two residency terms (the pre-fix code), not an invented
  line.

## Original non-test hunks (complete)

```
## b460f313c: only the dragon-egg guard belongs to this row
- crates/mc-core/src/vanilla_interact.rs: -123,7 +123,15 (has_chunk + resident-air before allows_block)
```

## Not carried

- The rest of `b460f313c` (mining frame, the three menu guards, stations comment) — (a) MINING,
  ENDER-CHEST-MENU-CLICK-THROUGH-COLD-OVERLAY and ANVIL-DAMAGE-WRITES-AT-COLD-MENU-COORDINATE carry them.
- `crates/mc-core/src/interact_parity_tests.rs` — (b) kept; the cold-overlay test is this row's control.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:interact_parity_tests::interact_parity_dragon_egg_never_teleports_into_a_cold_overlay`
  (`crates/mc-core/src/interact_parity_tests.rs:602`) — with the guard reversed the egg moves into the
  cold overlay: `assert_eq!(state.world.block_at(pos…), Some(egg), "the egg must not move into a
  non-resident chunk's cold overlay")` (`:671-675`) fails, and
  `assert!(state.merge_block_writes.is_empty(), "no block write is staged for a cold teleport target")`
  (`:676-679`) fails after it.
- Client ref `harness:entities:native-v2-ledger-dragon-egg` green receipt on the pinned candidate
  `b8122ab6e8` (pin `b8122ab6e8e5-server-x86_64-unknown-linux-gnu-incr`):
  `.analysis/validation/20261124T010528-native-v2-ledger-dragon-egg-fimi0tkj`, repeated
  `20261124T010621-…-glrm6885` and `20261124T010708-…-ow55yp3v` — `egg_left_original_cell`,
  `egg_teleported_exactly_once`, `egg_not_consumed` all true. The gate needed its own fix (11x19 pad,
  gravity, 16^3 scan boxes returned empty); see commit `e560a87189` and the scenario's `limits`.
