# BED-OCCUPIED-PROPERTY-NOT-MIRRORED — fix-hunk map

The production patch `BED-OCCUPIED-PROPERTY-NOT-MIRRORED.patch` is now the fix's **behaviour** only —
`crates/mc-core/src/sleep.rs` (the mirror and its writer) and `crates/mc-core/src/tick/dimension.rs`
(one call after the merge) — sha256
`fb6d7c78c3ad229b655fddb7960cb96e47224e53c7e3a24c960eec760c079361`, which is the digest the
re-promotion `20261124T205132-ledger-promote-x9e6a0hj` ran with on `0a785659af`. The earlier digests
(`d3314283…`, `06c5dace…`, `2d890e2a…`) are superseded; the sections below record why each was replaced.
The `TickState::occupied_beds` field and its `HashSet::default()` initialiser stay in the control tree
(deliberately kept: the kept test file reads them; the field without the mirror is inert).

## The fix

| file | hunk |
|---|---|
| `crates/mc-core/src/tick.rs` | `TickState::occupied_beds: HashSet<BlockPos>` — the derived set of beds the last pass mirrored, never persisted |
| `crates/mc-core/src/tick/state.rs` | its `HashSet::default()` initialiser |
| `crates/mc-core/src/sleep.rs` | `mirror_occupied_beds` (wanted = players' `sleepers` ∪ sleeping residents' `home`; clears a previously occupied bed that lost its sleeper) and `set_bed_occupied` (writes `occupied` on **both halves**, only when it differs, and never on a bed an accepted write already targets) |
| `crates/mc-core/src/tick/dimension.rs` | the call **after `state.merge`** and before the block phase |

## Why after the merge (review B1, 2026-11-24)

A break/replace intent carries `block_expect` for both bed halves and the merge rejects a whole intent
whose expect no longer matches (or whose cell was already written this tick). A mirror that ran before
the merge therefore invalidated a same-tick break of that bed. Running it after the merge, and skipping
a bed pair whose cell is already in `merge_block_writes`, makes the accepted write win: a broken bed is
never re-occupied and a same-tick break is accepted. (The first version ran inside `sleep::run`, after
the block phase, so its write landed a tick late and could re-apply an occupied bed over a break — the
v1 defect the block-phase ordering had already fixed in the other direction.)

## a/-side

The real pre-image: no `occupied_beds` field, no mirror function, no call. No adaptation line; the
tests live in `sleep_tests.rs` and are not part of the patch.

## Control

Reversing the patch reds the two proving refs at their own assertions
(`crates/mc-core/src/sleep_tests.rs:1300` "the sleeper's bed must read occupied" and `:1321`
"the resident's bed must read occupied"; the multi-line `assert!` macro's panic line is its opening
line; observed `20261124T100301-test-6_b51pwu` on the first version of the reverse tree). The B1
regression now also proves the CAS direction:

- `a_bed_broken_in_the_tick_its_player_lies_down_is_not_restored` — one tick with the lie-down and a
  `Dig::Finish` on the head; the break is accepted and the next tick does not restore the bed;
- `a_bed_broken_when_the_property_clears_is_not_restored` — the player is up, the mirror wants
  `occupied=false`, the same-tick break wins and the bed stays broken.

## Vanilla parity (review non-blocker, checked)

`BedBlock.updateShape` copies `OCCUPIED` to the connected half, so both halves carry the state; the
palette has `part=foot, occupied=true` states (`crates/mc-data/data/required_blocks.json`, e.g.
`minecraft:black_bed` id 2271), so the pair write is legal. The mirror writes both.

## Recorded gaps (not closed here)

- `occupied_beds` is runtime state. A checkpoint taken while a player sleeps, followed by a crash or a
  stop before any tick, leaves `occupied=true` in the saved world with an empty derived set; the next
  wake cannot clear it. Residents restore because their `sleeping` is persisted. Gameplay is unaffected
  (`use_bed` derives occupancy from `state.sleepers`, never from the property). A cheap follow-up is to
  re-derive `occupied_beds` on load by scanning the loaded beds once.
- The pass walks the chunk-indexed resident list every tick (one `row()` per resident). It is bounded
  by the resident count and empty in non-settlement worlds; deriving from the brain's sleep transitions
  would remove the scan and is left as an optimisation.
- The property is client-visible: promotion still needs a scene that reads the bed head during sleep
  (`OWNER-DECISIONS-NEEDED.md` item 2).

## Regenerated 2026-11-24 (`patch:d3314283…`, was `patch:06c5dace…`) — superseded by `fb6d7c78`

The recorded patch stopped reversing on the candidate: its `accepted()` helper was the v1 linear scan,
while `71261486d` (review N4) made it one pass-local `FxHashSet<BlockPos>`. A stale patch is refused by
the ledger ("does not apply in reverse on the candidate"), so the patch was regenerated from git
history, not edited: `git apply -R --3way` of the old patch (three files clean, `sleep.rs` conflicted),
the conflict region being exactly the two added functions, then `git diff -R` against HEAD restricted
to the four production files. The a/- side is therefore the real pre-image (no `occupied_beds`, no
mirror, no call) and the b/- side is HEAD after `71261486d`. Verified: `git apply -R --check` passes on
`292c3b7a0`.

## Resident (villager) client proof — review wave2-8 M5 (2026-11-25)

The row is about a *sleeping resident*, but the first scene covered only the player. The resident arm is
`native-village-occupied-bed` (scene-shot, windowed native): an unbound villager claims the roofed bed,
its brain sets `sleeping`, and the scene now asserts the server's own block predicate on **both halves**
(`execute if block ... [part=head,occupied=true]` / `[part=foot,occupied=true]`) plus free after the
player's one-tick `use` wakes it. Receipt on the promotion tree (pin `660717828dd4`, client
`530556e864c2`, windowed native):

`.analysis/validation/20261124T182026-native-village-occupied-bed-9mz_u_tp` — PASS 77.5 s;
`resident_bed_head_occupied_contains_0`, `resident_bed_foot_occupied_contains_0`,
`resident_bed_head_free_after_contains_0`, `resident_bed_free_after_contains_0`,
`villager_occupies_bed_*`, `occupied_bed_wakes_villager_*`, `villager_returns_after_cooldown_*` all true.

The player arm was re-taken on the same pin with the new `bed_foot_free_before` predicate (m7):
`.analysis/validation/20261124T181951-native-bed-occupied-state-bnymsftm` — PASS 31.4 s
(`bed_head_free_before`, `sleeper_lies_down`, `bed_head_occupied`, `bed_foot_occupied`,
`day_wakes_the_sleeper`, `bed_head_free_after`, `bed_foot_free_after`, every `*_contains_0` true).

## Graphical control — review wave2-8 m9 (2026-11-25)

The ledger's own control worktree has no registered pin, so `promote.json` records the graphical
control `unrunnable` (allowed: the cargo reds carry the control). The manual control below supplies the
missing real-client red: a detached worktree at `660717828dd4` with the four pre-image files (the
sleep/tick/dimension/state files at `178456759b`, i.e. without the M6 doc lines and the M8 early return,
neither of which touches bed occupancy) and the fix patch reversed with `git apply -R` (clean), so
`grep occupied_beds` finds nothing and the mirror call is gone.

Pin: `pins/660717828dd4-1be2cc9d-server-x86_64-unknown-linux-gnu` (built by `tools/dev/pin-release
--no-incremental` on that worktree, server binary sha256 `054800eb3094…`). Run on the same client pin
(`530556e864c2`, windowed native):

`.analysis/validation/20261124T200149-native-bed-occupied-state-o1dd47nk` — **FAIL** 37.6 s at
`native-bed-occupied-state assertion failed: bed_head_occupied_contains_0`; the client's own
`read_block` of the head while the sleeper lies reports `properties.occupied = "false"`
(`bed_head_during_sleep.json`), and the pre-sleep `bed_head_free_before` / `sleeper_lies_down`
assertions still pass — the scene ran, the bed state did not follow the sleeper. That is the row's
symptom on a real client with the fix removed.

So the row's proof now has: cargo red at `sleep_tests.rs:1300`/`:1321`, the manual graphical red above,
and the passing candidate receipts for both the player arm
(`20261124T181951-native-bed-occupied-state-bnymsftm`) and the resident arm
(`20261124T182026-native-village-occupied-bed-9mz_u_tp`).


## Review wave2-9 Minor-1 — first regeneration (`patch:2d890e2a…`, superseded: it did not compile as a control)

The recorded `patch:d3314283…` stopped reversing once `51e8c1c77` (the empty-pass early return in
`sleep.rs`) and the `tick.rs` doc line landed: `git apply -R --check` refused at
`crates/mc-core/src/tick.rs:442` and `crates/mc-core/src/sleep.rs:295`. Every commit between the
pre-image and HEAD that touches the four production files is one of the bed fix's own commits
(`34af8faf8`, `44c0c66a3`, `865c34a92`, `71261486d`, `51e8c1c77` — `git log 34af8faf86^..HEAD --
sleep.rs tick.rs tick/dimension.rs tick/state.rs`), so the patch is the whole fix's non-test hunks:

```
git diff 34af8faf86^..HEAD -- crates/mc-core/src/sleep.rs crates/mc-core/src/tick.rs \
    crates/mc-core/src/tick/dimension.rs crates/mc-core/src/tick/state.rs
```

sha256 `2d890e2a90ebff6b78218b5faf6798d5b94ecac43474e14a19117300e5553c25`, a/- side = the real
pre-image before `34af8faf8` (no `occupied_beds`, no mirror, no call), b/- side = HEAD.
`git apply -R --check` passes on `6e177ea0e` (verified). The row's `fix_commit` now names this digest;
the promotion JSON keeps the digest the promote actually ran with (`d3314283…` on `85269b467`) — it is
a receipt, not rewritten. The early return is part of the reverse control too (it only skips a pass
whose wanted set is empty, so reversing it cannot change an observed bed state).

## Review wave2-9 Minor-2 — the resident branch's graphical negative control

`.analysis/validation/20261124T201909-native-village-occupied-bed-xo4zzd56` — **FAIL** 73.3 s on the
already built reversed pin `pins/660717828dd4-1be2cc9d-server-x86_64-unknown-linux-gnu` (the four
files at `178456759b`, patch reversed; server binary sha256 `054800eb3094…`), client pin
`530556e864c2`, windowed native:

`scene-shot assertion failed: resident_bed_head_occupied_contains_0` — the scene reached the sleeping
resident (`villager_occupies_bed_*` passed) and the server's own `execute if block …
[part=head,occupied=true]` predicate did not match, i.e. the resident's bed stayed free with the fix
removed. Both split branches of the row (player and resident) now have their own real-client red.

## Re-generated for a compiling control (review wave2-9 Minor-1 closure; this is `fb6d7c78…`)

The five-commit patch (`2d890e2a…`) reversed cleanly but its control tree did not compile:
`51e8c1c77` added a test that reads `rig.state.occupied_beds`, and reversing `tick.rs`/`tick/state.rs`
removes the field, so the kept test file fails `E0609` (two errors; the promote's
`ledger-0-list` log `20261124T203802-ledger-promote-3h627vx9`). The patch is therefore the fix's
**behaviour** only:

```
git diff 34af8faf86^..HEAD -- crates/mc-core/src/sleep.rs crates/mc-core/src/tick/dimension.rs
```

sha256 `fb6d7c78c3ad229b655fddb7960cb96e47224e53c7e3a24c960eec760c079361`; a/- side = the pre-image of
both files (no `mirror_occupied_beds`, no `set_bed_occupied`, no call in `tick/dimension.rs`), b/- side
= HEAD. The `occupied_beds` field and its `HashSet::default()` initialiser stay (they are derived state
the kept tests read; the field without the mirror is inert), and that is the only adaptation — named
here per the review's rule. `git apply -R --check` passes on the promotion tree.
