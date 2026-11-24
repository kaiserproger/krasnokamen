# ISSUE-1-09-BUILDERS-CANNOT-BUILD-GATES-OR-MOST-WORKSHOPS — HEAD re-break control patch

Patch: `docs/bugs/promotions/ISSUE-1-09-BUILDERS-CANNOT-BUILD-GATES-OR-MOST-WORKSHOPS.patch`
(`patch:847e1fb125e326f5…`), the row's `fix_commit`. It reverses on the committed
tree (`git apply -R --check`).

## What the fix is (origin: the shipped fix patch, kept in `docs/bugs/fix-patches/ISSUE-1-09-BUILDERS-CANNOT-BUILD-GATES-OR-MOST-WORKSHOPS.patch`)

No single commit is the origin: the blueprint JSON assets landed across several
commits (`18f715367` among them) while the `SOURCES` registration came with them, and
the shipped fix patch is the only statement of the whole change — that is why the
`fix-patches/` copy was kept rather than moved or deleted (checked: its added lines
are not all contained in `18f715367` or any other one commit).

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

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/blueprints.rs` `@@ -32,7 +32,7 @@ pub type ItemId = u32;` (adapted; origin `23aad8f43`, discovered origin):
  - a/-side `const SOURCES: [&str; 13] = [` — no commit; in origin^: no
- `crates/mc-core/src/settlements/blueprints.rs` `@@ -40,7 +40,20 @@ const SOURCES: [&str; 13] = [` (adapted; origin `0bb22922d`, discovered origin):
  - HEAD `include_str!("../../assets/blueprints/barracks.json"),` — 18f715367 feat(settlements): Millénaire-style construction, autonomy, army and village fixes (issue #1); in origin^: yes
  - HEAD `include_str!("../../assets/blueprints/watch_tower.json"),` — 18f715367 feat(settlements): Millénaire-style construction, autonomy, army and village fixes (issue #1); in origin^: yes

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
