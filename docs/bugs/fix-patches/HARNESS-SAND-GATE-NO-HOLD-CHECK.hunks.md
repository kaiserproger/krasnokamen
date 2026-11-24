# HARNESS-SAND-GATE-NO-HOLD-CHECK — patch map

Fix: `e1d92ae4e` "native-v2-input-sand: check that the frozen server holds the falling block"
(2026-11-12). The gate compared client prediction only, so it stayed green with the server's frozen
hold (`crates/mc-core/src/falling.rs`) reversed. The fix reads the server's falling-block `Pos` with
`data get entity` over successive server ticks and requires two consecutive reads to agree.

`653414533` "native-v2-input-sand: attribute each server Pos read by its own marker" rewrote the added
block (marker first, then `data get`; "No entity was found" handled), so no original post-fix line
survives at HEAD. The committed patch
(`docs/bugs/fix-patches/HARNESS-SAND-GATE-NO-HOLD-CHECK.patch`, `patch:82a4a5fc…`) is the HEAD
equivalent: the current block and its `import uuid`, whose reversal restores the pre-fix gate that had
no server-hold check at all (a pure deletion — the a/-side invents nothing).

| original hunk (`e1d92ae4e`) | HEAD equivalent (`native_player_input.py`) | note |
|---|---|---|
| `@@ -3,6 +3,7 @@` — `import uuid` | line 6, identical | carried |
| `@@ -278,6 +279,26 @@` — the hold-read block (`data, reads, prior`, the `data get` loop, `falling_server_pos` report, `sand_server_holds_frozen_block` check) | the block at `sand()` (8-iteration loop, its own `reply()` helper, same report and check names) | carried; (c) `653414533` rewrote the loop body, the behaviour and the assertion names are the fix's own |
| `tools/harness/scenarios.py` — the scenario's `proves=` gains "frozen server holds the falling block between steps" | the same line at HEAD | carried; a description hunk of the same fix, its reversal changes no ref |
| `test_native_player_input.py` — the model that drops the block while frozen | present at HEAD | kept (not in the patch): it is the row's proving ref |
| `docs/bugs/README.md`, `docs/bugs/ledger.toml`, other `scenarios.py` rows | — | (a) row metadata and other rows' text |

a/-side adaptations: none (the patch deletes the fix's added code and adds none).

Proving ref and line: `python:tools.harness.test_native_player_input:SandGateTests.test_a_server_that_keeps_dropping_a_frozen_block_turns_the_gate_red`
red at `tools/harness/test_native_player_input.py:161` — `AssertionError: RuntimeError not raised` for
each latency subtest (0.05/0.25/0.5): with the hold check gone the gate stays green on a server that
keeps dropping the frozen block, which is the row's symptom. Green at HEAD, observed 2026-11-24.
