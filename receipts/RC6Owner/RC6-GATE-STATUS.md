# RC6 release gate status — 2026-11-28

**D150 complete: Gates A–C 3 PASS, 0 FAIL, 0 BLOCKED.** A–B passed on the mandated, sourced client stand in native-03; C retains native-02's successful archive smoke. Gate D remains Main-owned, NOT RUN here. This is not overall release approval. No rebuild, second pin, tag, publication, source change, or ledger edit.

Evidence root `R = /run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner` below. Paths relative to R are receipt locations, not additional checkouts.

## Identity and scope

- Candidate source: `e1f87c207ee826706e0a4b1f684565d93b6150c4`, detached `/run/media/kaiserroman/games/solaris-work/wt/RC6Hist`.
- Server pin: `/run/media/kaiserroman/games/solaris-work/pins/e1f87c207ee8-server-x86_64-unknown-linux-gnu/krasnokamen`; SHA-256 `b485f3feb31dc8b08bc091f621be606cd8ce26f1e1bb6b0d61befca18252fb23`.
- **Candidate binary from retained pin (build identity recorded at release time).** A fresh build is not claimed: the earlier pin-release attempt refused the already existing immutable pin (`build.log`, `build.exit`). Main then explicitly required using the retained pin, without rebuilding.
- Native client for A/B (`native-03`, shared stand): `/run/media/kaiserroman/games/solaris-work/pins/c879f0ae38b7-client-x86_64-unknown-linux-gnu-incr/krasnokamen-client`; SHA-256 `b9acdc363f2176e8ec64005249786814d06f908ddb16d0184b4577b6584b60a2`. Gate C (`native-02`) used the prior client `/run/media/kaiserroman/games/solaris-work/pins/9252cc625802-client-x86_64-unknown-linux-gnu-incr/krasnokamen-client`; SHA-256 `b1101043148dd6125e6385796c415056477db28c0e2c11e13747aecb7a3b346d`. The client pins are from the separate client repository, not the server commit.
- Owner archive: `/run/media/kaiserroman/games/solaris-work/receipts/RC6Owner/private-linux/krasnokamen-rc6-private-e1f87c207ee8-x86_64-unknown-linux-gnu.tar.gz`; SHA-256 `e2f458a74638816e12a8e92910582cee4629763daed67171b6d80023e919a559`. Its extracted executable matched the server SHA above before execution.
- Identity receipts: `native-03/identity.json` (A/B), `native-02/identity.json` and `native-02/archive-members.json` (C).
- Server gameplay settings were the packaged defaults, including seed 0, view 8, simulation 8. Only loopback bind, an ephemeral port, and the existing licensed server-content path changed. No operators, injected items, fixture commands, changed gamerules, or reduced rendering settings. Fresh native settings, ordinary survival inputs, actual windowed native renderer on private Xvfb, isolated hardware-free audio. Automation/offline-test transport is explicit, not a claim of a manual human playthrough.

## Gate results

D149 mapping: A = same-tree player path; B = restart and custody; C = owner package smoke; D = host benchmark.

| Gate | Requirement / claim source | Actual run and tree | Verdict / receipt | Next action |
|---|---|---|---|---|
| A: Same-tree player path | D150 mandatory client stand; candidate `docs/development/release-and-tags.md:16-23` actual client requirement | C2, retained e1f87c server + stand c879f0 client. Join, ordinary movement, mine/pickup/place; after restart AIR at `(127,70,320)` and `(127,70,321)`, DIRT at `(127,71,319)` | **PASS** — `native-03/player-world/survival/scenario.json`, `commands-and-observations.json`, screenshots; `native-03/result.json:player_path` | Complete |
| B: Restart and custody | D150 same-world restart/inventory/block/WAL requirement | C2, same server pin/world; console stop exit 0 + checkpoint, restart, same player UUID, exact equality of observed inventory (one dirt in slot 0), authoritative block readback, clean second stop | **PASS** — `native-03/player-world/custody-before.json`, `custody-after.json`, `server-first.log`, `server-restart.log`, `recovery-log-lines.json`, `commands-and-observations.json`; `native-03/result.json:restart_custody` | No forced-crash/nonempty replay-tail claim: clean restart logs report no replay event. Slots reported unknown remain unknown; proof covers exercised dirt custody, not every container/armor slot |
| Owner package smoke | Main's clean-directory/EULA/world/real-client requirement; `/run/media/kaiserroman/games/solaris-work/receipts/RC6Owner/OWNER-CHANGELOG.md:7-14` | C1, exact archive hash above extracted into empty `native-02/package-clean`; `eula=true` written before launch, packaged executable starts, fresh world generates, real native player joins and renders, console stop exits 0 and commits checkpoint | **PASS** — `native-02/package-clean/eula.txt`, `gate.toml`, `joined-player.json`, `server-package.log`, `commands-and-observations.json`, `screenshots/package-real-client.png`; `native-02/result.json:owner_package` | Licensed content cache is supplied externally, not bundled. Writing acceptance is not a claim that this server enforces an EULA prompt |
| Host benchmark | Main's registered quiet `-x` window; `/run/media/kaiserroman/games/solaris-work/receipts/RC6Owner/MAIN-BENCH-PREP.md:17-26` | **NOT RUN by this lane** | No benchmark verdict; no inferred performance pass | Main owns admission and measurement |
| Previously completed loopback package smoke | D149 explicitly retains the existing result, separate from graphical Gate C | Prior `e1f87c2` executable, binary SHA `b485f3feb31dc8b08bc091f621be606cd8ce26f1e1bb6b0d61befca18252fb23`; protocol 777 ping/login/chunk | **PASS (prior receipt, not rerun)** — `/run/media/kaiserroman/games/solaris-work/receipts/RC6Owner/private-linux/smoke-777/smoke.json` | Headless/protocol evidence only, not a graphical client |

A–B verdicts come from completed native-03 (exit 0); C comes from native-02 (exit 0), not prior loopback logs. Harness capture checked actual-window GPU completion. Retained native-02 screenshots and native-03's post-restart screenshot were visually inspected: rendered worlds and dirt in the expected hotbar/hand. Persistence uses received block states, not screenshot interpretation. Native-03 intentionally did not rerun C; its `owner_package: not run` does not invalidate the retained C PASS.

## Reproduction command C1

Run from `/run/media/kaiserroman/games/solaris-work/wt/RC6Hist`; output directory must be new. The exact executed invocation used `native-02` and `audio-02`:

```sh
MIMALLOC_VERBOSE=0 MIMALLOC_SHOW_STATS=0 \
/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/scene-slot.sh \
/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh -m 8G \
env MIMALLOC_VERBOSE=0 MIMALLOC_SHOW_STATS=0 \
PYTHONPATH=/run/media/kaiserroman/games/solaris-work/wt/RC6Hist \
KRASNOKAMEN_AUDIO_RECEIPT_DIR=/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/audio-02 \
/run/media/kaiserroman/games/solaris-work/wt/RC6Hist/tools/harness/screen_audio_graph.sh \
python3 -B /run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/run_native_gates.py \
/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/native-02
```

`audio-02` was created before invocation. Transcript/exit: `native-02.log`, `native-02.exit`. The focused evidence runner reuses the candidate harness's process lifecycle, native launch/capture, and ordinary survival probe; it is not presented as a built-in named profile or a replacement for the whole release suite. Each gate's exact server/client argv, cwd, log paths, and actions are in its `commands-and-observations.json` and `client-launch.json`. One heavy command at a time; no quiet/performance window requested.

## Earlier attempts and explicitly unrun coverage

- `native-01` **failed before client launch**: runner incorrectly used the client's reports-only directory as the server content cache (missing `version.json`). `native-01.log` and `native-01/result.json` retain the failure. Runner was corrected to use the already retained server cache `/run/media/kaiserroman/games/solaris-work/receipts/RC6Owner/offline/bundle/content`; native-02 is the successful run. No product code or pin was changed.
- Earlier broader-scope installer check: **PASS**, `/run/media/kaiserroman/games/solaris-work/wt/RC6Hist/.analysis/validation/20261128T082008-installer-l7h7t41t/result.json`.
- Earlier broader-scope harness-check: **FAIL**, inherited mimalloc stdout diagnostics contaminated an exact child-output assertion; 188 tests, one failure. Receipt `/run/media/kaiserroman/games/solaris-work/wt/RC6Hist/.analysis/validation/20261128T081947-harness-check-gtnx2lr0/result.json`. Not rerun after scope narrowing; no harness-check pass claimed.
- Earlier full correctness run: **INTERRUPTED**, explicitly stopped when Main narrowed this lane to three gates. Receipt `/run/media/kaiserroman/games/solaris-work/wt/RC6Hist/.analysis/validation/20261128T082041-correctness-x57gfx_f/result.json`. No candidate full-correctness pass claimed.
- Java client, macOS, sibling full suites, crash injection/nonempty WAL-tail replay, plugin GUI, networked content download/import, upgrade conversion, benchmark, and release publication: **NOT RUN in this three-gate execution**. Prior owner receipts and parent-tree 9310-test evidence are historical claims only, not substituted candidate gate passes.
- Historical claim audit remains in `claim-source-audit.log`: owner archive/version packaging, revision 33, prior offline import refusals/bridge evidence, and parent-tree correctness plus selector-only candidate delta. Candidate package's private RC6 VERSION does not change its rc5 Cargo package version. None of these facts proves the unrun gates above.

No ledger rows are owned or edited by this lane under D149. Main must combine these receipts with Gate D and any remaining release-policy decisions before declaring release readiness.

## D150 command C2 and mandatory client identity

Sourced `/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/client-stand` before the run. All exported content/resource/asset variables were inherited unchanged, not hand-wired by the runner.

- A–B client: `/run/media/kaiserroman/games/solaris-work/pins/c879f0ae38b7-client-x86_64-unknown-linux-gnu-incr/krasnokamen-client`.
- SHA-256: `b9acdc363f2176e8ec64005249786814d06f908ddb16d0184b4577b6584b60a2`.
- Identity/environment: `native-03/identity.json`, `native-03/environment.json`.
- Gate C still uses the earlier 9252cc client identity above; Main required the stand rerun for A–B only.

```sh
. /run/media/kaiserroman/games/solaris-work/solaris/tools/dev/client-stand
MIMALLOC_VERBOSE=0 MIMALLOC_SHOW_STATS=0 \
/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/scene-slot.sh \
/run/media/kaiserroman/games/solaris-work/solaris/tools/dev/mem-run.sh -m 8G \
env RC6_GATES=AB MIMALLOC_VERBOSE=0 MIMALLOC_SHOW_STATS=0 \
PYTHONPATH=/run/media/kaiserroman/games/solaris-work/wt/RC6Hist \
KRASNOKAMEN_AUDIO_RECEIPT_DIR=/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/audio-03 \
/run/media/kaiserroman/games/solaris-work/wt/RC6Hist/tools/harness/screen_audio_graph.sh \
python3 -B /run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/run_native_gates.py \
/run/media/kaiserroman/games/solaris-work/wt/D145Release/receipts/RC6Owner/native-03
```

`audio-03` was created before launch; cwd was the detached RC6Hist checkout. Logs/exit: `native-03.log`, `native-03.exit`. For historical C1 reproduction, its original runner is retained at `native-02/runner.py`; the current runner now requires the sourced D150 stand. Preserve these completed evidence directories; use fresh output directories for any future execution.
