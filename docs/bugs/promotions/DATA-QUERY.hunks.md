# DATA-QUERY: patch-mode fix identity

Original fix: `0c6f68d84` (`fix(commands): preserve native data queries and execute store outcomes`,
integrated as `5c060d5df` per BUGS.md B06) — it created `crates/mc-core/src/command_data.rs` (generic
entity/block `data get` paths, scales and error custody) and routed the `data` root to it. The module is
new in that commit and the pre-fix partial handler (`command_blocks::data`) no longer exists at HEAD, so
the row's HEAD pre-image is the routing: `data` was not handed to a data command at all.
`fix_commit = "patch:b23d0e101f9877ac…"` names the HEAD equivalent of the routing hunk in
`docs/bugs/promotions/DATA-QUERY.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote DATA-QUERY --fix-patch docs/bugs/promotions/DATA-QUERY.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_context.rs @@ -309,16 +311,29 @@ pub(crate) fn native` — the
  `if matches!(root, "function" | "schedule" | "reload" | "datapack" | "data")` root list and the
  `require_result` exception list; HEAD `crates/mc-core/src/command_context.rs:820-823`. Reversing it drops
  `data` from both lists (the a/-side is the prior list text, real code), so `/data` falls through to
  `commands::parse` and is refused as unknown. 838909abc later rewrote the list's formatting (c), the
  HEAD text is what the patch carries.

## Original non-test hunks (complete)

```
## 0c6f68d84 non-test hunks (kept test files: command_data_tests.rs of the commit)
- crates/mc-core/src/command_blocks.rs: -242,20 +242,20; -283,9 +283,6; -303,6 +300,14; -315,12 +320,12
- crates/mc-core/src/command_context.rs: -212,9 +212,9; -227,7 +227,7; -264,7 +264,9; -272,7 +274,7; -281,7 +283,7; -309,16 +311,29; -337,6 +352,9; -352,9 +370,9
- crates/mc-core/src/command_data.rs: -0,0 +1,335
- crates/mc-core/src/command_execution.rs: -7,6 +7,42; -356,7 +392,7; -371,7 +407,7; -384,7 +420,7; -394,7 +430,7; -430,11 +466,11; -446,15 +482,15; -470,12 +506,13
- crates/mc-core/src/command_nbt_path.rs: -0,0 +1,351
- crates/mc-core/src/command_store.rs: -0,0 +1,79
- crates/mc-core/src/command_world.rs: -346,24 +346,28; -432,9 +436,12; -458,9 +465,9; -472,7 +479,7; -570,7 +577,10; -653,7 +663,7
- crates/mc-core/src/core.rs: -2834,6 +2834,20; -2998,6 +3012,17
- crates/mc-core/src/datapacks.rs: -496,16 +496,16; -552,11 +552,14
- crates/mc-core/src/dimensions_commands.rs: -205,7 +205,8; -280,21 +281,23; -302,13 +305,14; -318,10 +322,17
- crates/mc-core/src/dimensions_travel.rs: -109,7 +109,8; -133,7 +134,7; -153,7 +154,7; -406,7 +407,7; -523,7 +524,9
- crates/mc-core/src/effects.rs: -669,12 +669,13; -727,6 +728,11
- crates/mc-core/src/journal.rs: -1174,6 +1174,22; -1247,16 +1263,18; -1280,21 +1298,23; -1314,7 +1334,7; -1331,34 +1351,36
- crates/mc-core/src/jukebox.rs: -99,6 +99,13; -108,9 +115,7
- crates/mc-core/src/lib.rs: -46,12 +46,15
- crates/mc-core/src/scoreboard.rs: -1,5 +1,6; -197,6 +198,13; -349,6 +357,27; -397,18 +426,24; -430,32 +465,35; -578,15 +616,18; -666,7 +707,7; -720,35 +761,38
- crates/mc-world/src/anvil/chunk_nbt.rs: -1084,6 +1084,45
```

## Not carried

- `command_data.rs`, `command_nbt_path.rs` (`-0,0 +1,335` and `+1,351`) — (b) the kept
  `command_data_tests.rs` calls `data get` through `command_context::native` after the routing hunk; the
  modules must stay for the control to compile. (The earlier promotion's patch reversed lines inside
  `command_data.rs` with a synthetic a/-side; that is withdrawn and replaced by this routing control.)
- `command_store.rs` and the `command_execution.rs` hunks — (a)/(b) EXECUTE-STORE's row and the
  `CommandResult` API (carried by that row's control).
- `command_blocks.rs`, `command_context.rs -212,9/-227,7/-264,7/-272,7/-281,7/-337,6/-352,9`,
  `command_world.rs`, `core.rs`, `datapacks.rs`, `dimensions_commands.rs`, `dimensions_travel.rs`,
  `effects.rs`, `journal.rs`, `jukebox.rs`, `scoreboard.rs`, `mc-world/anvil/chunk_nbt.rs` — (a) the same
  commit's other rows (command dispatch/arg plumbing, journal/checkpoint results, datapack commands).
- Test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:command_data::tests::data_player_queries_read_live_inventory_vitals_and_absent_paths`
  (`crates/mc-core/src/command_data_tests.rs:69`) and
  `cargo:mc-core:command_data::tests::data_block_accepts_every_registered_block_entity_kind` (`:169`) —
  their command helper parses the text through `command_context::native`; with `data` unrouted it returns
  the `Unknown command` error, so the helper's own assertion on the parse result fails (the review's
  observed red for the earlier control was the same "path/scale queries refused" class).
- Client ref `harness:entities:native-v2-data-query` green receipt on the gate-wave tree:
  `.analysis/validation/20261123T163228-native-v2-data-query-us7osww1/result.json`; a fresh receipt on this
  candidate is required for promotion (see HANDOFF-6).
