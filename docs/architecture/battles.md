# Battles

How a staged faction battle runs inside the native tick: ordering, steering,
facing, targeting, combat state and factions on the wire. Operator commands are in
[battle staging](../OPERATING.md#battle-staging-disposable-worlds-only);
persisted battle state is in
[persistence](persistence.md#battle-allegiance). Per-module detail lives in the
module docs under `crates/mc-core/src/battle/`. Gameplay rules move to a package
under [decision X2](../../DECISIONS.md#plugins-and-the-engine).

## Order in the tick

Faction battle preparation is ordered owner work before EntityStep admission,
not another independently scheduled graph node. Its frozen `Frame` feeds
dense-column/member snapshot, formation decisions, target pairing, steering,
combat (including archery/cavalry), morale, then native-slot drive projection.
The formation roster supplies the stable battle-index prefix; unregistered
faction bodies and horses occupy a derived tail. Targets, classification and
drive share this index. Slot caches are refreshed before consumers; native
compaction/removal never gives a cached row authority over another identity.

Production kernels consume indexed contexts and columns directly. Stable-ID
query adapters, explicit withdrawal/interval controls and diagnostic rank
fields used only by the regression fixtures are compiled only under `cfg(test)`;
the production battle API does not retain unused compatibility entry points.
Order transitions remain production-owned: the staged scenario's guarded
advance/pursue policy uses the same transition side effects as explicit orders.
An inactive battle clears the lengths of its native-slot projections without
resizing or filling them against the world's population; missing entries mean
zero drive/no look, preserving the ordinary world's sparse fast path.

## Rest and facing

Workers read slot-indexed drive/look arrays, not ID hash maps. A battle row
with zero thrust may use the existing rest proof only while grounded, motionless
and unaffected by impulse/hurt/effects or changed geometry, and only while its
stored rotation is exactly what the skipped step would write. Since D36 that
rotation is published once per battle tick by `battle::facing::project`
(right after `Columns::project`) for every battle row with a target and zero
drive, and both the rest guard (`early_entity_rest`) and the step
(`MotionInput::facing`) read that one value: the row rests when it is
bit-identical to the stored rotation. Standing soldiers keep head and body
apart: the head turns toward the target only after a 20 degree dead zone on a
smoothed bearing (15 degrees per tick, landing on the current bearing), pitch
has its own 8 degree dead zone, and the body replicates the client's
`BodyRotationControl` (cone 75, aligned after 10 stable ticks over 10), which is
what the client draws for a standing mob; a hold returns the stored bits, so a
holding row rests and journals nothing. Riders, head-on-body mobs and a fighter
whose charge bonus lasts keep the previous whole-body look: their step turns
them from where the body ends up and their rest keeps the half-quantum guard,
both as before (the projection publishes only that guard's decision). With
nothing projected (no target, battle paused or stopped) a head left off its
body returns with the vanilla head rule in the step. The facing state is
runtime-only: a restart rebuilds it from the stored rotation. A standing row
stores no yaw of its own (owner D48 kept the lazy variant of the D36
comparison): its stored yaw changes only when it really moves or leaves the
projection, so the wire keeps the yaw the row already had while its head and
its drawn body turn. A row leaving the projection, also on pause or stop or
while its chunk is not simulated, hands its drawn body to its step until a
step took it. Moving rows still use ordinary block collision,
ordered movement writeback and native intent merge. No position writer or
second physics path is introduced.

## Steering, targeting and combat

Crowd steering reserves half the free gap for each simultaneous neighbour.
After directional projections and field-boundary clipping, a common bounded
scale enforces all five neighbour limits together: neither operation may reopen a
previously closed gap. This uses only the frozen roster and fixed stack storage;
it does not write another body's position or introduce a collision authority.
An externally displaced body already outside the ticket retains its bounded
inward recovery step instead; crowd avoidance must not strand it outside.

Melee targeting retains exact nearest-enemy queries at six blocks. Registered
archers select a live member of the cached nearest enemy regiment in constant
work, validating allegiance and actual range before retaining it. Standalone
native faction units, which have no regiment, use the local spatial oracle.
Both policies stagger decisions by stable ID over ten ticks; retained targets
are checked every tick. All output lanes retain their deterministic order.
Sub-step timers are exposed only with `MC_CORE_PHASE_LOG`; concurrency is not
enabled merely by introducing columns. A measured kernel exceeding 5 ms is the
threshold for evaluating indexed parallel sub-steps on the existing worker pool.

Native battle arrows exclude same-faction mobs from the shared swept collision
snapshot, equivalent to disabling friendly fire for a battle team. Player
targets, unaffiliated mobs and existing scoreboard checks are unchanged.
This prevents rear-rank volleys from killing their own archers and triggering
casualty/panic routs while ammunition remains.

Melee removal odds follow §4.7 of `receipts/BattleVideo/research-combat-behavior.md`
(owner host, outside Git): the reference
10,000-man army, 600 front files and 600-second contact phase is scaled to the
deployed army size/front files and a 180-second film contact phase. The declared
width is retained after losses, not turned into a last-survivor damage bonus.
Shield reactions, human windup/recovery clocks and animation cadence are not
accelerated. The native 20k diagnostic reports deaths and removals per simulated
engaged minute; those rates are not wall-clock TPS or a recorded-video receipt.

## Counters

Population counters use the same sparse native faction membership owner.
The hook samples pre-motion; publication refreshes the sample after merge before
creating immutable `TickTimes.battle`. The extra observer pass is part of tick
CPU cost, not WAL or checkpoint work. Faction remains native persisted state;
all battle columns, targets, clocks and counters are restart-local.

## Combat state and actions

`battle/combat.rs` owns action timers and finite ammunition, `archery.rs` owns
regiment volley clocks, and `cavalry.rs` owns charge/refusal timers. Fighters
and charge state share the formation-aligned dense execution index; a cold
identity remap preserves actions and ammunition when roster layout changes.
These are runtime battle state: pause holds their clock; restart restores
bodies and mount custody, not an in-progress battle. The native projectile
owner alone moves arrows and resolves hits.
The shield's raised/lowered pose is different: it is native `appearance.extra`
metadata, persisted in WAL post-images and checkpoints. The combat runtime
owns decisions, while the shared changed-only metadata setter owns presentation
mutation; unchanged guard never copies or republishes appearance. Staging
pre-raises equipped shields so the first running tick makes no mass pose write.
The sole kernel tree is `mc-core/src/battle`; `hostile` holds runtime state,
not another implementation.

Sword/spear/mace cycles last 21/23/25 ticks, with contact at 10/11/12.
An unparried blow removes about eight times in a thousand on the authored
deterministic roll; seven removals in ten die on the spot and the rest
withdraw wounded through the ranks. Any landed blow finishes a wounded man, and
pursuit removes nearly every landed blow, nine in ten fatally. Front ranks
surge in section pulses whose fatigue follows the share of the front pulsing;
outside a pulse a front-rank man still trades blows at the cautious rate with
an enemy already in weapon reach. Riders and pursuers fight from any rank, and
fugitives pinned against the battle bounds turn at bay; a fugitive stays where
it met an edge instead of running on along it into a corner. A regiment worn near
its pulse floor while in contact erodes until it breaks; one whose enemy broke
steadies. Film pacing changes lulls, never swing speed. These are simulation
choices based on
[Sabin](https://gwern.net/doc/history/2000-sabin.pdf), not measured historical
per-swing probabilities. Volleys use 220 ticks sustained and 60 arrows per
archer; about one arrow in seventy finds an armour gap and kills, the rest are
one-HP hurts.
Each regiment has an absolute staggered phase, including after wavering/rejoin;
the soldiers within that regiment still draw and release together. The staged
battle uses sustained cadence; there is no unused opening-burst control.
Horses refuse steady frontal foot ranks and run out to a flank; accepted charge
bonus decays over 260 ticks. Once their own foot closes, unengaged riders swing
round the nearest enemy foot regiment into its rear, and ride down the
fugitives of the nearest broken foot regiment until those reach the edge of
the field. A rider who fled to the edge rallies there and rides back until he
falls, even when his regiment has broken; foot fugitives stay at bay on the
edge. A regiment rides these sweeps as
a block, never toward one shared point: each rider follows the sweep from its
own file/rank slot. Targets and swings are laid round the regiments' live
centroids, not their virtual leaders, which march ahead of files lagging on
relief. A rider attacking closes on its own
place round its target (golden-angle rings 1.2-3.6 blocks out), so riders that
chose the same foe do not pile onto one block. The work counters bound visits,
target checks, reaction checks, strikes, arc iterations and charge decisions;
they are not a 20-TPS performance measurement.
Mounted terrain turns are retained for up to 60 ticks, including while pursuing a
target or formation slot. A side slide must retain the headroom required by
steps in its projected direction; otherwise steering weighs another direction
instead of pressing into a canopy-blocked step. The bounded detour prevents
the slot from pulling the horse back into the same obstacle on its next tick.
The ten-second stall census runs after target validation. A soldier holding a
live enemy within six blocks is in contact, not advancing, even when its virtual
leader is farther ahead. An invalid retained target cannot hide a blocked soldier;
the census is derived telemetry and never changes motion.

Battle staging resolves its vanilla item carriers and husk/horse type IDs once
in the stable registry tables. Loadout component variants still use the canonical
item codec; resolved equipment enters the summon image before normal attribute
and wire-appearance restoration. No per-fighter item-name NBT round trip or
entity-name registry lookup is needed, and no combinatorial loadout cache is kept.

A new `battle live` hold replaces the holder's entire inventory with a fresh
copy of his kit. It retires plugin reservations before changing stock, clears
the old inventory, and routes the close through the ordinary
native/plugin/settlement menu owners. The cursor and crafting grid fit in that
emptied inventory, then are discarded with all non-kit contents; no overflow
item entities are minted. Menu ownership and the client's open window close
together. A held player's respawn, reconnect (once per connection) or repeated
`battle live` re-arms him under the same custody, but only with the kit pieces
he still carries (matched by every identity field except count and wear, wear
kept): a piece he dropped or stored is never issued a second time.

Large projectile batches (at least 16 shots) derive a reusable, counting-sorted
two-block XZ grid from the frozen native body snapshot. Conservative half-extent
expansion covers wide bodies, Y bounds reject other heights, and canonical
candidate sorting preserves hit order. Dragon/deployed geometry remains an
explicit exceptional superset. Sparse world spans fall back to the existing
native chunk query before allocating empty-world cells (maximum 1,048,576).
Differential flight tests exercise both paths against exhaustive collision.

Battle arrow/spectral-arrow ground lifetime is captured at launch as 100 admitted
ticks, independent of later owner death. Ordinary arrows retain their previous
1,201-tick expiry. Settled arrows anchor to native entity lifetime age; idle
projectile ticks neither take `row_mut` nor publish appearance merely to age.
Pickup, expiry and physical-profile custody changes still use native owners.
`grounded_at_age` and `ground_lifetime` persist in projectile state, paired with
native `krasnokamen:LifetimeAge`; vanilla `life` projects the effective ground age.
Hydration refuses an anchored projectile without a native lifetime-age long;
retained lifetimes accept only 1,201, or 100 for Arrow/SpectralArrow.
Block impact establishes the native-age anchor. Loss of support exits the idle
path and clears both the anchor and ground age before falling; a later landing
starts a fresh ground lifetime, including across checkpoint/WAL recovery.
Active-clock WAL replay, frozen chunks and wrapping entity age retain elapsed
time without one arrow image per tick. Loyalty tridents keep their dynamic
return path and are not treated as idle arrows.

Native mob NBT writes `krasnokamen:Faction` exactly once for a nonzero faction;
ordinary faction-zero mobs carry no faction key.

Mob riders use the same horse-owned UUID relation as player riders. Staging
uses `battle::cavalry::mount`, requires a nonzero rider faction and copies it to
the horse so native battle motion is not suppressed as ordinary tamed AI.
Recovery allows a live mob passenger rather than requiring a player UUID,
rejects item/boat/dead/nested passengers, and rebuilds reverse links/appearance.
Native regressions include checkpoint/WAL mount custody, real mounted movement,
100-tick grounded expiry, idle no-touch pickup, native lazy-clock replay, and
matching uninterrupted/restarted expiry through actual checkpoint and WAL producers.

## Factions on the wire

A mob's side is its persisted `Body::faction`; a live player's side is his
`battle live` hold (runtime only, his own faction stays 0). The snapshot
publishes both: `EntityView::faction` and the derived
`WorldSnapshot::player_factions` (same `Arc` while the holds are unchanged).
The edge (`mc-edge/src/faction_teams.rs`) projects them per connection into
vanilla scoreboard teams `sf <faction>` (mobs by UUID string, players by name;
nametag never, collision always, red/blue colours) that exist only on the
client: no core team, no persistence, no gameplay effect. A team leaves when
the snapshot holds no row and no player of its faction; respawns keep the
teams and a dimension change syncs them again. Each connection keeps the
entries it sent (mob UUIDs, player names) as its baseline. Entries whose
faction ends remain in that team until the whole team is removed; a change
to another nonzero faction moves the entry by an add. Projection packets
use only create (0), remove-team (1), and add (3), never remove-entry (4).
A routed core `/team` add implicitly moves an entry and the edge forgets
that projected membership without sending a removal. Holds end only by
`battle reset` or a restart (`spectate` and going offline keep them).
`battle reset` removes staged rows, but summoned faction rows keep their team.
An ex-holder keeps the projected team's hidden nameplate and team colour while
that team survives a reset, until the team leaves.
Nameplates are always hidden for projected teams, including with hidden HUD;
options enable friendly fire but not see-friendly-invisibles.
No `/team` command can name a team
`sf <n>` (team names are one token); a core team of exactly that name
(`WorldSnapshot::team_names`) blocks the faction. Because an add moves a
client's entry and a removal from the wrong team disconnects a vanilla client,
an entry gets a faction only while it is in no `/team` team
(`scoreboard::require_no_team`: summon with `UUID`, battle mount, `battle
live`), and `/team join` refuses battle-faction UUIDs and holders.

Allegiance is defined once by `battle::faction::relation`: two nonzero equal
factions are Same, two nonzero different factions are Enemy, and either zero
is Neutral. Melee selection uses Enemy, friendly support uses Same, and
arrows collide with everything except Same. Separately, `battle_row(faction)`
marks native roster/AI ownership (currently nonzero). Ordinary AI suppression,
battle columns, sparse roster indexing and battle arrow lifetime use this
marker, not an enemy predicate. Neither changes the persisted faction format.
