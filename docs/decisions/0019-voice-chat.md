# ADR 0019: proximity voice chat

Status: accepted design; draft implementation (native relay + Simple Voice Chat compatibility), real-client QA pending.

## Decision

Native Solaris clients capture the microphone (push-to-talk), encode 20 ms mono
48 kHz Opus packets (~24 kbps) and send them to the server on the Play custom
payload channel `solaris:voice`. `mc-edge` (`voice.rs` pure relay, `voice_service.rs`
delivery) forwards each packet to the *listening* native clients that are in the
same dimension and within `range_blocks` (default 48, eye to eye) of the
speaker, excluding the speaker, attaching the speaker's uuid and position.
Receivers decode and play it spatialized. The server never decodes audio, holds
no audio state and persists nothing. Vanilla and non-native clients never see
the channel (no VOICE_STATE, no relay). Core authority is untouched: the only
inputs are poses already published by the native tick; relay runs on the
connection read task and a tick subscriber, never on the tick.

Rejected for the native channel: a separate UDP voice port (second NAT/firewall
surface; Play TCP latency is adequate for a draft). Simple Voice Chat
compatibility (below) does need UDP because that is the mod's transport.
Rejected: server-side mixing/decoding (CPU, no need),
trusting client-claimed positions (the server uses published poses only).

## Wire (protocol 1)

Big-endian. Every payload is `u8 version=1, u8 kind, body`; unknown version/kind
or malformed body is ignored (never a kick, never an error response).

Client -> server:
- `0 AUDIO`: `u32 seq`, then 1..=1000 Opus bytes (one packet). `seq` increments
  per 20 ms frame and wraps; the server relays it verbatim. A client sends nothing
  while not talking; a receiver treats a >200 ms gap as end of speech.
- `1 LISTEN`: `u8 on` (0/1). A connection receives AUDIO only after `LISTEN(1)`
  (default off), so clients with voice off or deafened cost no bandwidth.

Server -> client (native clients only):
- `0 AUDIO`: `16B speaker uuid, f32 x, f32 y, f32 z, u32 seq, opus bytes`.
  x/y/z is the speaker's **eye** position (feet + 1.62) from the last published
  tick pose (at most one tick old).
- `1 VOICE_STATE`: `u8 enabled, u16 range_blocks`, sent once when the native
  client joins. `enabled = 0` when `[voice] enabled = false`; clients then neither
  send nor play.

## Relay rules and limits

- Listener set: registered, `LISTEN(1)`, has a published pose (alive), same
  dimension index as the speaker, squared eye distance <= `range_blocks`^2, and
  not the speaker. A speaker without a pose (dead/not yet published) is dropped.
- Per-connection token buckets (speaker side): 60 packets/s, burst 20 packets;
  64 KiB/s, burst 16 KiB (each packet charged opus length + 6). Overload drops
  the packet; it is never punished beyond that. The general play packet limiter
  (`[anti_cheat]`) also still applies to the custom-payload packet id.
- Delivery is best effort on the listener's reliable queue: a packet is dropped
  when that queue is less than half free, so audio can never starve control
  packets or retire a connection. One encoding serves all listeners of a packet.
- `[voice] enabled = true`, `range_blocks = 48` (1..=1024).

## Simple Voice Chat compatibility

SVC players (Fabric/NeoForge mod `henkelmax/simple-voice-chat`, compatibility
version 20, wire read from its source: `net/SecretPacket`, `voice/common/*Packet`,
`NetworkMessage`, `Secret`) and native players hear and talk to each other. The
server speaks SVC's *server* side in `svc.rs` (pure wire/crypto) and
`svc_service.rs` (glue); routing stays in `voice.rs`'s relay. Proximity audio
requires the same dimension, published poses and `range_blocks`, with no self-delivery.
Opus is forwarded byte for byte; there is no transcoding (both are 48 kHz mono
20 ms Opus; SVC codec VOIP, MTU 1275).

- Play plugin channels: on join the server sends `minecraft:register` listing the
  serverbound `voicechat:*` channels. `voicechat:request_secret` (client version
  must equal 20, otherwise ignored with a log line, like SVC) is answered with
  `voicechat:states` (all known players) then `voicechat:secret` (random
  per-player AES-128 key, UDP port, player uuid, codec, MTU, distance, keep-alive
  1000 ms, groups enabled, voice host, recording off). `voicechat:update_state`
  (disabled) marks an SVC player deaf; `voicechat:state`/`remove_state` are
  broadcast to SVC clients as players join/leave. SVC groups support passwords
  and normal, open and isolated routing; empty ephemeral groups are removed.
  Group members hear each other non-positionally; normal and isolated speakers
  are not relayed to native proximity listeners. Native clients do not join groups.
  Categories and recording are not bridged.
- UDP socket (`[voice] svc_bind_address:svc_port`, default `0.0.0.0:24454`):
  datagram `0xFF, uuid, VarInt len, AES-128-GCM(iv12 || ct || tag16)` client ->
  server; `0xFF, VarInt len, sealed` server -> client. Unknown uuid is only
  answered if it is the unauthenticated ping (`PING_V1`, raw echo). Authenticate
  binds the sender address to the player, then
  `AuthenticateAck`; `ConnectionCheck` -> ack; `KeepAlive` both ways each
  second (peer dropped after 10 s). `MicPacket` (Opus bytes, whispering) enters
  the relay as the speaker's AUDIO; an empty mic packet ends the burst.
- Bridging: SVC mic -> relay -> native listeners as `solaris:voice` AUDIO
  (sequence from the SVC mic counter) and SVC listeners as `PlayerSoundPacket`
  (channel id = sender uuid, the client plays it at that entity). Native AUDIO ->
  SVC listeners as `PlayerSoundPacket` with the native `seq`; a native burst
  ends (empty sound packet) after the same >200 ms gap rule. Whispering SVC
  speakers are heard at half range. Rate limits apply to SVC speakers too.
- Config: opt in with `[voice] svc = true` (default false), `svc_port = 24454`, `svc_bind_address`,
  `svc_voice_host` (empty = the address the client joined on). Bind failure is
  logged and native voice keeps working.
- Deploy: open **UDP 24454** (or `svc_port`) on the host/VPS firewall.
- Only vanilla-like SVC clients are supported; the SVC server plugin API,
  proximity categories, audio recording and `forceVoicechat`/permission checks
  are out of scope.

## Validation status

Unit tests (`svc_tests.rs`: handshake bodies, AES vectors cross-checked with the SDK encoding, datagram framing, state packets; `voice_tests.rs`: native<->SVC bridging) and relay tests cover range, dimension, self-exclusion, listening,
pose requirement, limits, wire decode and disabled mode. Real-client audio QA
(two native clients, microphone, speakers) is pending.

`svc_service_tests.rs` runs the whole SVC path in-process: plugin-channel
handshake, a UDP client that writes datagrams exactly like the mod (encrypted
Authenticate/ConnectionCheck/Mic), SVC<->SVC, SVC<->native bridging, end-of-speech,
range, forged-secret, disabled-client and disconnect cases. The AES-GCM and
byte-layout vectors come from a JDK program mirroring SVC's `Secret`/`toBytes`
code. A real Fabric/NeoForge client with the SVC mod against this server has not
been run; the protocol 20 layouts were also checked against the
`voicechat-fabric-2.6.27+26.3.jar` release.
