# JUKEBOX-HYDRATE-PANIC: patch-mode fix identity

Original fix: a4493fd0. `fix_commit = "patch:8887a3a2c197a934cb14cff0393707e3196698a1f2440bd1abed36b9e8dd87a1"` names the HEAD
equivalents of that fix's non-test hunks in `docs/bugs/promotions/JUKEBOX-HYDRATE-PANIC.patch` (a diff from the
pre-fix form to the candidate; `git apply -R` restores the pre-fix code). Re-audit:
`python3 -m tools.harness ledger promote JUKEBOX-HYDRATE-PANIC --fix-patch docs/bugs/promotions/JUKEBOX-HYDRATE-PANIC.patch --dry-run`
(plus `--client-receipt` for a real-client row). Rule (docs/bugs/README.md, step 1): every original
non-test hunk is carried unless it is another row's or owner's behaviour, API the kept tests call,
or code a later commit rewrote; each exclusion is named below.

Carried:
| original hunk | HEAD equivalent | reversed form |
|---|---|---|
| tick.rs -4443,130: hydration span `crate::jukebox::hydrate_chunk(..).unwrap_or_else(\|error\| panic!("resident jukebox rejected: {error}"))` (span line 116-122) | tick/hydration.rs:53-54 `jukeboxes: crate::jukebox::decode_chunk(..).map_err(\|e\| e.to_string())?` (the owner line that replaced it) | `.unwrap_or_else(\|error\| panic!("resident jukebox rejected: {error}"))` |
| jukebox.rs -2,7: `ChunkPos` import dropped | jukebox.rs:5 | import restored |
| jukebox.rs -138,30 (register part): decode the chunk record with `panic!("resident jukebox rejected")` -> `Jukebox::default()` + doc comment | jukebox.rs:159-163 | decode-with-panic restored, doc comment removed |

Not carried:
- jukebox.rs -121,12 and the rest of -138,30: `hydrate_chunk` -> `decode_chunk`/`apply_chunk` split. (b)/(c): structural; HEAD tick/hydration.rs `Decoded`/`apply` are built on it; the panic site itself is carried above.
- tick/hydration.rs (new file, 177 lines), tick.rs -37,6 (`mod hydration`), -589,6 and -1346,8 (`MergeSpawn`/`write_block` funnel), the rest of -4443,130: the single hydration owner and block-write funnel that every later block-entity change extends (refuse_record 82434561, chiseled/special storage, trial spawner, vault). (c): no compiling HEAD equivalent; this is why the `fix_commit` tree cannot run the refs (the tests live in `hydration.rs`'s `mod tests`).
- crafter.rs, shelf.rs: carried in their own rows. beacon.rs, conduit.rs, sculk.rs, shrieker.rs (validate at hydration), bees.rs, bottles.rs, brewing.rs, campfire.rs, copper.rs, copper_golem.rs, effects_trigger.rs, explosion.rs, lectern.rs, monster_spawner.rs, redstone/containers.rs, redstone/mod.rs, shulker.rs, tnt.rs, trial_spawner.rs, vault.rs, weather_lightning.rs: (a) other owners' policies or write_block call sites.

## Original non-test hunks

```
## a4493fd0 non-test hunks (kept test files: crates/mc-core/src/bees_tests.rs, crates/mc-core/src/brewing_tests.rs, crates/mc-core/src/campfire_tests.rs, crates/mc-core/src/crafter_tests.rs, crates/mc-core/src/datapacks_tests.rs, crates/mc-core/src/jukebox_tests.rs, crates/mc-core/src/monster_spawner_tests.rs, crates/mc-core/src/native_checkpoint_crash_tests.rs, crates/mc-core/src/redstone/containers_tests.rs, crates/mc-core/src/shelf_tests.rs, crates/mc-core/src/shulker_tests.rs, crates/mc-core/src/tick/hydration_tests.rs, crates/mc-core/src/trial_spawner_tests.rs, crates/mc-core/src/vault_tests.rs, crates/mc-core/src/world_conversion/old_world_import_tests.rs)
- crates/mc-core/src/beacon.rs: -70,6 +70,25 pub(crate) fn levels(state: &TickState, pos: Block; -78,14 +97,7 fn image(state: &TickState, pos: BlockPos) -> Vec<
- crates/mc-core/src/bees.rs: -580,21 +580,22 pub(crate) fn place_from_item(; -1403,8 +1404,7 pub(crate) fn use_on(
- crates/mc-core/src/bottles.rs: -241,8 +241,7 pub(crate) fn use_on(
- crates/mc-core/src/brewing.rs: -184,34 +184,45 fn is_stand(registry: &CoreRegistry, id: u32) -> b
- crates/mc-core/src/campfire.rs: -56,11 +56,12 fn lit(state: &TickState, pos: BlockPos) -> Option; -78,10 +79,13 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/conduit.rs: -152,14 +152,16 fn image(state: &TickState, pos: BlockPos) -> Resu
- crates/mc-core/src/copper.rs: -214,8 +214,7 pub(crate) fn use_on(state: &mut TickState, actor:
- crates/mc-core/src/copper_golem.rs: -679,8 +679,7 pub(crate) fn revive(state: &mut TickState, pos: B
- crates/mc-core/src/crafter.rs: -28,12 +28,13 fn is_crafter(registry: &crate::registry::CoreRegi; -47,13 +48,24 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/effects_trigger.rs: -168,8 +168,7 fn cobwebs(state: &mut TickState, pos: [f64; 3], r
- crates/mc-core/src/explosion.rs: -147,8 +147,7 pub(crate) fn apply(
- crates/mc-core/src/jukebox.rs: -2,7 +2,7; -121,12 +121,12 impl Jukebox {; -138,30 +138,29 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/lectern.rs: -51,12 +51,13 fn accepts(state: &TickState, book: &ItemStack) ->; -70,20 +71,21 pub(crate) fn hydrate_chunk(; -93,11 +95,21 pub(crate) fn hydrate_chunk(; -134,8 +146,7 fn set_property(state: &mut TickState, pos: BlockP
- crates/mc-core/src/monster_spawner.rs: -122,12 +122,12 impl Spawner {; -144,11 +144,14 pub(crate) fn hydrate_chunk(; -159,7 +162,6 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/redstone/containers.rs: -170,15 +170,18 pub(crate) fn register(state: &mut TickState, pos:; -193,30 +196,56 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/redstone/mod.rs: -485,12 +485,28 fn hydrate(state: &mut TickState) {
- crates/mc-core/src/sculk.rs: -301,13 +301,16 pub(crate) fn checkpoint(state: &mut TickState) {
- crates/mc-core/src/shelf.rs: -183,11 +183,12 pub(crate) fn scheduled(state: &mut TickState, pos; -201,21 +202,17 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/shrieker.rs: -244,17 +244,19 pub(crate) fn validate_record(bytes: &[u8]) -> Res; -416,8 +418,7 fn set_shrieking(state: &mut TickState, pos: Block
- crates/mc-core/src/shulker.rs: -44,11 +44,12 fn decode(bytes: &[u8], items: &ItemRegistry) -> R; -64,13 +65,9 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/tick.rs: -37,6 +37,8 mod container_access;; -589,6 +591,13 pub struct MergeSpawn {; -1346,8 +1355,7 impl TickState {; -4443,130 +4451,6 fn set_lit_prop(
- crates/mc-core/src/tick/hydration.rs: -0,0 +1,177
- crates/mc-core/src/tnt.rs: -100,8 +100,7 pub fn prime_block(state: &mut TickState, pos: Blo
- crates/mc-core/src/trial_spawner.rs: -209,11 +209,12 impl Spawner {; -230,10 +231,10 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/vault.rs: -26,11 +26,12 fn persist(state: &mut TickState, pos: BlockPos, v; -47,10 +48,10 pub(crate) fn hydrate_chunk(
- crates/mc-core/src/weather_lightning.rs: -39,8 +39,7 pub(crate) fn strike_blocks(state: &mut TickState,
```
