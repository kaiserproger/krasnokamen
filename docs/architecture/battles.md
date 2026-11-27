# Battles

How a staged faction battle runs inside the native tick: ordering, steering,
facing, targeting and combat. Operator commands are in
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

Melee removal odds follow `research-combat-behavior.md` §4.7: the reference
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

