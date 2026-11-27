# SCENARIO-RANDOM-BREAK-NO-PROGRESS — fix-hunk map

Patch `patch:b684c3f352101d4df3b32e887a04aa34dd4997d549bc05f2ca86ebeb3066de36`
(`docs/bugs/fix-patches/SCENARIO-RANDOM-BREAK-NO-PROGRESS.patch`), re-promoted 2026-11-27 on tree
`84fc5c955ca0` (artifact `.analysis/validation/20261127T110920-ledger-promote-wyufbgyp/promote.json`,
control pin `84fc5c955ca0-0b9a8af7`).

## The fix

The defect commit is `5cbed1e33` ("matrix runner import and seeded random scenario mode"): the
harness sent the older player-action ordinals, so a break's second phase was `Cancel` instead of
`Finish`. The patch contains three ordinal hunks across two harness files:

| file | hunk | content |
|---|---|---|
| `tools/harness/protocol_input_verbs.py` | `@@ -210,7 +210,7 @@` | mining `abort` 1→2 and `stop`/`complete` finish 2→3 |
| `tools/harness/protocol_input_verbs.py` | `@@ -239,14 +239,14 @@` | release-use 5→6 and swap-hands 6→7 |
| `tools/harness/protocol_matrix_client.py` | `@@ -260,7 +260,7 @@` | `drop()` stack 3→4 and single item 4→5 |

Omitted: `5cbed1e33`'s `docs/AGENT_TOOLING.md` and `docs/bugs/ledger.toml` hunks — no behaviour, and
their context has drifted since (reason (a): a docs hunk with no runtime effect).

## Control (measured)

- `python:tools.harness.test_protocol_input_verbs:EncodingTests.test_break_block_sends_start_then_stop_with_sequence`
  red at `tools/harness/test_protocol_input_verbs.py:64` (AssertionError on the sent action ids).
- `python:…:EncodingTests.test_break_block_phases_send_exact_actions` red at
  `tools/harness/test_protocol_input_verbs.py:84`.
- `harness:matrix-random:protocol-random-break-seed-4` red at `legal_break_made_progress`: the
  control receipt's own error reads `random:4 assertion failed: legal_break_made_progress`
  (`tools/harness/matrix_domains/random.py`), the named form `ledger._scenario_assertion_failed`
  counts. That form is what BASE's harness commit `973e6e60e` added.

`python:…:RandomMatrixTests.test_mining_waits_for_exact_ack_including_sequence_zero` stays green on
the control (it stubs the actor, so it cannot flip) and is the compatibility guard, excluded from
`proving_refs`. All four are green on the restore.
