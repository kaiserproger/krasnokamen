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


## The control as built (REVIEW-wave2-14 Major-2, Main's ruling)

`docs/bugs/promotions/BATTLE-LIVE-RESPAWN-FAR-FROM-FIGHT.preimage.patch` (`patch:0cf17053a18f903a5e067a51671ebf4322e659e6a0b639c558d268ec5e6a48a8b22ac22eeafd24fec986da4c447101331e48b401f959d232`) turns HEAD's
`respawn_slot` back into the decision `reenter` took at `f90997cc1^` — the historical a-side, taken from
`git show f90997cc1^:crates/mc-core/src/battle/live_players.rs` — with that commit's own `contact_file`
helper. Only these lines are adaptations to symbols that exist at HEAD (no behaviour invented):

| a-side line | adaptation | why |
|---|---|---|
| `contact_file`: `in_contact(battle, bi)` | `in_contact(&state.entities, battle, bi)` | HEAD's signature carries the entity store |
| `respawn_slot`'s tail | `Ok(Pick { slot, contact, reason: None, centre })` instead of returning `(regiment, file)` | `Pick` is the type the fix introduced; the decision tree above it is the historical one |

Measured with the same scene and harness: candidate pin `f75930bf7ffc` gives
`min_enemy_distance = 11.17` (`20261125T012343-…-b75mwfj8`), the reversed pin
`61836aebd089-3e1fa324` gives **30.17** (`20261125T020437-…-flawmwf2`, red on
`holder_sees_an_enemy_after_respawn`), and trial-7's symptom is ~230 blocks. Bound: 20 blocks.
Note the reversal also prints `contact=true` — the pre-fix already had `contact_file`; the fix's own
change is the ordered fallback (own regiment, then own files, then the side), which is what the
distance measures.
