# JOURNAL-BASELINE-BEHIND-CHECKPOINT-PARKS-TICK: patch-mode fix identity

Fix commits: `e40a26d4e` (hold scheduling) and `75a8a7cf9` (owner-built answer guard,
writer drain on stop). `patch:c329db74025d9a6e9db290455345922026524b08297fd852ccd2ec235d685869` names
`docs/bugs/fix-patches/JOURNAL-BASELINE-BEHIND-CHECKPOINT-PARKS-TICK.patch`, a diff from the
adapted pre-fix form to the candidate (HEAD); `git apply -R` restores the pre-fix scheduling.

Carried (the scheduling/liveness delta in `crates/mc-core/src/core/journal.rs`):
- owner hold loop (drain queued work into `pending`, publish after the baseline's answer)
  replaced by the pre-fix blocking `durable.recv()` wait;
- the native writer's drop of jobs it never ran removed, so a stopped writer leaves a queued
  baseline unanswered.

Not carried, and why (the automatic control keeps the registered refs, so the reversed form
must still compile): the API shape introduced by the fix — `JournalWork::RequireGenerated { capture, wake }`,
`JournalWork::BaselineDurable`, `NativeWork::RequireGenerated { chunks, wake }`,
`BaselineWake`, `next_baseline_serial`, `pub(super) JOURNAL_HOLD_LIMIT` — is named directly by
`crates/mc-core/src/core_journal_backpressure_tests.rs`. Reversing those hunks makes the
registered refs uncompilable (the same class as #23's excluded `RefusedChunk` hunks), so the
pre-fix form keeps them as inert plumbing: with the hold loop gone they are never used.

Control evidence (same tree, own deadlines, from the review cycle): candidate PASS
`.analysis/validation/20261123T151109-test-kkkdqjpm` (13/13); reversed form RED
`.analysis/validation/20261123T151216-test-cdgzv60i` (symptom test at its 5 s send deadline,
hold-limit test at its 20 s admission deadline; run ended in 54.5 s).
