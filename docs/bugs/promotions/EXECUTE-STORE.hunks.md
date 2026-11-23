# EXECUTE-STORE: patch-mode fix identity

Original fix: `0c6f68d84` (`fix(commands): preserve native data queries and execute store outcomes`) —
it introduced `CommandResult`, the `execute` return type, and `supports_store_result` with the native
mutators that produce a real result/success count. The candidate's `supports_store_result`
(`crates/mc-core/src/command_execution.rs:32`) already has the whole list except the two mutator arms
this row names, so the row's HEAD equivalent is that pair of lines. `fix_commit =
"patch:ddb8f835cbaeb31923…"` names them in `docs/bugs/promotions/EXECUTE-STORE.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote EXECUTE-STORE --fix-patch docs/bugs/promotions/EXECUTE-STORE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_execution.rs @@ -7,6 +7,42 @@` (the `supports_store_result` creation) — the carried HEAD
  equivalent is `| Command::Give { .. }` + `| Command::Clear { .. }`, HEAD
  `crates/mc-core/src/command_execution.rs:40-41`. The rest of that original hunk's list is already
  present at HEAD and is not a hunk of the patch (it was installed before this candidate's pre-fix image).

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

- `command_data.rs`, `command_nbt_path.rs`, `command_store.rs` (the new modules) — (b) the kept
  `command_data_tests.rs` calls `data`/`execute store` through `commands::parse` → `execute`, so the
  modules must stay for the control to compile; the row's symptom is reproduced by removing the two
  mutator arms from the whitelist (the store result is `Unavailable` and the mutator is rejected before
  mutation).
- The `CommandResult`/`execute` return-type hunks in `command_execution.rs` (`-356,7`, `-371,7`,
  `-384,7`, `-394,7`, `-430,11`, `-446,15`, `-470,12`) — (b) every kept command path returns
  `CommandResult`; reversing the signature stops the control from compiling.
- `command_blocks.rs`, `command_context.rs`, `command_world.rs`, `core.rs`, `datapacks.rs`,
  `dimensions_commands.rs`, `dimensions_travel.rs`, `effects.rs`, `journal.rs`, `jukebox.rs`,
  `scoreboard.rs`, `mc-world/anvil/chunk_nbt.rs` — (a) the same commit's other rows (native data queries'
  entity/block NBT paths, the journal/checkpoint result plumbing, datapack commands), all carried by
  their own rows' controls or out of this row's path.
- Test files — (b) kept.

## Proving ref (file:line of the failing assertion)

- `cargo:mc-core:command_data::tests::execute_store_give_and_clear_execute_once_and_return_real_counts`
  — `crates/mc-core/src/command_data_tests.rs:421`
  `assert_eq!(score(state, "result"), expected.unwrap_or(0), "{command}")`: with the two arms reversed
  `give`/`clear` report `0` and `success 0` (the review's observed red: "`give @s stone 7` reports 0
  (mutator refused)"), then `:448` `assert_eq!(count(&state), 7)` stays 0.
