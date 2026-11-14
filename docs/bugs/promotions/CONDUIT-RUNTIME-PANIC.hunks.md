# CONDUIT-RUNTIME-PANIC: patch-mode fix identity

Original fix: 82434561. `fix_commit = "patch:b4d2de2e4c6b42baf2b7b316d2186080140bfd565e765b5a533d3ec5f8f4f825"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/CONDUIT-RUNTIME-PANIC.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote CONDUIT-RUNTIME-PANIC --fix-patch docs/bugs/promotions/CONDUIT-RUNTIME-PANIC.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried: conduit.rs -240,8: `let old = image(state, pos).unwrap_or_else(|err| panic!("{err}"))` after `frame_size` -> `has_chunk` skip + `match image { Err => refuse_record; continue }` = HEAD conduit.rs:272-285.

Not carried: beacon.rs, shrieker.rs (own rows); sculk.rs -365,11 (sculk sensor/catalyst reader, (a) no row here); tick.rs -41,7 (`pub use refuse_record`) and tick/hydration.rs -85,6 (`refuse_record` helper): shared by the readers that stay fixed in this control, (a); boundary.rs, tick.rs -802,7/-813,6, mc-edge plan/player_changes.rs: outbox `TextBurst`, (a) another row (the reason the `fix_commit` tree does not compile: kept datapacks_tests uses `TextBurst`). Earlier layer not in this commit: a4493fd0 conduit.rs -152,14 (validate at hydration).

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
