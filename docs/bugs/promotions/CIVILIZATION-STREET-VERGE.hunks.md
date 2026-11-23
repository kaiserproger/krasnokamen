# CIVILIZATION-STREET-VERGE — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-STREET-VERGE.patch`
(`patch:9c2f9605dd3e676f…`), the row's `fix_commit`. It reverses on the
committed tree (`git apply -R --check`).

## What the fix is (origin `898af4725`, `settlements/layout.rs` only)

`layout::Network` walks roads, plaza **and their walkable verges** (steps of at
most one block on loaded ground) instead of only `Cell::Road | Cell::Plaza`, and
`street_index` keeps a door fronting an actual street while the walk may detour
over a verge. That is what lets a funded workshop reach a lot past a generated
road obstruction.

## Why only this file

The other production files of the same slice belong to other rows and are
**omitted (a)**: `settlements/construction.rs` (street-builder release and the
funded-kind scan), `settlements/construction_availability.rs` (active-sheep
wool sources), `villager/shepherd.rs` and `villager/brain.rs` (worked shearing).
This patch carries only the layout hunks; test files stay in the control.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the real-client red is the
integrated sweep's control run, not claimed here.
