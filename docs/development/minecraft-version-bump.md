# Bumping the Minecraft version

Lesson from 26.1.2 to 26.3 (protocol 775 to 777, 2026-10): unit and TCP tests
stayed green while real clients disconnected. Only real-client gates prove a
bump. Old-version support is described in
[OPERATING.md](../OPERATING.md) "Client protocol compatibility" and
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
4. **Legacy translation** (`crates/mc-edge/src/legacy/`) converts each changed
   layout back. Grep it for hard-coded component and registry ids and override
   guards: duplicate components get through vanilla (last wins) but break the
   native client. Test the exact payload and decode strictly in the legacy TCP
   gate (`cargo test -p mc-edge --lib legacy` plus the ignored TCP join test with
   `SOLARIS_LEGACY_CONTENT_CACHE`).
5. Refresh fixtures and wire ordinals from the local reports of the right
   version.
6. Re-import the content cache (old caches fail the new verify), rebuild the
   singleplayer content bundle and re-pin it in the launcher
   ([releases](release-and-tags.md#client-release-and-the-singleplayer-content-bundle)).

## Native client (`solaris-client`)

1. One test asserting that every packet id the client dispatches or sends
   (Login, Configuration, Play, both directions) equals `packets.json` by name.
   Fixing ids one live failure at a time wasted hours.
2. One version constant per data source; grep the protocol and content loaders
   for hard-coded version literals.
3. The facts bundle comes from about eleven Java capture programs kept in the
   client's ignored `.analysis/` (`player-research`, `evidence`,
   `inventory-contract`, `ui-contract`). Port each to the new jar classpath, fan
   them out by file group, and accept a file only with full coverage of state
   ids and items, a by-name diff against the old capture with every difference
   named, and the new blocks present. Bind vanilla tags before capture.
   Reflection "owner" lookups break when vanilla introduces delegators or
   renames classes; resolve the real override.
4. Entity metadata: check serializer registration order against the new jar;
   holder-typed serializers encode id + 1 with 0 meaning an inline value.
5. Strict clippy: compare diagnostics by (lint, path) against the baseline
   commit, not totals.
6. Bump `SOLARIS_REVISION` in the client's `release.yml` to the pushed
   new-protocol core commit and smoke the packaged singleplayer.

## Gates (release server binary as `SOLARIS_SERVER_BINARY`)

- Java, new version: `run core-client|inventory|survival|entities --client java`
  with `SOLARIS_LOADER_ROOT` pointing at a Loader checkout for the new version.
  Prewarm Gradle once (`:fabric-agent:classes`) or the first runs time out.
- Java, previous version: the same `core-client` with the old Loader and
  `SOLARIS_LEGACY_CONTENT_CACHE`.
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
