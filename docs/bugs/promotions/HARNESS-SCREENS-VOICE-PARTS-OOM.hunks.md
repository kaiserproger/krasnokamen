# HARNESS-SCREENS-VOICE-PARTS-OOM — patch mode, one commit

`fix_commit = patch:ddeead85aeac35f3f141667e55c80d2a1a99bfd1eb7c8c138d6ffe01b34dad45`
(`docs/bugs/fix-patches/HARNESS-SCREENS-VOICE-PARTS-OOM.patch`, `git show 725ee9efe1 --format= --
tools/harness/backends/screens_voice_qa.py`).

## The row's path

Two complete legs (server, client and Xvfb each) overlapping inside one 12 GiB `mem-run` scope were
OOM-killed after 173 s before either leg wrote a verdict. The fix is the *driver's* decision to run
the legs one at a time.

## The patch (1 hunk, `tools/harness/backends/screens_voice_qa.py` @ -127,7 +127,10)

| original hunk | HEAD equivalent | part of the row's path |
|---|---|---|
| `_split_run` calls `run_split(ctx, "screens-voice-qa", [...])` → passes `sequential=True` with the two-comment rationale | identical, still at `screens_voice_qa.py:130-133` (no later commit touched it) | yes — this is the whole fix: `sequential=False` (the reverse) launches both legs at once again |

Reversing it keeps `run_split`'s sequential machinery intact and only makes this gate concurrent.

## Original hunks left out

`725ee9efe1` also changed `tools/harness/profiles.py` (+113/−…): the `sequential: bool = False`
parameter and the per-part deadline measured from each part's start. Omitted for reason **(b)**: it is
API the kept tests call — `test_parts_run_one_at_a_time_so_two_full_legs_never_share_a_scope`,
`test_each_sequential_leg_gets_its_own_full_timeout` and `test_other_split_gates_keep_the_concurrent_start`
construct `run_split(..., sequential=True)` themselves, so reversing those hunks stops the control from
running at all. The per-part budget is a separate concern (review item 1 of the D123Harness review) and
its own tests stay green under this control.

## Proving ref and line

`python:tools.harness.test_split_parts:SplitPartsTests.test_the_screens_voice_split_driver_runs_its_legs_in_sequence`
— asserts the call order `spawn:a, wait:a, spawn:b, wait:b` at the driver seam
(`test_split_parts.py`, its `assertEqual(calls, expected)`), and with the hunk reversed the order is
`spawn:a, spawn:b`: the scope overlap the row describes. It names the row's symptom (concurrent legs in
one scope), not a setup assertion: `spawn`/`wait` are recorded before anything is launched.

Not evidence: the other three refs. They pass `sequential=True` themselves, so they stay green in the
control and prove `run_split`'s semantics, not the gate's scope.

## Guards older than the finding

None: the sequential call is the fix, added by `725ee9efe1` (2026-11-23) after the OOM.
