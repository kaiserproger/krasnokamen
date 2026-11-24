# CIVILIZATION-STREET-VERGE — narrowed compiling control (attempted 2026-11-24)

Row fix identity stays `patch:9c2f9605dd3e676f…` (`promotions/CIVILIZATION-STREET-VERGE.patch`);
this file records the *narrowed* reversal tried for the real-client control and why the row is
still open.

## Why the recorded patch cannot be the control

`promotions/CIVILIZATION-STREET-VERGE.patch` reversed at HEAD `3cf141664` does not compile:
it removes `Network::street_index` (layout.rs:992) and two of its call sites, but a later
commit's use at layout.rs:951 survives — `error[E0599]: no method named street_index found …
layout.rs:951` on the control pin (2026-11-24, `wt/D123CivGatesMapCtl`).

- `street_index` and the call at layout.rs:951 — **(c) later commit's use**: the guard's body
  at HEAD (`matches!(cell(...), Cell::Road | Cell::Plaza).then(|| self.index.get(...))`,
  layout.rs:993-995) and the caller `construction_access` (layout.rs:951) are HEAD code a later
  commit added/changed; no HEAD equivalent of the original removal exists, and reversing them
  cannot build. Kept identical on both sides of the narrowed control.
- Every other hunk of the original patch is layout.rs. There is **no a/-side text** in the
  narrowed patch: its only pre-image line is `                        Cell::Road | Cell::Plaza`
  (layout.rs:833, identical to the real pre-fix blob) and its only post-image line adds
  `| Cell::Verge`.

## The narrowed reversal (`CIVILIZATION-STREET-VERGE.narrowed.patch`)

One hunk: the walk's expansion filter at layout.rs:833 goes back to
`Cell::Road | Cell::Plaza`, i.e. roads and the plaza again without walkable verges. It applies
in reverse on the candidate (`git apply -R --check`), builds, and pins
(`pins/a8667a74ae84-c6c9232c-server-x86_64-unknown-linux-gnu-incr`).

## Result: the narrowed control is GREEN, so this scene does not discriminate

`settlement-map-qa --scenario verge` on the narrowed control passed
(`20261124T035833-settlement-map-qa-moan3xgz`, tree `a8667a74a` + the reversed hunk). With the
verge excluded from the walk, the funded Mine is still admitted at the same lot: the wall spans
one road column at the storage's z, but the street grid reaches every frontage column around it,
so no frontage is hidden. The unit test that made this a defect uses `street_plateau`: every
lot column except one is stone to the sky, so the only frontage is the one behind the wall.
A client fixture would have to reproduce that plateau *and* its one open lot, which this scene
does not.

Consequence: the row stays `open`. The candidate receipt `20261124T035325` is real client
evidence that the behaviour is present on this tree (Mine admitted, status page
`Active projects: 1/8`, wall intact), but a gate that cannot go red on a reversal is not a
check (docs/bugs/README.md rule 4, Main 2026-11-24).
