# Releases and tags

[AGENTS](../../AGENTS.md) grants Main small commits/pushes to `main`; workers
stay on their own worktree branches and do not push.
Tags/releases/other publication need explicit owner authorization. Before every
tag: [two independent reviewers](review-committee-and-breaker.md#release-reviewers-owner-2026-11-07)
even with green gates; before tag **and announcement**:
[release-candidate breaker pass](../QA_AGENT_PROTOCOL.md). No advisor approval token.

Repositories: `kaiserproger/krasnokamen`, `krasnokamen-client`,
`krasnokamen-loader`, `krasnokamen-default-plugins`.
`run …` below means `python3 -m tools.harness run …` under the
[owner-host runner](heavy-commands-and-memory.md).
[Historical wording](README.md) retains original examples.

## Core release cut

| Stage | Required proof / action |
|---|---|
| Final tree | `run correctness` once after last code change (rerun only a failed test after a fix). Also `run harness-check`, `installer`, `build --release`, `java` (Loader Gradle tests) |
| Actual clients | `core-client` is chat-only: plugins/economy/Loader need `standard-pack` (also `--real-client`), actual purchase/refund QA, `loader-live` on every advertised platform against frozen server, plus no-Loader refusal. Previously blocked owner case must rerun exact profile/actor/refusal, not a substitute |
| Clean checkout | Detached release commit, no `data/vanilla`, private target: `cargo check --locked --workspace --all-targets`; then local jar extraction with `tools/extract-vanilla-data.sh` and `run test` to simulate CI. Production/bench registries pass `vanilla_data_dir`; `CoreRegistry::load(None)` is test-only, no compile-time `include_str!` of vanilla data |
| macOS core | No local Mac: authorized push of candidate to `rc-check/<tag>`, `.github/workflows/rc-check-macos.yml` on `macos-15`, 26.3 cache, nextest `ci` profile (fixed slow-test exclusions), lib tests for `mc-core`, `mc-world`, `mc-world-conversion`, `mc-world-tools`. Pass requires green log printing candidate SHA; save under `.analysis/releases/<tag>/`, then `git push origin --delete rc-check/<tag>`. Not client/package proof |
| Siblings first | Rebuild shipped `plugin.wasm` against current WIT. Commit/push plugins and Loader before core; update `KRASNOKAMEN_PLUGINS_REVISION` / `KRASNOKAMEN_LOADER_REVISION` in core `.github/workflows/ci.yml`. Every RC brings each local sibling `main` to shipped line and runs read-only `tools/dev/wt-gc` (week-old dirty/unmerged trees); owner 2026-11-07 after client main lagged 150 commits |
| Receipt | `.analysis/releases/<tag>/receipt.yaml`: every gate's receipt, scope and `not_run`. Reviews saved beside it |

**Upgrade traps: diff previous tag → candidate and document each applicable step.**

| Change | Operator action / limitation |
|---|---|
| `REQUIRED_FILES` / `REQUIRED_REGISTRY_PAYLOAD_DIR`, `crates/mc-edge/src/content_cache.rs` | Old cache may fail “no vanilla content cache found”: re-import and republish/re-pin singleplayer content (below) |
| `WORLDGEN_REVISION`, `crates/mc-worldgen/src/lib.rs` | “identity mismatch: world was generated with worldgen revision N”: stop server, `krasnokamen world upgrade-worldgen --config <cfg>`. Existing chunks retain generation. Singleplayer also needs upgrade |
| Rename `b24f0ce3` changed store tag/persisted IDs | First post-RC5 release refuses RC5 server and singleplayer worlds (“this world was written by a Solaris build from before the Krasnokamen rename”). Stop RC5, use its archived `solaris world export-anvil`, then `krasnokamen world import-anvil` into a **nonexistent** directory. Loses plugin state/settlements/block log; pre-rename `solaris:*` entity data remains unread (`IMPORT-ANVIL-PRE-RENAME-KEYS-IGNORED`); RC5 export unavailable on Windows (`WINDOWS-BACKUP-CONVERSION`). See [interchange](../OPERATING.md#native-worlds-and-anvil-interchange) |

**Release text:** `docs/releases/<tag>.md` becomes archive `CHANGELOG.md` and
GitHub body. Include upgrades, first-start `content import ... --download` and
validation boundaries. Before review write `.analysis/releases/<tag>/claims.md`:
every note claim → receipt path and run count. Read **each actual receipt**, not
agent summary (RC4 Windows: only 2/3 runs used fresh profiles). Say “3 of 3”, not
“always”; uninstrumented causes stay “likely”; another version's equivalence
needs that same scenario there; unrun platforms say not run (three of four RC5
review rejections were claim errors).

**Authorized publication, only after reviews:** no local-only `.analysis/`,
`data/vanilla/`, `versions/`, `libraries/`, `perf.data` or stray targets in staging.
One Conventional Commit with owner identity from AGENTS;
`git merge-base --is-ancestor origin/main HEAD`; annotated `git tag -a <tag>`;
push main and tag as separate commands with separate statuses. CI prereleases
`v0.*` and tags containing `-` (including `-rc.N`). Watch asynchronously:
`gh run watch <run-id> --exit-status`, timeout ≥3600 s.

## A pushed tag whose CI failed

1. Save `gh run view <run> --json jobs` and failed log under
   `.analysis/releases/<tag>/`; inspect `gh release view <tag>` (absent if
   publication skipped).
2. Small CI-required fix atop tagged commit; core SHA change also repins client
   `KRASNOKAMEN_REVISION` in `release.yml`.
3. Draft exact R1 core-main push, R2 retag, R3 client-main push, R4 client tag:
   command, from/to, fast-forward proof, current
   `git ls-remote origin refs/tags/<tag>` object. Owner prefers a **new tag**
   over moving an announced one; no self-authorized rollback of published refs.
4. If retag authorized: `git tag -f -a <tag> <sha> -m '<same message>'`, then
   `git push --force-with-lease=refs/tags/<tag>:<old object> origin refs/tags/<tag>`.
   Never force without lease. Cancel broken tag's running CI (`gh run cancel`)
   before asset upload.
5. Watch may exit 1 while a binary job still runs: inspect
   `gh run view --json status`. Success means publish job succeeded **and**
   `gh release view <tag>` lists assets. Client branch push triggers no release
   CI (tags/PRs/dispatch do); verify branch via `ls-remote`.
6. Append each command's exit, run ID, conclusion and URLs to draft before
   presenting next step. Deployment remains separate approval.

## Client release and the singleplayer content bundle

Launcher downloads pinned bundle on first Play into
`<data root>/servers/content/<ver>` for local `krasnokamen`'s `vanilla_data_dir`.
Pin + `version.json` trust is not server acceptance: RC2 (2026-10-31) omitted
`assets/minecraft/lang/en_us.json`, breaking fresh singleplayer despite green
server/multiplayer tests.

1. Before **client or core** tag, diff `REQUIRED_FILES`, download pinned bundle,
   run `<new krasnokamen> content verify --cache <extracted>/content`;
   `--config` instead needs full config.
2. Build from complete fresh import; publish script covers all required
   top-level dirs, verifies content and prints pins. Seed from **live manifest**
   to retain facts/older bundles. At static host `/srv/static/`
   ([operations](test-server-ops.md)): copy bundle + file list, back up manifest
   as `krasnokamen-content.json.pre-r<N>`, write `.tmp`, `chmod a+r`, atomic `mv`,
   download again and verify.
3. Update `CONTENT_NAME`/`CONTENT_SHA256` in `launcher/src/pins.rs`; bump client
   version in both Cargo manifests and both lock entries; release workflow pins
   `KRASNOKAMEN_REVISION`/`KRASNOKAMEN_PLUGINS_REVISION` to pushed commits.
4. **Packaged UI proof twice:** local package before tag, then downloaded release
   (`gh release download ...`, `sha256sum -c`). Package includes client, launcher,
   server and released `plugins/`; set `XDG_DATA_HOME`, start with empty
   `servers/content` to force download. Title → Singleplayer → Create New World
   twice (creation does not start) → Play Selected World → overworld
   `minecraft_wait_for_play` → screenshot → disconnect to Title. Verify installed
   language file and `installed.json` new name/SHA.
5. Owner 2026-10-24: no client release/demo with known bugs. Failed minimum gate
   leaves release draft; live chest/inventory smoke precedes tag. QA crash:
   cancel run and delete **unpublished** tag. Core notes name matching client tag.

## Loader release

`../solaris-loader`, Java 25/Gradle, isolated `<work>/wt/`:

1. Version everywhere: `build.gradle.kts` allprojects, `fabric.mod.json`,
   Forge `mods.toml`, NeoForge `neoforge.mods.toml`, runtime `LOADER_VERSION`,
   README downloads and its plugin API version.
2. Through runner:
   `./gradlew --no-daemon --no-configuration-cache clean :loader-core:test :loader-fabric:test :loader-neoforge:test :loader-forge:test`
   plus three platform `:jar` tasks. XML results report passed/skipped separately.
3. Core `run scenario loader-v2-settlements-smoke --platform fabric|neoforge|forge
   --server-bin <release krasnokamen> --plugins-dir <empty dir>`, each platform;
   `KRASNOKAMEN_LOADER_ROOT=<worktree>`, `KRASNOKAMEN_CLIENT_JAR=<version jar>`
   (required to compile Java agent). Plain profile rejects `--server-bin`.
   Forge asset-download timeout may get **one network retry**; inspect screenshot.
4. Check every mixin `@Inject` target with `javap -p` against the new client jar;
   `InvalidInjectionException` appears in `client.log`. NeoForge serverbound
   payloads require `minecraft:register` declaration or
   `Payload krasnokamen:loader/hello may not be sent to the server!`.
5. Authorized publish: fast-forward main, annotated `v<ver>`, stage three jars,
   `sha256sum ... > SHA256SUMS`;
   `gh release create v<ver> -R kaiserproger/krasnokamen-loader --verify-tag --prerelease --notes-file NOTES.md <jars> SHA256SUMS`;
   download again, `sha256sum -c`. Notes name platform baselines/minimum server
   per adapter, Loader required only when server marks bundle required, and
   actual gate scope (Gradle adapters, not every launcher).
