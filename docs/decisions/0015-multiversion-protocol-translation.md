# ADR 0015 - Per-session adjacent protocol translation

**Date:** 2026-10-21; chain decision 2026-11-07  
**Status:** Accepted. Canonical is Java 26.3 (777). The standing support window
is **every Java release from 26.1.2 through 26.3**, not just its endpoints.
A bump adds a step and never drops a previously supported release.

## Context and measured window

`mc-core` owns version-independent gameplay, persistence and plugins. `mc-edge`
owns transport, wire identities, presentation and connection state. Multiple
client releases must coexist without a protocol branch in core authority.
The owner also requires a newer client on an older canonical server to be
possible without bumping that core. Future backfill reaches 1.7; it is not
implemented or advertised by this decision.

[Mojang's version manifest](https://piston-meta.mojang.com/mc/game/version_manifest_v2.json)
was fetched on 2026-11-07. Selecting `type=release`, from 26.1.2 through 26.3
inclusive in release order, yields exactly:

| Release | Protocol | World version | Client SHA-1 |
|---|---|---|---|
| 26.1.2 | 775 | 4790 | `4e618f09a0c649dde3fdf829df443ce0b8831e65` |
| 26.2 | 776 | 4903 | `2dc72797acbc1b63fc16a11c4ac393605f453754` |
| 26.3 | 777 | 5023 | `e877b6a07acd633fb3bb475002175cec036e7b87` |

Protocol/world constants were read with local `javap -p -c -constants` of each
client's `net.minecraft.SharedConstants`; packet registrations/record fields
were captured with `tools/dump-vanilla-protocol.sh`. Artifacts are ignored:

- `.analysis/protocol-chain/version_manifest_v2.json` and `<release>.json`;
- `.analysis/protocol-chain/SharedConstants-<release>.txt`;
- `.analysis/protocol-chain/protocol-<release>-javap.txt`;
- `.analysis/protocol-chain/26.1.2-to-26.2.diff`;
- `.analysis/protocol-chain/26.2-to-26.3.diff`;
- additional per-class `javap` captures/diffs in the same directory.

The dump script's historical printed 26.1.2 label is not a version oracle;
these filenames, manifest checksums and SharedConstants identify each input.
Mojang jars, classes, reports and derived snapshots remain local, never in Git
(ADRs [0001](0001-vanilla-data-as-runtime-input.md) and
[0002](0002-vanilla-protocol-metadata-as-reference.md)).

### Adjacent wire deltas

**775 -> 776:** Configuration registrations are unchanged. Play renames
`spectate_entity` to `spectator_action`, whose optional entity id needs a
payload conversion. Login `login_finished` gains the session UUID; Play `login`
gains `onlineMode`. `CommonPlayerSpawnInfo`, light masks, movement packets,
player-action ordinals, tag slot displays and pot decorations retain their 775
layouts. Slime's ageable inheritance/size metadata changes in **776**, so that
metadata rewrite belongs to 776 -> 775, not 777 -> 776. The actual 776/777
class is `net.minecraft.world.entity.monster.cubemob.Slime`, extending
`AbstractCubeMob -> AgeableMob -> PathfinderMob -> Mob -> LivingEntity -> Entity`.
Local captures of that complete hierarchy (`slime-captures.json` and
`net_minecraft_world_entity_*-<release>.txt`) show Entity's 8 accessors,
LivingEntity's 7, Mob's 1 and AgeableMob's 2 (`DATA_BABY_ID`, `AGE_LOCKED`);
`AbstractCubeMob.ID_SIZE` is index **18** in both 776 and 777. In 775 Slime
extends Mob directly and `ID_SIZE` is **16**. Earlier captures using the old
package for 776/777 reported “class not found” and are not evidence.
Content identities come from separate release reports, regardless of unchanged
packet names.

**776 -> 777:** Configuration adds `post_effects`. Play adds
`add_transient_block`, `post_effects`, `swing_animation` and serverbound `punch`,
removing `swing`. Changed layouts include spawn-info game modes, movement step
lists/position paths, particles, explosion sound flag, sign editing, teams,
advancement coordinates, tag HolderSets, pot decorations and light BitSets.
The session UUID and `onlineMode` already exist in 776 and must be retained by
this step. The shared `sulfur_cube_content` component remains an item template
in both releases, with its nested ids translated rather than an empty payload.
The full measured 775 -> 777 delta was originally retained under
`.analysis/vanilla-26.3/`; it is historical evidence, not a direct runtime step.

## Decision

Use a **per-session adjacent chain in `mc-edge/src/translation/`**, analogous
to ViaVersion. A route from canonical 777 to client 775 is exactly
**777 -> 776 -> 775**. No direct 777 -> 775 bypass exists. Clientbound visits
steps in that order; serverbound visits them in reverse. Each step has its own
session state (phase, teleports, lossy inventory images and metadata tracking).
Compression/framing is decoded/re-encoded once for the whole route at the
existing `connection.rs` boundary, not once per step.

The original six requirements remain:

1. The handshake chooses an immutable available version profile. Unsupported
   Login handshakes receive `multiplayer.disconnect.incompatible`. Status
   advertises the selected available release/protocol; unknown or disabled
   ids advertise canonical and therefore appear incompatible.
2. Only connection state owns the selected route. Core inputs, player changes,
   persistence, plugins, tick and world authority do not branch on protocol.
3. Profile/step decoders turn peer ids/layouts into canonical edge inputs before
   core admission. Malformed or untranslatable payloads fail with context before
   mutation; deliberately unrepresentable presentation packets are omitted.
4. Canonical presentation is translated before framing/compression, never in
   tick/fanout authority. Pre-framed fanout batches cross the same boundary.
5. Every **version profile** owns its exact registry/tag/known-pack snapshot.
   Every **step** owns name-paired adjacent registry mappings, including block
   states, items, entities, components and synced registries. Matching numeric
   ids is never evidence of matching identity. Lossy forward mappings do not
   become authoritative reverse mappings; ingress uses exact name matches.
6. Accept a release only with local registration/layout evidence, synthetic
   per-step mapping/layout tests, a real-cache raw Login -> Configuration -> Play
   gate, and a graphical gate covering ordinary gameplay. TCP is not graphical
   acceptance. A missing graphical adapter is an explicit release-specific gap.

### A step above canonical

The route is independent of `mc_protocol::PROTOCOL_VERSION` being its largest
endpoint. For a newer client on an older canonical core, clientbound uses
**up** codecs and serverbound uses **down** codecs. Step factories create
stateful translators with an explicit traversal direction. This is not the
inverse of a lossy older-client map. The measured 777/776 production factory
also builds its 776 -> 777 codec from the opposite source snapshots, with
explicit changed-packet codecs in both directions. Its factory regression runs
canonical=776 / peer=777 through Login, Configuration, Play login, light and
block updates; a 777-only creative item is refused rather than inverted from
its older-client stone stand-in. Swing upgrades use the measured WHACK(1)/6
default; old particle speed applies to all three new axes and uses measured
RandomizationType.DEFAULT(0). An unsupported new destroy-direction input errors.
Synthetic routing tests additionally exercise both sides of canonical and
reverse serverbound order. No unmeasured release above 777 is advertised.
Adding an up step changes edge codecs/profiles, not core authority.

### Data ownership and automatic installation

`mc_data::protocol_versions::SUPPORTED` owns the release window. `content import
--download` and a fresh first start install every endpoint using the existing
verified Mojang download, datagen and vanilla RegistryData capture path. This
does **not** depend on the proposed native importer in
[ADR 0022](0022-content-without-java.md), whose owner questions remain open.
Only canonical content needs the gameplay extractors; other endpoints need
reports, their own definitions/tags and exact captured registry payloads.

For a canonical cache `content/26.3`, version caches are `content/26.1.2` and
`content/26.2`. Each adjacent step separately owns mapping-only snapshots in
`content/steps/776-775/` or `content/steps/777-776/`: both sides' reports,
version identities and ordered synced-registry entry names. Import derives
these from both adjacent caches and validates before atomic publication.
Valid older steps are immutable across canonical re-imports and bumps. For an
arbitrary explicit cache root, the sibling layout is documented in
[operations](../OPERATING.md#client-protocol-compatibility).

Endpoint availability and route availability are distinct:

- Missing/broken **endpoint** data disables logins for that version only. A
  775 route can still cross 776 using its step-owned snapshot when the 776
  endpoint cache is absent. Each failure names release, protocol, path and cause.
- Missing/broken **step** data disables every route crossing that pair, with a
  clear pair-level log line. It never silently bypasses the missing step.
- Canonical sessions do not require translation data. Existing content is reused
  offline; an unusable existing canonical cache fails closed, not auto-overwritten.

Each cache/step publication is atomic, not a whole-window transaction. A failed
window import reports failure even if earlier artifacts were successfully
published; rerunning repairs missing artifacts. No retained pre-upgrade cache
or retired cache override is required.

### Backfill constraints: Configuration, ids, chunks, light and chat

These constrain future steps; they do not implement or advertise older releases:

- **Before 1.20.2:** there is no Configuration wire phase. Phase state belongs
  inside each step, not a global chain enum or the core. A future phase bridge
  must distinguish its two endpoints, synthesize canonical configuration
  acknowledgements and peer login/play setup, and coordinate reader/writer
  halves. Wire transitions are events, not packet-id arithmetic. The transport
  boundary must support bounded zero/one/many frame emissions plus opposite-
  direction control actions when that bridge is introduced; an absent peer
  phase must never make canonical startup wait for an impossible peer packet.
- **Before 1.13:** blocks/items use numeric id plus metadata, not flattened state
  ids. A step needs a typed name/property <-> numeric-id/metadata mapping;
  version-neutral core identity cannot become a global legacy numeric id.
  Unrepresentable states take explicit nearest mappings, not guessed ordinals.
- **Chunks/light:** a step owns its endpoint's palette, section, height, biome,
  bulk-chunk and lighting layouts. Repacking and packet splitting/combining
  belongs there; a universal modern chunk envelope must not leak into the
  step-factory contract. Bounds must be explicit before buffering/coalescing.
- **Chat:** pre-signed-chat and older text component formats require per-step
  chat codecs and session/security policy. Never invent signatures, claim secure
  chat for unsigned messages, or let a text conversion bypass core moderation.
- **Runtime reconfiguration:** current steps implement initial Login ->
  Configuration -> Play. The edge does not emit runtime Play -> Configuration
  today. Before adding it, shared phase barriers must coordinate both halves
  before any post-transition Play write; copying a phase at split is not enough.

### Polyfill policy

Use the nearest name/property-preserving representation for missing features.
Dynamic registries can carry server-sent data where the client accepts it;
items may use `item_model` plus a server resource pack. **Datapacks cannot
polyfill blocks, entities or mechanics.** Substitution changes presentation,
not authoritative inventory/world identity. New-only attributes, statistics or
block entities with no representation are filtered explicitly; unknown shapes
fail closed rather than dropping an inventory/world update. This is not a
claim that all arbitrary components or every vanilla mechanic are supported.
Each adjacent step explicitly maps particle types absent from its target
registry to the nearest available presentation. Preserve options when the
target layout supports them; otherwise consume the source options and discard
only those the stand-in cannot express. Any deliberate omission needs an
explicit documented reason; an unknown id or unmapped type fails closed.

Missing packet ids are not an omission policy: only `add_transient_block` and
`post_effects` may be skipped for an older peer. Other missing target packets
error with the pair, phase, packet name and source id. A block state missing
from the source report is an error, distinct from a known state's documented
nearest mapping. Dimension types, painting variants and display kinds are
name-paired too; equal current ordinals do not authorize copying them.

Creative inventory echoes compare the saved peer projection whenever **any
recursive part** of a stack loses information: item/registry substitutions,
omitted components/removals/tooltips, or pot sides, nested counts and nested
components. An unchanged outer item id does not make a stack lossless.
All three releases' creative packets use the untrusted stack codec: each outer
added component is byte-length-prefixed; nested templates are not. The old
client also elides the decorated pot's default four-brick component when it
reserializes a stack. Echo matching accounts for component order and that
measured default, rather than comparing a trusted clientbound byte image.
An empty creative write retains the old lossy image as client-cursor provenance;
returning that unchanged projection restores the saved canonical stack, even
after an authoritative empty-slot refresh. A negative-slot drop restores that
canonical stack once and consumes the provenance. Closing the container clears
the provenance. A window-0 full refresh clears slot images but preserves the
separate creative cursor when its InventoryMenu carried field is empty; a
nonempty carried field or explicit cursor packet updates the saved cursor.
Count-only edits of a matching projection rewrite the canonical count and
preserve all hidden components. Changes to the item or component projection
translate normally; arbitrary creative edits are not component-preservation proof.

This is presentation provenance, **not authoritative creative cursor custody**.
Local `javap -p -c net.minecraft.server.network.ServerGamePacketListenerImpl`
from the measured 26.1.2 and 26.3 server jars shows
`handleSetCreativeModeSlot`: slots 1–45 call `inventoryMenu.getSlot(...).setByPlayer`
and broadcast changes, negative slots call `ServerPlayer.drop`, and neither
branch assigns the server menu's carried stack. The 26.1.2 client
`AbstractContainerMenu.removed` also returns immediately for non-`ServerPlayer`.
Thus picking up and disconnecting loses the client-only creative cursor in
vanilla. Closing with that cursor does not create a server-owned return.
Creative can mint stacks; the translator must not invent survival-style custody
or restore an old image after a close/drop. The measurements are reproducible
with the jar hashes and capture commands below; Mojang bytecode remains local.

`mc-data::legacy_26_1_2` remains a durable-record migration owner, and
`enchanting_26_1_2` remains protocol-neutral gameplay; neither is a session
profile, a chain bypass, nor an alias for the retired translator module.

## Validation and historical evidence

Current focused gates are the `translation` unit tests and
`translation::join_tests::protocol_{775,776,777}_login_configuration_play`.
The ignored TCP cases require real imported caches/steps and exercise known
packs, full RegistryData, chunks, block placement/breaking and chest opening.
A missing cache fails; it is not a skipped/pass result. Set `SOLARIS_CONTENT_CACHE`
to the isolated canonical root for a fresh-install gate.

`java-protocol-pot-creative-echo` is the graphical counterpart to the lossy
creative-echo unit regression. With a 26.1.2 Java adapter, it physically picks up
a named decorated pot with missing sides and a named nested count-three brick
template, returns it to another hotbar slot and back, and compares complete
`debug menu` authoritative component snapshots. It uses the actual creative
screen's mouse path, not the survival-menu click RPC. Run with
`python3 -m tools.harness run scenario java-protocol-pot-creative-echo --client java`;
the release pin, adapter checkout and fresh-cache environment belong in its receipt.

The original 2026-10-28 graphical evidence is historical, not proof of this
chain cutover: 775 survival/inventory receipts were
`.analysis/validation/20261028T065419-survival-qu1chur_/` and
`20261028T065648-inventory-lyaogt0n/`; 777 core-client/inventory/survival/entities
receipts were `20261028T095727-core-client-ny4t4eqy/`,
`20261028T095829-inventory-gvlkvyux/`, `20261028T095920-survival-_pf6nb4e/` and
`20261028T100001-entities-74xk4fiv/` in their respective validation checkouts.
Those bounded passes still observed trial-spawner/pot NBT presentation defects;
they did not establish complete parity. New chain receipts and its negative
control must be reported against the exact committed tree and pinned release.
