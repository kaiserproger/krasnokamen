# HC-1 — hunk map

Row: `HC-1` (client, major) — "Reconnect retains old voice capability/registration/listening and may
reopen microphone on non-voice server." Refs:
`harness:screens-voice-qa:native_voice_resets_on_non_voice_server` and
`client:krasnokamen-client:session::title_connection_tests::disconnect_forgets_the_previous_server_voice_negotiation`.

## Real fix and its pre-image

Client `a35187ca396ebcc6cafa82ef6cc5232745630fbc` ("Port native screen sharing and voice support",
2026-11-11) predates the client rename `f0a6abf6` (2026-11-12), and it is a bulk feature port: 38 files,
~8055 insertions (the voice transport `src/voice.rs`, `src/voice/app.rs`, the screen-share relay
`src/computer/*`, the Loader block/resource plumbing, the UI/options wiring). No buildable pre-fix
client exists, so the route is a committed-reversal pair on the tip (docs/bugs/README.md:269-281):

- reversal `cce9fb57cbb258b23303861561d3a3154eb67057` — deletes exactly the row's two lines in
  `Session::disconnect`, keeps every test (the row's unit ref reds on this tree:
  `negctl/hc1-unit-control.log`, exit 101 at `!session.voice_net.registered`).
- re-land `a78495f632423584760c339451900c5d018f7f71` — restores them and is the row's `fix_commit`
  (`negctl/hc1-unit-restore.log` green).

`git diff cce9fb57cb a78495f632` is those two lines only; patch-id equals the re-land's own non-test
patch (the ledger re-checks it with `_check_client_control_delta`).

## The row's path (a35187ca's only hunk this row owns)

```diff
--- a/src/session.rs
+++ b/src/session.rs
@@ -626,6 +653,10 @@ impl Session {
         self.network = None;
         self.loading = None;
         self.loader.reset();
+        self.screen_relay = Default::default();
+        self.screen_audio.clear();
+        // Voice capability, registration and LISTEN belong to one connection.
+        self.reset_voice();
         self.content = None;
         if let Some(audio) = &self.audio {
             audio.stop();
```

HEAD position: `src/session.rs:735-739`. The pair reverses the two voice lines; the two
`screen_relay`/`screen_audio` lines of the same hunk stay in place because they belong to the
screen-share rows (`P3`, `NATIVE-SCREEN-AUDIO-CONSENT`), not to HC-1 — one hunk, two owners.

## The kept definition (`src/session.rs`, a35187ca `@@ -805,6 +836,112`)

`reset_voice()` itself (and `register_voice`, `set_voice_mic`, `audio()`, `screen_dimension`) is kept
on both sides: the definition is also called from `src/session/packets.rs`'s `START_CONFIGURATION`
arm (a35187ca `@@ -600,6 +621,9`), and removing the method would not compile. Only the disconnect
call is deleted in the control, which is exactly the behaviour the row describes (a reconnect keeps
the old registration instead of clearing it).

## The rest of a35187ca and why it is not in this pair

The commit is the wave-1 voice + screen-share feature port; its remaining non-test hunks add those
features and are owned by other rows, so they are (b)/(c) here:

| group (files) | what it adds | reason |
|---|---|---|
| `src/voice.rs`, `src/voice/app.rs`, `src/audio/voicemix.rs`, `src/audio.rs` | the voice transport, jitter/render mixer and its audio hooks | (b) the voice feature itself (rows: `NATIVE-SCREEN-AUDIO-CONSENT`, voice sessions); the row only needs the disconnect reset |
| `src/computer/{audio,loopback,mod,native,native/sharing,nested,relay,texture,testsource,wayland,windows}.rs`, `src/computer/native/*` | the screen-share relay and per-backend capture | (b) `P3` and the screen-share rows |
| `src/ui/desktop.rs`, `src/ui/screens.rs`, `src/ui/state.rs`, `src/ui/options.rs`, `src/render.rs`, `src/render/frame.rs` | consent dialogs, monitor faces, voice HUD | (b) consent (`NATIVE-SCREEN-AUDIO-CONSENT`) and HUD rows |
| `src/loader/*`, `src/protocol/content/{loader_blocks,loader_resources}.rs`, `src/lib.rs`, `src/session/actions.rs`, `src/session/observation.rs` | Loader channel/capability plumbing and observations | (b) protocol/Loader rows; the row's observation (`voice`) already existed |
| `src/session/packets.rs` `@@ -600,6 +621,9` | resets screen + voice on `START_CONFIGURATION` | (a) kept: same commit, different event; not the disconnect path and not needed to reproduce the row |
| new files (`src/voice/*`, `src/computer/*`) | created by this commit — no pre-image | (c) nothing to reverse |

`Cargo.lock`/`Cargo.toml`/docs hunks of the commit are dependency and documentation lines: (c)
unrelated to behaviour.

## Gate

`screens-voice-qa`, leg `voice` (`SCREENS_VOICE_QA_LEG=voice`): the profile negotiates voice with a
voice-enabled server, then `voice_reconnect()` disconnects, asserts the client's `voice` observation is
`{registered:false, server_enabled:false, listening:false}` and no HUD microphone node, reconnects to a
server with `[voice] enabled = false` and requires the same cleared state. On the control tree the
disconnect keeps the old registration, so that assertion is false.
