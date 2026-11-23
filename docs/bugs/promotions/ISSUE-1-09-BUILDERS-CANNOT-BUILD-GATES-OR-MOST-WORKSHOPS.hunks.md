# ISSUE-1-09-BUILDERS-CANNOT-BUILD-GATES-OR-MOST-WORKSHOPS — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-09-BUILDERS-CANNOT-BUILD-GATES-OR-MOST-WORKSHOPS.patch`
(`patch:847e1fb125e326f5…`), the row's `fix_commit`. It reverses on the committed
tree (`git apply -R --check`).

## What the fix is

The gate and the workshop blueprints were registered in
`settlements/blueprints.rs` `SOURCES` together with their JSON assets (the shipped
fix patch adds `armorer/butcher/cartographer/cleric/fisherman/fletcher/leatherworker/librarian/toolsmith/weaponsmith`,
`gate`, and a longer `barracks`/`watch_tower`). Without the registration the
planner can never order them, so `harness:buildings-qa:{gate,barracks,watch_tower,
armorer,butcher,cartographer,fisherman,…}.L*` can never be built or upgraded.

## HEAD equivalent and sequencing with ISSUE-1-16

The shipped patch is against `SOURCES: [&str; 9]`; HEAD has 26 entries. This patch
is the same registration at HEAD: reversed, `SOURCES` keeps only
`house, storage, farm, mason, shepherd, lumber_camp, mine, wall` and the Spanish
set — the gate, barracks, watch_tower and the ten workshops disappear.

`wall.json` is **not** removed here: it belongs to ISSUE-1-16's control patch, so
the two rows stay disjoint (Main asked to sequence them in one chain; the chain
is ISSUE-1-16 → this patch, and applying both would remove the wall as well).

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the graphical red is the
integrated sweep's control run, not claimed here.
