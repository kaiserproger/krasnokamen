# DIMENSION-SWITCH-KNOCKBACK-DISCONNECT — fix-hunk map

Fix commit `5810902c7` (2026-11-15). The production patch is the behaviour half only,
`patch:27ac5e8f6ab948317e0e344fd7ae3d3f3dc86ed2aa069874aab6e7bcb55ac58f`
(`git diff 5810902c7^..5810902c7 -- crates/mc-edge/src/server/pump.rs`), 14 removed lines.

| file | what it does |
|---|---|
| `crates/mc-edge/src/server/pump.rs` | `close_skipped_knockback` no longer raises `dimension-switch-explosion-knockback` when a viewer's queued explosion is not in *his* current dimension: the writer already fences queued explosions by dimension (`explosion_queue`'s dimension reset), so the pump's cross-check disconnected a traveler for an old-world blast that was correctly discarded. The `skipped-explosion-knockback` counter test stays. |

Not changed by the patch: the tests (`crates/mc-edge/src/explosion_queue_tests.rs`, where the row's
two refs live — the row's module paths were stale and now name the file), the dimension-reset fencing
itself, and the docs commits that came with `5810902c7`.

## Control

The pump hunk reverses cleanly on HEAD (`git apply -R --check` passes). Reversing it restores the old
disconnect branch, so `explosion_queue_tests::same_tick_dimension_switch_discards_only_old_world_knockback`
must red at its `skipped-explosion-knockback` / connection-survives assertion. (Seen or not: recorded
with the receipt in HANDOFF-17.)

## Real-client scene

`native-v2-dimension-switch-knockback` — an explosion knocks the player back on the same tick a portal
teleport switches dimension; the connection survives, the client stays in the new dimension, no
old-world knockback. Fuse alignment: `/tick freeze`, then `/tick step` until the TNT's fuse is one tick
from the detonation that coincides with the portal's travel tick; the portal cooldown and the client's
own dimension change are read from the client state (`dimension`), the connection from the absence of a
disconnect in the server log and the client staying in `playing`.
