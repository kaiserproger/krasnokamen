# Bumping the Minecraft version

Lesson from 26.1.2 to 26.3 (protocol 775 to 777, 2026-10): unit and TCP tests
stayed green while real clients disconnected. Only real-client gates prove a
bump. The standing support window is **every release from 26.1.2 through 26.3**
(775/776/777), with 777 canonical. A bump **adds an adjacent chain step and keeps
all existing versions**, never replacing the previous profile. See
[OPERATING.md](../OPERATING.md#client-protocol-compatibility) and
[ADR 0015](../decisions/0015-multiversion-protocol-translation.md).

## Sources of truth

- `reports/packets.json` and `registries.json` from the **new** content cache;
  the previous version's reports for legacy packet ids.
- `javap` on the new server jar for concrete ambiguities, with a classpath of
  the bundle's `META-INF/versions` and `META-INF/libraries` jars and JDK 25.
- Never ids or layouts from memory. Facts may be read from local jars; Mojang
  bytes and values copied from decompiled or `javap` output never enter Git
  (AGENTS.md; client provenance rule in [real-client gates](real-client-gates.md)).

## Server (this repository)

1. **Tags.** Content import must publish every tag that the synchronized
   registry payloads reference, in every tag registry (list the synchronized
   registries with `javap`). A missing tag disconnects vanilla clients in
   Configuration (`missing item tag ...`, protocol error at
   FinishConfiguration). Import, verify and startup fail closed on this.
2. **Extractors** that bootstrap vanilla bind the bundle's tags first: block
   predicates such as suffocation and view blocking consult tags.
3. **Changed codecs.** Diff every codec that changed shape (in 26.3: slot
   displays, light masks, some block-entity NBT). Tests that compare encoded
   packets with in-memory structs compare the decoded wire form.
4. **Adjacent translation** (`crates/mc-edge/src/translation/`) adds the
   new-to-previous step; preserve all older steps, profiles and caches. Enumerate
   releases from Mojang's manifest and capture local `javap` deltas per adjacent
   pair under `.analysis/`. Pair packet/registry identities by name, never by
   ordinal; check component payloads, duplicate patches and entity metadata.
   Tests use synthetic reports per step, composition in both directions relative
   to canonical, and `translation::join_tests` against every real endpoint cache.
   Remove one changed-packet translation and demonstrate a red regression.
5. Refresh fixtures and wire ordinals from the local reports of the right
   version.
6. Extend `mc_data::protocol_versions::SUPPORTED`, then run `content import
   --download`: it installs every endpoint and missing adjacent step snapshot.
   Steps own mapping data separately from endpoint caches; re-importing canonical
   must not churn valid older steps. Update Status/handshake coverage for the
   added protocol. A step above canonical needs explicit clientbound-up and
   serverbound-down codecs, not an inverse of lossy down mappings. Rebuild the
   singleplayer content bundle and re-pin it in the launcher
   ([releases](release-and-tags.md#client-release-and-the-singleplayer-content-bundle)).

## Native client (`krasnokamen-client`)

1. One test asserting that every packet id the client dispatches or sends
   (Login, Configuration, Play, both directions) equals `packets.json` by name.
   Fixing ids one live failure at a time wasted hours.
2. One version constant per data source; grep the protocol and content loaders
   for hard-coded version literals.
3. **The facts bundle (owner-local step).** The twelve files in the client's
   `dist/facts` (listed in `tools/publish_facts.sh` of the client repository:
   `action-state-facts.json`, `block-state-predicates.json`,
   `block-use-classes.json`, `block_collision_shapes_26_3.bin`,
   `creative-facts.json`, `entity-use-static.json`, `interaction-facts.json`,
   `item-placement-facts.json`, `item-properties.json`, `item-use-owners.json`,
   `placement-state-facts.json`, `terrain-facts.json`) come from about eleven
   Java capture programs that live only in the owner's ignored client
   `.analysis/` (`player-research`, `evidence`, `inventory-contract`,
   `ui-contract`). **They are not tracked in any repository.** A contributor
   with only the repositories cannot rerun them: the facts-regeneration part of a
   bump is blocked until the owner supplies the programs or a contributor writes
   equivalents. To recreate one, write a Java program per facts file, compile it
   against the new server jar's classpath (JDK 25, the bundle's
   `META-INF/versions` and `META-INF/libraries` jars) and have it read the file's
   data from the live vanilla registries, never from memory. The tracked pattern
   to copy is this repository's `tools/extract-block-collision-shapes.sh` with
   `tools/extract-block-collision-shapes/CollisionShapeExtractor.java` (and the
   sibling `tools/extract-block-{light,mining,explosion,map-colors}.sh`). For a
   regenerated file: port it to the new jar classpath, fan the files out by
   group, and accept it only with full coverage of state ids and items, a
   by-name diff against the old capture with every difference named, and the new
   blocks present. Bind vanilla tags before capture. Reflection "owner" lookups
   break when vanilla introduces delegators or renames classes; resolve the real
   override.
4. Entity metadata: check serializer registration order against the new jar;
   holder-typed serializers encode id + 1 with 0 meaning an inline value.
5. Strict clippy: compare diagnostics by (lint, path) against the baseline
   commit, not totals.
6. Bump `KRASNOKAMEN_REVISION` in the client's `release.yml` to the pushed
   new-protocol core commit and smoke the packaged singleplayer.

## Gates (release server binary as `KRASNOKAMEN_SERVER_BINARY`)

- Java, new version: `run core-client|inventory|survival|entities --client java`
  with `KRASNOKAMEN_LOADER_ROOT` pointing at a Loader checkout for the new version.
  Prewarm Gradle once (`:fabric-agent:classes`) or the first runs time out.
- Java, **every retained version**: the same gate with that version's Loader
  automation checkout and the automatically installed endpoint caches. An
  intermediate version without automation must name its exact blocker; TCP is
  not a substitute for graphical acceptance.
- Native: `native-lifecycle --client native` to Play with chunks rendered, one
  interaction, clean exit, on the release client with the real published
  content and facts.
- Read `result.json`, and grep `client.log` for nonfatal decode errors: they
  reveal NBT or format drift even on a pass.
- The Loader needs its own bump: mixin targets and channel declarations (see
  [releases](release-and-tags.md#loader-release)).

## Pareto approximations

When the new version adds a contextual rule the facts schema cannot hold (for
example near-plane view blocking or a contextual composter chance): keep the
old-style value, list the affected states, record it as a known gap in the
release notes, and describe the real impact by checking which code reads the
field. No new subsystem for it.
