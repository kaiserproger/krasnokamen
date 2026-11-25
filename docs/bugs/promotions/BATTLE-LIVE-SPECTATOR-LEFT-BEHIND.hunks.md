# BATTLE-LIVE-SPECTATOR-LEFT-BEHIND — fix-hunk map

Fix `c6fa94da3` in tree; this map's patch is its **behaviour-only** form,
`patch:58f02bb7959a01a3a4ca24e750f090e138eee5e0e057c9210f0cbd0d2e66f29d` (`docs/bugs/promotions/BATTLE-LIVE-SPECTATOR-LEFT-BEHIND.preimage.patch`), which
reverses at HEAD.

| file | what it does |
|---|---|
| `crates/mc-core/src/battle/live_players.rs` | the two `follow(...)` calls `c6fa94da3` added — one in the respawn path (`update`, after the kit re-place) and one in the placement path (`place`) — so a bound spectator is re-placed behind his holder on every respawn and re-arm. |

Adaptations: none in behaviour. The commit's own added lines are deleted; `follow()` and
`place_behind()` stay in the file (the pre-fix tree has them only as reachable-from-`battle spectate`
code), so the reversal compiles (`warning: function `follow` is never used`). No stub or early return
is introduced.

## Control

- **cargo**: the row's three refs are expected to be *green* on this reversal (they test the rebind
  helpers through `battle spectate`), so per docs/bugs/README.md (proving_refs) they are structural and
  the flipping ref is the scene;
- **graphical**: `spectator_camera_followed_the_respawn` (the camera eye within 8 blocks of the holder
  after his respawn) is the row's symptom and must be red on a pin of this reversed tree.
