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

The a/-side is the candidate with exactly those four guarded regions removed; there is **no** invented
code, counter or oracle on it, and no test-visible API line. `69650602b` rewrote the module around them,
so a historical blob (`3be486570b^:crates/mc-core/src/command_summon.rs`) is not byte-comparable; the
forward-port is per-hunk and the differences are only formatting from that rewrite (for example
`explosion_power`'s arm was a two-line body before `b2b442ded6` and is one line at HEAD). The oracle the
control reds on is the kept gate's own acceptance assertion over the real summoned entities; the pre-fix
tree refuses the fields in `supports()` and creates no body, which is the shape the patch restores. The
`-R` and forward checks were both run: the patch reverses on HEAD and does not apply forward there.

## Original non-test hunks (complete)

```
## 3be486570b non-test hunks (kept test file: crates/mc-core/src/command_summon_tests.rs)
- crates/mc-core/src/command_summon.rs: -234,6 +234,8; -528,6 +530,23; -559,6 +578,15; -842,6 +870,22 (the last includes tnt::restore and falling::configure)
- tools/harness/backends/native_v2.py: the native-v2 summon-nbt scenario body (+41)
- tools/harness/backends/client_probes.py: the entity-row probe helper (+2)
- tools/harness/scenarios.py: the scenario registration (+8)
```

## Not carried

- `crates/mc-core/src/command_summon_tests.rs` — (b) kept test API: the row's three cargo refs live there
  and call the summon path the patch guards.
- `tools/harness/backends/native_v2.py`, `client_probes.py`, `scenarios.py` — (b) kept gate API: the row's
  real-client ref `harness:entities:native-v2-summon-nbt` is the scenario these files define and the
  assertion the control reds lives in them; reversing them would delete the scenario instead of failing
  at its assertion. They are the kept gate, not product behaviour, and they run identically on both sides.

## Proving refs (file:line of the failing assertion)

- `harness:entities:native-v2-summon-nbt` — the gate settles on either the two bodies or the refusal line
  and asserts acceptance (`tnt_summon_accepts_fuse`, `falling_block_summon_accepts_block_state`,
  `tools/harness/backends/native_v2.py::_ledger_summon_nbt`), so a reversed server would red an
  **assertion**, not a wait. In the recorded control (`20261124T031554-ledger-promote-evc_j_f4`) this ref
  was **unrunnable**, not red: a promote controls a worktree without a release pin of its own and refuses
  to reuse the candidate's (`ledger-control-candidate/...ledger-builds/<sha>.json` missing). The red that
  carried the promotion came from the two cargo refs below; the client half is green on the candidate
  (`20261124T031405-native-v2-summon-nbt-9pbmsgbg`, tree `6fd3089969`).
- `cargo:mc-core:command_summon::tests::summon_tnt_accepts_fuse_and_explosion_power_and_applies_them` and
  `cargo:mc-core:command_summon::tests::summon_falling_block_accepts_block_state_and_time_and_defaults_to_sand`
  (`crates/mc-core/src/command_summon_tests.rs`) — the same refusal reds them in the control (observed
  `FAIL`).
- `cargo:mc-core:command_summon::tests::summon_rejects_tnt_and_falling_block_fields_on_wrong_entities_and_bad_values`
  stays green with the patch reversed (it asserts refusals) and is **not evidence**.
- The gate's fuse assertion is `500 < fuse <= 1000` because the fuse ticks down between the summon and the
  read (first candidate run read `801s`); it is timing-dependent, so the scene needs ≥3 runs before it is
  used as a red half (see `PROGRESS.md` run log).
