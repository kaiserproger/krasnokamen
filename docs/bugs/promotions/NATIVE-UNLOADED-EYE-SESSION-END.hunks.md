# NATIVE-UNLOADED-EYE-SESSION-END — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-UNLOADED-EYE-SESSION-END.json`.
Client pair: reversal `client:390a7659af` → re-land `client:6048e4756e` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

Wave 1 tried the historical pre-image (`12e076e9`, `cc3a7c7b`) and both died before Play in the
current private graph. `ace7a804`'s own production patch cannot be reversed wholesale because its
`tools/unloaded_eye_scene.json` hunk no longer applies (a later commit moved the scene marker); the
registered `native-unloaded-eye` scene reads the tree's current copy, so only the three source hunks
are reversed and re-landed, and the control keeps the current scene.

## Original commit

`client:ace7a804` "fix(client): retain sessions while teleport terrain loads" (5 files; the three
production sources below, plus `tools/unloaded_eye_scene.json` and `docs/AGENT_TESTING.md`, which
are not part of the pair).

## Hunks of `client:ace7a804`

| file | hunk | content | disposition |
|---|---|---|---|
| `src/player.rs` | `@@ -42,6 +42,13 @@ pub struct InteractionWorld` | the unloaded-eye pose/medium carrier on the player | REVERSED |
| `src/player.rs` | `@@ -638,12 +645,16 @@ impl Player` | eye medium stays `None` while the destination terrain is absent | REVERSED |
| `src/player.rs` | `@@ -655,7 +666,11 @@ impl Player` | the same for the camera eye | REVERSED |
| `src/player/movement.rs` | `@@ -89,8 +89,13 @@ impl Player` | keep the observed pose while the world is not presentable | REVERSED |
| `src/player/movement.rs` | `@@ -458,7 +463,7 @@ impl Player` / `@@ -494,13 +499,12 @@` / `@@ -518,7 +522,7 @@` | movement/physics gates on presentation readiness | REVERSED |
| `src/session/observation.rs` | `@@ -179,9 +179,9 @@ impl Session` | session end decision defers while the eye terrain is unloaded | REVERSED |
| `src/session/observation.rs` | `@@ -881,9 +881,11 @@ impl Session` | the retained-session observation fields | REVERSED |
| `tools/unloaded_eye_scene.json` | — | the scene marker | NOT REVERSED — its pre-image no longer exists; the control runs the current registered scene |
| `docs/AGENT_TESTING.md` | — | documentation | OMITTED |

## a/-side adaptation

None for the three source files: the reverse patch applies with intact context, so the reversed
lines are the historical pre-image, and `git diff 390a7659af 6048e4756e` is exactly that patch
(patch-id `6c61c8634efc22abb38de0065919ed6b37c1b78c`).

## Proving ref

`harness:scene-shot:native-unloaded-eye` → `unloaded_eye_session_survives`, which the row already
registers. The scene forwards unmodified server packets, holds the stream after the far teleport,
and asserts the session survives after the initial Play, null eye medium while unloaded, advancing
GPU-completed actual-window frames, and the water medium once terrain arrives.
