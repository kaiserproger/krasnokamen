# Pitfalls

Mistakes already made in this repository, each with the rule that prevents it
and the check that catches it. Add one in the same change whenever a mistake is
worth not repeating; fold a new case into an existing entry when the rule is
the same. Dates are the owner's time zone (UTC+7). The rules the owner set are
in [RULES.md](RULES.md).

## Controls and tests

**A fix patch's "before" side was invented, so its control proved nothing.**
In the D123 promotions (2026-11-24) several stored patches carried a synthetic
a/-side: a stub written for the control instead of the real pre-fix code, and
one went red only at a counter the stub zeroed itself. Reviews rejected them;
each was rebuilt from the real pre-image and observed red at the row's own
assertion. The a/-side of a fix patch is the code before the fix, taken from
Git, never code written for the control. Check: every `-` line of the patch
matches `git show <fix_commit>^:<path>`.

**Controls that could not fail were counted as negative controls.** Three
battle promotions (2026-11-24) were rejected because their counters incremented
only on the reverted side or their assertions were constant on every tree; a
cavalry gate asserted "no 45-degree turns" under a 20°/tick cap that made such
turns impossible; two harness checks were `check(..., True)`. A control goes
red at the row's own symptom on the reverted tree and green on the candidate,
through code both trees share. Check: name the observed value that differs
between the two runs; if the assertion's inputs cannot differ, it is no control.

**Synthetic fixtures agreed with our encoder and missed the real format.** The
LZ4 region tests (2026-11) wrote checksums with our own unmasked encoder, while
vanilla masks xxHash32 with `0x0fffffff`, so every valid vanilla LZ4 chunk was
refused and the tests passed. Format and protocol tests read bytes vanilla
produced (a generated world, a wire probe), not bytes we wrote. Check: each
format has at least one fixture from a real vanilla run.

**The tests were green and the feature was broken.** The tests of a new config
key (D52, 2026-11-19) only parsed the key and never saw the value reach the
server; thirteen tests asserted a constant against its own literal
(`RELIABLE_QUEUE_BOUND == 512`). A test drives the path a player or operator
uses and observes the outcome; delete one that passes with the code returning a
default. Check: break the code under test once and watch the test fail.

**A matching recovered row was taken as proof that the WAL recovered it.** A
live-writer hot copy was refused (2026-11-07), and rows that matched did not
show whether the checkpoint, the selected root or the replay produced them. A
recovery claim fences its source (stopped, or a consistent snapshot), shows
what the last checkpoint held and what only the WAL held, and names the root it
opened. Check: `krasnokamen world probe` and `world census` on the stopped world
([offline subcommands](development/constantinople-fixture.md#offline-subcommands)).

**An A/B pass with a negative control was taken to prove the mechanism.**
(2026-11-04) Both show that a change matters, not why. Instrument the cause
before naming it in release text ([release evidence](development/release-and-tags.md#core-release-cut)).

**Spawned was reported as active and sent as accepted.** The one-million-entity
rungs (2026-11-03) died in an 8 GiB cgroup OOM, and a 16k-mob fixture refused a
17 MB checkpoint manifest made mostly of scheduled fluid ticks. A scale claim
needs that rung's crash-and-reopen receipt and counts what the server ticked
and accepted. Check: the receipt names the rung, its entity count after reopen
and its peak RSS.

**A "fresh seed" run reused a run directory.** Persisted world metadata
overrode the configured seed (2026-08-28). A fresh-seed claim starts from an
empty run directory. Check: read the seed back from the world's metadata.

## Claims, commits and pipelines

**Results named a head that held nothing.** Workers reported done with receipts
while their head was still the base and the change sat staged, uncommitted
(2026-11-14). A result names a commit that exists on your branch and contains
the change, and its receipts come from that tree; a gate receipt binds HEAD plus
the uncommitted state, so commit anything unrelated only after the promote.
Check: `git -C <worktree> log -1 --format=%H` is the head you report and
`git -C <worktree> status --short` lists none of your files.

**A pipeline hid the failure it should have reported.** A piped test run was
read as green from the last command's status (2026-11-23), and a pin-citation
scan with `rg -o -h` printed nothing, so a cited crash pin was classified
uncited and planned for deletion (2026-11-27). Use `set -o pipefail`, read the
command's own exit (`${PIPESTATUS[0]}` or the harness `result.json`), and give
every scan a positive control that must be found. Check: break it once (make
the build fail, hide a known citation) and see the check fail.

## The owner host

**A relative path edited Main instead of the worktree.** File tools resolve a
relative path against Main's checkout whatever the shell's `cd` says; a
worker's relative `crates/...` edits landed in Main (2026-10-16). Use absolute
worktree paths in every command and file tool. Check: `git -C <main> status`
shows none of your files; to investigate, look for successful relative edits in
the agent transcript ([creating a worktree](development/worktrees-and-integration.md#creating-a-worktree)).

**Directories were deleted by age, and some were not caches.** A cleanup
(2026-11-16) removed 54 directories under `targets/` because they were idle for
two hours and not on the keep list; they included fixture inputs that are now
lost, a pinned Horizon build, Loader Gradle outputs and perf fixture caches that
reports cited. Delete only with per-item proof: no Git work, cited nowhere, not
on `DISK-KEEP.txt`; build caches go through `slot-gc`. Check: the proof list is
written before the first `rm`, and `tools/dev/wt-done` refuses a tree whose
`.analysis` holds worlds, WAL, stores or crash copies.

**Cited pins and raw WAL were deleted after a scan that did not look
everywhere.** A pin classifier (2026-11-27) searched two receipt lanes; 357 of
the 407 clean pins it deleted were cited elsewhere under `receipts/`, and a
raw-data list removed WAL batches a battle handoff cited as evidence for two
blockers. A rebuilt pin is not guaranteed to match the old binary hash. A
deletion scan covers all of `receipts/`, tracked files and the ledger; failed
runs, controls and cited data are never deleted. Check: the scan finds a known
citation before anything is unlinked.

**A full disk looked like a hang.** The journal died, no subscriber saw its
channel close, and an L2 run hung for 54 minutes (2026-10-09). When a gate
hangs, check free disk first. Check: `df -B1G --output=avail /run/media/kaiserroman/games`.

**A daemon started inside a runner scope kept the scope alive.** An sccache
server started inside a slice kept its scope populated, and orphaned scopes
made admission refuse work on an idle machine (2026-10-24). Never set or
override `RUSTC_WRAPPER`: the runner starts the cache wrapper itself. Check:
`systemctl --user list-units 'krasnokamen-mem-run-*'` shows no scope of yours
after the command ends.

**Numbers taken on a loaded host were wrong.** Graphical gates failed with
`stale-revision` while a Cargo matrix ran (2026-10-07), and an unpinned server
was CFS-throttled at 23 threads under a 6-core quota while its tick tails were
read (2026-10-10). Measure in a quiet `-x` window from a non-incremental pin,
interleave A/B as AB/BA, name the hardware and core count, and publish the slow
cases too; above a load of about 4 only ratios within one session count. Check:
record `uptime` and the pin manifest beside every number
([performance windows](development/performance-windows-and-profiling.md)).

## Code

**Per-tick fields journaled as full images flooded the WAL.** Writing an entity
image whenever a per-tick clock changed (egg timers, ages, cooldowns, work and
sleep clocks) produced about 30 MB of WAL per tick and 15 GiB RSS (2026-10-16).
Store `(value, base_tick)` and derive the current value. Check: idle mobs write
zero images over N ticks, and live state equals replayed state.

**Entity changes that skipped the journal lost or duplicated items.** Changing
persisted fields of an existing entity, or an item stack count, without marking
it changed (`appearance_changed`) kept it out of the WAL: after checkpoint and
replay a payment was journaled but the entity state was not, or a reward
replayed twice (2026-10-13). Every mutation of persisted entity state marks it
changed. Check: the regression runs checkpoint, action and restart with replay,
not only in-memory ticks.

**A player was sent their own equipment update.** The client wrote the
main-hand item into its selected slot (2026-10-14). Equipment updates go to
trackers only. Check: a test that the acting player receives no
`SetEquipment` for itself.

**Speculative API fields cost a four-repository revert.** WIT and API fields
added for a path that was later abandoned had to come out of core, SDK, tools
and seven manifests (2026-10-05). Add a field when a caller needs it. Check:
every new field has a reader in the same change.

**Release content gaps passed narrow checks.** RC2's content bundle lacked the
language file, Loader mixins were stale and NeoForge declared no channel;
only broader player-path checks found them (2026-10-31). A release candidate
passes the player path on every advertised client and loader before the tag.
Check: the [break-it protocol](QA_AGENT_PROTOCOL.md) receipt covers each
advertised client.
