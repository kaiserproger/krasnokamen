# Bug regression ledger

`ledger.toml` records each bug with `id`, `title`, `area` (`gameplay`, `client`,
`harness`; set from the symptom at filing and not changed by fixers),
`severity` (`blocker`, `major`, `minor`), `status` (`open`, `fixed`),
`source`, `repro` (`manual`, `client`, `cargo`, `source`) and
`regression`. Historical reports are not proof of the current candidate.

Add a failing-first check for a fix. `cargo:<crate>:<exact test path>` names
an exact test; `harness:<profile>:<scenario>` names an existing runnable
scenario. Every green Cargo check in `regression`, even for an open row,
runs on every integration; scenario checks run in the client tier. Keep a known-red reproducer in `repro_check`
instead; it is not a gate. Never weaken or delete checks to make a run green.

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
- `native-v2-axolotl-play-dead` and `native-v2-potion-custom-impact` fail closed
  when their original effect, target-release or graphical-impact requirements
  remain unproven. Their registration does not promote their ledger rows.

`MODEL-BANNER-PLACEMENT` covers the banner symptom only. The profiled-head
fixture still needs a verified listed player's published skin
(`MODEL-PLACED-HEAD-PROFILE`); decorated-pot presentation has a separate
string-versus-item-template sherd mismatch (`MODEL-PLACED-POT-DECORATIONS`).
Neither open split is closed by the banner's wire or screenshot assertions.

The shared native wire observer uses the pinned 26.3 packet IDs and vanilla
`ByteBufCodecs.BIT_SET` byte-array masks. Heightmap long arrays are a different
field. KeepAlive is not chunk data.

Only promote a row after recording its fixed-build and failing pre-fix
receipts. Repeat timing-variable probes at least three times. A probe's
registration or screenshot alone is not acceptance evidence.
An unchanged green pre-fix scene or a capture-only visual comparison is not a
negative control: the scenario must fail at the affected assertion. Written
visual review supplements that machine check; it does not replace it.
