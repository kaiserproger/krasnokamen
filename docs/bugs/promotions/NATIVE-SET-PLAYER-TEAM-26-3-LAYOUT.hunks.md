# NATIVE-SET-PLAYER-TEAM-26-3-LAYOUT — hunk map for the committed reversal pair

Promotion receipt: `promotions/NATIVE-SET-PLAYER-TEAM-26-3-LAYOUT.json`.
Client pair: reversal `client:07dd26cce6` → re-land `client:5aca2e4a1c` (the row's `fix_commit`).

## Why a reversal and not the pre-image pin

The historical fix `client:ef4dc465` is post-rename and its single production hunk reverse-applies
on the wave-2 tip, but wave 1 blocked this row on the stage-3 client: the pre-fix client they pinned
(`f4bd26d4`) and every pre-rename restore could not start in the private graph. The committed
reversal pair keeps the tip's protocol/admission code and deletes only the decode change, so the
control is a working client that fails exactly at the row's symptom.

## Original commit

`client:ef4dc465` "fix(ui): decode SetPlayerTeam in the exact 26.3 layout" (2 files:
`src/ui/hud.rs` production, `src/ui_hotfix_tests.rs` the `rc1_team_packet_decodes_the_26_3_parameter_order`
unit test, which stays).

## Hunks of `client:ef4dc465`

| file | hunk | content | disposition |
|---|---|---|---|
| `src/ui/hud.rs` | module doc | names SetPlayerTeam as the 26.3 layout | REVERSED |
| `src/ui/hud.rs` | `const TEAM_COLOR_NONE` | `ChatFormatting.RESET` sentinel (21) | REVERSED — back to the old VarInt formatting read |
| `src/ui/hud.rs` | `@@ -309,14 +305,8 @@ fn decode` | 26.3 order: display, prefix, suffix, Visibility, CollisionRule, `Optional<TeamColor>`, options byte | REVERSED — old order (display, options, visibility, collision, formatting, suffix) |
| `src/ui_hotfix_tests.rs` | — | the unit ref | KEPT (test hunk, excluded from the pair by `non_test_patch`) |

## a/-side adaptation

None: `git apply -R` of `ef4dc465`'s production patch applies to the wave-2 tip with context intact,
so every reversed line is the historical pre-image, and `git diff 07dd26cce6 5aca2e4a1c` is exactly
that patch (patch-id `9809f73b444a1ce22ec6e8d2c5b6517ada2410f7`).

## Proving ref

`harness:entities:native-v2-team-deaths` (added to the row's `regression`; the row had only the
client unit ref, and a `client:<sha>` fix needs a graphical ref to carry the control). The scene
issues `team add red` / `team add blue` / `team join` as its first fixture commands: a client that
decodes Play 0x6f in the pre-26.3 order drops the session on the first packet, so the control run
fails at the first assertion after the team commands; the restore run passes the whole scene
(generic_kill and mob-kill texts on both clients and every `deathMessageVisibility` value).
`proving_assertions` records the assertion the control run was observed to fail.
