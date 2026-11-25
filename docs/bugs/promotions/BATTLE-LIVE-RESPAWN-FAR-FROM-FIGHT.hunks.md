# BATTLE-LIVE-RESPAWN-FAR-FROM-FIGHT — fix-hunk map

Fix `f90997cc1` in tree. There is NO behaviour-only patch: the earlier `09c7ef28` was a hand
re-break (its a-side never existed in history) and is withdrawn per REVIEW-wave2-14 Major-2. The
control is the fix's real parent tree `f90997cc1^`.

| file | what it does |
|---|---|
| `crates/mc-core/src/battle/live_players.rs` | `respawn_slot`'s ordered pick: the file of his own regiment in contact nearest where he fell, else while the regiment stands any living file of it, else the side's contact nearest where he fell, else `choose_slot`'s centre file with `rule=centre`. |

The reversal is one early return to `choose_slot(.., Some(keep))` with `centre: true` and everything
else (helpers, the `Pick` fields, the log line) still compiled — reversing the *commit's* hunks instead
drops `nearest_file` and the test tree does not build (`E0425`), which is why this form exists.

## Controls

- **cargo**: the four `battle::live_players::tests::a_respawn_*` refs cannot be built from the
  ledger's reversal (E0425 `nearest_file`); if they are green on the parent pin they are excluded as
  non-proving per docs/bugs/README.md (proving_refs), and the graphical control carries the row.
- **graphical**: `holder_sees_an_enemy_after_respawn` — the enrolled holder's own client position
  against the enemy side in the server's battle image (8 blocks) — must be green on the candidate pin
  and red on the parent-tree pin `f90997cc1^`; `contact=`/`rule=centre` are strings the fix writes and
  are not the control.
