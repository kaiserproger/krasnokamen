# CONTAINERS-003: patch-mode fix identity

Original fix: `2ccf8cc1a` (`fix(core): restore mount lectern and command container custody`). Its
`container_kind_for_state` hunk changed the crafting table's block backing size from `10` to `0`
(`tick.rs @@ -4194,9 +4204,11 @@` at the time), so the grid/result custody belongs to the player's
`Menu` instead of a ten-slot block backing. With the ten-slot backing the click shape guard
(`crates/mc-core/src/inventory/click.rs:777`, `kind.container_size()`) treats the grid clicks as
outside the backing storage and closes the crafting menu on the first pickup.

The whole commit does not reverse on HEAD (it also creates `lectern.rs`, `mounts/menus.rs`,
`command_block_entity.rs` and touches the redstone, world and protocol paths for other rows), so the
row moves to a scoped patch: `fix_commit = "patch:0d9f12319f244b46f7f9c31286aaea89f088ad03b40bb0fb13721ec1d177d00c"`
names the HEAD equivalent of the crafting-table hunk only, in
`docs/bugs/promotions/CONTAINERS-003.patch` (a diff from the pre-fix form `- 10,` to the candidate's
comment + `0,`; the a/-side is the real pre-image line of `2ccf8cc1a^`, no adaptation).
Re-audit:
`python3 -m tools.harness ledger promote CONTAINERS-003 --fix-patch docs/bugs/promotions/CONTAINERS-003.patch --dry-run`.

Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's
or owner's behaviour (a), API the kept tests call (b), or code a later commit rewrote (c).

## Carried

- `tick.rs @@ -4194,9 +4204,11 @@ fn container_kind_for_state` — the crafting table's backing size
  `10` -> `0` with its comment. The file was split into owner child modules by
  `db1dc0403` `refactor(core): ... split god files into owner child modules`, so its HEAD equivalent is
  `crates/mc-core/src/tick/containers.rs:462-467`, four lines identical apart from the changed size and
  the added comment. Nothing else of the hunk is kept.

## Original non-test hunks (complete)

```
## 2ccf8cc1a non-test hunks
# test files (b, kept so the control compiles and runs the refs)
- crates/mc-core/src/books_tests.rs: -136,16 +136,34; -153,7 +171,45
- crates/mc-core/src/mounts_tests.rs: -114,9 +114,106
- crates/mc-core/src/plugin_inventories_tests.rs: -221,6 +221,142
- crates/mc-core/src/tick_tests.rs: -5662,6 +5662,63
- crates/mc-edge/src/decode_tests.rs: -1057,6 +1057,15
- crates/mc-protocol/src/packets/play/tests/serverbound_and_slots.rs: -727,6 +727,24
# production
- crates/mc-core/src/boundary.rs: -285,6 +285,8; -387,6 +389,7; -898,6 +901,11
- crates/mc-core/src/command_block_entity.rs: -0,0 +1,119 (new file)
- crates/mc-core/src/command_world.rs: -12,6 +12,7; -59,9 +60,17; -98,6 +107,7; -373,12 +383,15;
  -400,11 +413,18; -471,6 +491,7; -529,7 +550,7; -556,6 +577,16; -749,6 +780,7; -768,7 +800,7;
  -852,19 +884,28
- crates/mc-core/src/inventory.rs: -210,6 +210,15; -218,6 +227,13; -232,7 +248,7; -242,6 +258,7;
  -268,6 +285,8; -296,6 +315,8; -311,6 +332,8; -328,6 +351,7; -378,6 +402,7
- crates/mc-core/src/inventory/click.rs: -345,6 +345,24; -397,6 +415,7; -680,6 +699,22
- crates/mc-core/src/journal.rs: -2284,6 +2284,11
- crates/mc-core/src/lectern.rs: -0,0 +1,342 (new file)
- crates/mc-core/src/lib.rs: -41,6 +41,7; -116,6 +117,7
- crates/mc-core/src/monster_spawner.rs: -16,7 +16,7; -40,7 +40,10; -159,6 +162,17
- crates/mc-core/src/mounts.rs: -7,6 +7,7; -18,6 +19,10; -30,6 +35,8; -113,6 +120,23;
  -272,7 +296,7; -303,6 +327,32; -405,11 +455,34
- crates/mc-core/src/mounts/menus.rs: -0,0 +1,223 (new file)
- crates/mc-core/src/mounts/persistence.rs: -124,6 +124,42
- crates/mc-core/src/player.rs: -500,9 +500,9; -515,10 +515,19; -546,7 +555,7
- crates/mc-core/src/redstone/mod.rs: -105,6 +105,7; -134,6 +135,13; -661,6 +669,11
- crates/mc-core/src/redstone/signal.rs: -64,9 +64,11; -148,6 +150,7
- crates/mc-core/src/tick.rs: -393,6 +393,7; -487,6 +488,7; -695,9 +697,11; -1704,6 +1708,7;
  -1719,6 +1724,10; -2246,6 +2255,8; -2255,8 +2266,6; -2797,6 +2806,7; -4194,9 +4204,11 (carried);
  -4485,6 +4497,12; -4549,7 +4567,7; -5058,6 +5076,21
- crates/mc-core/src/tick/input/interact.rs: -181,6 +181,9
- crates/mc-core/src/tick/input/inventory.rs: -157,6 +157,12; -420,6 +426,9
- crates/mc-core/src/vehicles/containers.rs: -131,17 +131,33
- crates/mc-core/src/world.rs: -1421,6 +1421,26; -1440,6 +1460,13; -1460,13 +1487,6;
  -1511,6 +1531,11; -1531,11 +1556,6; -1626,6 +1646,22; -1646,22 +1682,6
- crates/mc-edge/src/decode.rs: -339,6 +339,9
- crates/mc-edge/src/plan/player_changes.rs: -529,6 +529,19
- crates/mc-protocol/src/packets/play.rs: -129,13 +129,13
- crates/mc-protocol/src/packets/play/inventory.rs: -241,6 +241,33
- crates/mc-world/src/anvil/chunk_nbt.rs: -1370,7 +1370,7
- docs/OPERATING.md: -788,6 +788,23
```

## Not carried

- `lectern.rs` (new), `tick.rs -4485,6 +4497,12` (lectern chunk hydration),
  `tick/input/interact.rs -181,6 +181,9` and `tick/input/inventory.rs -420,6 +426,9` — (a)
  CONTAINERS-002 (lectern hold/open/page/take menu); its own control carries these lines.
- `mounts.rs`, `mounts/menus.rs` (new), `mounts/persistence.rs`, `tick.rs -2797,6 +2806,7` (mount menu
  maintain), `tick/input/inventory.rs -157,6 +157,12`, `inventory.rs -296,6 +315,8` and `-311,6 +332,8`,
  `boundary.rs -898,6 +901,11` — (a) CONTAINERS-001 (chested donkey/mule cargo menu) and HS-2 (hopper
  into a container vehicle); their own controls carry these lines.
- `command_block_entity.rs` (new), `command_world.rs` (all hunks), `tick.rs -393,6 +393,7`,
  `-1719,6 +1724,10`, `-5058,6 +5076,21`, `journal.rs`, `mc-edge/src/decode.rs`,
  `mc-edge/src/plan/player_changes.rs`, `mc-protocol/src/packets/play/inventory.rs` — (a) the
  command-block custody rows (`COMMAND-BLOCK-SETBLOCK-AUTO-NEVER-RUNS`,
  `COMMAND-BLOCK-DATA-SUCCESSCOUNT-REFUSED`, `COMMAND-CHILD-SENDER-DROPPED` and the block-entity sync
  rows); each has its own row and control.
- `redstone/mod.rs`, `redstone/signal.rs`, `tick.rs -2246,6 +2255,8`, `-2255,8 +2266,6` — (a) the
  redstone rows (REDSTONE-DAYLIGHT and the forced-chunk cell work); their own controls carry these.
- `monster_spawner.rs`, `world.rs` (all hunks), `tick.rs -1704,6 +1708,7` — (a) the spawner/chunk
  custody rows; (c) for the parts `b24f0ce35`/`db1dc0403` rewrote.
- `player.rs`, `inventory/click.rs`, `boundary.rs -285,6 +285,8` and `-387,6 +389,7`,
  `inventory.rs` (remaining hunks), `vehicles/containers.rs`, `mc-protocol/src/packets/play.rs`,
  `mc-world/src/anvil/chunk_nbt.rs`, `docs/OPERATING.md` — (a) menu/custody/dimension rows;
  (b) the `MenuKind`/`PlayerChange` API the kept tests call stays.
- Test files — (b) kept so the control compiles and runs the registered refs.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:tick_tests::predicted_table_use_waits_for_accepted_placement` — with the sizing hunk
  reversed the first grid pickup closes the menu and the ref fails at
  `crates/mc-core/src/tick_tests.rs:6337` `crafting table closed on click 37` (observed:
  `.analysis/validation/20261124T092927-test-6163a_zr/test.log`); restored green.
- `harness:entities:native-v2-ledger-crafting-pickup` — the real client opens the table, takes the
  plank stack, and asserts `crafting_menu_stays_open_after_pickup` with the same menu id and the carried
  stack (`tools/harness/backends/native_v2.py:254-281`). Passed receipt on the candidate:
  see the promotion JSON.
