# BEACON-RUNTIME-PANIC: patch-mode fix identity

Original fix: 82434561. `fix_commit = "patch:c83e5cc831c4ea2804935279a476838d750ac17c933730c55f35305c2ec934e1"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/BEACON-RUNTIME-PANIC.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote BEACON-RUNTIME-PANIC --fix-patch docs/bugs/promotions/BEACON-RUNTIME-PANIC.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried (all six beacon.rs hunks of the commit):
| original | HEAD | reversed |
|---|---|---|
| -78,8 doc of `validate_chunk` | beacon.rs:81-82 | old doc |
| -89,16 `image(&TickState) -> Vec` with `.expect("hydration validated resident beacon")` -> `image(&mut) -> Option` refusing via `refuse_record` | beacon.rs:92-111 | old `image` |
| -199,7 `open`: `let fields = image(..)` -> `let Some(..) else return` | beacon.rs:211-213 | old |
| -253,6 `configure`: image read moved before payment | beacon.rs:267-270 | removed there |
| -274,7 `configure`: old `let mut fields = image(..)` after the sync updates | after beacon.rs:291 | restored there |
| -317,8 `run`: `has_chunk` skip + `let Some(..) else continue` | beacon.rs:334-344 | old `let mut fields = image(..)` |

Not carried: as CONDUIT. Earlier layer: a4493fd0 beacon.rs -70,6/-78,14 (validate_chunk at hydration; the row title's `read_network(..).expect`/`panic!` are that commit's pre-image).

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
