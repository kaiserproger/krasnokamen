# DATA-QUERY — control map for `patch:d41098ff…`

Row: generic entity/block `data get` paths/scales were unavailable; query errors must not be read
as empty custody (`docs/bugs/rows/DATA-QUERY.toml`). Real commit: **`0c6f68d84`**
(`fix(commands): preserve native data queries and execute store outcomes`).

## Carried hunk (HEAD equivalent)

| original hunk | HEAD | class |
|---|---|---|
| `crates/mc-core/src/datapacks.rs` `command`: `+ return crate::command_data::command(state, connection, executor, text);` replacing the pre-fix `crate::command_blocks::data(...)` call | identical line at HEAD (the surrounding `-> bool` signature became `CommandResult` in a later refactor) | (c) later rewrite of the enclosing signature only; the fix's own line is carried verbatim |

## Omitted hunks

| hunk | class | why |
|---|---|---|
| `crates/mc-core/src/command_data.rs` (added), `crates/mc-core/src/command_nbt_path.rs` (added), the `mod` lines in `lib.rs` | (b) | the kept refs (`data_player_queries_read_live_inventory_vitals_and_absent_paths`, `data_block_accepts_every_registered_block_entity_kind`, and the new dispatch test's helpers) call these modules, so reversing their creation stops the control compiling. They stay compiled and imported, unreferenced by the reversed dispatch — the accepted kept-helper class (README:250-256) |
| `crates/mc-core/src/command_blocks.rs`, `command_execution.rs` (spotting an error as `Failed` instead of an empty success, `supports_store_result`) | (a) | `EXECUTE-STORE`'s behaviour: reversing them would turn that row's assertion red instead of this one |
| `crates/mc-core/src/command_context.rs` | (a) | routing shared with `function`/`schedule`/`datapack`; the `data` arm there predates the fix (`0416a3d4c`) and is not what the row lacked |
| `crates/mc-core/src/command_data_tests.rs` (the fix's own tests) | kept | test file: the control keeps every test, only production hunks reverse |

## Proving refs

* `cargo:mc-core:command_data::tests::data_queries_answer_through_the_native_command_pipeline` —
  **new**, added by this pack: it enters through `command_context::run` (the console's own path) and
  asserts the console was given the answer (`commands.data.entity.query` with `13.5f`, and one refusal
  line for an absent target). **Red at `command_data_tests.rs:2107`** (`shown.len() == 1`, left 0) with
  the dispatch removed.
* `harness:entities:native-v2-data-query` — assertion `data_get_entity_health_20`, now read in bounded
  steps and asserted (`native_promotion_wave1._data_query`) instead of a `steps.wait`, because a wait
  timeout is unrunnable as a ledger control.
* `cargo:mc-core:command_data::tests::data_player_queries_read_live_inventory_vitals_and_absent_paths`
  and `…data_block_accepts_every_registered_block_entity_kind` are **not** proving: they call
  `query`/`entity`/`block` directly, so they answer with or without the dispatch (measured:
  `20261123T212911-test-7u1iawnh`); they are green on the control and excluded from `proving_refs`
  (README:1330-1346).
