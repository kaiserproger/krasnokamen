# Lessons

Non-obvious failures and fixes from Krasnokamen development, newest first. Each
entry says what happened and what to do now; details live on the linked page.
Dates are when the lesson was learned (owner time zone, UTC+7).

## 2026-11

- **2026-11-07 — recovery evidence must fence its source.** A live-writer hot
  copy of a world was refused, and recovered rows that matched expectations
  still did not show whether the checkpoint was absent, which root was selected
  or whether the WAL replay caused the result. Recovery and durability claims
  need: the source fenced (stopped, or a consistent snapshot); proof of the
  checkpoint/WAL boundary (what the last checkpoint held, what only the WAL
  held); and proof of the selected root. A hot copy, or a recovered row that
  matches, is not causal proof that the WAL recovered it.
  (`krasnokamen world probe` and `world census` read a stopped native world:
  [Constantinople fixture](constantinople-fixture.md#offline-subcommands))
- **2026-11-07 — the advisor no longer approves publication.** A bare
  `[approved]` arrived 41 turns late without a request identity, and the
  advisor kept stopping owner-authorized releases. A repeated `[approved]`
  had also been dropped as duplicate on 2026-10-29. The current authorization
  and independent review rules are in [release review](review-committee-and-breaker.md#release-reviewers-owner-2026-11-07).
- **2026-11-04 — an A/B pass and negative control do not prove the
  mechanism.** Instrument the cause before asserting it in release text
  ([release evidence](release-and-tags.md#core-release-cut)).
- **2026-11-03 — 1M-entity R&D rungs hit an 8 GiB cgroup OOM**, and a 16k mob
  fixture refused a 17 MB checkpoint manifest because scheduled fluid ticks
  dominated it. Large-scale claims need the exact rung's crash/reopen receipt;
  spawned is not active, sent is not accepted.

## 2026-10

- **2026-10-31 — RC2 release gaps** (content bundle missing the language file,
  stale Loader mixins, NeoForge undeclared channel) were found only by broader
  player-path checks; see the QA protocol introduction and
  [releases](release-and-tags.md).
- **2026-10-24 — an sccache daemon started inside a runner scope keeps the
  scope alive** and, under the old admission design, orphan Xvfb and sccache
  scopes made admission refuse work on an idle machine. Start sccache outside
  the slice (the runner now selects the cached wrapper itself; no
  `env RUSTC_WRAPPER=` needed).
- **2026-10-16 — per-tick fields must not be journaled as full images.**
  Journaling an entity image whenever a per-tick clock changed (egg timers, ages,
  cooldowns, work/sleep clocks) produced about 30 MB of WAL per tick and 15 GiB
  RSS. Store `(value, base_tick)` and derive; test that idle mobs produce zero
  images over N ticks and that live state equals replayed state. WAL recovery
  streams one batch at a time.
- **2026-10-14 — never send a player their own equipment update**: the client
  writes the main-hand item into its selected slot. Equipment goes to trackers
  only.
- **2026-10-13 — mutations of existing entities must reach the WAL.** Changing
  an existing entity's persisted fields or an item stack count without marking it
  changed (`appearance_changed`) left the WAL without it: after checkpoint and
  replay, a payment was journaled but the entity state was not (loss), or a
  reward replayed twice (dupe). Regression tests go through checkpoint, action
  and restart/replay, not only in-memory ticks.
- **2026-10-10 — an unpinned server was CFS-throttled** (23 threads under a
  6-core quota); pin before reading tick tails.
- **2026-10-09 — a full disk looked like a hang.** The journal died, no
  subscriber saw the channel close, and an L2 run hung for 54 minutes. Check
  free disk first when a gate hangs.
- **2026-10-07 — graphical gates fail on a loaded host** (`stale-revision`
  while a cargo matrix ran); run them on a quiet host.
- **2026-10-05 — speculative API fields cost a four-repository revert.**

## 2026-08/09

- **2026-08-28 — never reuse a prior run directory for a "fresh seed" claim:**
  persisted world metadata overrides the configured seed.
- **2026-09 — fixed-seed tuning was stopped by the owner** in favour of owner
  field testing, then varied-seed exploration; terrain-blocked stays blocked
  (now in AGENTS.md).
