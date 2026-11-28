# Batch 13 — landing receipt

Pushed `241bf6a9a..84af5aa67` to `origin/main` (the push used `kaiserproger/solaris.git`,
which GitHub redirects to `kaiserproger/krasnokamen.git`); signal `solaris-main-landed` sent. Under the owner rule that a not-yet-green
commit may land with a fix-forward, the L2 below was still running at push time.

## Commits

| commit | subject |
|---|---|
| `fce0a1f00` | docs(bugs): CmdBlocks command/block row controls (furnace, harvest, lumber, mine) |
| `43bff7de8` | Merge branch `wt/CmdBlocks` |
| `aeda9aadc` | test(harness): capture fixture acceptance before damage and scene shots |
| `86ec8d5ec` | Merge branch `wt/LedgerHarnessEv` |
| `84af5aa67` | docs(ledger): name the autonomy row's required control shape |

## Verification actually run

| check | result |
|---|---|
| `python3 -m unittest tools.harness.test_ledger` | 75/75 OK (includes the new JUnit 5 stack-frame case) |
| `python3 -m unittest tools.harness.test_regressions` | 178/178 OK |
| `python3 tools/doclinks.py` | 0 broken, 305 unreachable, 411 files |
| client pin input key, before/after the `assets` fix | `d9e0f9fff870…` → `ded5865da99a…`; an untracked model file moves it to `9238ffb29f40…` and deleting it restores `ded5865da99a…` |
| SPEAR promotion | ledger reversal exit 1 red at the packet assertion, candidate exit 0, `docs/bugs/promotions/JAVA-BATTLE-SPEAR-PLAIN-ATTACK.json` |
| five CmdBlocks patches | `git apply --reverse --check` clean on HEAD for furnace, harvest, lumber, mine, forest |

## L2

`python3 -m tools.harness run correctness` in
`.analysis/validation/20261128T093209-correctness-uwlz_vpd/` started at 16:32:09
on `86ec8d5ec`, before the docs-only commit `84af5aa67` (16:32:47). It was still
running at the push; the exact start fingerprint will be in its `result.json`. The earlier
batch-12 L2 (`.analysis/validation/20261128T084217-correctness-_i94jl4b/`) was
abandoned after its push when main moved under it: it reported fmt, clippy and
code-health green and 2028 of 9546 tests with zero failures before it was
stopped. A red result becomes a fix-forward commit, not a retraction.

## Rows promoted

`JAVA-BATTLE-SPEAR-PLAIN-ATTACK` — see its promotion receipt; the row's status is
`fixed` on main.

## Rows waiting on gates (window W1, head `84af5aa67`)

- Five CmdBlocks rows need one `civilization-seed-qa` real-client run each set of
  assertions proves: furnace, harvest, lumber, mine, forest-site.
- `DAMAGE-GATE-REJECTED-REGENERATION-RULE-REMAINDER` and `TEST-JAVA-CAPABILITIES`
  need their gate re-run on this head plus the ledger's own reversal
  (`--fix-patch`), and `HARNESS-BATTLE-CRASH-HANDOFF-LOST-ON-EARLY-SIGKILL` needs
  the stalled-first cut reorder before its two arms.
- `NATIVE-FROZEN-FALLING-DRAWN-LATE` cannot be gated here: the assertion
  `frozen_falling_presented_current_tick` does not exist in `tools/` at this head.

## Rows returned open with a recorded reason

- `DATAPACK-NEW-PACK-WORLDGEN-SKIP` — server-start symptom, no client scene can show it.
- `DATAPACK-LIST-HOVER` — no control shape under the mechanical-adaptation rule.
- `DATAPACK-GENERAL-TERRAIN-CUSTOMIZATION-UNSUPPORTED` — generator feature, owner scope.
- `HARNESS-ADMIN-TEST-TEMP-DIR-LEAK` — the named fix commit has no production hunks (`fix commit abc5fd559 has no non-test hunks to reverse`), so it belongs to the test-only closure path, not a promotion.
