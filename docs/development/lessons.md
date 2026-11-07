# Lessons

Non-obvious failures and fixes from Solaris development, newest first. Each
entry says what happened and what to do now; details live on the linked page.
Dates are when the lesson was learned (owner time zone, UTC+7).

## 2026-11

- **2026-11-07 — a late `[approved]` is ambiguous; intermediate releases need
  none.** A bare `[approved]` arrived 41 turns after it was issued, with no way
  to tell which request it answered. Requests now carry an ID, only one is open
  at a time, and an `[approved]` counts only if issued after that request
  (`turns_ago`). The owner then lifted the advisor gate for intermediate
  releases: it stays only for an RC the owner names as risky and for 1.0.0.
  ([releases](release-and-tags.md#a-pushed-tag-whose-ci-failed))
- **2026-11-06 — RC5 notes: three of four advisor rejections were unproven
  claims** ("RC4 behaves the same" without running RC4, "always" from one run,
  an untested platform). Map every claim to a receipt and a run count before
  the approval request. ([releases](release-and-tags.md#core-release-cut))
- **2026-11-04 — pooled target dirs gave false greens.** Pooled Cargo target
  dirs were shared across git worktrees. Cargo freshness is mtime-based and
  dep-info paths are relative to the package root, so a worktree whose files
  were older than another tree's build in the same target dir reused that
  tree's binaries: new tests were silently absent and runs passed. The runner
  now marks each slot with its source tree (`.solaris-tree`), prefers the
  tree's own slot and touches every tracked file on a tree switch. When a test
  you added does not run, suspect the target dir first.
  ([heavy commands](heavy-commands-and-memory.md))
- **2026-11-04/05 — negative controls must speak the current protocol.** A
  pre-26.3 commit fails at login (protocol 775), which proves nothing about the
  row's assertion. Reverse-apply only the row's fix on the current candidate.
  ([real-client gates](real-client-gates.md#promotion-with-a-negative-control-2026-11-0405))
- **2026-11-04 — release text from receipts, not summaries.** An agent summary
  claimed fresh profiles for all Windows soak runs; one reused a profile. Keep
  causes "likely" unless instrumented; an A/B pass plus a negative control does
  not prove the mechanism. ([releases](release-and-tags.md))
- **2026-11-04 — client provenance.** Entity geometry and UVs copied from
  decompiled or `javap` classes are Mojang-derived numbers and stay out of Git;
  rigs are authored by eye from public visuals.
- **2026-11-03 — `/dev/shm` has a per-user quota**; `df` free space is not
  usable headroom.
- **2026-11-03 — 1M-entity R&D rungs hit an 8 GiB cgroup OOM**, and a 16k mob
  fixture refused a 17 MB checkpoint manifest because scheduled fluid ticks
  dominated it. Large-scale claims need the exact rung's crash/reopen receipt;
  spawned is not active, sent is not accepted.

## 2026-10

- **2026-10-31 — RC2 release gaps** (content bundle missing the language file,
  stale Loader mixins, NeoForge undeclared channel) were found only by broader
  player-path checks; see the QA protocol introduction and
  [releases](release-and-tags.md).
- **2026-10-29 — a bare repeated `[approved]` was once dropped as a duplicate.**
  Each request now carries an ID and only one is open at a time (2026-11-07).
- **2026-10-28 — a stray slice `CPUQuota` throttled every build to 2 CPUs for
  two days.** Symptom: host CPU pressure about 80% while `vmstat` shows >40%
  idle. The runner now clears the quota on every call.
- **2026-10-28 — scenario runs duplicated about 4 MB of plugin `.wasm` per run**
  (7 of 11 GB). Delete only those copies from finished runs.
- **2026-10-27 — real-client gates use a release server**; a debug server drops
  to about 10 TPS while a joining player's chunks generate (AGENTS.md).
- **2026-10-24 — 26 parallel agents burned 30% of the weekly usage in one
  hour.** Keep the fleet small; re-list `proc://` after cancelling, because
  same-name orphans kept running for 10-17 hours. ([agent fleet](agent-fleet.md))
- **2026-10-24 — Windows disconnects (`os error 997`/`10053`) came from polling
  the socket with a 10 ms receive timeout.** A timed-out blocking Winsock
  receive leaves the connection indeterminate; use a blocking reader thread.
- **2026-10-24 — noVNC "connected" was a false positive** when only the canvas
  existed; require the `noVNC_connected` class. The VNC path drops Shift.
  ([Windows QA](windows-and-macos-client-qa.md))
- **2026-10-24 — an sccache daemon started inside a runner scope keeps the
  scope alive** and, under the old admission design, orphan Xvfb and sccache
  scopes made admission refuse work on an idle machine. Start sccache outside
  the slice; run Cargo with `env RUSTC_WRAPPER=`.
- **2026-10-24 — advisor escalations need source checks.** A "baseline is
  broken" report was wrong: the functions existed and were public at the pinned
  commit.
- **2026-10-23 — Windows resource names contained `\`**
  (`block\acacia_door_bottom`): the client built resource names from file
  system paths. Normalise separators when deriving names from paths.
- **2026-10-23 — RCON and operators on the test server.** `ops.json` loads only
  at startup; grant operator over RCON.
- **2026-10-18 — transparent huge pages gave no measurable gain**; rejected.
- **2026-10-17 — inlined perf frames misattribute cost.** Use `--no-inline`
  and caller views before assigning a hotspot.
- **2026-10-16 — relative paths in agent file tools edit the main checkout**,
  not the worktree. Absolute paths always. ([worktrees](worktrees-and-integration.md))
- **2026-10-16 — replacing a shared script in place breaks running callers.**
  Bash reads incrementally; write a copy and `mv` it.
- **2026-10-16 — per-tick fields must not be journaled as full images.**
  Journaling an entity image whenever a per-tick clock changed (egg timers, ages,
  cooldowns, work/sleep clocks) produced about 30 MB of WAL per tick and 15 GiB
  RSS. Store `(value, base_tick)` and derive; test that idle mobs produce zero
  images over N ticks and that live state equals replayed state. WAL recovery
  streams one batch at a time.
- **2026-10-15 — plugin operation receipts are never pruned** by count or age;
  eviction lets a retried operation id run twice.
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
- **2026-10-09 — idle vanilla clients stalled without periodic `SetTime`;
  respawn needed GameEvent 13.** ([live-gate triage](live-gate-triage.md))
- **2026-10-07 — graphical gates fail on a loaded host** (`stale-revision`
  while a cargo matrix ran); run them on a quiet host.
- **2026-10-05 — speculative API fields cost a four-repository revert.**
- **2026-10-03 — hand-concatenated patches glue `diff --git` headers** onto a
  file's last line; deliver `git diff` output only.
- **2026-10-03 — nested `sdk/rust/target` and `.analysis` copies filled the
  disk** when snapshotting worktrees with rsync.
- **2026-10-02 — the agent host was OOM-killed with every agent in it**; all
  heavy commands go through the runner. ([heavy commands](heavy-commands-and-memory.md))

## 2026-08/09

- **2026-09-27 — the focused test loop is build-bound** (about 58 s build,
  0.4 s tests); moving code or splitting test files does not speed rustc.
- **2026-08-28 — never reuse a prior run directory for a "fresh seed" claim:**
  persisted world metadata overrides the configured seed.
- **2026-09 — fixed-seed tuning was stopped by the owner** in favour of owner
  field testing, then varied-seed exploration; terrain-blocked stays blocked
  (now in AGENTS.md).
