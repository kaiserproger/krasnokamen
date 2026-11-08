# Releases and tags

Authorization rules are in [AGENTS.md](../../AGENTS.md): Main has standing
authorization for small commits and pushes to `main`; workers stay on their own
worktree branches and do not push. Tags, releases and other publication need
explicit owner authorization. The two release reviewers and the advisor's role are in
[review committee and breaker](review-committee-and-breaker.md#release-reviewers-owner-2026-11-07).
The breaker passes before the tag and before the announcement are in the
[break-it QA protocol](../QA_AGENT_PROTOCOL.md) "Release-candidate breaker
pass". This page is the release procedure for the four repositories:
`kaiserproger/solaris` (core), `solaris-client`, `solaris-loader` and
`solaris-default-plugins`.

## Core release cut

1. **Final tree gates.** `run correctness` once after the last code change
   (rerun only a failed test after a fix), plus `run harness-check`,
   `run installer`, `run build --release` and `run java` (the Loader's Gradle
   tests). `core-client` is chat-only; cover plugins, economy and Loader with
   `standard-pack` (also `--real-client`), real-client purchase/refund QA, and
   `loader-live` on every advertised platform against the frozen server with a
   no-Loader refusal check. Rerun any previously blocked owner scenario exactly
   (same profile, actor and refusal type); never substitute a similar one.
2. **Clean-checkout proof.** A detached worktree of the release commit with no
   `data/vanilla`: `cargo check --locked --workspace --all-targets` in its own
   target dir; then simulate CI by extracting vanilla data from the local
   server jar (`tools/extract-vanilla-data.sh`) and running `run test`.
   Vanilla data loads at runtime from the content cache: no compile-time
   `include_str!` of `data/vanilla`, and every production or bench registry load
   passes a `vanilla_data_dir` (`CoreRegistry::load(None)` is test-only).
   **macOS:** there is no local Mac. Push the candidate commit to
   `rc-check/<tag>` and delete the branch afterwards;
   `.github/workflows/rc-check-macos.yml` runs the `mc-core`, `mc-world`,
   `mc-world-conversion` and `mc-world-tools` lib tests (nextest `ci` profile,
   which excludes a fixed list of tests that were
   slow in one old run) on `macos-15` with the 26.3 content cache. Pass = a
   green run whose log prints the candidate SHA; save the log under
   `.analysis/releases/<tag>/`, then `git push origin --delete rc-check/<tag>`.
   This covers core tests only; the notes must not claim a macOS client or
   package run.
3. **Siblings first.** Rebuild every shipped `plugin.wasm` against the current
   WIT. Commit and push `solaris-default-plugins` and `solaris-loader` before
   core, then update `SOLARIS_PLUGINS_REVISION` / `SOLARIS_LOADER_REVISION` in
   `.github/workflows/ci.yml`. At every RC bring each sibling's local `main` up
   to the line that ships (owner, 2026-11-07: the client's local `main` had
   fallen 150 commits behind `pauline/client-next`), and run `tools/dev/wt-gc`
   to list dirty or unmerged worktrees older than a week.
4. **Upgrade traps.** Diff between the previous tag and this one:
   `REQUIRED_FILES` / `REQUIRED_REGISTRY_PAYLOAD_DIR` in
   `crates/mc-edge/src/content_cache.rs` (an old cache then fails at startup
   with "no vanilla content cache found": re-import) and `WORLDGEN_REVISION` in
   `crates/mc-worldgen/src/lib.rs` (startup fails with "identity mismatch: world
   was generated with worldgen revision N": run `solaris-v2 world
   upgrade-worldgen --config <cfg>` on the stopped server; existing chunks stay
   as generated). Put both steps in the release notes; singleplayer worlds
   need the same upgrade. A content-cache change also means a new singleplayer
   content bundle (below).
5. **Release notes.** `docs/releases/<tag>.md`: CI copies it into every server
   archive as `CHANGELOG.md` and uses it as the GitHub release body. Include
   upgrade notes, the first-start `content import ... --download` step and the
   validation boundary. Before writing QA results into the notes, check each
   run's attributes in its own receipt, not the agent's summary (an RC4 Windows
   soak summary implied fresh profiles; only two of three runs had them). Keep
   a cause "likely" unless the mechanism was instrumented. Before the release
   review, write `.analysis/releases/<tag>/claims.md`: every claim of the notes
   with its receipt path and run count. State counts ("3 of 3 runs"), never
   "always"; a claim about another version ("RC4 behaves the same") needs the
   same scenario run on that version; an advertised platform without a run is
   named as not run. (RC5: three of four review rejections were such claims.)
6. **Receipt.** `.analysis/releases/<tag>/receipt.yaml`: every gate with its
   receipt path, scope notes and `not_run` items.
7. **Commit, tag, push** (authorized only). Stage nothing local-only
   (`.analysis/`, `data/vanilla/`, `versions/`, `libraries/`, `perf.data`, stray
   `target/`). One Conventional Commit as the owner identity in AGENTS.md;
   check `git merge-base --is-ancestor origin/main HEAD`; `git tag -a <tag>`;
   push main and the tag, each command alone with its own exit status. The
   release job marks `v0.*` tags and any tag containing `-` (such as `-rc.N`)
   as prereleases. Watch with
   `gh run watch <run-id> --exit-status` (async, timeout >= 3600).
8. **Release reviewers** before tagging, even when gates are green: release
   contents and core state as a whole
   ([procedure](review-committee-and-breaker.md#release-reviewers-owner-2026-11-07)).

## A pushed tag whose CI failed

1. Diagnose: `gh run view <run> --json jobs` plus the failed job log (save under
   `.analysis/releases/<tag>/`); `gh release view <tag>` (not found when publish
   was skipped).
2. Fix as one small commit on top of the tagged commit, only what CI needs.
   Repin the client's `SOLARIS_REVISION` in its `release.yml` if the core
   commit changes.
3. Draft every step (R1 push core main, R2 retag, R3 push client main, R4
   client tag) with exact command, from/to, fast-forward proof and the current
   `git ls-remote origin refs/tags/<tag>` object.
4. Retag with lease: `git tag -f -a <tag> <sha> -m '<same message>'` then
   `git push --force-with-lease=refs/tags/<tag>:<old object> origin
   refs/tags/<tag>`. Never force-push without a lease; never revert published
   refs on your own initiative.
5. `gh run watch` can exit 1 while the last binary job still runs: re-check
   `gh run view --json status`. Pass = the publish job succeeded and
   `gh release view <tag>` lists assets. A client branch push runs no CI
   (release workflow triggers on tags, PRs and dispatch); verify with
   `ls-remote`.
6. After each step append exit codes, run id, conclusion and URLs to the draft,
   then present the next step. Deployment is a separate approval.

The owner preferred a new tag (for example the next `-rc`) over moving a tag
that was already pushed and announced. Cancel a broken tag's running CI
(`gh run cancel`) before it uploads assets.

## Client release and the singleplayer content bundle

The client's singleplayer starts a local `solaris-v2`. On first Play the
launcher downloads the pinned content bundle into
`<data root>/servers/content/<ver>` and uses it as `vanilla_data_dir`. The
launcher trusts the bundle by pin and `version.json`, so a bundle the new server
rejects breaks singleplayer on every fresh install while server tests and
multiplayer gates stay green (RC2, 2026-10-31: the bundle lacked
`assets/minecraft/lang/en_us.json`).

1. **Detect** before tagging client or core: diff `REQUIRED_FILES` between the
   core tags, download the pinned bundle and run
   `<new solaris-v2> content verify --cache <extracted>/content`
   (`--config` needs a full config; `--cache` is the direct form).
2. **Republish:** start from a complete freshly imported cache; make sure the
   client's content publish script covers every top-level directory the server
   requires; seed the output manifest from the **live** manifest so facts and
   older bundles stay listed; the script runs `content verify` and prints the
   pins. Upload to the static host's `/srv/static/` (see
   [test-server operations](test-server-ops.md)): copy bundle and file list,
   back up the manifest as `solaris-content.json.pre-r<N>`, write the new one to
   a `.tmp`, `chmod a+r`, `mv` into place, and verify by downloading again.
3. **Pin** `CONTENT_NAME`/`CONTENT_SHA256` in the launcher (`launcher/src/pins.rs`)
   and bump the client's `version` (`Cargo.toml`, `launcher/Cargo.toml`, both
   lock entries). Bump `SOLARIS_REVISION`/`SOLARIS_PLUGINS_REVISION` in the
   client's `release.yml` to the pushed core/plugin commits.
4. **Prove singleplayer through the packaged client UI**, once on the local
   package before tagging and once on the downloaded release tarball
   (`gh release download ...`, `sha256sum -c`): package dir with
   `solaris-client`, `solaris-launcher`, `solaris-v2` and `plugins/` (the release
   plugin set); a data root whose `servers/content` is empty so the launcher
   really downloads; `XDG_DATA_HOME` set. Title, Singleplayer, Create New World
   twice (creating does not start it), Play Selected World,
   `minecraft_wait_for_play` in the overworld, screenshot, disconnect back to
   Title. Assert the language file exists in the installed bundle and
   `installed.json` lists the new bundle and SHA.
5. No client release or demo with known bugs (owner, 2026-10-24): a failing
   minimum gate keeps the GitHub release a draft. Tag the client only after a
   live chest/inventory smoke; if QA finds a crash, cancel the run and delete
   the unpublished tag. Name the matching client tag in the core notes.

## Loader release

Repository `../solaris-loader` (Gradle, Java 25), in a worktree under `<work>/wt/`.

1. Bump the version everywhere: `build.gradle.kts` (`allprojects { version }`),
   `fabric.mod.json`, Forge `mods.toml`, NeoForge `neoforge.mods.toml`,
   `LOADER_VERSION` in the loader-core runtime settings, the README download
   table and the plugin API version it names.
2. Build and test under the runner:
   `./gradlew --no-daemon --no-configuration-cache clean :loader-core:test
   :loader-fabric:test :loader-neoforge:test :loader-forge:test` plus the three
   `:jar` tasks. Report passed and skipped separately from the XML results.
3. Real-client gate on all three platforms from the core repo:
   `run scenario loader-v2-settlements-smoke --platform fabric|neoforge|forge
   --server-bin <release solaris-v2> --plugins-dir <empty dir>` with
   `SOLARIS_LOADER_ROOT=<loader worktree>` and `SOLARIS_CLIENT_JAR=<client jar
   for the version>` (without it the Java agent fails to compile). Use
   `run scenario <id>`: the plain profile rejects `--server-bin`. Forge can time
   out downloading Mojang assets; one retry is a network retry. Look at a
   screenshot.
4. Version-bump traps: mixin crashes appear in `client.log` as
   `InvalidInjectionException`; check every `@Inject` target with `javap -p`
   against the new client jar. NeoForge sends serverbound payloads only on
   channels declared via `minecraft:register`, otherwise
   `Payload solaris:loader/hello may not be sent to the server!`.
5. Publish (authorized only): fast-forward main, annotated tag `v<ver>`, stage
   the three jars, `sha256sum ... > SHA256SUMS`, `gh release create v<ver> -R
   kaiserproger/solaris-loader --verify-tag --prerelease --notes-file NOTES.md
   <jars> SHA256SUMS`, then download again and `sha256sum -c`. Notes name the
   platform baselines and the minimum server version per adapter, say the Loader
   is required only when the server marks a client bundle required, and claim
   only what the gates covered (Gradle adapters, not every launcher).
