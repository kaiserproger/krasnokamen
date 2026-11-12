# Settlements

A settlement is a village the server builds up and runs by itself. A planner lays out and builds houses, workshops, storage, mines and camps from real materials; villagers take building, mining and lumber jobs, keep their work across nights, can be recruited as soldiers, and trade through the settlement economy. Players claim and manage a settlement through server-driven dialogs, with no client mod.

Settlements are native to the core (`mc-core`), not a plugin, and tick with the world. The rule that makes them predictable: nothing is free. Blocks, tools, food and equipment come from real storage and real inventories, and settlement work happens only in loaded, simulated chunks. Module paths, persistence and the plugin boundary are in [settlement architecture](architecture/settlements.md).

## Quick start

1. Join. The server announces `/settlement guide`, an in-game tutorial; `/help settlement` lists the subcommands.
2. Generated vanilla villages become ownerless settlements when their meeting bell loads. Stand inside the village and run `/settlement claim`, or press **Claim settlement** in `/settlement`. The first claim saves your UUID as owner; nobody else can claim it afterwards. Until claimed, an autonomous settlement refuses all management, even from operators. See [villages and jobs](settlements/jobs.md#autonomous-villages).
3. To found your own, run `/settlement create <name> [radius]` (radius 1 to 256, default 48).
4. Put real materials in settlement storage. A founded settlement starts with no stock: certify a storage building first. Generated villages start with a saved bootstrap chest. See [construction](settlements/construction.md#planning-and-streets).
5. Certify buildings you built (`/settlement building certify ...`), or order blueprint buildings with `/settlement build <blueprint>` or the **Projects** screen. See [construction](settlements/construction.md).
6. Founding or claiming automatically binds eligible unbound villagers already inside the settlement's radius. For later unbound arrivals, use `/settlement bind <name>` next to one. Open **Residents**, choose a resident, then choose an eligible workplace. Assignment records the native job; it does not mean the resident has arrived there. See [jobs](settlements/jobs.md#allocating-jobs).

`/settlement layout` prints the street plan of the settlement you stand in.

## Permissions

- Anyone can found a settlement and read lists, info, layout, certificates and dialogs.
- Only the owner or an operator can change an owned settlement. The check repeats on every dialog click.
- `relocate` needs the owner of both settlements, or an operator.
- Autonomous settlements refuse every player management change until claimed.

The full command table with who can run each command is in [dialogs and commands](settlements/menus.md#commands).

## Menus

Right-click a settlement's anchor bell to open the compact hub; other bells only ring. The hub opens the graphical settlement dashboard, existing army and neighbours entry points, and paged help. `/settlement` or `/settlement menu [main|buildings|map|certify|residents|storage|production|army|projects|settings]` remains available for the settlement around you, otherwise the one you own. Labels follow the client language: Russian for `ru*` locales, English for the rest.

- **Dashboard:** item icons lead to the map, buildings, projects, residents, resources and production. Read-only visitors can inspect; management repeats native ownership/operator and player-mode checks at the action.
- **Map:** pan, zoom and overview controls stay with the map; the legend has its own page. Click a coloured cell to inspect its building, street or construction site. Overlaps open a paged chooser containing every feature, not only the tooltip summary. Back from a building restores the map viewport or building-list context that opened it.
- **Buildings:** a certificate-level review is separate from ordering actual blueprint construction. Review the requested target before confirming. Neither certification nor recording an assignment claims that physical work has happened.
- **Projects:** the construction card shows the native project phase, worker and work-unit counter when a native snapshot is available. Missing-material icons retain the full required count even when an icon's stack overlay is capped; select one for its explanation and storage link. Unavailable native status or cause is labelled unknown, not replaced with an invented completion or shortage.
- **Residents:** the census, individual cards and household pages use native residents, jobs and household membership. A home certificate is a claim on its native bounds, not a counted bed or inferred family. Eligible workplace choices use the existing assignment path.
- **Resources and storage:** resource cards distinguish total, reserved and free stock without merging component-distinct stacks. Incomplete storage reads are marked unavailable/partial. Real transfers use the existing fenced storage commit, not a displayed estimate.
- **Production:** inspect the native recipe, alternative/tagged inputs, output, per-craft duration, quantity and worker/automatic binding before confirmation. Existing job pause/resume/cancel operations retain their native authority.
- **Help:** the fief checklist explains only state the domain currently exposes. Unavailable feudal systems are not presented as working civil features.

Cards are snapshots: refresh or reopen to observe later work. Construction confirmations recheck the reviewed target level and queued-order intent; background simulation progress alone does not invalidate them. A changed target or queue must be reviewed again. Existing commands and their permissions remain described in [dialogs and commands](settlements/menus.md).

## Reference pages

| Page | What it covers |
|---|---|
| [Dialogs and commands](settlements/menus.md) | Every screen, permissions, the chat command table. |
| [Construction and buildings](settlements/construction.md) | Blueprint files, planner and streets, Builders, owner orders, certification, natural zones, radius growth. |
| [Villages and jobs](settlements/jobs.md) | Village adoption and claiming, job allocation, rest and routes, miners, lumberjacks. |
| [Production and economy](settlements/production.md) | Production orders, autocraft, delivery, economy commands, recipes, the upgrade warning for existing worlds. |
| [Breeding and food](settlements/breeding.md) | Villager breeding, farmers, wool. |
| [Army](settlements/army.md) | Recruitment, equipment, orders. |
| [Acceptance gates](settlements/acceptance-gates.md) | What the real-client settlement profiles prove. |
| [War, mount and equipment decisions](settlements/OWNER_DECISIONS.md) | Owner-approved values for mechanics not yet implemented. |

Related: [village generation](VILLAGE_GENERATION.md), [server-driven dialogs](DIALOGS.md).
