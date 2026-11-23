# FALL-002: patch-mode fix identity

Original fix: `35308579c` (`fix(interact): rebase onto 7fe49052 — axe strip/scrape/unwax, sponge,
brushing, painting, spawn protection, dragon egg, special storage`) — it added the `brushing` handler and
its two dispatch terms (`use_item_on` and `use_item`). The handler module is new in that commit, so the
row's HEAD pre-image is the dispatch: before the fix the brush use was never reached. `fix_commit =
"patch:414ec86d1f67efd5…"` names the HEAD equivalents in
`docs/bugs/promotions/FALL-002.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote FALL-002 --fix-patch docs/bugs/promotions/FALL-002.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/input/interact.rs @@ -310,6 +322,11 @@` — the `|| crate::brushing::use_on(...)` term in
  `use_item_on`; HEAD `crates/mc-core/src/tick/input/interact.rs:330-332` (a/-side: the prior
  axe-only branch, real code).
- `tick/input/interact.rs @@ -586,6 +603,7 @@` — the same term in `use_item`; HEAD
  `crates/mc-core/src/tick/input/interact.rs:625-627`.

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

- `brushing.rs @@ -0,0 +1,132 @@` — (b) the kept tests assert the brush deposit at the block-entity
  level and `crate::brushing::run` is ticked from `tick.rs` (`-4482,6 +4496,7`); the module must stay for
  the control to compile. Reversing the two dispatch terms reproduces the row's symptom.
- Every other file — (a) the rebase's other rows (axe, dragon egg, painting, spawn protection, special
  storage, sponge, chest/lectern custody, edge/config plumbing).
- `example.toml`/`docs/QUIRK_PARITY.md` — (b) docs/config text.
- Test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:interact_parity_tests::interact_parity_brushing_completes_once`
  (`crates/mc-core/src/interact_parity_tests.rs:360`) — with both terms gone the brush use never starts,
  so the loop at `:414-470` observes no `BlockEntityData` reveal and the final
  `assert_eq!(state.world.block_at(POS...), Some(block(&state, result, &[])))` (the source→result pair,
  `suspicious_sand`→`sand`) fails.
- Client ref `harness:entities:native-v2-ledger-brush` green receipt on the gate-wave tree:
  `.analysis/validation/20261123T192307-native-v2-ledger-brush-czmbmk7s/result.json`; a fresh receipt on
  this candidate is required for promotion (see HANDOFF-6).
