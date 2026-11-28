verdict: changes

# SPAININTEG-F2-FOUNDING-OUTPOSTS review (committee-sol, 2026-11-28)

Scope: `wt/SpainF2` head `04911e5570`, commits `dccd36366`, `b952c7543`,
`8f9d2b3a8`, `e00161aea` (porting `7e9ad6c24`, `2195981eb`, `98eab23e3`,
`a0ae2b282`), row `docs/bugs/rows/SPAININTEG-F2-FOUNDING-OUTPOSTS.toml`, lane
receipts `receipts/SpainF2/`. Reviewer read the refs/reflog, the squashed Git
diff, the code and the six control receipts; individual commit bodies were not
reachable through its tools and it does not certify parent-by-parent identity.
No test run.

## (a) Split and WAL — fail on the real tick; local atomicity pass

`construction_founding.rs:404-468` checks all four lots before `detach`/`form`;
a refusal moves only the cursor. `households.rs:88-204` changes the household key
of two adults without touching their beds, inventories or settlement membership.
Children, the old house and the stock stay. Settlement and villager records land
in one WAL batch (`core/journal.rs:415-430,646`; `core/tick_loop.rs:293-325`),
and the SIGKILL test separates unpublished, torn and published batches
(`construction_founding_split_tests.rs:371-503`).

But the production tick takes `other_dimensions` **before** local planning and
returns them only after it (`dimensions_tick.rs:175-183,221-232,262-266`;
`tick/dimension.rs:474-477`; `settlements/mod.rs:39-43`). So `split_members`
cannot see the third adult in another dimension, and `household_ready` can take
two local adults for a whole family without seeing the rest
(`construction_founding.rs:148-174,206-238`). The test claimed as proof of the
cross-dimension case calls `admit` directly while the neighbouring world is still
available (`construction_founding_split_tests.rs:212-245`).

## (b) Cursor — fail; work payment — pass; route bound — fail

The cursor is separate from autonomous construction and grows by one per attempt,
but it does not guarantee uniqueness even within one sweep: at `radius = 96` the
east side at offset `+136` and the south side at `-136` give the same corner.
The first five attempts in the test do not cover it
(`construction_founding.rs:359-383`; `construction_founding_split_tests.rs:274-311`).
The seven starter columns write no blocks themselves; clearing runs through the
ordinary work list with provenance checks and drop credit before the block write
(`construction_founding_access.rs:48-69`; `forestry.rs:1199-1248`;
`plugin_structures.rs:1204-1261`). However `layout.rs:391-456` bounds the route on
X/Z only: start and passage may sit entirely below the reserved volume, and
`construction_preparation.rs:376-392` includes such a corridor in the executor
bounds. This is not free clearing, but the stated bound is not kept.

## (c) The six controls — fail as Git pre-image proof

The red outcomes are the row's symptoms, but not all come from a real reversal.
`control-02a-survey-cursor.log:6-13` describes a hybrid: reading returned to
`record.construction_cursor` while the write stayed in `founding_cursor`. Such a
control can stall without the autonomous cursor overwrite. The rest:

| control | what the log shows | pre-image status |
|---|---|---|
| 01a | no lateral admission (`construction_founding_tests.rs:330`) | unconfirmed: the published expression without the divisor differs from the pre-port `construction_founding.rs:255` |
| 01b | the plan refuses without an independent route (`construction_founding_tests.rs:390`) | the old call matches the squashed Git diff (`SPAININTEG-F2-FOUNDING-OUTPOSTS.patch:452-468`); the applied delta is not attached |
| 02b | a single closed starter column yields `None` (`construction_founding_access_tests.rs:107`) | description, not a Git hunk (`control-02b-apron-seed.log:6-8`) |
| 03 | no split admission (`construction_founding_split_tests.rs:321`) | describes `split = None` with helpers kept; without a delta it does not prove restoring `8f9d2b3a8^` (`control-03-household-split.log:7-12`) |
| 04 | the lateral lot is not reached (`construction_founding_split_tests.rs:308`) | a one-line reverse hunk is given, but equality with `e00161aea^` is unconfirmed (`control-04-survey-divisor.log:3-9`) |

The squashed patch is also unusable as an executable control: reversing it
deletes `outpost_access` and restores the four-argument `forestry::cells`, while
the un-reverted tests keep calling the new module with a fifth argument
(`.patch:247-252,440-450`; `construction_founding_access_tests.rs:107`;
`settlement_builds_preparation_tests.rs:412`), and the harness runs tests exactly
after such a reversal (`tools/harness/ledger.py:3067-3079`). A graphical proof for
`repro = "client"` is likewise missing (`receipts/SpainF2/ROWS.md:95-98`;
`tools/harness/regressions.py:832-835`).

## (d) Old-record compatibility — pass; cursor survival — fail

F2 changes no serialised schema: `founding` was already `#[serde(default)]` and
`HouseholdOrigin::Split` already existed. The old-WAL test checks not only `None`
but a name that could only come from replay (`registry.rs:65-85`;
`households.rs:10-17`; `construction_founding_tests.rs:484-568`). The new cursor
is a runtime-registry field, not part of `SettlementRecord`; checkpoint and WAL do
not carry it, so after a restart the survey starts from zero. That is documented
but does not meet the requirement to continue the survey without repeating it
(`registry.rs:139-140`; `persistence.rs:107-153,169-199`;
`docs/architecture/settlements.md:360-366`).

## (e) Tests that pass on a default — fail

`hungry_pair` and `refused_site` go green with an empty `admit`, and
`forestless_outpost_requires_a_route` with constant `None`/`Err`
(`construction_founding_split_tests.rs:253-273,357-367`;
`construction_founding_access_tests.rs:129-152`).
`player_touched_corridor_log_is_avoided_and_preserved` can leave the loop before
the first work operation and still assert the trunk survives
(`construction_founding_access_tests.rs:239-259`).

## Must-fix

1. Use the full cross-dimension household composition in production planning;
   test through `run_tick` with an adult left in another world and refuse to take
   an incomplete family for a whole one.
2. Remove duplicate corners, continue the cursor after replay without a civil
   side effect on refusal, and bound the route by the reserved volume; test a full
   sweep, a restart mid-sweep and a height step.
3. Attach the exact Git deltas of the controls with red/green pairs; for 02a
   revert both the read **and** the write of the old cursor. Make the ledger
   reversal compile and attach a graphical check.
4. Give every negative test a positive outcome on the same fixture; for the
   protected trunk prove the executor actually advanced work.

VOTE SPAININTEG-F2-FOUNDING-OUTPOSTS: amend — local atomicity is confirmed in
code, but cross-dimension admission, the cursor and the control evidence need
fixes.
