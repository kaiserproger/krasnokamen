# SPECIAL-001 — control map for `patch:53952b4d…`

Row: pot/shelf no storage (`docs/bugs/rows/SPECIAL-001.toml`). Real commit: **`35308579c`**
(`fix(interact): rebase onto 7fe49052 — axe strip/scrape/unwax, sponge, brushing, painting, spawn
protection, dragon egg, special storage`, 33 files, 1143 insertions — a bulk commit carrying six
rows' behaviour).

The commit's own hunks no longer reverse at this head: the pot/shelf dispatch statement it added is
not a standalone line any more. `docs/bugs/promotions/SPECIAL-001.patch`
(`fix_commit = patch:53952b4d6e6da92a1986b8e8d13d89d61ea082a90e3739b6f89c13040c67ec37`) therefore
carries exactly its HEAD equivalent, and `git apply -R --check` passes on the candidate.

## Carried hunk (HEAD equivalent)

| original hunk | HEAD | class |
|---|---|---|
| `crates/mc-core/src/tick/input/interact.rs` `use_item_on`, the `+ if crate::special_storage::use_on(…)* { return; }` statement inserted before `crate::shelf::use_on` | `if crate::special_storage::use_on(state, entity, *hand, *position, *face, *cursor) \|\| crate::chiseled_shelf::use_on(…)` — the patch deletes only the `special_storage` call and keeps `chiseled_shelf` in place | (c) `be01ee35f` (CHISELED-BOOKSHELF) merged the two calls into one condition, so the fix's line does not exist verbatim; nothing is invented, the deletion is of the fix's own statement |

## Omitted hunks

| hunk | class | why |
|---|---|---|
| `crates/mc-core/src/special_storage.rs` (added, 136 lines) | (b) | the kept refs (`interact_parity_decorated_pot_stores_one_item`, `shelf::tests::shelf_checkpoint_native_chunk_restart_preserves_stacks_and_alignment`) call this module, so reversing its creation stops the control from compiling. It stays compiled and imported (`lib.rs:402 mod special_storage;`) and unreferenced by the reversed dispatch — the accepted kept-helper class, README:250-256 |
| `spawn_protection.rs` (added), `tick/input/interact.rs` `dig` spawn-protection and dragon-egg statements, `vanilla_interact.rs` | (a) | PLACE-001's and FALL-001's behaviour; reversing them would red someone else's assertion |
| `painting.rs`, `brushing.rs`, `sponge.rs`, `redstone/containers.rs`, `tick/input/combat.rs`, `tick/auth.rs` | (a) | the axe / sponge / brushing / painting / container rows' behaviour |
| `boundary.rs`, `config.rs`, `core.rs`, `dimensions_recovery.rs`, `entity_loot.rs`, `fire.rs`, `journal.rs`, `lib.rs`, `player.rs`, `registry.rs`, `tick.rs` | (a) | shared plumbing the other behaviours of the same commit need (journal records, entity images, config switches); none of them is pot/shelf storage |
| `crates/mc-edge/*`, `crates/mc-protocol/packets/play/entity.rs` | (a) | server properties and packet shapes other rows own |
| `docs/QUIRK_PARITY.md`, `example.toml` | (a) | docs/example config, no behaviour |

## Proving refs

* `cargo:mc-core:interact_parity_tests::interact_parity_decorated_pot_stores_one_item` — **red at
  `crates/mc-core/src/interact_parity_tests.rs:103`** with the patch reversed
  (`20261125T144102-test-5_em1pk1`: `assertion left == right failed: left: 3, right: 2` — the hand
  keeps all three diamonds, the pot stores none). Green on the candidate.
* `harness:entities:native-v2-ledger-pot-shelf` — assertion `pot_stored_one_item` (the gate now
  reads the client's own count in bounded steps and asserts it, so its verdict is an assertion and
  not a wait timeout, which a ledger control cannot use).
* `cargo:mc-core:shelf::tests::shelf_checkpoint_native_chunk_restart_preserves_stacks_and_alignment`
  is **not** proving: the patch does not touch the shelf module, so it is green with and without the
  fix; it is excluded from `proving_refs` (README:1330-1346).
