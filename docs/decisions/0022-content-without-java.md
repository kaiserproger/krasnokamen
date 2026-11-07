# ADR 0022 - Vanilla content without Java on the user's machine

**Date:** 2026-11-07  
**Status:** Proposed. The native `RegistryData` encoder (step 1) is
implemented and proven against vanilla captures but not wired into
`content import`. Nothing else has changed; the owner's questions are at the end.  
**Related:** [0001](0001-vanilla-data-as-runtime-input.md) (local vanilla
content, no copied Mojang implementations),
[0002](0002-vanilla-protocol-metadata-as-reference.md) (protocol provenance),
[0015](0015-multiversion-protocol-translation.md) (26.1.2 clients through a
legacy cache), [client release and the singleplayer content bundle](../development/release-and-tags.md#client-release-and-the-singleplayer-content-bundle),
the launcher in `../solaris-client` (`launcher/src/pins.rs`,
`tools/publish_facts.sh`, `tools/publish_content.sh`).

## Problem

Owner goal, 2026-11-07: running Solaris (dedicated server and singleplayer)
must not need a manually obtained Minecraft jar or a locally installed JDK, and
must start and play adequately on weak hardware.

Where that stands at RC5:

- **Dedicated server.** `solaris-v2 content import` downloads both server
  bundles from Mojang (size and SHA-1 checked), then needs JDK 25 for three
  steps: the bundle's datagen (`--reports`); the in-tree extractors compiled
  with `javac` against the bundle (`tools/extract-*`,
  `tools/VanillaWorldgenOracle.java`); and a temporary vanilla server whose
  Configuration handshake is recorded as the exact `RegistryData` payloads
  (`mc_vanilla::registry_capture`). Measured 2026-11-07 on the owner host
  (12 cores, load about 14, RC5 binary, `--download`): 73 s, of which 16 s
  downloads, 8 s datagen, 34 s extractors and 11 s capture; 114 s CPU; 0.6 GB
  peak RSS.
- **Singleplayer** (client `v1.0.0-rc.5`). The launcher pins two bundles on
  the Solaris static server: the facts bundle `solaris-facts-26.3-r2`
  (1.4 MB) and the content bundle `solaris-content-26.3-r5` (2.8 MB). The
  content bundle is a complete derived cache, Mojang's data pack subset and
  language file and `registry_network_nbt` included, published by owner
  decision 2026-10-28 (`tools/publish_content.sh`). Singleplayer needs no JDK
  and no server jar, but Mojang bytes and payloads derived from them are served
  from the Solaris static server, every release republishes the cache, and a
  dedicated server cannot use it.
- **26.1.2 clients.** The importer accepts only `--version 26.3`, so nothing
  can create the legacy cache ADR 0015 needs; operators must keep an old one.

## What the cache holds

The full inventory with producers, sizes and runtime consumers is
`/run/media/kaiserroman/games/solaris-work/receipts/ContentNoJava/inventory.md`
(owner host). The 26.3 cache is 21.8 MiB in 9 904 files (57 MiB on disk because
of block overhead):

| Kind | What | Size | Produced today by |
|---|---|---:|---|
| Mojang bytes | 26.1.2 data pack subset, 26.3 tags, lang, version files | 8.9 MiB | zip copy, no Java; three worldgen dirs come from the oracle but are JSON-equal to the jar (104/104) |
| Pure fact tables | `blocks.json`, `registries.json`, item components, block light, mining, explosion, map colours, `overworld_expanded.json` | 12.8 MiB raw, 246 KiB zstd -19 | datagen, extractors, oracle |
| Derived, excluded from the facts bundle | `registry_network_nbt` (432 payloads), `packets.json` | 163 KiB | capture, datagen (published today only inside the content bundle) |

The facts bundle already carries the 26.3 `blocks.json`, `registries.json`,
`block_light.json`, `block_mining.json` and item components. Not yet in it:
`block_explosion.json`, `block_map_colors.json` and `overworld_expanded.json`.
The last is not a file in any jar: the oracle runs vanilla's overworld biome
builder, and ADR 0001 forbids porting that code, so it can only be a fact table.
The 775 path reads only the 26.1.2 registry dirs, tags, `registry_network_nbt`,
`packets.json`, `registries.json` and `blocks.json`
(`mc-edge/src/translation/mod.rs`, profile loading).

## Decision (proposed)

The user path runs no JDK and no Mojang code, and the Solaris static server
serves no Mojang bytes. Each kind of content has one source:

1. **Mojang bytes** come from Mojang on the user's machine, read natively as
   zips: the SHA-1-pinned server bundles for a dedicated server, as `content
   import` downloads them today. For singleplayer the jars the launcher already
   has serve the same purpose: the 26.3 `client.jar` (downloaded for textures)
   holds the 26.3 data pack, `version.json` and `en_us.json` byte-identical to
   the server jar's (9 883 of 9 883 data files), and the 26.1.2 `client.jar`
   (36 MiB) holds the 26.1.2 data pack byte-identical to its server jar's
   (8 282 of 8 282). The oracle's `noise_settings`, `density_function` and
   `noise` exports become plain jar copies.
2. **Pure fact tables** come from the one pinned facts bundle the launcher
   already installs (`solaris-facts-<version>-r<N>`, built by
   `tools/publish_facts.sh`, pinned by name and SHA-256 in
   `launcher/src/pins.rs`). It gains `block_explosion.json`,
   `block_map_colors.json`, `overworld_expanded.json` and, for the legacy
   cache, the 26.1.2 `blocks.json` and `registries.json`. `solaris-v2` pins the
   same name and SHA-256 for its release.
3. **RegistryData payloads** are encoded by Solaris from the release's data
   pack: `mc_data::registry_network::encode_entry` (implemented, see the proof
   below).
4. **Packet names by id**, the only use of `packets.json`
   (`mc-edge/src/translation/adjacent/tables.rs`), would become Solaris tables
   for every release in the support window, like the 26.3 packet ids already are.
5. **Java stays only with maintainers**: datagen, the extractors and the oracle
   build the facts bundle; the vanilla capture and datagen's `packets.json`
   are the oracles the encoder and the packet tables are checked against
   (ignored maintainer tests, run at every version bump and bundle rebuild).

One producer then builds every cache: the 26.3 cache for dedicated servers and
singleplayer, and the 26.1.2 legacy cache from the 26.1.2 jar. The content
bundle and its per-release republishing go away.

## Proof: native RegistryData, byte for byte

The encoder reproduces vanilla from four protocol rules, each a table in
`crates/mc-data/src/registry_network.rs`: fields network codecs omit (variant
`spawn_conditions`, biome generation settings); environment attributes vanilla
never syncs (the 18 registered without `syncable()`, read with `javap` from both
jars); the 17 field paths whose codec is a double, long or `Vec3i` (everything
else follows the JSON: integer `Int`, fraction `Float`, boolean `Byte`); and
`CompoundTag`'s `java.util.HashMap` key order, which needs the codec field
order for the 55 key pairs that share a bucket. Floats are rounded as
`Float.parseFloat` rounds the JSON decimal.

Vanilla is not reproducible against itself: it writes synced attribute maps
from `Map.copyOf(Maps.filterKeys(...))`, whose iteration order depends on a
per-JVM salt and identity hashes. Three independent 26.3 captures (Main's cache,
`wt/release-vanilla-pre-tags263` and a fresh import on 2026-11-07) differ
pairwise byte for byte in 7 to 10 of 432 payloads, 12 distinct payloads in all,
and are equal as NBT in all 432; every difference is the order of attribute
keys that share a bucket. The maintainer check therefore accepts exactly that
permutation and nothing else: equal tag types and values, equal key sets, the
same bucket sequence, and a different order only inside a
`dimension_type`/`worldgen/biome` `attributes` or `timeline` `tracks` map.

Results on 2026-11-07 (logs `proof-*.log` beside the inventory receipt):

| Release | Capture | Registries | Payloads | Byte-identical | Vanilla's attribute-map order only | Failures |
|---|---|---:|---:|---:|---:|---:|
| 26.3 | Main's `data/vanilla` | 32 | 432 | 421 | 11 | 0 |
| 26.3 | fresh import 2026-11-07 | 32 | 432 | 424 | 8 | 0 |
| 26.3 | `wt/release-vanilla-pre-tags263` | 32 | 432 | 427 | 5 | 0 |
| 26.1.2 | legacy cache (`~/.local/share/solaris/content/26.1.2`) | 28 | 382 | 378 | 4 | 0 |

Every registry matches, including the four only 26.3 synchronizes
(`block_transformer`, `decorated_pot_pattern`, `sulfur_cube_archetype`,
`worldgen/block_state_provider`); none needed an exception. The whole check,
jar read and all 432 encodings, takes 0.2 s in a debug build. The first run used
`serde_json` numbers and failed three `sulfur_cube_archetype` payloads by one
ulp (`-0.9499999992549419` read as `-0.949999999254942`): its default float
parsing is not correctly rounded, so the encoder reads number text itself.

Run it with the release's server jar (bundle or inner jar) and a capture:

```sh
SOLARIS_REGISTRY_JAR=<server.jar> SOLARIS_REGISTRY_CAPTURE=<cache root> \
  cargo nextest run -p mc-data --lib --run-ignored only --no-capture \
  native_payloads_match_the_vanilla_capture
```

The input is the SHA-1-pinned jar of the pinned release, and the encoder is a
deterministic function of it, so a user's machine produces exactly the bytes a
maintainer verified. A new release changes the tables only through this check.

## Options compared

**B: Mojang's Java runtime and the current import.** Mojang's launcher metadata
(`launchermeta.mojang.com/v1/products/java-runtime/2ec0cc96c44e5a76b9c8b7c39df7210883d12871/all.json`)
lists `java-runtime-epsilon` 25.0.1 for linux, windows-x64, mac-os and
mac-os-arm64. Each build is a jlinked JDK: its `release` file lists
`jdk.compiler` and its manifest has `bin/javac`
(`jre.bundle/Contents/Home/bin/javac` on macOS). So B needs no extractor
rewrite: download that runtime and run the import unchanged.

**C: the RC5 content bundle for dedicated servers too.** `solaris-v2` fetches
the pinned `solaris-content-<version>-r<N>` on first start, as the launcher
does.

| | A: native (this ADR) | B: Mojang runtime + Java import | C: content bundle everywhere |
|---|---|---|---|
| First-start download, dedicated | 26.3 bundle 59.4 MiB + 26.1.2 bundle 57.6 MiB from Mojang; facts 1.4 MB | the same bundles + runtime 75.5–78.3 MiB LZMA (100–110 MiB unpacked, 147–411 files, per-file SHA-1) | 2.8 MB from Solaris |
| First-start download, singleplayer | 26.1.2 `client.jar` 36 MiB from Mojang (26.3 data is in the client jar it has); facts as today | the same as dedicated, about 190 MiB | 2.8 MB (today) |
| First-start work | zip reads, JSON to NBT for about 800 entries, facts unpack: seconds, no JVM | 53 s of Java work and 114 s CPU on a 12-core host, more on weak CPUs; 0.6 GB RSS | unpack: seconds |
| Mojang bytes on the Solaris static server | none | none | data pack subset, lang, derived payloads |
| Maintenance per release | four encoder tables and the packet tables, each checked by an ignored test against the Java oracle; facts bundle rebuilt and re-pinned in client and server | a runtime installer (manifest, LZMA, exec bits, macOS bundle layout) in launcher and server; import, capture and extractors as today | full cache republished and re-pinned in client and server; the Java import stays for the maintainer who builds it |
| Failure modes | facts bundle unreachable (one VPS, plain HTTP; the pin keeps integrity, not availability); a release needing a new table row fails the maintainer check, never a user | JVM fails to start (memory, antivirus, macOS quarantine); the temporary vanilla server binds a local port (Windows firewall prompt); 180 s readiness timeout on slow machines; datagen or bootstrap changes break the extractors | the same VPS dependency, for all content; a bundle the new server rejects breaks every fresh install (RC2) |
| 26.1.2 legacy cache | built by the same producer from the 26.1.2 jar | still no producer | needs a second content bundle |

## Consequences and migration, if accepted

1. Native encoder: done (`mc_data::registry_network`), unwired.
2. `tools/publish_facts.sh` gains the new fact files; a new bundle revision;
   launcher and `solaris-v2` pin it.
3. `content import` builds the 26.3 cache from the Mojang jars, the facts
   bundle and the encoder, and builds the 26.1.2 legacy cache the same way.
4. Packet-name tables for 777 and 775 in `mc-protocol`; `legacy/tables.rs`
   stops reading `packets.json`.
5. The launcher's singleplayer runtime runs that import against the jars it
   has instead of installing the content bundle; `tools/publish_content.sh` and
   the content pin are deleted.
6. The Java steps leave the user path. What remains of them depends on
   question (c).

When the owner accepts, the transition "vanilla content: Java import and the
published content bundle → Mojang jars + facts bundle + native encoder" becomes
a row in [open transitions](../STATUS.md#open-transitions); until then it is
listed there as an undecided proposal.

## Alternatives considered

- **Compile the fact tables into `solaris-v2`** (246 KiB zstd for 26.3,
  155 KiB for the 26.1.2 tables): no Solaris-hosted runtime dependency for
  dedicated servers. Not proposed because the facts would then live in Git and
  release binaries rather than the one published bundle the client already
  shares (`crates/mc-data/data/required_blocks.json` shows the repository
  already tracks some derived tables).
- **Port the overworld biome builder** to drop `overworld_expanded.json`:
  ADR 0001 forbids mechanically translating Mojang implementations.

## Owner questions

**(a)** May the server's remaining fact tables join the same published facts
bundle as new allowlisted files: the 26.3 `block_explosion.json`,
`block_map_colors.json` and `overworld_expanded.json` (a derived climate table,
under `facts/`), and the 26.1.2 `blocks.json` and `registries.json` for the
legacy cache? *Recommendation: yes.* They are pure non-asset tables of the kind
already allowlisted; the bundle grows by under 0.5 MB.

**(b)** Should first start fetch content automatically (Mojang jars and the
facts bundle) when no cache exists, or stay an explicit `content import` that
now needs no JDK? *Recommendation: automatic for singleplayer (the launcher
already fetches on first Play) and for a dedicated server started without a
cache, printing each download with its source; `content import` stays as the
offline command (`--from` local jars and a local facts archive).*

**(c)** Does the Java import stay as an operator path? *Recommendation: no.*
The native path becomes the one way for users and operators; the Java steps
move to the maintainer pipeline (`tools/publish_facts.sh` inputs and the
ignored oracle tests) and are deleted from `solaris-v2`, so there is one
importer and one cache producer.

**(d)** Does the RC5 content bundle (owner decision 2026-10-28) retire once the
native path ships, so that the Solaris static server serves no Mojang bytes?
*Recommendation: yes.* It costs singleplayer one 36 MiB download from Mojang on
first Play, and removes the per-release cache republishing and its RC2-style
breakage.
