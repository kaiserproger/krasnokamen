# Bug regression ledger

`ledger.toml` records each bug with `id`, `title`, `area` (`gameplay`, `client`,
`harness`; set from the symptom at filing and not changed by fixers),
`severity` (`blocker`, `major`, `minor`), `status` (`open`, `fixed`),
`source`, `repro` (`manual`, `client`, `cargo`, `source`) and
`regression`. Historical reports are not proof of the current candidate.

Add a failing-first check for a fix. `cargo:<crate>:<exact test path>` names
an exact test; `harness:<profile>:<scenario>` names an existing runnable
scenario. Every green Cargo check in `regression`, even for an open row,
runs on every integration; scenario checks run in the client tier. Keep a known-red reproducer in `repro_check`
instead; it is not a gate. Never weaken or delete checks to make a run green.

A row is fixed only when its check passes and is the same or stricter than the
original repro. Owner/manual and real-client repros, and all client-surface
bugs, require a real-client check; source/test findings may use a Cargo test
through the same real tick/gameplay path or a TCP/native scenario. Tests must
assert vanilla or owner-accepted behavior, not merely the current output.
One row describes one symptom: split covered behavior from remaining gaps.
The phase reviewer confirms each promotion batch matches the symptom and
failed before the fix. A unit test cannot close a known client failure.
Client promotion requires an assertion matching the repro; visual-review
scenarios require recorded review.

Run `python3 -m tools.harness run regressions` on every integration and
`python3 -m tools.harness run regressions-client` when promoting a
client-surface fix and before release. The independent breaker in
`../QA_AGENT_PROTOCOL.md` remains a release requirement.
