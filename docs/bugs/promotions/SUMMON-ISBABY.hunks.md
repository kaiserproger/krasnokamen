# SUMMON-ISBABY: patch-mode fix identity

Original fix: `69650602b` (`fix(summon): rebase onto 7fe49052 — Age for water/horse families, IsBaby for
zombie/piglin/zoglin`; BUGS.md names the pre-rebase fix `1e5de7f5`) — the `IsBaby` field stopped being
limited to `zombie_villager` and became `crate::entity_life::uses_is_baby(name)`.
`fix_commit = "patch:1d24d42295a3d114…"` names the HEAD equivalent of this row's non-test hunk in
`docs/bugs/promotions/SUMMON-ISBABY.patch` (a diff from the pre-fix form to the candidate; `git apply -R`
restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SUMMON-ISBABY --fix-patch docs/bugs/promotions/SUMMON-ISBABY.patch --dry-run`.
Rule (docs/bugs/README.md, step 1): every original non-test hunk is carried unless it is another row's or
owner's behaviour, API the kept tests call, or code a later commit rewrote.

## Carried

- `command_summon.rs @@ -206,8 +206,14 @@ pub(crate) fn supports` — the `IsBaby` binding becomes
  `crate::entity_life::uses_is_baby(name)`; HEAD `crates/mc-core/src/command_summon.rs:353`.

## Original non-test hunks (complete)

```
## 69650602b non-test hunks (kept test files: crates/mc-core/src/command_summon_age_tests.rs)
- crates/mc-core/src/blueprint.rs: -1337,39 +1337,7
- crates/mc-core/src/command_summon.rs: -206,8 +206,14; -511,6 +517,15; -577,7 +592,7; -585,7 +600,7; -609,7 +624,7; -623,3 +638,7
- crates/mc-core/src/core.rs: -1532,7 +1532,15; -1561,7 +1569,7; -2670,6 +2678,11; -2894,6 +2907,12
- crates/mc-core/src/entity_life.rs: -160,7 +160,7; -737,7 +737,7; -820,8 +820,8; -832,6 +832,7; -840,6 +841,8; -847,6 +850,8; -854,6 +859,33
- crates/mc-core/src/journal.rs: -2385,7 +2385,7
```

## Not carried

- `entity_life.rs -854,6 +859,33` — (a) SUMMON-AGE's own families (`skeleton_horse`/`zombie_horse`,
  `squid`/`glow_squid`), carried by that row's control; the other `entity_life` hunks are the
  `uses_is_baby` helper lists and earlier ageability lists — (b) the carried line calls
  `uses_is_baby`, so the helper must stay for the control to compile (its family lists are the behaviour
  the kept test asserts for piglin/zoglin and the zombie family, and they are what makes the reversed
  binding red only for this row's species).
- `command_summon.rs`'s other hunks (`-511,6`, `-577,7`, `-585,7`, `-609,7`, `-623,3`) — (a)/(b) the
  `Age` plumbing of SUMMON-AGE and the summon-field validation the kept test drives.
- `core.rs` summon plumbing and `journal.rs` persistence — (a) other rows' command plumbing; `blueprint.rs
  -1337,39 +1337,7` is a rebase artefact with no HEAD equivalent (c).
- Test files — (b) kept.

## Proving refs (file:line of the failing assertion)

- `cargo:mc-core:command_summon::age_tests::summon_is_baby_zombie_family`
  (`crates/mc-core/src/command_summon_age_tests.rs:226`) and
  `cargo:mc-core:command_summon::age_tests::summon_is_baby_piglin_and_zoglin` (`:237`) — with the binding
  reversed, `IsBaby` is refused for `piglin`/`zoglin` and the zombie subclasses
  (`Entity does not support summon NBT field IsBaby`, the review's observed red).
- Client ref `harness:entities:native-v2-summon-age` is green on the gate-wave tree
  (`20261123T163717-native-v2-summon-age-x8bekgh1`); unrunnable in the control.
