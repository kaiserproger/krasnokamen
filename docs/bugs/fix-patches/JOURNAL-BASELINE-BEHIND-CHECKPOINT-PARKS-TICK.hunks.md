# JOURNAL-BASELINE-BEHIND-CHECKPOINT-PARKS-TICK — negative-control patch

Fix: `docs/bugs/fix-patches/JOURNAL-BASELINE-BEHIND-CHECKPOINT-PARKS-TICK.patch`
(`fix_commit = "patch:c31baef245918fe396da17c256f74e40ad7d5d6d9fa2719d0554f38560a2e1d7"`, sha256 of that file).
It is a diff from the adapted pre-fix form to the candidate (HEAD); `git apply -R` restores the pre-fix form.
An earlier partial patch omitted the owner-built answer guard, the writer wake and the serial counter; the reviewer
rejected it for that reason, the row was reopened, and that identity no longer appears in the tree.

## Commits covered

- `e40a26d4e` "fix(core): keep tick admission while a baseline waits behind a checkpoint" — the owner-side hold
  (drain `pending`, publish nothing before the answer), the tick carries its queue sender inside the baseline job,
  the writer answers through it, the hold's item bound `JOURNAL_HOLD_LIMIT` sized from the measured checkpoint
  window, and the ADR/docs statements.
- `75a8a7cf9` "fix(core): close a dropped baseline answer and keep its wake" — the answer guard is built by the owner
  (`BaselineWake { done, signal, serial }`), so a job the writer never runs closes its answer and wakes the owner; the
  writer drains unrun jobs before returning; a closed answer is final when the wake arrives.

Which part of the row's path each closes: `e40a26d4e` removes the stall itself (the tick's sends keep being admitted
while the baseline waits); `75a8a7cf9` removes the liveness regression the first commit could have introduced when the
writer stops before running the queued baseline (previously immediate `Disconnected`, now an explicit closed answer).

## Carried hunks (all in `crates/mc-core/src/core/journal.rs`)

- `BaselineWake` struct, its `answer` and its `Drop` (a/-side: absent);
- `NativeWork::RequireGenerated` back to `{ chunks, done }`, writer arm
  `let _ = done.send(chunks.fill_generated());` (a/-side: the candidate's `{ chunks, wake }` and `wake.answer(...)`);
- owner arm back to `JournalWork::RequireGenerated { capture, wake: _ }` with the pre-fix wait
  `let outcome: Option<Result<(), CoreError>> = durable.recv().ok();` under a "Pre-fix scheduling" comment (a/-side
  adaptation: the base's `match durable.recv() { … }` is written as the `ok()` form so the rest of the arm, which is a
  candidate addition, stays identical);
- `next_baseline_serial` and its handoff use (a/-side: absent);
- the native writer's post-stop drain `while let Ok(work) = work_rx.try_recv() { drop(work); }` (a/-side: absent).

## Not carried, with the ground (docs/bugs/README.md:226-236 classes)

(b) API the kept tests call — `JournalWork::RequireGenerated { capture, wake }` (core_journal_backpressure_tests.rs
:414, :839), `JournalWork::BaselineDurable` (:74), `journal::JOURNAL_HOLD_LIMIT` (:1159). These names stay in the
pre-fix form as inert plumbing; reversing them would make the registered refs uncompilable.
(c) Behaviour the row does not own — `crates/mc-core/src/core.rs` (`JournalWork` enum, `JOURNAL_WORK_QUEUE` doc) and
`crates/mc-core/src/core/tick_loop.rs` (the `wake: work_tx.clone()` field on both baseline sends) are the queue API
the hold fix needs; they are not scheduling behaviour and are left in place.
Docs only — `journal_budget.rs` scope paragraph, ADR 0013, `docs/OPERATING.md`, `docs/architecture/persistence.md`
describe the window; no control depends on them.

## a/-side adaptations (named)

1. The pre-fix owner wait keeps the candidate's surrounding code and only replaces the hold loop with
   `durable.recv().ok()` (the base wrote it as a `match`); recorded because diffing against the base text would also
   reopen unrelated arms.
2. `JournalWork::RequireGenerated { capture, wake: _ }` keeps the tick-supplied field and ignores it, because the
   registered refs construct that variant.
3. `JOURNAL_HOLD_LIMIT` and `JournalWork::BaselineDurable` stay defined.

## Proving assertions (red in the control)

- `crates/mc-core/src/core_journal_backpressure_tests.rs:640` — "tick sends must not park behind the held checkpoint"
  (5 s send deadline).
- `:1172` — "admission must not park below queue plus hold" plus the `Full` assertion that follows it, in
  `more_than_the_hold_limit_publishes_every_batch_in_order`.
- `:1009` (`writer_death_before_a_queued_baseline_halts_without_tick_work`) fails at the 60 s lifecycle bound in the
  pre-fix form because the queued baseline is never answered.

## Control receipts

- Candidate PASS on this tree: `.analysis/validation/20261123T151109-test-kkkdqjpm` (13/13).
- Reversed form RED: `.analysis/validation/20261123T151216-test-cdgzv60i` (symptom test at 5 s, hold-limit test at 20 s,
  run ended in 54.5 s). Earlier cycle: `20261123T143537-test-071lylqf` (4 red, bounded).
