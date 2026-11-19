# Settlement screens and commands

This page explains the settlement screens and their chat commands.

Next: [construction](construction.md), [jobs](jobs.md), [production](production.md), and [army](army.md).

## Open and permissions

Right-click the settlement's anchor bell to open its compact management hub; the bell also rings. Other bells only ring. Alternatively, use `/settlement` or `/settlement menu [main|buildings|map|certify|residents|storage|production|army|projects|settings]`. The command selects the nearest settlement containing you, or one you own if none contains you. Icon lists and storage use 9×6 chest menus; forms, reviews, help and the map use [server-driven dialogs](../DIALOGS.md).

Anyone can view; only the owner or an operator can manage a claimed settlement. Permissions are rechecked on each click. An autonomous settlement must first be claimed while standing inside its radius, even for an operator to manage it.

The client locale selects the language: `ru*` uses Russian; all other locales use English. Materials use the client's translated item names, workshops use localized profession labels, and authored settlement names stay unchanged. Labels below are English.

## Main

The hub opens **Management** (the icon dashboard), **Next step**, **Army**, and paged **Help**. The dashboard leads to buildings, construction, residents, resources, storage and production. A settlement without storage points directs **Next step** to storage certification. Unclaimed settlements offer a claim review before management.

**Back** returns to the screen that actually opened the current one, including chest↔dialog transitions and the map viewport. Paging and refresh replace the current view instead of adding navigation steps. History is connection-local, capped at 16 previous screens and cleared on disconnect or when switching settlements. Dialogs restore the last displayed snapshot; actions revalidate current authority and state, and chest views reread live world state.

## Map

Open **Map** from the dashboard, a building card, or `/settlement menu map`. The 13×13 grid is north-up; its scale gives the square of blocks represented by each cell. Pan, zoom and overview controls stay on the map. Green marks territory, dark grey marks outside, orange marks active construction sites and magenta marks you when visible. Certified buildings have type-specific colours. **Legend** opens a separate page.

The street plan is drawn under buildings: grey **Road (paved)**, pale **Road (planned)** and light blue **Lamp (planned)**. Hover for details; building hovers include type, number, level and corners. Click a building for its icon card, a construction site for status, or a street feature for its details. Overlapping cells open a paged chooser retaining every feature, not just the hover summary.

The map is a snapshot: **Refresh** redraws it. **Buildings** opens the building list; **Back** restores the actual previous screen. Back from a building card restores the map viewport that opened it.

## Buildings

Buildings are a paged chest list with type filters and an icon card for each certificate. Empty lists explain how to order new construction or certify your own building. A card shows level requirements, workplaces and staffing. Certificate checking is separate from ordering physical blueprint construction; managers review the target before confirming either action. Free workplaces open resident selection, missing workstations are marked, and storage buildings link to **Storage**.

**Certify new building** opens a dialog: choose **Building type**, enter two inclusive opposite corners as `x y z`, and set **Height above corners**. For **Corner 1 here** / **Corner 2 here**, stand on the floor: marks use the supporting cell, `floor(feet Y − ε)`, including slabs and beds. After the first mark, close the dialog and walk; the clickable chat link reopens the saved type and marks. A complete one-block wall ring around marked inner floor corners is included automatically; typed bounds remain exact. **Certify** reports localized found/required counters and what to add. Success offers residents/jobs and the next certificate level. See [construction](construction.md) for the rules.

## Residents & jobs

The resident chest list has filters and individual cards with profession, workplace and home claims. Home/work links explicitly open those locations on the map. Choose a resident's workplace action to select a certified workplace; a free workplace on a building card opens the resident chooser. See [jobs](jobs.md) for work behavior.

## Storage

**Storage** shows combined founding and certified storage as item icons, with stored and reserved amounts. Managers move real items:

- Left-click an item to take up to a stack onto an empty cursor; right-click takes half, rounded up. Shift-click sends up to a stack to your inventory.
- Shift-click your own stack to deposit it, or click the storage grid with a held stack; right-click deposits one item.
- Production and construction reservations cannot be withdrawn. Your own reserved inventory items cannot be picked up, thrown or stored below their reservation floor.
- A transfer can be partial when space is limited. If nothing fits, the transfer is refused; spectators can browse but inventory moves only resync.

## Assign residents

Choose a resident icon and an eligible certified workplace with a free live workstation, or choose a free workplace first and then a resident. Assignment records the native job claim; it does not claim that the resident has already arrived.

Assignment is refused for children, nitwits, soldiers, builders outside builder huts, residents busy with a production order, or a traded resident changing craft. It is also refused if the resident already works there, the building has no jobs, no free place has a live workstation, or the resident or building no longer exists. Dismiss soldiers on the [Army](army.md) screen first; traded residents can move to the same craft.

## Production orders

View and manage orders from **Production orders**; see [production](production.md).

## Army

Manage the roster and orders from **Army**; see [army](army.md).

## Construction

View active projects and pending orders, or select an authored blueprint from **Construction**. The material estimate shows translated item names and the amount physically in storage. A successful order response names up to three missing materials with quantities and counts any remaining kinds; its single **Storage** / **Status** / **Back** action row stays reachable at 720p. **Status** retains the full translated deficit list; recipe-funded orders show no shortage. Reviews of a new blueprint never display another pending order's bill. Stock your founding chest if one was granted, or certify your own storage; no order creates free materials. See [owner build requests](construction.md#owner-build-requests).

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
Builder: <resident name or unbound>
```

The phase is the live preparation/build phase, not a constant; progress is the
structure ledger's completed/total cells. Each pending order has one line:
`Pending #<index>: <display name> L<level> · <reason>`, with indices 1..count.
Material icons remain below the project and order lists. Queued orders retain their item/count shortage details in the review and status dialogs, with client-translated item components. English command diagnostics retain their existing spelling; the management UI localizes the waiting categories.
The map gives each active site a hover naming its blueprint, level and corner;
overlapping map cells retain all site descriptions. Its legend also lists each
site as `Active site: <display name> L<level> · <x> <y> <z>`.
Clicking any active site opens the complete Construction status. The summary
count/head does not replace the project list.
The Construction chest has one icon per active site (facts in its lore, click
for status) and one icon per pending order (click to review cancelling that
exact indexed intent). A separate map icon opens the map. The clock opens the complete status dialog; material
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
