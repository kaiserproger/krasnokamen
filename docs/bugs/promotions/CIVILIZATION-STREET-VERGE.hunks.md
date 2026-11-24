# CIVILIZATION-STREET-VERGE — the recorded control (narrowed), row still open

Row fix identity `patch:55ef8747f4ea750339d9214de19f019521d57d94bca47c8a173af0dfa5e86009`
(`promotions/CIVILIZATION-STREET-VERGE.patch`). One patch only: the earlier full-slice patch
was deleted (it removed `Network::street_index`, which a later commit's call site still uses, so
it could never build a control at HEAD); this narrowed reversal is the one the row keeps.

## Why the narrowed hunk is the whole control

One hunk: the walk's expansion filter at `layout.rs` goes back to
`Cell::Road | Cell::Plaza`, i.e. roads and the plaza again without walkable verges. Its only
a/-side text is `Cell::Road | Cell::Plaza` (identical to the real pre-fix blob from
`898af4725`); its only post-image line is the same with `| Cell::Verge`.

- `street_index` and the caller at layout.rs:951 — **(c) later commit's use**: the guard body at
  HEAD (`matches!(cell(...), Cell::Road | Cell::Plaza).then(|| self.index.get(...))`) and the
  caller are HEAD code a later commit added; the original patch's removal of them has no HEAD
  equivalent that builds, so they are identical on both sides here.
- The slice's other production files are other rows — **(a)**: `construction.rs` =
  STREET-BUILDER-RETENTION, `shepherd.rs`/`brain` = EMPLOYED-SHEARING,
  `construction_availability.rs` = INACTIVE-SHEEP-TARGET.

## Reverse-apply evidence

`git apply -R --check docs/bugs/promotions/CIVILIZATION-STREET-VERGE.patch` on the committed
tree succeeds, and the reversed tree builds (`pins/a8667a74ae84-c6c9232c-server-…`).

## Result: a real reversed-tree run exists and it is GREEN (2026-11-24)

Correction (Main, 2026-11-24; confirmed by D123CivGatesF): the earlier quoted run
`20261124T035833-settlement-map-qa-moan3xgz` is **not** a reversed-tree control. Its
`result.json` records `tree.head = a8667a74a`, `tree.dirty = false` and
`tree.sha256 = e3b0c442…` (the empty diff), the same clean tree and client binary as the
candidate pass `…T035325`; an earlier draft of this file and the row called it "tree
a8667a74a + the reversed hunk", which was wrong.

A real reversed-tree run was then made here: this patch reversed on `a57a56706e`
(`wt/D123Civ-ctl-verge`, fingerprint `5123419d…`, server pin
`a57a56706e85-5123419d-server-…`) and `settlement-map-qa --scenario verge` run against it —
**PASSED**, `verge_connected = true` and
`verge_project_admitted_on_the_status_page = true`
(`wt/D123Civ-ctl-verge/.analysis/validation/20261124T113936-settlement-map-qa-oali0iw_`,
tree dirty, diff sha `5123419dcb6f`).

So the scene does not discriminate: with the verge excluded from the walk the funded Mine
is still admitted at the same lot, because the wall spans one road column at the storage's
z while the street grid reaches every frontage column around it. The unit test that made
this a defect uses `street_plateau`: every lot column except one is stone to the sky, so the
only frontage is the one behind the wall. A client fixture would have to reproduce that
plateau *and* its one open lot, which this scene does not.

Consequence: the row stays `open`. The candidate receipt `20261124T035325` is real client
evidence that the behaviour is present on this tree (Mine admitted, status page
`Active projects: 1/8`, wall intact), but a gate that cannot go red on a reversal is not a
check (docs/bugs/README.md rule 4, Main 2026-11-24).
