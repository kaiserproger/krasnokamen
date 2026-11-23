# VILLAGER-RATION-PATH-STARVATION — forward-ported fix patch

Forward patch: `docs/bugs/fix-patches/VILLAGER-RATION-PATH-STARVATION.patch`
(`patch:37387f76c070961b39fbfa14b80bbfe20f396a158ab8b2f1bc352db000558975`),
the row's `fix_commit`. It applies in reverse on the candidate `wt/D123Civ` at
`f9a8d45596696880fc6a0c9d92f5cd1758002651` (`git apply -R --check`), so the
promotion's automatic control reverses only its two hunks.

## Why patch mode

The row's original commit is `01767ab8714af7c27792790897f885f40e2e2971`
"fix(villager): keep ration searches from starving delivery paths". The patch
supersedes that commit as the reversal target: `brain.rs` was rewritten around
it (the `tick` body moved from line ~782 to ~982 and `v.deferred`/offscreen
work landed between), and the commit also carries the field's storage, which
must stay at HEAD.

## Commit hunks → HEAD equivalents

| 01767ab87 hunk | HEAD equivalent | disposition |
|---|---|---|
| `villager/brain.rs`: `ration_due` gains `&& state.tick >= resident.ration_at` | `brain.rs:1036`, in the patch | **included** |
| `villager/brain.rs`: on `ration_due`, `v.ration_at = state.tick.saturating_add(40)` with its comment | `brain.rs:1053-1057`, in the patch | **included** |
| `villager/mod.rs`: the `pub(crate) ration_at: u64` field (and its `0` init) | present at HEAD, untouched by the patch | **omitted, reason (b)**: `settlements/construction.rs:2763` reads `v.ration_at` in the drain-duty trace and `villager/brain/social.rs:835` reads and `:852` writes it, so removing the field stops the control compiling. The patch's a-side keeps the field and only drops this row's two uses, exactly the pre-fix behaviour |
| `villager/persistence.rs:472`: `ration_at: 0` at the default | present at HEAD, untouched by the patch | **omitted, reason (b)**: same field; the initializer cannot exist without the declaration |
| `villager/brain_tests.rs` (53 test lines) and `villager/persistence_tests.rs` (2 lines) | present | kept (test) |
| `docs/settlements/breeding.md` (2 lines) | at HEAD | kept (docs) |

No a/-side adaptation: the patch's a-side is the HEAD text with only the two
included hunks' lines removed.

## Proving ref

`cargo:mc-core:villager::brain::tests::exhausted_spare_bed_search_does_not_starve_a_lumber_delivery`
— with the patch reversed a spare-bed search that exhausts the path budget runs
on every brain tick again, so the carried log is never delivered inside the
scenario's tick bound; the control is red at its delivery assertion
(`promote.json`, `negative_control`).
