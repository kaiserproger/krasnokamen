# CIV-FIXTURE-INACTIVE-MINE — HEAD re-break (2026-11-24, D123CivE)

Patch: `docs/bugs/promotions/CIV-FIXTURE-INACTIVE-MINE.patch`
(`patch:8b9db998064389c903dbe57b23c0e219e5537a36f9597a9f1fc3151b3f7baf2d`), the
row's `fix_commit`. Reverse-applies at `978d656826` (`git apply -R --check`).

## Origin and pre-image

The fix is the hunk of `4e6874ded` ("land CivSim-5 civ-sim slice") in
`tools/harness/backends/village_autonomy_qa.py`:

```
+    if civilization:
+        # The village's mine must tick beyond the four-chunk presentation window.
+        replacements += (("simulation_distance = 4", "simulation_distance = 6"),)
```

The pre-image is that block's absence: the `if civilization:` statement, its
comment and the `replacements +=` line. `4e6874de^` holds exactly this absence
(the same function body, then named `run()` with artifact paths).

## HEAD equivalence

At HEAD the same three lines live in the extracted `server_config_text(source,
*, port, world_dir, plugins_dir, seed, civilization)` (line 614). The patch
removes those three lines there. Nothing else is reversed.

### a/-side adaptation (named, behaviour-neutral)

A later commit (review N6) inserted a five-line operator comment *inside* the
`if civilization:` block:

```
        # Known limitation (review N6): the civilization gate's primary player is an
        # operator level 4 ... is not exercised by this fixture.
```

Those lines belong to that later commit, not to this row's fix, so the patch
keeps them and only dedents them from eight to four spaces (a comment; Python
ignores comment indentation). No code line outside the origin hunk changes.

## Red and green (both observed here)

- Reversed tree (`git apply -R`): `python3 -m unittest
  tools.harness.test_village_autonomy_qa.VillageAutonomyQaTests.test_civilization_fixture_raises_only_the_simulation_distance`
  → `AssertionError: 'simulation_distance = 6' not found`, exit 1
  (`receipts/D123/D123Civ/neg-civ-inactive-mine.log`).
- Restored tree: the same test passes, exit 0.

## What is still missing (not a control problem)

`harness:civilization-seed-qa:all` has no passed real-client receipt: the gate
stops on its own progression rule (CIV-6 wood bootstrap,
`../OWNER-DECISIONS-NEEDED.md`), so no verdict is reached. The unit red proves the
fixture line only; the row stays `open` until a candidate gate receipt exists.
