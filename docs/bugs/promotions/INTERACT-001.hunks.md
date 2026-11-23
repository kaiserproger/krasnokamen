# INTERACT-001: patch-mode fix identity

Original fix: `35308579c` (`fix(interact): rebase onto 7fe49052 — axe strip/scrape/unwax, sponge,
brushing, painting, spawn protection, dragon egg, special storage`) — it added the axe dispatch term in
`crates/mc-core/src/tick/input/interact.rs` and the `vanilla_interact::axe` handler. The handler module is
new in that commit, so the row's HEAD pre-image is the *call site*: before the fix the axe use was not
reached at all. `fix_commit = "patch:db1440f93672cc52…"` names the HEAD equivalent of that dispatch hunk
in `docs/bugs/promotions/INTERACT-001.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote INTERACT-001 --fix-patch docs/bugs/promotions/INTERACT-001.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `tick/input/interact.rs @@ -310,6 +322,11 @@` — the `crate::vanilla_interact::axe(...)` term in the
  `use_item_on` chain; HEAD `crates/mc-core/src/tick/input/interact.rs:330-332`. The whole five-line block
  was added by `35308579c` (`-310,6 +322,11`), and the patch reverses only this row's disjunct.
  **a/-side line: (a)-split.** The surviving `if crate::brushing::use_on(...) { return; }` half belongs to
  FALL-002's row — the same hunk installs both handlers, so a whole-block reverse would red the brush row
  instead; it never existed alone in this branch's history, and it is not a kept-API constructor.

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

- `vanilla_interact.rs @@ -0,0 +1,70 @@` — (b) the kept tests call `crate::vanilla_interact::dragon_egg`
  directly (`interact_parity_tests.rs:592`) and `tick/input/interact.rs` keeps the dragon-egg call sites
  the control compiles; the module must stay. Its `axe` function becomes unreferenced in the reversed
  tree.
- `brushing.rs`, `sponge.rs`, `painting.rs`, `spawn_protection.rs`, `special_storage.rs` (the other new
  modules) — (a) FALL-002, SPONGE, SPECIAL-001 and the painting/spawn rows; their controls reverse their
  own call sites/bodies.
- The same commit's four other `tick/input/interact.rs` hunks — `-60,7 +60,8`, `-78,6 +79,12`,
  `-166,6 +173,8`, `-178,6 +187,9` — (a) the dragon-egg and spawn-protection routing of the same rebase,
  carried by those rows/gates; they are not the axe dispatch.
- All remaining files — (a) the rebase's other rows (dragon egg, painting, spawn protection, special
  storage, chest/lectern custody, edge/config plumbing); (b) test files kept.
- `example.toml`/`docs/QUIRK_PARITY.md` — (b) docs/config text, never reversed.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:interact_parity_tests::interact_parity_axes_preserve_properties_and_wear`
  (`crates/mc-core/src/interact_parity_tests.rs:38`) — with the axe term gone
  `crates/mc-core/src/interact_parity_tests.rs:48-52`
  `assert_eq!(state.world.block_at(POS...), Some(block(&state, result, props)), "{source}")` fails on the
  first case (`oak_log` stays `oak_log`), and `:54` (wear `Some(3)`) fails too.
- Client ref `harness:entities:native-v2-ledger-axe` green receipt on the candidate `9c78eee8e`:
  `.analysis/validation/20261123T211938-native-v2-ledger-axe-146xzfla/result.json` (promotion record
  `docs/bugs/promotions/INTERACT-001.json`).
