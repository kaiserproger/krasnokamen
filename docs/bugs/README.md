# Bug regression ledger

`ledger.toml` records each bug with `id`, `title`, `area` (`gameplay`, `client`,
`harness`; set from the symptom at filing and not changed by fixers),
`severity` (`blocker`, `major`, `minor`), `status` (`open`, `fixed`),
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
- `regression`: a list of green `cargo:<crate>:<exact test path>` or
  `harness:<profile>:<scenario>` checks; initially it may be empty.
- `repro_check` is optional: a nonempty string or list of nonempty strings for
  known-red reproducers, kept out of the green regression gate.
- `ingress_proof` is optional: a nonempty string or list of strings (`file:line` write paths with what each
  validates). See the unreachable-state rule below.
- `failing_first` is optional and, when present, is `reported` or `not recorded`;
  report the actual pre-fix evidence, never infer it from a passing test.

The phase reviewer checks the original symptom, failing-first and fixed-build
evidence before promoting a row to `fixed`; fixers do not self-promote based
only on an implementation or an unrelated green check.

Add a failing-first check for a fix. `cargo:<crate>:<exact test path>` names
an exact test; `harness:<profile>:<scenario>` names an existing runnable
scenario. Every green Cargo check in `regression`, even for an open row,
runs on every integration; scenario checks run in the client tier. Keep a known-red reproducer in `repro_check`
instead; it is not a gate. Never weaken or delete checks to make a run green.

A manual/source row whose state no ingress can produce (importer, commands, WAL replay, plugins,
structure placement all validate) is closed by a Cargo check through the real tick path plus a
recorded ingress proof (refs to each validating write path).

A row is fixed only when its check passes and is the same or stricter than the
original repro. Owner/manual and real-client repros, and all client-surface
bugs, require a real-client check; source/test findings may use a Cargo test
through the same real tick/gameplay path or a TCP/native scenario. Tests must
assert vanilla or owner-accepted behavior, not merely the current output.
One row describes one symptom: split covered behavior from remaining gaps.
The phase reviewer confirms each promotion batch matches the symptom and
failed before the fix. A unit test cannot close a known client failure.
Client promotion requires an assertion matching the repro; visual-review
scenarios require recorded review.

Run `python3 -m tools.harness run regressions` on every integration and
`python3 -m tools.harness run regressions-client` when promoting a
client-surface fix and before release. The independent breaker in
`../QA_AGENT_PROTOCOL.md` remains a release requirement.

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
- `native-v2-auth-budget`: the shipping `solaris-auth` package behind the native login gate.
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
  records are validated by the importer through their block and never reach the server.
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
- `native-v2-map-*`: separate frame waypoint, banner removal/locked
  toggle, inventory clone tracking, pumpkin hiding, terrain shading and Nether
  terrain repros. Read-only receipts reconstruct the graphical viewer's exact
  map packets; screenshots still require review, especially marker visibility.
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

Client-tier scenarios run the native client at `SOLARIS_CLIENT_ROOT` (default:
the sibling `solaris-client` checkout). The P2, mount-kind and map-marker
promotions were gated with client branch `pauline/client-post-rc4` (`e1b22a5`);
the released client `5d34e88` lacks those fixes, so point `SOLARIS_CLIENT_ROOT`
at a checkout of that branch or later.

Only promote a row after recording its fixed-build and failing pre-fix
receipts. Repeat timing-variable probes at least three times. A probe's
registration or screenshot alone is not acceptance evidence.
An unchanged green pre-fix scene or a capture-only visual comparison is not a
negative control: the scenario must fail at the affected assertion. Written
visual review supplements that machine check; it does not replace it.
