# BATTLE-LIVE-RESPAWN-FAR-FROM-FIGHT — fix-hunk map

Fix `f90997cc1` in tree. This map's patch is its **behaviour-only** form (the BED pattern, review
wave2-13 M3/Main): `patch:09c7ef28c66dcaa8cb2fd933751f49b253573a162257dd40289427a1b8205f4c`,
`git apply -R --check` passes at HEAD.

| file | what it does |
|---|---|
| `crates/mc-core/src/battle/live_players.rs` | `respawn_slot`'s ordered pick: the file of his own regiment in contact nearest where he fell, else while the regiment stands any living file of it, else the side's contact nearest where he fell, else `choose_slot`'s centre file with `rule=centre`. |

The reversal is one early return to `choose_slot(.., Some(keep))` with `centre: true` and everything
else (helpers, the `Pick` fields, the log line) still compiled — reversing the *commit's* hunks instead
drops `nearest_file` and the test tree does not build (`E0425`), which is why this form exists.

## Controls

- **cargo**: the row's four `battle::live_players::tests::a_respawn_*` refs must red on the reversal
  (`ledger promote --fix-patch` runs exactly that).
- **graphical**: the scene's `holder_sees_an_enemy_after_respawn` (an enemy within 8 blocks sampled
  from the holder's own client after his death and respawn) is the row's symptom and must be red on a
  pin of the reversed tree; `contact=`/`rule=centre` are strings the fix itself writes and are not
  the control.
