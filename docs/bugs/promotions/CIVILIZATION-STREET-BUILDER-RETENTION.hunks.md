# CIVILIZATION-STREET-BUILDER-RETENTION — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIVILIZATION-STREET-BUILDER-RETENTION.patch`
(`patch:8c7807a9008fb24d…`), the row's `fix_commit`. It reverses on the committed
tree (`git apply -R --check`).

## What the fix is (origin `898af4725`, `settlements/construction.rs`)

A completed road or lamp (street work, `layout::infra`) releases its builder like
a finished furnace; before the fix only the furnace branch released, so street
work kept the only reachable adult bound as Builder and starved extraction and
wool.

## HEAD equivalent

HEAD restructured `finish_project` (project list, `access::take_project`, a
cloned project): the original pre-image ("release only for the furnace, complete
the infra project without releasing") no longer exists textually. The HEAD
equivalent keeps the completion path and only skips the release for non-furnace
infra:

```rust
if project.blueprint == furnace::NAME {
    release_builder(state, id, project.builder);
}
```

The other nine construction.rs hunks of the same slice are **omitted (a)**: they
are STREET-VERGE / EMPLOYED-SHEARING / INACTIVE-SHEEP / funded-kind behaviour.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the real-client red is the
integrated sweep's control run, not claimed here.

## a/-side audit (B5, 2026-11-24)

Mechanical check: `git show <origin>^:<file>` vs the file after `git apply -R`
of this patch at the audit head; script and full output in
`receipts/D123/D123CivGates/a-side-audit.md`. Every line outside the origin
commit's own change, with the commit that last changed it and whether the
line is in `origin^:file`:

- `crates/mc-core/src/settlements/construction.rs` `@@ -2854,9 +2854,7 @@ pub(crate) fn finish_project(state: &mut TickState, id: SettlementId, structure:` (adapted; origin `ab417a937`, discovered origin):
  - a/-side `if project.blueprint == furnace::NAME {` — 898af4725 fix(settlements): site search follows verges; working adults shear active sheep; in origin^: yes
  - a/-side `release_builder(state, id, project.builder);` — no commit; in origin^: no

HEAD-side lines whose last change is another commit are that later commit's
form: reversing them is the HEAD drift a control must reverse to reach the
pre-fix decision (rule (c)), not part of the fix. An a/-side line that is not
in `origin^` is an adaptation this control relies on; each such line is named
in the list above and changes no behaviour beyond the pre-fix decision.
