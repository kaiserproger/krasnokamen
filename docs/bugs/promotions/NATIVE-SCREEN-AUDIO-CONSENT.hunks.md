# NATIVE-SCREEN-AUDIO-CONSENT — hunk map

Row: `NATIVE-SCREEN-AUDIO-CONSENT` (client, major) — the permission dialog promised pictures only
while Allow granted screen-audio capture/transmission. Refs:
`harness:screens-voice-qa:owner_consent_names_app_audio`, `harness:screens-voice-qa:owner_deny_sends_no_audio`.

## Real fix and its pre-image

Client `87967272` ("fix(client): disclose app audio in screen sharing consent", 2026-10-24) is the fix;
it predates the client rename `f0a6abf6`, so the route is the committed-reversal pair. Its a/-side is
real history and is restored verbatim:

| element | pre-fix (a/-side, restored in the control) | fix (re-land) |
|---|---|---|
| title | `Allow desktop viewing?` | `Allow screen and audio sharing?` |
| body[1] | `Pictures only, while looking. No remote input.` | `Includes app audio, only while looking. No remote input.` |
| choice 1 | `Allow viewing all my open screens` | `Allow all my open screens and their app audio` |
| allow message | `Viewing allowed while they look. F8 revokes ALL viewers; Shift+Esc closes` | `Screens and app audio shared while they look. F8 revokes ALL viewers; Shift+Esc closes` |

- reversal `dc21342f52` (pre-fix strings), re-land `fa434351c6` (the row's `fix_commit`).
- `git diff dc21342f52 fa434351c6` is exactly the four strings (patch-id equals the re-land's own
  non-test patch).
- Kept unit ref `computer::native::sharing::tests::permission_prompt_defaults_to_deny_and_discloses_scope`
  (added by the same commit): red on the control (`negctl/consent-unit-control.log`, right
  `Allow screen and audio sharing?`), green on the re-land (`negctl/consent-unit-restore.log`).

## Other hunks of 87967272 and why they are not in the pair

| file | hunk | reason |
|---|---|---|
| `src/computer/relay.rs` | `@@ -1,4 +1,4` module doc comment ("desktop pictures" → "desktop pictures and app audio") | (c) a doc comment, not the dialog behaviour |
| `docs/AGENT_TESTING.md` | the optional-permission section | (c) documentation |

The test hunk of `87967272` (the assertions added in `sharing.rs`'s `mod tests`) is kept unchanged on
both sides — it is the row's unit ref.

## Gate

`screens-voice-qa`, leg `screens`: a native owner receives the consent dialog for a viewer's request.
`owner_consent_names_app_audio` requires "audio" in the title, "app audio" in the body and "audio" in
choice 1; `owner_deny_sends_no_audio` requires that Deny starts no capture and sends no packets. On the
control the wording verdict is false; the deny verdict is unchanged by wording and must stay green on
both sides (it proves the deny path, not the words).
