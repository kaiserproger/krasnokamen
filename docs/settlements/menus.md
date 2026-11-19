# Settlement dialogs

This page explains the settlement screens and their chat commands.

Next: [construction](construction.md), [jobs](jobs.md), [production](production.md), and [army](army.md).

## Open and permissions

Right-click the settlement's anchor bell to open its main screen; the bell also rings. Other bells only ring. Alternatively, use `/settlement` or `/settlement menu [main|buildings|map|certify|residents|storage|production|army|projects|settings]`. The command selects the nearest settlement containing you, or one you own if none contains you. The dialogs are [server-driven](../DIALOGS.md).

Anyone can view; only the owner or an operator can manage a claimed settlement. Permissions are rechecked on each click. An autonomous settlement must first be claimed while standing inside its radius, even for an operator to manage it.

The client locale selects the language: `ru*` uses Russian; all other locales use English. Item, blueprint, profession and settlement names use their game-data spelling. Labels below are English.

## Main

A claimed settlement shows its owner, radius, working and idle residents, building counts and storage. An unclaimed settlement shows its resident and building counts, **Claim settlement**, **Buildings**, **Residents & jobs** and **Projects**.

## Map

Open **Map** from the claimed main screen, **Buildings**, or `/settlement menu map`. The 13×13 grid is north-up; the scale below it gives the square of blocks represented by each cell. Green marks territory, dark grey marks outside, orange marks every active construction site and magenta marks you when visible. Certified buildings have type-specific colours. The legend names the visible features and counts buildings by kind.

The street plan is drawn under buildings: grey **Road (paved)**, pale **Road (planned)** and light blue **Lamp (planned)**. Hover for details; building hovers include type, number, level and corners. Click a building to open its icon page or a construction site to open project status. Street features have hovers but no click action. When features overlap, the site takes priority over buildings, which take priority over streets.

The map is a snapshot: **Refresh** redraws it. **Buildings** and **Back** are below the legend; scroll to reach them. With no buildings, the legend says **No buildings yet.**

## Buildings

The list shows certificates, levels, job slots and next-level requirements, with upgrade buttons for managers. **Certify new building** opens a form: choose **Building type**, enter the two inclusive opposite corners as `x y z`, and set **Height above corners** to extend above the higher corner. Alternatively stand at each corner and use **Corner 1 here** and **Corner 2 here**; reopen `/settlement menu certify` after walking to the second corner. **Certify** reports missing requirements on failure. See [construction](construction.md) for building rules.

**Icons** opens a chest of buildings. Click one for its next-level requirements, workplaces and, where available, **Upgrade to level N** or **Open storage**. A free workplace opens the resident selection; a missing workstation is marked. A map building click opens the same page.

## Residents & jobs

This screen lists filled and free job slots and where residents work. Use **Assign crafts** for the icon-based assignment screen; see [jobs](jobs.md) for work behavior.

## Storage

**Storage** shows combined founding and certified storage as item icons, with stored and reserved amounts. Managers move real items:

- Left-click an item to take up to a stack onto an empty cursor; right-click takes half, rounded up. Shift-click sends up to a stack to your inventory.
- Shift-click your own stack to deposit it, or click the storage grid with a held stack; right-click deposits one item.
- Production and construction reservations cannot be withdrawn. Your own reserved inventory items cannot be picked up, thrown or stored below their reservation floor.
- A transfer can be partial when space is limited. If nothing fits, the transfer is refused; spectators can browse but inventory moves only resync.

## Assign crafts

Select a resident icon and then a certified workplace with a free live workstation. The resident walks to the workstation and takes the craft on arrival. A free workplace on a building page also lets you choose a resident.

Assignment is refused for children, nitwits, soldiers, builders outside builder huts, residents busy with a production order, or a traded resident changing craft. It is also refused if the resident already works there, the building has no jobs, no free place has a live workstation, or the resident or building no longer exists. Dismiss soldiers on the [Army](army.md) screen first; traded residents can move to the same craft.

## Production orders

View and manage orders from **Production orders**; see [production](production.md).

## Army

Manage the roster and orders from **Army**; see [army](army.md).

## Projects

View and request construction from **Projects**; see [owner build requests](construction.md#owner-build-requests).

### Construction text contract (D8)

Up to eight projects build concurrently and up to eight owner orders wait in
entry order (`MAX_PROJECTS = 8`, `MAX_ORDERS = 8`). An unfunded head order keeps
autonomous admission waiting, but a funded later owner order can start. A
site-refused order stays at its original position while later funded orders
are tried. Materials already needed by active sites are not available twice.

English chat replies (blueprint identifiers, not display names):

- Started: `Started house construction.` (unchanged).
- Queued: `Queued house: <reason>` (unchanged; reason describes this request,
  not another order at the head).
- Full: `Construction queue is full: MAX_ORDERS=8.`
- `/settlement build cancel [index]` cancels one pending order, using the
  one-based index shown on screen; omitted index means 1 (the head).
  Success remains `Cancelled pending construction order.` No pending order:
  `No pending construction order.` Invalid index:
  `No pending construction order #<index>.`

The **Construction** screen and its status dialog list **all** active projects
and pending orders, never just the first. The status body starts with
`Active projects: <count>/8` and `Pending orders: <count>/8`, followed by
`Head: <display name> L<level>` or `Head: none`. Each active project has these
four consecutive lines (the corner is its stable site identity):

```text
Storage L1 · Building
5/100
-12 70 24
Builder: <uuid or unbound>
```

The phase is the live preparation/build phase, not a constant; progress is the
structure ledger's completed/total cells. Each pending order has one line:
`Pending #<index>: <display name> L<level> · <reason>`, with indices 1..count.
Existing material icons remain below the project and order lists. For example,
`Pending #1: House L1 · not enough: minecraft:white_wool x3` and
`Pending #2: Farm L1 · busy`. Existing funding/site reasons retain their spelling.
The map gives each active site a hover naming its blueprint, level and corner;
overlapping map cells retain all site descriptions. Its legend also lists each
site as `Active site: <display name> L<level> · <x> <y> <z>`.
Clicking any active site opens the complete Construction status. The summary
count/head does not replace the project list.
The Construction chest has one icon per active site (facts in its lore, click
for its map) and one icon per pending order (click to review cancelling that
exact indexed intent). The clock opens the complete status dialog; material
icons remain paged below the lists. A visitor can inspect but cannot cancel.

The `village-owner-order-qa` gate observes two waiting orders, a funded later
order lifting over an unfunded head, the head fencing autonomous admission, two
simultaneously advancing sites, all sites on the map, the same set after restart,
the named queue limit and indexed cancellation. Its D8 negative control runs
the **unchanged gate** against a separate tree with the parallel cap reverted
to one and its own `pin-release` binary. No environment switch skips commands
or manufactures a failing observation.
The gate follows exactly one Farm and one Storage from their accepted requests
to new active-site identities. Any additional site, including another Farm,
fails the autonomous-admission fence; each checked sample must retain a free
slot under the eight-project contract. Count parsing accepts the displayed
capacity so the cap-one control reaches `two_projects_advance`, rather than
failing to parse its status screen.

## Settings

**Settings** shows the name, radius and revision. The owner or an operator can enter a new name and select **Rename**.

## Commands

Names contain 1–64 ASCII letters, digits, `_` or `-`. For management commands below, owner/operator means the owner of the claimed settlement or an operator; an autonomous settlement must first be claimed. Operator-only transit commands are a separate custody-control path. Additional action-specific requirements still apply.

| Command | Purpose | Who can run |
|---|---|---|
| `/settlement guide` | In-game tutorial. | Anyone |
| `/help settlement` | Command help. | Anyone |
| `/settlement create <name> [radius]` | Found at your position; radius 1–256 blocks, default 48. | Any authenticated player |
| `/settlement claim` | Claim the autonomous settlement containing you. | Any player standing inside it |
| `/settlement list` | List settlement names. | Anyone |
| `/settlement info [name]` | Settlement details, migration attractiveness/components, last decision and inbound/outbound transit; without a name, selects the first record. | Anyone |
| `/settlement layout` | Show the street plan of the settlement containing you. | Anyone |
| `/settlement rename <name> <new-name>` | Rename. | Owner/operator |
| `/settlement remove <name>` | Dissolve and release membership. | Owner/operator |
| `/settlement bind <name> [villager-uuid]` | Bind a resident. | Owner/operator of the target settlement |
| `/settlement unbind [villager-uuid]` | Release a resident. | Owner/operator of the resident's settlement |
| `/settlement relocate <source> <household> <destination>` | Move a household; housing claims are reacquired. | Owner of both settlements or operator |
| `/settlement transit depart <source> <household> <destination>` | Put the household in durable transit from its current position; members must be together and free of construction/economic reservations. Arrival waits for the destination's simulated edge. | Operator (level 2+) or console |
| `/settlement transit list` | Read this dimension's transit records, member counts, status and departure/arrival game times. | Operator (level 2+) or console |
| `/settlement` or `/settlement menu [page]` | Open the screens above. | Anyone; mutations require owner/operator |
| `/settlement building list` | List certificates in the settlement containing you. | Anyone |
| `/settlement building certify <kind> <x1> <y1> <z1> <x2> <y2> <z2>` | Certify a building. | Owner/operator |
| `/settlement building upgrade <id>` | Upgrade a certificate. | Owner/operator |
| `/settlement build <blueprint>` or `/settlement build upgrade <id>` | Request construction or a native-built upgrade in the settlement containing you. | Owner/operator |
| `/settlement build cancel [index]` | Cancel the pending order at the one-based index (default: head). | Owner/operator |
| `/settlement army list` | List soldiers in the settlement containing you. | Anyone |
| `/settlement army recruit`, `/settlement army dismiss <uuid>`, `/settlement army order <all\|uuid> <follow\|guard\|patrol\|return>` | Manage soldiers. | Owner/operator |
| `/settlement production <name> status`, `/settlement commerce <name> status`, `/settlement payroll <name> status`, `/settlement caravan <name> status` | Read economy status. | Anyone |
| `/settlement production\|commerce\|payroll\|caravan <name> <operation> ...` | Other economy operations; see [production and economy](production.md). | Owner/operator, including production readbacks other than `status` |

Without a UUID, `bind` and `unbind` select the nearest living eligible villager within 8 blocks. Supplying a native UUID selects that entity explicitly instead of using the proximity search.
