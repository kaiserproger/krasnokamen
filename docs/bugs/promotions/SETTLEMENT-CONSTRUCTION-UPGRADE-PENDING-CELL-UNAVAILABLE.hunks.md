# SETTLEMENT-CONSTRUCTION-UPGRADE-PENDING-CELL-UNAVAILABLE — control hunks

The fix under test is `983178bde2` (fix(settlements): repair an upgrade's inherited level-image
cell), on top of `597352d5a7`. The candidate is `f532864432`; its only later commits are the ledger
row, the docs and clippy/code-health hygiene, so the fix's hunks are unchanged from the commit.

Patch mode is used for one reason: **class (b) — API the kept tests call.** The fix's
`blueprints.rs` hunk adds `Blueprint::inherited`/`Blueprint::image`, and the kept test
`settlements::construction::stall_tests::a_committed_house_upgrade_reports_its_missing_certificate_requirement`
(which this fix had to adapt) calls `Blueprint::inherited` to lay the inherited shell. Reversing that
hunk stops the control from compiling, so the patch carries only the behaviour hunk; the helper stays
present (unused in the control, a `dead_code` warning) exactly as the kept tests need.

| hunk | site | pre-fix state |
|---|---|---|
| 2 (kept) | `settlement_builds.rs`: `fn inherited(..)` and the `reopen_emptied` loop over it | absent — `reopen_emptied` scans only the record's own cells (`597352d5a7` text, verbatim) |
| 1 (omitted, class b) | `blueprints.rs`: `Blueprint::image`/`Blueprint::inherited` | kept in the control; reversing it breaks the kept stall test's build |

`git apply -R --check` passes on `f532864432`; the reversed tree compiles and the row's own cargo
ref goes red at its assertion (`construction did not finish`, house L2 at 100% Committed,
`20261125T115102` is the same red on the unpatched base). The harness ref is not rerun for the
control (no reverted-tree pin; it is `unrunnable` there and green on the candidate receipt
`20261125T141741-buildings-qa-bdfzq_2x`).
