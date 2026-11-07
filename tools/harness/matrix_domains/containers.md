# Container matrix adapters

`containers.py` registers all 33 `containers.*` subjects from `scenarios/subjects2.py:6-40`. It uses the shared `Adapter` / `MatrixContext.boundary()` interface; there is no separate runner or action DSL. The 268 container rows in the current matrix select these adapters; 79 select the protocol driver. Registration is not evidence that those rows pass.

## Fixture and oracle contract

- Fixtures occupy x/z 26..40, y 75..86, outside basic-economy's automatic shop zone. Setup uses the server facade's dimension-scoped console commands and never changes selected game mode, permission, dimension, or difficulty.
- Before modifying terrain, setup forceloads the four fixture chunks, teleports into the area, and requires `wait_for_chunk(position)` receipts for all four corners. The initial live double-chest row exposed `Position is not loaded`; no mutation is retried or treated as successful.
- Counts are finite. Commands are followed by received-state observations. Menu slot indices are protocol indices, and the player suffix is counted only once. Missing `menu`, inventory, components, properties, or capacity raises `MissingCapability`; it never becomes an empty/default passing state.
- The adapter calls the shared boundary once while its menu/action is live. After reconnect, block/entity menus are reopened and transient station inputs are restored **from the player's returned finite inventory**, not re-granted. Server lifecycle, permission fixtures, restart/backup proof and network injection remain shared harness responsibilities.
- Spectators retain their selected mode and receive read-only custody checks. Creative XP/lapis expectations differ from survival/adventure; setup does not force survival.
- Timing uses received state or dashboard simulation tick progression. No wall-clock sleeps synchronize actions.
- Each `Check` records expected/actual, vanilla version `26.1.2`, and a subject-specific Minecraft Wiki source URL. Plugin shop behavior cites its local contract instead of claiming vanilla parity.
- Ordinary container deposits additionally issue `data get block 32 80 32 Items[0].count 1`; unknown command blocks as `data_get_block`. It requires Main's rebuilt binary containing DataCommands, not the original Hotfix binary.
- Direct spectator/lectern block interactions require a real integer submission token and received `block_ack_sequence >= token`; Java/native without that surface block explicitly. Generic revision changes never stand in for admission.
- Personal crafting also checks window zero and offhand round-trip. Table crafting removes/restores a recipe input to check result recomputation. Cooking checks decreasing fuel and advancing progress properties. Enchanting checks all three offer bounds and same-seed reopening. Grindstone checks two-tool repair. Beacon checks effect expiry outside the tier-four radius.

## Additional observation capabilities

The baseline shared snapshot is sufficient for simple finite-stock menu transfer. Rich subjects also require:

- `menu.slot_count`, numeric menu properties, and complete sparse menu slots (including player suffix), cursor;
- `stack.components` for potion/enchantment/name/trim/banner component checks;
- `menu.recipes` with recipe index/result, `menu.offers` with costs/result/uses/XP, merchant progress;
- `snapshot.entities` with type/id, effects with id/amplifier, map scale/locked fields, experience totals/levels;
- `read_block_state` for barrel `open` and trapped chest redstone wire `power`.
- `wait_for_chunk(position)` after a fixture teleport; a teleport acknowledgement alone does not prove the new area loaded.

The code uses canonical optional verbs: `close_container`, `open_inventory`, `read_block_state`, `interact_entity`, `toggle_crafter_slot(slot, disabled)`, `container_button`, `rename_item`, `set_beacon`, `select_trade`, `use_item`. Unimplemented methods fail with `client:<verb>`.

Numeric menu property layouts are grounded in the inspected release source: `inventory/furnace.rs:51-59`, `brewing.rs:27-28`, `inventory/stations.rs:72-76,91-92`, and `crafter.rs:512-523`. These are decoding facts, not product behavior oracles. Expected behavior comes from the separately cited vanilla sources.

Pinned 26.1.2 data facts were also read under `/run/media/kaiserroman/games/solaris-work/solaris/data/vanilla/data/minecraft/recipe/`: `chest.json` (3×3 ring of eight planks), `oak_planks.json` (four planks per log), `iron_ingot_from_smelting_iron_ore.json` (200 ticks, 0.7 XP), `cooked_beef_from_smoking.json` (100 ticks, 0.35 XP), `stone_slab_from_stone_stonecutting.json` (two slabs), and `netherite_chestplate_smithing.json` (template + diamond chestplate + netherite material).

The `two_players_same_slot_same_tick` and `break_container_while_open` origins add actions beyond context factors. Until coordinated collision/destruction receipts exist, they explicitly block as `same_tick_admission_fence` and `container_break_while_open_custody`. They must never inherit a passing ordinary transfer check.

## Oracle completeness inventory

The counts below use **subjects**, not matrix rows. Before this continuation the clause audit found 15 complete base-subject adapters and 18 real-but-partial adapters. There were no spec-only subjects and no registration-only stubs.

After filling clauses supported by the existing interface: **0 spec only, 0 registered stubs, 30 complete base-subject adapters, 3 real-but-partial adapters**. “Complete” describes action/oracle implementation, not availability of its driver fields, product correctness, or coverage of all context combinations. Shared `d4cba654` requires own-target and shared-target all-actor batches for N8 and explicit concurrent triples; these domain batches await the actual hook. Ordinary N2 is actor plus received observer evidence. Explicit same-tick/destruction origins remain guarded.

`CLAUSE_GAPS` enumerates all three partial subjects and the exact capability blocking their remaining clauses:

| Subjects | Remaining clause | Capability |
| --- | --- | --- |
| hopper | exact eight-tick transfer cadence | `observation.menu.transition_ticks` |
| brewing_stand | exact 400-tick brew interval | `observation.menu.transition_ticks` |
| plugin_chest_menu | owning plugin receives click | `observation.plugin_menu_click` |

These functions contain real actions and checks. Their otherwise-green result raises the named `MissingCapability` rather than claiming the missing clause passed; an already observed failed check remains a product failure. Dashboard polling is not an exact transition-tick trace. No new server/driver observation fields are fabricated.

The rich-observation batch adds successful anvil material repair and two-item combination, per-operation XP/input debits, trader exhaustion/refused extra payment, and villager restock verified by `data_get_entity` plus reopening the received offer. Merchant fields follow the shared `input1/input2/output` and menu `xp/show_progress` contract. Enchantment/potion numeric IDs are resolved through `snapshot.registries` from received configuration or the supplied authoritative vanilla report, never hardcoded IDs.

Native MCP's actual namespaced enchantment map/potion name is accepted directly; it need not fabricate a numeric registry. Non-storage N2 subjects without a received observer oracle block as `observer_subject_receipt`; storage must record every extra viewer. Property arrays wait for each received property packet, since OpenScreen/content can precede the last crafter property; an incomplete array is not an empty/default value.

The entity-observation batch uses shared `1e20dd5f` packets: copper golem moves 16 items **from** copper chest **to** empty wooden chest (the matrix's “sort into” shorthand is corrected to [vanilla direction](https://minecraft.wiki/w/Copper_Golem#Chest_interactions_and_item_transportation)); loaded minecart moves on powered rails and retains stock; powered/unpowered activator rails disable/enable hopper-cart pull; destroying a loaded boat conserves its 16 items across inventory plus dropped-item entities, including a later no-duplicate check. XP uses real `snapshot.xp` or native XP fields; numeric effect IDs use the authoritative mob-effect registry. These new clauses are implemented, not yet live-proven by a receipt.

Entity-menu writes wait for a received non-player window before inspecting slots. Beacon startup waits for its first actual effect packet; a not-yet-received effect map is never interpreted as an empty map.

Cooking places coal before the input. With input first, the server can consume newly placed coal in the same tick, so awaiting an intermediate fuel stack races valid smelting. The received burn/cook properties and finite output remain the behavioral oracle.

Mount cargo now deposits a finite 16-stone stack in cargo slot 2, checks retention across the requested boundary, withdraws it, and checks empty cargo plus exactly 16 returned stones. The earlier menu-size/chest-debit-only oracle did not prove cargo transfer. Lectern buttons wait for received page 1 and exactly one returned book; raw page property 0 is sourced from `fix-menus/crates/mc-core/src/lectern.rs` open/turn property emissions. These adapter regressions are contract-tested; product-fix live proof belongs to FixMenus.


## Known product gaps and verification

Source-confirmed missing mount/lectern menus and the live-reproduced crafting-table first-click closure are documented in `/run/media/kaiserroman/games/solaris-work/scenarios/domains/containers-bugs.md`. They are failures to implement expected vanilla behavior, not successful skips. A missing client observation can block a run before it reaches a product gap; report those separately.

Focused contract tests: `python3 -B -m unittest tools.harness.test_matrix_containers`. These check subject registration, missing-field fail-closed behavior, exact numeric feedback parsing, sparse capacity, custody counting and boundary enforcement. They do not exercise a server or establish matrix coverage. No Rust crate is touched, so Cargo/clippy is not applicable.

Canonical live command follows the shared `README.md`, using `--subject containers.chest` for supplemental probes or `--matrix-id <exact id>` for a matrix row, `--port 26400 --dashboard-port 26401`, and the release binary. Only finished canonical receipts may enter the ledger; supplemental probes must not be recorded as matrix coverage.

On release SHA `7e7ba95853ca0bdf34ec1cd23c6a6143ba03916da7c664898e95a04727f3f299`, the exact double-chest disconnect row passed five checks (90 slots, deposit 16, reopen other half preserves 16, withdrawal empties storage, total 16); receipt `20261024T034035-protocol-matrix-g_5632sk`. The crafting-table disconnect row failed before disconnect when the first inventory click closed the table; receipt `20261024T034129-protocol-matrix-p7v6hg0q`. Both used the complete ten-plugin set and shared `mem-run.sh -m 3G` admission. Neither result establishes coverage of the other matrix rows.
