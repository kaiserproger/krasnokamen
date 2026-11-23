# CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP — HEAD re-break control patch

Patch: `docs/bugs/promotions/CIV-SEED24-CROSS-VILLAGE-TREE-SWEEP.patch`
(`patch:deb17c038aeff04d…`), the row's `fix_commit`. It reverses on the fixed
tree (`git apply -R --check`).

## What the fix is (origin `197beb170`, `villager/lumber.rs`)

The lumberjack search starts at the worker's feet and visits nearby columns in
square rings (`Progress::search_origin` + ring `column_at`), restarting at the
feet after a harvest; a natural-zone grove starts its cursor at the proved grove
footprint. Before the fix the sweep walked the whole settlement radius square in
fixed order, so a lumberjack walked past nearby trees to a far target.

## HEAD equivalent

The re-break restores the pre-fix shape at HEAD: no `search_origin` field
(`Progress`/`StoredProgress`/`From`), grid `column_at(anchor, radius, index)`,
the radius-square `search()` with the chunk-strip skip, no cursor reset in
`replant`, and `work`'s natural-zone cursor preset removed. The broken tree was
compile-checked; see HANDOFF-7 for the receipt.

## Commit-chain note

The re-break was accidentally included in commit `c832d7f14e` (whose subject is
the ISSUE-1-09 patch) and restored by the next commit, which also lands this
control patch. The final tree carries the fix; the control patch is the exact
reversal.

## Reverse-apply evidence

`git apply -R --check` on the committed tree succeeds; the graphical red is the
integrated sweep's control run, not claimed here.
