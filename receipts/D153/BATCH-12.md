# Batch 12 (D153) — landing receipt

Tree: local `main`, base `origin/main` + 18 commits (17 code/docs + this receipt).
L2: `python3 -m tools.harness run correctness` through `tools/dev/mem-run.sh -m 12G`,
validation directory `.analysis/validation/20261128T084217-correctness-_i94jl4b/`.

## Commits in the batch

| commit | subject |
|---|---|
| `7569513cf` | fix(dev): make the client stand sourced-safe (D153) |
| `b24a2a952` | Merge branch 'wt/BEDLane' (BED occupied-property mirror, promoted) |
| `4cd4c9473` | docs(ledger): promote BED-OCCUPIED-PROPERTY-NOT-MIRRORED |
| `cc250e5eb` | docs(ledger): refresh BED's patch for the crate split |
| `50bcc7f83` | docs(spain): close the wt/SpainInteg decision per commit |
| `c1b6a2968` | docs(ledger): mark the SpainInteg row as the family tracker |
| `b4c241427` | feat(dev): land the D92 task-timing telemetry |
| `8c8108bc5` | docs(fleet): document the D92 check-in line |
| `16c0ad621` | docs(ledger): give the test-only rows a closure path |
| `18899905c` | docs(ledger): file the rejected loader doc claims |
| `f4469baa4` | docs(loader): restore the provisional-bundle rewrite, corrected |
| `bb2c01792` | docs(ledger): point the autonomy row at the main ancestor |
| `4fa5115db` | docs(ledger): drop the copied latch advice from four rows |
| `1c070bd1a` | docs(ledger): mark the append-halt removal patch as defect injection |
| `073298389` | docs(loader): required = false lives in [[client.bundles]] |
| `da15c7ecb` | docs(spain): F2 founding outposts port in progress on wt/SpainF2 |
| `257d62bbb` | docs(spain): state the 21-assigned, one-formatting-only closure counts |
| `9438c824f` | docs(fleet): record the live lane ownership map (`receipts/D153/LANES.md`) |

## L2 stages

| stage | result |
|---|---|
| fmt | green |
| clippy (`--workspace --all-targets -- -D warnings`) | exit 0 in 303.7 s |
| code-health (`xtask code-health`) | exit 0 in 8.2 s |
| test (`cargo nextest run --profile default --ignore-default-filter --no-fail-fast --no-tests=fail --workspace --all-targets`) | running at 1025/9546 tests, 0 failures, 16:16 +07 |

The `test` stage runs while twelve lanes compile and run their own gates, so its
pace is far below a quiet host; no result is claimed until the stage exits.

## Rows touched

- `BED-OCCUPIED-PROPERTY-NOT-MIRRORED` — promoted before the batch (control red
  1300/1321 on the reverted tree), merged as `b24a2a952`.
- `LOADER-*` doc claims — the rejected doc text is filed, the false row deleted,
  the doc-text test deleted per docs/RULES.md.
- `SETTLEMENT-AUTONOMY-VILLAGE-ROTATION-STARVES-TICK-GATES` — stays open: the
  committed test calls the private `gate_due`, so it cannot be the control; the
  row now carries the required control shape (`autonomy::tick`, eight villages,
  400 ticks) and is owned by `wt/SpainD7Re`.

## Landing

Pushed `3511c5e81..6c14f3002` to `origin/main` at 16:2x +07 (remote reports the
repository moved to `git@github.com:kaiserproger/krasnokamen.git`; the `solaris`
remote still redirects). The owner rule allows a not-yet-green commit to land and
a broken push to get a follow-up fix, so the batch went out while the `test`
stage was still running at 1472/9546 with 0 failures; this file records the L2
verdict when the stage exits, and a red stage becomes a fix-forward, not a
retraction.

Additions after the table above: `69043d026c` (frozen-falling row points at its
verified client pair), `d712ed32d7` (spear row gets a control that compiles),
`6c14f3002a` (client pins key the embedded asset models — measured: an untracked
model file moves `inputs_sha256` from `ded5865da99a…` to `9238ffb29f40…`).
