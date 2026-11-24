# SUMMON-NBT-FUSE-BLOCKSTATE: patch-mode fix identity

Original fix: `3be486570b` (`wip(stage0): LedgerRowsCommands uncommitted work at the 21:00 stop`) —
`/summon` learned the vanilla `fuse`, `explosion_power`, `block_state`/`BlockState` and `Time` fields for
TNT and falling blocks, plus the per-entity admission, the typed normalisation and the authored-payload
application. `b2b442ded6` later rewrote only the `explosion_power` line (single-line
`clamp(0.0, 128.0)`), and `69650602b` rebased the surrounding module, so the fix commit's diff no longer
applies on HEAD.

`fix_commit = "patch:13b3dcfa11aee8d3…"` names the HEAD equivalent of this row's non-test hunks in
`docs/bugs/promotions/SUMMON-NBT-FUSE-BLOCKSTATE.patch` (a diff from the pre-fix form to the candidate;
`git apply -R` restores the pre-fix code — verified `--check` at HEAD). Re-audit:
`python3 -m tools.harness ledger promote SUMMON-NBT-FUSE-BLOCKSTATE --fix-patch docs/bugs/promotions/SUMMON-NBT-FUSE-BLOCKSTATE.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_summon.rs @@ -234,6 +234,8 @@ supports` — the TNT/falling-block field admission
  (`"fuse" | "explosion_power" | "block_state" => name == "minecraft:tnt"`,
  `"BlockState" | "Time" => name == "minecraft:falling_block"`). HEAD
  `crates/mc-core/src/command_summon.rs:375-376`.
- `command_summon.rs @@ -528,6 +530,23 @@ authored_fields` — the typed arms (`fuse` → clamped `Short`,
  `explosion_power` → clamped `Float` as rewritten by `b2b442ded6`, `Time` → validated nonnegative `Int`,
  `block_state`/`BlockState` → compound). HEAD `:676-690`.
- `command_summon.rs @@ -559,6 +578,15 @@ prepare` — the vanilla falling-block default `BlockState`
  (`minecraft:sand`) when the summon omits it. HEAD `:722-735`.
- `command_summon.rs @@ -842,6 +870,22 @@ apply` — the authored primed-TNT/falling-block payload
  replaces the spawn defaults, then `tnt::restore` and `falling::configure` run for the authored state.
  HEAD `:1025-1041`.

The a/-side is the exact HEAD pre-image (each carried region deleted); there is **no** a/-side
adaptation. The `-R` and forward checks were both run: the patch reverses on HEAD and does not apply
forward there.

## Original non-test hunks (complete)

```
## 3be486570b non-test hunks (kept test file: crates/mc-core/src/command_summon_tests.rs)
- crates/mc-core/src/command_summon.rs: -234,6 +234,8; -528,6 +530,23; -559,6 +578,15; -842,6 +870,22 (the last includes tnt::restore and falling::configure)
- tools/harness/backends/native_v2.py: the native-v2 summon-nbt scenario body (+41)
- tools/harness/backends/client_probes.py: the entity-row probe helper (+2)
- tools/harness/scenarios.py: the scenario registration (+8)
```

## Not carried

- `crates/mc-core/src/command_summon_tests.rs` — (b) kept: the row's three cargo refs live there.
- `tools/harness/backends/native_v2.py`, `client_probes.py`, `scenarios.py` — (b) the registered gate
  (`harness:entities:native-v2-summon-nbt`) the row's real-client ref runs; reversing it would delete the
  scenario instead of failing at its assertion. These files are gate code, not product behaviour.

## Proving refs (file:line of the failing assertion)

- `harness:entities:native-v2-summon-nbt` — with the patch reversed the `fuse` summon is refused
  (`Unknown or unsupported summon NBT field fuse`), the scenario settles on that refusal and reds
  `tnt_summon_accepts_fuse` (`tools/harness/backends/native_v2.py`, `_ledger_summon_nbt`). The candidate
  run must be `passed` and carry this tree's fingerprint.
- `cargo:mc-core:command_summon::tests::summon_tnt_accepts_fuse_and_explosion_power_and_applies_them` and
  `cargo:mc-core:command_summon::tests::summon_falling_block_accepts_block_state_and_time_and_defaults_to_sand`
  (`crates/mc-core/src/command_summon_tests.rs`) — the same refusal reds them in the control.
- `cargo:mc-core:command_summon::tests::summon_rejects_tnt_and_falling_block_fields_on_wrong_entities_and_bad_values`
  stays green with the patch reversed (it asserts refusals) and is **not evidence**.
