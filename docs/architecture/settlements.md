# Native settlement architecture

Settlements are native `mc-core` tick authority, not a first-party plugin
simulation. Player-facing commands and behaviour are in the
[settlements guide](../SETTLEMENTS_GUIDE.md). The plugin API exposes bounded
queries and requests; it does not own the native resident or physical inventory.

## Code map

All paths below are under `crates/mc-core/src/settlements/` unless noted:

- `registry.rs`, `mod.rs`: settlement records, identities and binding.
- `allocator.rs`: native resident home, job-site and meeting-point allocation;
  `crates/mc-core/src/villager/` owns resident brains and state.
- `buildings.rs`, `construction.rs`, `autonomy.rs`, `blueprints.rs`: certified
  buildings, construction and generated-village adoption.
- `households.rs`, `districts.rs`, `production.rs`, `economy.rs`, `commerce.rs`,
  `caravan.rs`, `army.rs`: native domain policy and execution.
- `inventory.rs`: physical container custody; `commands.rs`, `ui.rs` and
  `ui_menus.rs`: native command and presentation entry points.
- `persistence.rs`: record encoding and validation;
  `crates/mc-core/src/plugin_settlements.rs`: plugin request/response admission.

Each dimension owns its settlement records; a global monotonic sequence allocates
IDs, which are not reused when a record is deleted. A villager's native
`VillagerState::settlement` is membership authority, not a duplicate roster.
Binding uses the existing entity, not a replacement plugin NPC. Housing
allocation uses resident POIs and shares the native tick decision/path budget
with villager brains; denied work does not authorize loading arbitrary chunks.
Certified building work points own the corresponding job slots.

## Durability and plugin boundary

Touched settlement post-images and deletions share the dimension-wrapped tick
WAL. Checkpoints store individual `settlement/<id>` rows and the shared
`settlement_sequence` high-water row, not a single unbounded registry blob.
The villager membership column is persisted with the entity in the same
recovery boundary. Recovery validates settlement references instead of silently
clearing them; unsupported old plugin settlement state does not become a
second native authority. Building state lives in its settlement record.

`crates/mc-script/wit/plugin.wit` defines `query-settlement` and
`request-settlement-action`; `plugin_settlements.rs` routes the requests to the
native owner. Guests receive handles and outcomes rather than direct access to
records or entity slots. Native admission revalidates identities and custody.
The shared [persistence contract](persistence.md) governs replay and backups.
