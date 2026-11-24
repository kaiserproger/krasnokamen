# SPONGE: patch-mode fix identity

Original fix: `35308579c` (`fix(interact): rebase onto 7fe49052 — axe strip/scrape/unwax, sponge,
brushing, painting, spawn protection, dragon egg, special storage`) — it added the `sponge` absorption
pass and the tick hook that runs it after block writes. `fix_commit = "patch:057a21dcc61f608c…"` names
the HEAD equivalent of the tick hook in `docs/bugs/promotions/SPONGE.patch` (a diff from the pre-fix form
to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SPONGE --fix-patch docs/bugs/promotions/SPONGE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- Original hunk `tick.rs @@ -2881,6 +2888,7 @@` — the `crate::sponge::after_blocks(state,
  &mut block_writes, &mut popped_drops);` line inserted in `run_dimension_tick_inner` right after
  `crate::portals::nether::after_blocks(...)`. At HEAD that function moved to
  `crates/mc-core/src/tick/dimension.rs`, so the HEAD equivalent is that call plus its `block_laps.lap("sponge")`
  line at `crates/mc-core/src/tick/dimension.rs:378-379`. Reversing them removes the pass from the tick, so
  a sponge placed by a real `UseItemOn` never absorbs (the a/-side is the surrounding prior hook list,
  `nether::after_blocks` → `fluids::schedule`, unchanged HEAD lines).

## Original non-test hunks (complete)

```
## 35308579c non-test hunks (kept test files: interact_parity_tests.rs, interact_failing_first_tests.rs and the other *_tests.rs)
- crates/mc-core/src/boundary.rs: -55,6 +55,8; -342,6 +344,7; -613,6 +616,8
- crates/mc-core/src/brushing.rs: -0,0 +1,132
- crates/mc-core/src/config.rs: -47,6 +47,8; -105,6 +107,7
- crates/mc-core/src/core.rs: -541,6 +541,7; -1568,6 +1569,7; -1685,6 +1687,12; -2989,6 +2997,7
- crates/mc-core/src/dimensions_recovery.rs: -38,6 +38,9; -184,6 +187,7
- crates/mc-core/src/entity_loot.rs: -654,6 +654,9
- crates/mc-core/src/fire.rs: -174,7 +174,11
- crates/mc-core/src/journal.rs: -1099,6 +1099,8; -1144,6 +1146,8
- crates/mc-core/src/lib.rs: -354,3 +354,15
- crates/mc-core/src/painting.rs: -0,0 +1,178
- crates/mc-core/src/player.rs: -154,6 +154,14; -956,6 +964,10; -1060,7 +1072,9
- crates/mc-core/src/redstone/containers.rs: -113,6 +113,7; -410,6 +411,9
- crates/mc-core/src/registry.rs: -133,6 +133,7; -512,6 +513,7
- crates/mc-core/src/spawn_protection.rs: -0,0 +1,15
- crates/mc-core/src/special_storage.rs: -0,0 +1,136
- crates/mc-core/src/sponge.rs: -0,0 +1,60
- crates/mc-core/src/tick.rs: -341,6 +341,9; -614,6 +617,9; -2795,6 +2801,7; -2881,6 +2888,7; -2909,6 +2917,7; -3664,6 +3673,7; -3676,6 +3686,9; -3688,6 +3701,7; -3716,7 +3730,7; -4002,7 +4016,7; -4482,6 +4496,7; -4851,6 +4866,9
- crates/mc-core/src/tick/auth.rs: -13,6 +13,7
- crates/mc-core/src/tick/input/combat.rs: -40,6 +40,7
- crates/mc-core/src/tick/input/interact.rs: -60,7 +60,8; -78,6 +79,12; -166,6 +173,8; -178,6 +187,9; -310,6 +322,11; -586,6 +603,7
- crates/mc-core/src/vanilla_interact.rs: -0,0 +1,70
- crates/mc-edge/src/admin.rs: -151,6 +151,10
- crates/mc-edge/src/config.rs: -187,6 +187,9; -207,6 +210,7; -586,6 +590,9; -733,6 +740,7
- crates/mc-edge/src/plan.rs: -1917,6 +1917,21
- crates/mc-edge/src/server.rs: -362,7 +362,10; -920,7 +923,15; -2886,7 +2897,7
- crates/mc-edge/src/server_properties.rs: -35,6 +35,7; -68,3 +68,13
- crates/mc-protocol/src/packets/play/entity.rs: -699,6 +699,8; -814,6 +816,7; -916,6 +919,7; -945,6 +949,7; -978,6 +983,11; -1207,6 +1217,13; -1474,3 +1491,7
- crates/mc-protocol/src/packets/play/entity.rs: -0,0 +1,16 (new submodule)
- example.toml: -10,6 +10,8
```

## Not carried

- `sponge.rs @@ -0,0 +1,60 @@` — (b) the kept `interact_parity_sponge_capacity_and_radius` test calls
  `crate::sponge::after_blocks` directly (`interact_parity_tests.rs:561`, `:580`), so the module must stay
  for the control to compile. The row's registered ref
  (`interact_parity_sponge_absorbs_adjacent_water`) drives the real `UseItemOn` path, which is exactly
  what the reversed tick hook disables.
- Every other file — (a) the rebase's other rows (axe, brushing, dragon egg, painting, spawn protection,
  special storage, custody, edge/config plumbing).
- `example.toml`/`docs/QUIRK_PARITY.md` — (b) docs/config text; test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:interact_parity_tests::interact_parity_sponge_absorbs_adjacent_water`
  (`crates/mc-core/src/interact_parity_tests.rs:57`) — `use_on` at `:62` places the sponge but no pass
  runs, so `:63-66` `assert_eq!(state.world.block_at(1, Y + 1, 0), Some(block(&state, "wet_sponge", &[])))`
  fails (the water cell stays water and the sponge stays dry).
- Client ref `harness:entities:native-v2-ledger-sponge`: no receipt is on file for this candidate; a fresh
  real-client run is required for promotion (see HANDOFF-6).
