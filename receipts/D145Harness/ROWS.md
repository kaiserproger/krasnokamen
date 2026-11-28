# D145Harness owned rows (D149 restructuring)

Owner: `LedgerHarness`, branch `wt/D145Harness`. Scope is exactly these five,
in this order; every other row is dispositioned in `DISPOSITION.md` and is not
worked here.

| # | Row | Fix in tree | Graphical ref | Proof that is still missing |
|---|---|---|---|---|
| 1 | HARNESS-CIV-GATE-LOCAL-SCAN-UNLOADED | 457d77778 | `harness:civilization-seed-qa:all` | one passed `civilization-seed-qa` receipt on the promoted tree |
| 2 | HARNESS-CIV-GATE-HOUSE-DOOR-UNLOADED | aa2afc814 | `harness:civilization-seed-qa:new_house` | same receipt; patch-mode control (reversal removes kept API) |
| 3 | HARNESS-CIV-GATE-STORAGE-READ-TIMEOUT | a0c0f8902 | `harness:civilization-seed-qa:all` | same receipt |
| 4 | HARNESS-CIV-GATE-PLANNER-READ-BLOCKED-BY-STATUS-DIALOG | a0c0f8902 | `harness:civilization-seed-qa:all` | same receipt |
| 5 | CIV-IRON-CAPTURE-OUTPUT-COLLECTED | d7f98cd4 | `harness:civilization-seed-qa:iron_smelted` | receipt whose `milestones.iron_smelted.source == "output_slot"` and `iron_smelted` verdict true |

One `run civilization-seed-qa --client native --windowed` gate on the candidate
tree is shared by all five. Its `result.json` supplies the per-row verdict and,
where the report carries `assertions`, the exact assertion name for
`proving_assertions`.

Preconditions already verified on this branch: the python guard refs are green
(`control-green.log` per row, `ALL-PYTHON-REFS-green.log`), the negative controls
are red on the row's own assertion/symptom, and every row now carries a
real-client `harness:` ref (the ledger refuses a client-surface row without one).
