# Batch 14 — 2026-11-28

Push: `3d5dd3f40..ab04b2aa5` on `origin/main`; `tmux wait-for -S solaris-main-landed`
sent once at the push. L2 started on `ab04b2aa5` (job `bg_11236`,
`python3 -m tools.harness run correctness`) after the push, per the standing rule
that a not-yet-green commit may land and a broken push gets a follow-up fix.

## What landed

| commit | content | proof |
|---|---|---|
| `d1b0da59f`, `afaabc4b9`, `3d5dd3f40` | client test work cherry-picked onto `main` from outside Main at 16:48 (see `docs/PITFALLS.md`) | not gated on `main`; the batch L2 is their first full run |
| `11fac926b` | `receipts/W1Gates/*` — the 44 open patch rows, their scenarios, the two promotion refusals | docs only |
| `6e7b0e453` | `receipts/SpainF2Review/REPORT.md` + the F1/F2 review table in `receipts/D142/SPAIN-DISPOSITION.md` | docs only |
| `4881424ac` | delete `TEST-NATIVE-IMPORT` (false positive) + `receipts/LedgerCleanup/DELETED-20261128.md`; open rows 202 → 201 | guard test `Ran 1 test, OK` |
| `3b0103d62` | `docs/PITFALLS.md`: gate-freeze and shared-checkout entries | doclinks 0 broken |
| `ab04b2aa5` | merge `wt/HarnessAsserts` `a72dd06a0`: one graphical assertion per 13 open rows, `civ_row_signals.py`, `entity_relative_trajectory_scene.json`, `datapack_reload` skip assertion, `scene_shot_checks` drift rule | lane: 486 harness tests OK (1 skipped), fmt/code-health/harness-check PASS, worldgen reversed-hunk control FAILED as designed; on `wt/IntegSp`: `test_civ_row_signals`+`test_datapack_worldgen_skip`+`test_entity_relative_trajectory` 20/20 OK |

## Still owed by this batch

- The L2 verdict (`bg_11236`); a red run gets a fix commit, not a repeal of the push.
- The 13 rows stay `open`: their new refs are unobserved in a full scenario run, and
  `ledger promote` needs that receipt. The civilization run at the frozen head
  (`wt/W1Freeze`, `2216e1069`) is queued behind two scene slots and does not cover
  scenario code newer than `ab04b2aa5`; a fresh civilization run on this head is the
  next gate once a slot frees.
