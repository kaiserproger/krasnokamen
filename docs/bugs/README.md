# Bug regression ledger

`ledger.toml` records each bug with `id`, `title`, `area` (`gameplay`, `client`,
`harness`; set from the symptom at filing and not changed by fixers),
`severity` (`blocker`, `major`, `minor`), `status` (`open`, `fixed`; `fixed`
only via `ledger promote`),
`source`, `repro` (`manual`, `client`, `cargo`, `source`) and
`regression`. Historical reports are not proof of the current candidate.

## Filing a bug

Copy a `[[bug]]` block in `ledger.toml` and fill in every required field:

```toml
[[bug]]
id = "AREA-SHORT-SYMPTOM"
title = "Describe the observable failure"
area = "gameplay"
severity = "major"
status = "open"
source = "Repo-relative report or plain description"
repro = "manual"
regression = []
failing_first = "not recorded"
```

- `id`: unique, stable uppercase hyphenated symptom identifier; `title`: concise symptom.
- `area`: `gameplay`, `client`, or `harness`, assigned at filing and not changed by fixers.
- `severity`: `blocker`, `major`, or `minor`; `status`: file as `open`.
- `source`: repo-relative location or plain description, not an owner-local path.
- `repro`: `manual`, `client`, `cargo`, or `source`, according to the original report.
- `regression`: a list of green `cargo:<crate>:<exact test path>`,
  `harness:<profile>:<scenario>`, or `harness:<profile>:<verdict>` checks;
  initially it may be empty. Profile verdicts run the windowed native client,
  except `settlement-map-qa:localized_text`, which needs the Russian locale and
  runs vanilla Java with `--language ru_ru`.
  A profile may publish boolean `details.verdicts` keyed by behaviour; the
  referenced key must be exactly `true`. The `all` key denotes the whole
  profile; when no boolean verdict map exists, `all` requires a passing
  profile receipt. Legacy profiles expose their existing `PASS` behaviour
  maps: `village-qa` has `door_walk`, `sleep_return`, `night_door`,
  `farmer_harvest`, `miner_deposit`, and `lumberjack`; autonomy, claim and
  army expose their named behaviours. `buildings-qa` uses `<blueprint>.<behaviour>`, for
  example `wall.L1_built`. Partial or resumed profile overrides are rejected.
  A passing profile is evidence only for symptoms its assertions actually cover.
- `repro_check` is optional: a nonempty string or list of nonempty strings for
  known-red reproducers, kept out of the green regression gate.
- `ingress_proof` is optional: a nonempty string or list of strings (`file:line` write paths with what each
  validates). See the unreachable-state rule below.
- `failing_first` is optional and, when present, is `reported` or `not recorded`;
  report the actual pre-fix evidence, never infer it from a passing test.

`fix_commit` (optional until promotion) is the lowercase hex id of the commit
that fixes the row; `promotion` is set only by `ledger promote`, see
[the `fixed` rule](#the-fixed-rule-single-home).

Add a regression check for a fix. `failing_first` records pre-fix evidence only
if you saw it; `ledger promote` runs the negative control.
`cargo:<crate>:<exact test path>` names
an exact test; `harness:<profile>:<scenario>` names an existing runnable
scenario. A profile verdict runs that profile once for all rows citing it:
the child receipt must pass and the named behaviour must be true in its
boolean verdict map, or `PASS` in its legacy behaviour map. Unknown profiles
and verdicts fail validation. `harness:loader-live:<scenario>` is valid for
graphical `loader-live` scenarios: the runner executes them exactly as the
loader-live gate does (Fabric platform, the tree's own built release server
via `--server-bin`, an empty `--plugins-dir`, the private audio wrapper, and
the Loader checkout/Gradle home/client jar from `KRASNOKAMEN_LOADER_ROOT`,
`GRADLE_USER_HOME` and `KRASNOKAMEN_CLIENT_JAR`), records the harness Xvfb
display in the receipt, and fails closed (unrunnable, never green) when any
of those are missing or the scenario is not a registered loader-live entry.
`regressions-client` builds its own release server (a known exception to the
`pin-release` rule, listed in [open transitions](../STATUS.md#open-transitions))
in the enclosing runner's pooled target, then hard-links it (copies if it
cannot) outside the pooled target before executing clients.
`KRASNOKAMEN_REGRESSIONS_BIN_DIR` selects that directory; the receipt records the
pinned path and SHA-256. Pin any supplied client binary separately.
Every green Cargo check in `regression`, even for an open row, runs on every
integration; client checks run in the client tier. Keep a known-red reproducer
in `repro_check` instead; it is not a gate. Never weaken or delete checks to
make a run green.

A manual/source row whose state no ingress can produce (importer, commands, WAL replay, plugins,
structure placement all validate) is closed by a Cargo check through the real tick path plus a
recorded ingress proof (refs to each validating write path).

A row is fixed only when its check passes and is the same or stricter than the
original repro. Owner/manual and real-client repros, and all client-surface
bugs, require a real-client check; source/test findings may use a Cargo test
through the same real tick/gameplay path or a TCP/native scenario. Tests must
assert vanilla or owner-accepted behavior, not merely the current output.
One row describes one symptom: split covered behavior from remaining gaps.
The phase reviewer confirms each promotion batch matches the symptom and has
the required failing negative control; seeing the original failure before
editing is recorded when available, not required. A unit test cannot close a
known client failure.
Client promotion requires an assertion matching the repro; visual-review
scenarios require recorded review.

`python3 -m tools.harness run regressions` is part of L2 (`correctness`); run it
alone only after an integration that skips L2. Run
`python3 -m tools.harness run regressions-client` when a client-surface fix
is promoted and before release. The independent breaker in
`../QA_AGENT_PROTOCOL.md` remains a release requirement.

## Write-ups for blockers, data loss, dupes and agent incidents

For a `blocker` row, a data-loss or dupe bug, or an incident an agent caused,
the write-up lists the contributing factors and which defense layers (tests,
gates, reviews, guards) held or failed, not a single root cause: each fault was
necessary and only together they were sufficient
([Cook](https://how.complexsystems.fail/), points 3 and 7). Write what was
known when each action was taken, without "should have" (point 8). Judge a
fix by whether it removes a factor or adds a defense layer, not by who erred.

## The `fixed` rule (single home)

A row becomes `fixed` only through `python3 -m tools.harness ledger promote
<ID>`. Nobody sets `status = "fixed"` or `promotion` by hand: `run regressions`
fails a `fixed` row without a `promotion` receipt, and a receipt that does not
match the row's `id` and `fix_commit`. The only rows exempt are those already
`fixed` at base `ffd52010` (frozen list `tools/harness/ledger_base_fixed.py`; a
reopened row leaves the list in the same edit).

**What workers do:** add `regression` refs (and `repro_check`, `failing_first`
evidence) and record `fix_commit` (the one commit with the fix; the test may be
in it or earlier, never only in later commits). They do not edit `status` or
`promotion`, and they do not decide that a row is fixed: a green test that also
passes without the fix proves nothing, which is why the script, not the
worker, runs the negative control.

**What `ledger promote` checks**, all of which must hold:

1. `fix_commit` is recorded and is contained in the current tree, or, for a fix
   not yet committed, is `patch:<sha256>` of the non-test hunks of the git diff
   passed as `--fix-patch` (a mismatch prints the value to record; the patch
   must still apply in reverse on the candidate, so a later edit or re-break of
   it refuses). The candidate is the
   exact tree: `HEAD` plus its uncommitted diff and untracked files
   (`docs/bugs/` aside), identified by a fingerprint; no commit is needed. The
   control and the restore worktrees reproduce it, verified by that fingerprint.
2. The row has at least one registered check.
3. For `area = "client"` or `repro` in `manual`/`client`: a graphical harness
   ref, and `--client-receipt <result.json>` of a real-client run whose status
   is `passed`, which covers one of the row's real-client refs (a scenario ref
   by `named_scenario.id`; a profile ref by profile plus the named verdict),
   and whose `tree` field, written by the harness at run start, is the
   candidate's fingerprint (HEAD and the hash of the uncommitted state). A
   receipt without that field, from another revision or with other
   uncommitted changes is refused: rerun the gate on the candidate, and after
   any later edit. The receipt must also name a release server binary that
   the tooling built from exactly that candidate (`ledger probe --client` and
   `promote` register every server they build in
   `.analysis/ledger-builds/<sha256>.json`): a pinned or foreign
   `KRASNOKAMEN_SERVER_BINARY`, or a binary built from an older source, proves
   nothing about this source and is refused. A unit test cannot close a client
   failure.
4. Automatic negative control: the fix commit's non-test hunks (test files,
   `docs/` and Rust hunks after the first `#[cfg(test)] mod` stay) are
   reverse-applied onto the candidate in a temporary worktree; only if
   they no longer apply there, onto a worktree at `fix_commit`
   (`control_tree: head|fix_commit` in `promote.json`). Then every ref runs
   and at least one must be **red** at its assertion. A control that stays
   green, or whose reversal does not build, is refused (split the fix commit
   so tests do not need its new API, or promote nothing). A hunk straddling
   the test-module boundary is refused. Graphical refs run in the control on
   the real client.
5. The restore, always on a worktree of the exact candidate (never at
   `fix_commit`): every ref is **green**, so a regression after the fix
   refuses promotion. A ref already passed by the supplied client receipt (same
   `HEAD`) is not rerun, unless it carried the red control. A client ref is red
   only when the failed receipt carries an explicitly failing verdict for that
   profile check; a timeout, a client that never reached play (`in_play`), a
   missing prerequisite or any failure without a negative verdict is
   unrunnable and cannot be a control. A scenario ref has no verdict map; it is
   red only when the failed receipt's `details.named_scenario.id` is the ref and
   its own `error` reads `<id> assertion failed: <name>` (never the argv of the
   child), with `<name>` not a connection check (`in_play`, `*_entered_play`,
   join) and the failure not tagged before the scenario body (`join`, start-up
   phases). Any other failure, a timeout or a join flake never counts. A profile
   ref whose run cannot name a failing verdict follows the same assertion rule:
   `all` of a profile without a verdict map (`autocraft-qa`,
   `village-autonomy-qa`) is red when the failed receipt's `error` reads
   `<id> assertion failed: <name>`, and a `buildings-qa` `<blueprint>.<behaviour>`
   that was never recorded is red when that assertion's name starts with
   `buildings_<blueprint>_` (a probe check raises before the verdict is written).
   Control worktrees
   are temporary `git worktree`s beside the other worktrees (`<work>/wt/`),
   one promotion at a time (a lock), at fixed paths (`ledger-control-candidate`,
   `ledger-control-fix`). Every cargo build or test for such a tree runs through
   `mem-run.sh` with `cwd=<tree>` and without the caller's `CARGO_TARGET_DIR`, so
   the runner gives the tree its own slot (marks it, touches on a switch) and
   the fixed path reuses it instead of growing the target dir; the release
   server build checks the slot marker against the tree actually built. No
   incremental cache.
   Each has its own copy of `data/vanilla` (never links into the source tree)
   and is removed afterwards.

On success it writes `docs/bugs/promotions/<ID>.json` (commit, files reversed,
per-ref control and restore results, client receipt) and sets
`status = "fixed"` and `promotion = "docs/bugs/promotions/<ID>.json"` in the
row; commit both. `--dry-run` runs everything and writes only its receipts under
`.analysis/validation/<stamp>-ledger-promote-*/` (`promote.json`), never the
ledger; on an already `fixed` row it re-audits the fix (a reopened-row check).
A refusal prints the reason and exits 1.

```
python3 -m tools.harness ledger promote ID [--client-receipt PATH] [--fix-patch PATH] [--dry-run]
```

Heavy: on the owner host run it under `mem-run.sh` (`mem-run.sh -m 12G python3
-m tools.harness ledger promote ...`), which caps its memory and gives it a
build token. Lock order is that token, then the control lock; each build stage
runs `mem-run.sh` again and reuses the promote's token, so a promote waiting for
the lock never holds a token a running promote's stage needs. A promote that
took the lock without a token deadlocked with wrapped promotes waiting for it
(2026-11-12, the host stalled for an hour); the tool refuses to start in a
worktree under `<work>/wt/` without a held token.

## Probing rows before assignment

`python3 -m tools.harness ledger probe <ID...>|--open [--client] [--out PATH]`
runs each row's registered refs (`regression`) on the current tree, once per
distinct ref, and prints one line per row:

- `green`: every ref passed;
- `red [ref: status]`: a ref failed; the row is still broken here;
- `needs-real-client [cargo ref green, ...]`: a harness ref was not run (pass
  `--client` to run it; it builds a release server and drives the real client);
- `unrunnable(reason)`: no registered check, unknown or ignored test, build error,
  or a ref that cannot run.

Cargo discovery and execution select only the packages named by the requested
refs (`-p <crate>`), not the whole workspace. Exact-name matching and refusal
of missing or ignored tests still apply.

`--out` writes the full per-ref result as JSON (also valid YAML). Hand a worker
only rows that are not `green`: a row already green at base is fixed in
practice and needs promotion (or a better check), not a fix. Probe never
changes the ledger; it is not promotion.

`--client` (and `promote` of a client row) needs the real-client environment of
[real-client gates](../development/real-client-gates.md#3-environment-block):
a pinned client/server pair of the same protocol, the data, resource and asset
roots, and a fresh `XDG_DATA_HOME` whose `krasnokamen-client/` symlinks `facts
assets data resources mojang`; run it through `scene-slot.sh` and `mem-run.sh`.
The tool builds the release server itself and ignores `KRASNOKAMEN_SERVER_BINARY`;
that is a second pinning mechanism next to `tools/dev/pin-release`, kept until
its [open transition](../STATUS.md#open-transitions) closes. Do not extend it.
A stale `target/debug` client fails with `target protocol mismatch`, a missing
XDG with `missing content file`: both are reported `unrunnable`, never red.

## Block-entity packet privacy

Chunk records and block-entity update packets must expose only the relevant
vanilla update-tag fields, not saved container contents, loot seeds, commands,
bees or unrelated components. Open container menus have their own viewer-scoped
content packets. Decorated pots intentionally withhold their item and loot data
even where vanilla sends it. The `mc-world` wire privacy and registry tests and
the `mc-edge` direct-packet tests cover the fields in their fixtures; they do not
establish exhaustive coverage of every nested value or every real-client path.

## Native promotion probes

The `entities` profile includes windowed native-client probes:

- `native-v2-lease-navigation`: in a private operator-built fixture, a 120-second
  native action retains its controller, a cancelled writer is replaced without
  losing the replacement's held input, and navigation starts at a supported edge
  and traverses an open door from an offset pose. Cancellation scheduling is
  nondeterministic; deterministic owner-fence ordering remains a client unit test.

- `native-v2-end-restart`: enter the End on hard difficulty, identify the live
  dragon, switch to peaceful, save and restart, then require the same player,
  End world and dragon UUID. This covers live peaceful-fight persistence, not
  the dead-dragon branch or dragon model parity.
- `native-v2-entity-gallery`: remove a projectile during a mixed live-entity
  tick, then require the other fixture UUIDs and an ordered server response.
  Its negative control must show the original body-slot compaction panic,
  not merely a disconnected client.
- `native-v2-custody-mount-export`: create imported donkey cargo using the
  shipping export/import CLI, withdraw seven diamonds, deposit five emeralds,
  export/import again and assert both mount and player custody.
- `native-v2-p2-optional-pack`: observe a Configuration pack offer, a matching
  decline and Play on one error-free connection, then require playable chat.
- `native-v2-destroyed-chest`: records visible menu closure and custody after
  chest destruction. The MCP refusal to click an already-closed menu is not
  a stale server-packet test; this probe alone cannot close HS-1.
- `native-v2-destroyed-chest-race`: the HS-1 packet race. A second headless client populates the
  Nether so commands run through the shared (multi-world) path. In six rounds per branch the
  extraction or deposit click is issued right behind the destroying command, before the client has
  seen the menu close, with the simulation frozen. Whichever the server handles first, the one
  diamond must remain exactly once (inventory or ground) and the menu must be closed; a branch
  whose click never left the client fails the run. Custody is read from the final inventory and
  ground items, not from a server reply.
- `native-v2-duplicate-login`: a second headless native client of the same offline account
  joins while the first is in Play. The second reaches Play with the same player and exactly
  one diamond, the first leaves Play, and the first client's re-login replaces the second.
- `native-v2-auth-budget`: the shipping `krasnokamen-auth` package behind the native login gate.
  `/register`, and after a reconnect `/login`, run while the server's plugin worker thread shares
  one core with seven CPU hogs (the descheduling of a loaded host), so Argon2 takes several
  times its idle wall time. The plugin's reply must arrive and ordinary chat, refused while
  locked, must then be echoed. The unfixed host retires the plugin and kicks the player.
- `native-v2-corrupt-block-entities`: a jukebox, sculk shrieker and beacon, each alone in its own
  chunk, are saved, exported to Anvil, corrupted by one field in the exported copy, imported
  again and loaded beside the rejoining player. The server must refuse exactly the two
  undecodable chunks (one error naming each record), never panic, keep ticking past the beacon's
  80-tick pass and keep the player's own chunk. The checkpoint after the refusal and the shutdown
  checkpoint must both commit (one error lists every refused chunk), and a restarted server
  refuses the same chunks again. The beacon record is decodable by construction: that fixture is
  the runtime-path control and must not be refused. Crafter, shelf, shulker box and conduit
  records are validated by the importer through their block and never reach the server that way:
  a crafter and an oak shelf, each alone in its own chunk, instead reach it as forged opaque images
  in a tick-WAL batch (`wal_forge.py`) published over a clean shutdown checkpoint, under a foreign
  record id that replay's id-based validator does not check. The next start must refuse exactly
  those two chunks (one error naming each owner), never panic and keep ticking; a hydration owner
  that panics instead kills the tick thread before the server listens, which fails
  `server_starts_over_the_forged_wal_batch`. A conduit record has no such route (replay
  validation and `conduit::validate_loaded` refuse it at start, imports validate it), so
  CONDUIT-RUNTIME-PANIC has no check in this scenario that goes red under its fix.
- `native-v2-corrupt-chunk-near-camera`: known-red repro, not a gate. The same corruption in the
  player's own chunk keeps the native client on its loading screen.
- `native-v2-cursor-close`: pick up exactly three apples, close and reopen both
  ordinary inventory and crafting table, and require an empty received cursor
  before any repair click while retaining exactly three inventory apples.
- `native-v2-banner-placement`: ordinary placement transfers the red-cross /
  blue-stripe banner layers and both patterned/plain custom names into received
  block-entity NBT; the actual patterned and plain banners are captured for review.
- `native-v2-snow-golem-shear`: ordinary shears clear the same golem's received
  pumpkin flag, reveal its face and yield one carved-pumpkin pickup.
- `native-v2-peaceful-removal`: creates a zombie and cow with natural spawning
  disabled, switches to peaceful and checks that the same zombie disappears
  while the same cow remains. A graphical receipt and negative control are
  required before this scenario can support promotion.
- `native-v2-command-score-store`: checks generic damage, main-hand totem and
  custom death protection, writable death/player/total kill counters and a swap,
  stored result/success for give, clear and failed clear, and inventory counts.
  A graphical receipt and negative control are required before promotion.
- `native-v2-happy-ghast-harness`: an ordinary survival harness debit admits the
  graphical player and three independent native peers; a fifth peer is refused.
  Received passenger packets prove distinct seats and independent dismount.
  The public NBT view hides `krasnokamen:` keys, so the refusal is fenced by the
  fourth rider's later dismount in the same ordered passenger stream: no
  passenger event before it may carry a fifth id. The focused native
  checkpoint regression separately checks harness/rider custody on disk.
- `native-v2-mob-metadata-spot`: one frog, strider, camel and sniffer stand in
  frame on a private pad, a small slime is then eaten by the frog; the client
  must stay in play and its log must carry no decode failure (sniffer state is
  accessor 18; frog tongue, strider cold and camel dash are 19).
- `native-v2-p4-identities`: seven named `/damage` identities must reach the
  actual client's damage stream with health loss, followed by a lever-provoked
  warden's sonic attack through a preplaced wall, death screen and respawn. Command cases prove identity
  admission/emission, not freeze, wall-collision or cramming mechanics.
- `native-v2-p4-death-protection`: a held stick with nonempty custom consume
  actions saves the holder at one health, consumes one item, clears a preexisting
  slowness effect and applies authored speed without ordinary totem effects. The full P4 damage
  scenario still includes this check and the separate physical-source probes.
- `native-v2-team-deaths`: two windowed native clients (victim in team `red`,
  peer in team `blue`) read the chat their own client renders for a player
  `/kill` ("was killed", generic_kill rather than out_of_world), a zombie kill
  ("was slain by Zombie", the vanilla translation component rather than plain
  English text) and one death per `deathMessageVisibility` value (`always`,
  `never`, `hideForOtherTeams`, `hideForOwnTeam`); the victim's death screen
  is asserted by class and a retained capture, its message text is not exposed
  by the client observation.
- `native-v2-automation-storage`: the sculk latch section tests ordinary and
  calibrated sensors at multiple distances, including a rejected frequency.
  Comparators face their input: east toward the sensor for frequency readout,
  south toward the cake for calibration. Downward bulbs retain each pulse;
  the perpendicular bulb is a negative geometry control. The fixture supplies
  solid support beneath every component, removes ambient mobs and disables
  natural spawning so unrelated footsteps cannot toggle the latches. First
  bulbs must be unlit before the note is powered. The latch fixture runs
  unfrozen so scheduled comparator updates keep their ordinary timing; the
  preceding sensor probe observes active, then inactive (the ten-tick
  cooldown is asserted by the Rust sculk tests, not sampled by the client).
- `native-v2-map-*`: separate frame waypoint, banner removal/locked
  toggle, inventory clone tracking, pumpkin hiding, terrain shading and Nether
  terrain repros. Read-only receipts reconstruct the graphical viewer's exact
  map packets; screenshots still require review, especially marker visibility.
- `native-v2-input-menus`: physical number/F/Q/Ctrl-Q keys and sub-tick pointer
  edges in survival and creative inventory. Server snapshots check the source,
  reopened destination and survival carried stack. Creative cursors are local:
  captures must show the held stack, then its exact count must survive placement
  and reopening.
- `native-v2-input-pick`: physical middle-click and gameplay F in both modes;
  block/entity picking must not create free items in survival.
- `native-v2-input-camera`: frozen-world teleport and F1 hide/restore, checking
  captured camera and HUD pixels while the local client loop continues.
  `admin-commands` checks frozen/stepped server game time, not a stopped client
  clock: native input and rendering must keep running while the world is frozen.
- `native-v2-input-sand`: `/setblock` airborne sand in a frozen world becomes a
  rendered entity after `/tick step`, and the server holds it between steps:
  `data get entity … Pos` read in successive server ticks must stop changing
  (a server that keeps dropping it never does). The client predicts gravity
  between received position updates, then the sand lands and clears its source.
  Vanilla resends a moved falling block every 20 server ticks even while frozen,
  so the gate runs one tick per second, starts on a received update and steps
  twice in one command: the predicted position must equal vanilla's two-tick
  fall and the drawn pose must lie on that path, below the received one.
  Received, predicted and presented positions are distinct observations; only
  the captured frame and its reviewed pixels prove drawing.
- Axolotl play-dead and potion custom-impact probes are unregistered prototypes;
  they cannot be run from the harness, and `AXOLOTL-PLAY-DEAD` and
  `POTION-CUSTOM-PAYLOAD` stay open.

`MODEL-BANNER-PLACEMENT` covers the banner symptom only. The profiled-head
fixture still needs a verified listed player's published skin
(`MODEL-PLACED-HEAD-PROFILE`); decorated-pot presentation has a separate
string-versus-item-template sherd mismatch (`MODEL-PLACED-POT-DECORATIONS`).
Neither open split is closed by the banner's wire or screenshot assertions.

The shared native wire observer uses the pinned 26.3 packet IDs and vanilla
`ByteBufCodecs.BIT_SET` byte-array masks. Heightmap long arrays are a different
field. KeepAlive is not chunk data.

The operator-only `/debug menu` probe replies privately with one
`KRASNOKAMEN_MENU` JSON line containing the current container ID, carried stack,
and every menu slot. Each nonempty stack reports its item identifier, count and
complete serialized stack data under `components`; an empty stack has a null
item, zero count and null components. The image comes from server-owned menu
and backing storage, not client prediction. If the backing state cannot be
resolved, the command reports an error rather than inventing empty slots.
The craft-drag probes use ordered replies after pointer press, movement, release
and menu close; the crafting result slot is virtual and excluded from custody
totals. The paced Java and native probes also check right-button pickup and
release outside the menu. Screenshots remain necessary for visual review.

Client-tier scenarios run the native client at `KRASNOKAMEN_CLIENT_ROOT` (default:
the sibling `krasnokamen-client` checkout). The P2, mount-kind and map-marker
promotions were gated with client branch `pauline/client-post-rc4` (`e1b22a5`);
the released client `5d34e88` lacks those fixes, so point `KRASNOKAMEN_CLIENT_ROOT`
at a checkout of that branch or later.

A probe's registration or screenshot alone is not acceptance evidence; repeat
timing-variable probes at least three times before `ledger promote`.
An unchanged green pre-fix scene or a capture-only visual comparison is not a
negative control: the scenario must fail at the affected assertion. Written
visual review supplements that machine check; it does not replace it.
