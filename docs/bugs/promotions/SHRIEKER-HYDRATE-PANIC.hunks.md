# SHRIEKER-HYDRATE-PANIC: patch-mode fix identity

Original fix: 82434561 + 3b5e0be7. `fix_commit = "patch:eedff8b85ba150da7b9973ea563bd9c13212821eedcccc9ad1e9f5a094b358f9"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/SHRIEKER-HYDRATE-PANIC.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote SHRIEKER-HYDRATE-PANIC --fix-patch docs/bugs/promotions/SHRIEKER-HYDRATE-PANIC.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried:
- 82434561 shrieker.rs -278,12: `decode(..).unwrap_or_else(|e| panic!("sculk_shrieker at ..."))` -> `match decoded { Err => refuse_record; return }` = HEAD shrieker.rs:281-289.
- 3b5e0be7 tick.rs -1266,6 (its only non-test hunk): `sync_chunk_block_entities(state)` at tick entry + comment = HEAD tick.rs:1296-1298.
Fix in several commits (docs/bugs/README.md, step 1): 82434561 replaces the panic in the shrieker's runtime decode, and 3b5e0be7 validates block entities before the tick's vibration hydration, which is the tick path in the row title. Reversing 3b5e0be7 on its own stays green on the candidate (receipts/PromoteCorruptChunks/controls/README.md M3, `.analysis/validation/20261111T133938-test-ettba_8s`), because 82434561's refusal covers the vibration path, so both go in one patch. With the whole patch reversed, both `tick::hydration::tests::corrupt_target_block_entities_are_rejected_before_ticks` and `runtime_readers_refuse_the_chunk_of_an_unvalidated_record` panic at shrieker.rs:284:37 (receipts/PromoteCorruptChunks/patch-mode/local_controls.out:85-86). Proving ref: `corrupt_target…` (the tick path). `runtime_readers` calls `shrieker::changed()` directly (hydration_tests.rs:99).

Not carried: as CONDUIT; a4493fd0 shrieker.rs -244,17/-416,8 (validate_record at hydration) earlier layer.

## Original non-test hunks

```
## 82434561 non-test hunks (kept test files: crates/mc-core/src/datapacks_tests.rs, crates/mc-core/src/dimensions_tests.rs, crates/mc-core/src/tick/hydration_tests.rs, crates/mc-edge/src/server_lifecycle_tests.rs)
- crates/mc-core/src/beacon.rs: -78,8 +78,8 fn decode(bytes: &[u8]) -> Result<Vec<(String, Tag; -89,16 +89,25 pub(crate) fn validate_chunk(state: &TickState, ch; -199,7 +208,9 pub(crate) fn open(; -253,6 +264,10 pub(crate) fn configure(; -274,7 +289,6 pub(crate) fn configure(; -317,8 +331,17 pub(crate) fn run(state: &mut TickState) {
- crates/mc-core/src/boundary.rs: -850,6 +850,12 pub enum PlayerChange {; -1079,3 +1085,13 pub enum PlayerChange {
- crates/mc-core/src/conduit.rs: -240,8 +240,20 pub(crate) fn run(state: &mut TickState, intents: 
- crates/mc-core/src/sculk.rs: -365,11 +365,19 pub(crate) fn changed(state: &mut TickState, pos: 
- crates/mc-core/src/shrieker.rs: -278,12 +278,15 pub(crate) fn changed(state: &mut TickState, pos: 
- crates/mc-core/src/tick.rs: -41,7 +41,7 mod entity_lifecycle;; -802,7 +802,8 impl TickState {; -813,6 +814,27 impl TickState {
- crates/mc-core/src/tick/hydration.rs: -85,6 +85,18 pub(crate) fn sync_chunk_block_entities(state: &mu
- crates/mc-edge/src/plan/player_changes.rs: -63,6 +63,22 pub(crate) fn plan_routed_player_changes(; -231,17 +247,7 fn plan_change(change: &PlayerChange, viewer: &mut; -302,6 +308,13 fn plan_change(change: &PlayerChange, viewer: &mut
```

```
## 3b5e0be7 non-test hunks (kept test files: crates/mc-core/src/tick/hydration_tests.rs)
- crates/mc-core/src/tick.rs: -1266,6 +1266,9 fn run_dimension_tick_inner<'a>(
```
