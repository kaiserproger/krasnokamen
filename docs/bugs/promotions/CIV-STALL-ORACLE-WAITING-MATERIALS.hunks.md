# CIV-STALL-ORACLE-WAITING-MATERIALS — HEAD re-break (2026-11-24, D123CivE)

Patch: `docs/bugs/promotions/CIV-STALL-ORACLE-WAITING-MATERIALS.patch`
(`patch:b3538e8718f36b34509a7cfa6b5eb13e8e5786ef5e8e41300c33e9530012db6f`), the
row's `fix_commit`. Reverse-applies at `978d656826` (`git apply -R --check`).

## Origin and pre-image

Fix `7ed4d504` changed the civilization gate's stall oracle in
`tools/harness/backends/civilization_seed_qa.py`: it added `stall_signature()`
and replaced the diagnosis expression with a call to it. The pre-image
(`7ed4d504^`) at that call site is verbatim:

```
        # Unrelated resource growth must not conceal a stalled project.
        # Project UI retains its actual material blocker and progress.
        signature = (status.get("project"), status.get("project_ui"),
                     village._listed_buildings(status["buildings_now"]))
```

The patch restores exactly those four lines at HEAD's call site (line 959) and
changes nothing else.

## Why `stall_signature` is kept (class b)

A later 2026-11-24 commit (review D123Civ6 / N1) rewrote the *body* of
`stall_signature` to read `status["projects"]` and to sign every parsed project;
its test `test_a_visible_project_reaches_the_completion_match` calls
`qa.stall_signature(...)` directly. Deleting the function would break that kept
test at import/attribute level (not at an assertion), so the function stays with
its HEAD body and the call site carries the reversal. That later body is class
(c) drift relative to the origin hunk and is named here, not reversed.

## Red and green

- Reversed tree: `test_waiting_project_materials_do_not_hide_a_stalled_active_project`
  fails — the run reaches `bounded observation limit reached; missing
  milestones: new_house, births, second_building, iron_smelted` instead of the
  one-day stall it asserts (`receipts/D123/D123Civ/neg-civ-stall-oracle.log`,
  exit 1). That is the row's own symptom: a waiting project's gathered materials
  hide the stalled slot holder again.
- Restored tree: both waiting-material tests pass.

## What is still missing (not a control problem)

`harness:civilization-seed-qa:all` has no passed real-client receipt: the gate
stops on the progression rule (CIV-6 wood bootstrap,
`../OWNER-DECISIONS-NEEDED.md`). The row stays `open`.
