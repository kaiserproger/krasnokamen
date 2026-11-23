# SETBLOCK-SNBT: patch-mode fix identity

Original fix: `2ccf8cc1a` (`fix(core): restore mount lectern and command container custody`) — it made
`block_spec` parse the `{…}` block-entity compound into network NBT instead of refusing it, added
`BlockSpec.nbt`, and fed it through `setblock`/`fill`. `fix_commit =
"patch:43dcf616a9d89ec544…"` names the HEAD equivalents of this row's non-test hunks in
`docs/bugs/promotions/SETBLOCK-SNBT.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SETBLOCK-SNBT --fix-patch docs/bugs/promotions/SETBLOCK-SNBT.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_world.rs @@ -59,9 +60,17 @@ pub(crate) fn block_spec` — the refusal
  `Block entity SNBT is not supported` is replaced by `command_snbt::parse` + compound check +
  `mc_nbt::write_network`; HEAD `crates/mc-core/src/command_world.rs:84-96`.
- `command_world.rs @@ -98,6 +107,7 @@ pub(crate) fn block_spec` — `nbt: None` → `nbt`; HEAD
  `crates/mc-core/src/command_world.rs:132`.

## Original non-test hunks (complete)

```
## 2ccf8cc1a non-test hunks (kept test files: the commit's *_tests.rs)
- crates/mc-core/src/boundary.rs: -285,6 +285,8; -387,6 +389,7; -898,6 +901,11
- crates/mc-core/src/command_block_entity.rs: -0,0 +1,119
- crates/mc-core/src/command_world.rs: -12,6 +12,7; -59,9 +60,17; -98,6 +107,7; -373,12 +383,15; -400,11 +413,18; -471,6 +491,7; -529,7 +550,7; -556,6 +577,16
- crates/mc-core/src/inventory.rs: -210,6 +210,15; -218,6 +227,13; -232,7 +248,7; -242,6 +258,7; -268,6 +285,8; -296,6 +315,8; -311,6 +332,8; -328,6 +351,7
- crates/mc-core/src/inventory/click.rs: -345,6 +345,24; -397,6 +415,7; -680,6 +699,22
- crates/mc-core/src/journal.rs: -2284,6 +2284,11
- crates/mc-core/src/lectern.rs: -0,0 +1,342
- crates/mc-core/src/lib.rs: -41,6 +41,7; -116,6 +117,7
- crates/mc-core/src/monster_spawner.rs: -16,7 +16,7; -40,7 +40,10; -159,6 +162,17
- crates/mc-core/src/mounts.rs: -7,6 +7,7; -18,6 +19,10; -30,6 +35,8; -113,6 +120,23; -272,7 +296,7; -303,6 +327,32; -405,11 +455,34
- crates/mc-core/src/mounts/menus.rs: -0,0 +1,223
- crates/mc-core/src/mounts/persistence.rs: -124,6 +124,42
- crates/mc-core/src/player.rs: -500,9 +500,9; -515,10 +515,19; -546,7 +555,7
- crates/mc-core/src/redstone/mod.rs: -105,6 +105,7; -134,6 +135,13; -661,6 +669,11
- crates/mc-core/src/redstone/signal.rs: -64,9 +64,11; -148,6 +150,7
- crates/mc-core/src/tick.rs: -393,6 +393,7; -487,6 +488,7; -695,9 +697,11; -1704,6 +1708,7; -1719,6 +1724,10; -2246,6 +2255,8; -2255,8 +2266,6; -2797,6 +2806,7
- crates/mc-core/src/tick/input/interact.rs: -181,6 +181,9
- crates/mc-core/src/tick/input/inventory.rs: -157,6 +157,12; -420,6 +426,9
- crates/mc-core/src/vehicles/containers.rs: -131,17 +131,33
- crates/mc-core/src/world.rs: -1421,6 +1421,26; -1440,6 +1460,13; -1460,13 +1487,6; -1511,6 +1531,11; -1531,11 +1556,6; -1626,6 +1646,22; -1646,22 +1682,6
- crates/mc-edge/src/decode.rs: -339,6 +339,9
- crates/mc-edge/src/plan/player_changes.rs: -529,6 +529,19
- crates/mc-protocol/src/packets/play.rs: -129,13 +129,13
- crates/mc-protocol/src/packets/play/inventory.rs: -241,6 +241,33
- crates/mc-world/src/anvil/chunk_nbt.rs: -1370,7 +1370,7
```

## Not carried

- `command_world.rs @@ -12,6 +12,7 @@` (`BlockSpec.nbt` field), `-373,12 +383,15` and `-400,11 +413,18`
  (setblock/fill `prepare` + image plumbing), `-471,6`, `-529,7`, `-556,6 +577,16` — (b) API the rest of
  the kept command path calls: HEAD `command_world.rs:464`, `:494`, `:499` (`filter.nbt`) and `:742`
  (`spec.nbt`) read the field, and `command_block_entity::prepare` is the image both `edit_region` calls
  take. Reversing the field alone stops the control from compiling; the row's symptom (the parser's
  refusal) is already reproduced by the two carried hunks, which reject the whole argument before any of
  those lines run.
- `command_block_entity.rs` (new module) — (b) the same call path (`prepare`) and the kept tests.
- All other files — (a) the mount/lectern/menu-custody rows of the same commit
  (CUSTODY-ENTITY-MENU-FLOOR, mount exports, lectern) and their tests.
- Test files — (b) kept.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:command_world::tests::setblock_snbt_installs_native_chest_contents_and_replaces_same_block`
  — `crates/mc-core/src/command_world.rs:1229` `.unwrap()` on `apply(...)`: with the parser reversed the
  command returns `Err("Block entity SNBT is not supported")` (the review's observed red at
  `command_world.rs:994`), so the test panics before `:1232` `assert_eq!(slots[0].count, count)`.
- Client ref `harness:entities:native-v2-ledger-setblock-snbt` is green on the gate-wave tree
  (`20261123T163519-native-v2-ledger-setblock-snbt-vmcadngj`); unrunnable in the control.
