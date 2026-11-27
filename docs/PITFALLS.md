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
uncited and planned for deletion (2026-11-27). The opposite also bit deploy
scripts: under `set -e` a `| grep -v` filter with empty output exits 1 and stops
the script. Use `set -o pipefail`, read the command's own exit
(`${PIPESTATUS[0]}` or the harness `result.json`), and give every scan a positive
control that must be found. Check: break it once (make the build fail, hide a
known citation) and see the check fail.

**A relayed number was wrong.** An owner decision (D60, 2026-11-19) quoted
"96/96 green" for a driver that number never described and an 8/8 helper count
for the failed script, both passed along from other agents' reports. Never
forward an agent's or advisor's claim as fact: read the receipt or the
`file:line` yourself. Check: every number in a report names the receipt it
came from.

**Release notes claimed more than the receipts showed.** RC4's Windows note
rested on three runs of which only two used fresh profiles, and three of the four
RC5 review rejections were claim errors, not code. Every note claim names its
receipt and run count, read from the receipt itself; say "3 of 3", not "always",
and "not run" for an unrun platform. Check: `.analysis/releases/<tag>/claims.md`
maps each sentence to a receipt before review
([release text](development/release-and-tags.md#core-release-cut)).

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
everywhere.** A pin classifier (2026-11-27) searched two receipt lanes. Of the
407 deleted clean pins, 357 had matching short hashes elsewhere under `receipts/`;
that includes bare commit mentions, not just pin citations. Some receipts named
deleted pins explicitly. A raw-data list also removed WAL batches cited by a
battle handoff as evidence for two blockers. A rebuilt pin is not guaranteed to
match the old binary hash. A deletion scan covers all of `receipts/`, tracked
files and the ledger; failed runs, controls and cited data are never deleted.
Check: the scan finds a known citation before anything is unlinked.

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

## Live gates and plugins

Start a failing gate from its receipt
([receipts and failure diagnosis](AGENT_TOOLING.md#receipts-and-failure-diagnosis)),
and classify it (fixture, environment, stale binary, product) before you change
product code.

**A gate ran a stale or foreign binary.** `cargo test` does not rebuild the
`krasnokamen` binary; pooled target directories once served another tree's
artifacts (an `unresolved import` of a known symbol, new tests silently
absent); `touch crates/*/src/main.rs` to force a rebuild created empty bin
targets (E0601) and an empty `crates/xtask/src/lib.rs`. A gate counts only for
the binary, plugin package and content it ran. Check: the receipt names the
server binary's path and SHA-256, and it matches the pin you meant.

**A broad fix let read-only actions mutate.** A "status poll completes pending
work" reconciliation let a pending start intent leak into a mutation path. A
reconciliation fix names the exact case it handles (action, intent, state,
epoch), and read-only actions never mutate on a generic condition. Check:
restore the broad condition once and watch the fix's test fail.

**Every plugin command went silent after one step.** The host retires an
instance after an invalid guest answer; causes seen were an operation id outside
`[a-z0-9_-]` or over `MAX_SCRIPT_ID_BYTES` (64), and a storage value over
`MAX_PLUGIN_STORAGE_VALUE_BYTES` (4096) built from an unbounded list. Bound the
value in the plugin (or store a digest) and unit-test the worst case. Check:
`grep 'plugin instance retired' server.log`.

**A WIT change broke every prebuilt plugin.** After a WIT change each old
`plugin.wasm` failed at load (`failed to convert function to given type`); a
guest's API version must equal `COMPONENT_PLUGIN_API_VERSION`. Rebuild every
first-party package and bump each `plugin.toml` in the same change, and bump the
API version only when a surviving consumer needs it; a capability name core's
manifest parser does not know is a core bug. Check: the standard pack loads on
the changed core.

**A shared plugin candidate gave passes that proved nothing.** Gates that ran on
a candidate other agents wrote into, or rebuilt in place, could not be tied to
one core state, and the pack fingerprint fails when inputs change mid-build.
Build each candidate into a new directory with
`../solaris-default-plugins/tools/build_standard_pack.py`, one per core state,
and hand over write ownership explicitly; passes on mixed candidates are
provisional. Check: the candidate's `content-receipt.json` input hashes match
the sources you changed.

**Admission was reported as completion.** An `Applied` move receipt proves the
slot and route check only; the body could still never arrive, and refusal-only
receipts said nothing about the feature working. Arrival or cancellation comes
from a later query of native state; acceptance needs world evidence (block
states before and after, item transfers), not reply strings or HP loss. Check:
the gate asserts the world change ([durable operation receipts](plugins/OPERATIONS.md#native-server)).

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

**An idle player's client stalled.** The server sent nothing to an idle player,
and the vanilla client stopped. Send `SetTime` periodically, as vanilla does
every 20 ticks. Check: an idle client stays connected past the keepalive window.

**A same-dimension respawn left the client on the loading screen.** The respawn
burst lacked GameEvent 13 (start waiting for chunks), and the server did not
forget and resend the view. Send both. Check: a real-client respawn reaches
Play with chunks rendered.

**A dropped best-effort packet left a mob invisible forever.** The server had
optimistically marked the viewer's cache before the packet was dropped. A drop
undoes the mark. Check: a test that drops the packet and sees it sent again.

**Encoding the whole join view on the connection task starved the client.** It
took about 11 s in a debug build. Chunk streaming belongs to the pump, not the
connection task. Check: join-to-Play time on a debug server stays bounded.
