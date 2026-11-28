# Live lanes (D145/D151, host snapshot 2026-11-28 16:00 +07)

Measured with `git rev-list --count main..HEAD` in each worktree; heads are short
SHAs at that moment. An area lane owns every open row of its area unless a row
names another lane; a row is closed only by `python3 -m tools.harness ledger
promote` on the lane's own receipts.

| lane (agent) | worktree | head | ahead of main | area / current work |
|---|---|---|---|---|
| LedgerGameplay | `wt/D145Gameplay` | `fcc7fcd73` | 2 | gameplay: planner + clock controls done; foot-steering reversal next |
| LedgerGameplay2 | `wt/D145Gameplay2` | `503e95bd0` | 1 | gameplay tail: evidence blockers mapped, pin build |
| CmdBlocks | `wt/CmdBlocks` | `bbf5b71e5` | 0 | gameplay commands/blocks (dirty tree, 13 paths) |
| LedgerClient | `wt/D145Client` | `d1ae7e453` | 3 | client: mounted candidate; LOADER-ZOMBIE-SHIELD-MIXIN next |
| LedgerClientTail | `wt/D145ClientTail` | `4722b06a0` | 7 | client non-blocker tail: frozen-presentation pair in preparation |
| LedgerClientNet | `wt/LedgerClientNet` | `d03fe0bd5` | 3 | client network rows (dirty tree, 3 paths) |
| LedgerClientMinor | `wt/CMinorA` | — | — | client minor rows |
| LedgerHarness | `wt/D145Harness` | `e54e4fb3b` | 0 | harness rows (dirty tree, 5 paths) |
| LedgerHarnessEv | `wt/LedgerHarnessEv` | `aeda9aadc` | 1 | SPEAR: control + restore measured on the pinned core; loader branch `wt/LedgerHarnessEvSpear` |
| ReleaseRC6 | `wt/D145Release` | `bbf5b71e5` | 0 | RC6 gate audit, `receipts/RC6Owner/RC6-GATE-STATUS.md` |
| SpainD7Re | `wt/SpainD7Re` | `ecdcb2269` | 1 | SpainInteg family 1 terrain port (`construction_reach.rs`) |
| SpainF2 | `wt/SpainF2` | `3511c5e81` | 0 | SpainInteg family 2 founding outposts (12 files under edit) |

Open rows at the same moment: 204 (blocker 4, major 141, minor 59) across
gameplay 78, client 82, harness 44. The four open blockers:
WINDOWS-CLIENT-BUILD-BROKEN (blocked: needs a Windows/MSVC host, D149 excludes
the VM), LOADER-ZOMBIE-SHIELD-MIXIN (owned: LedgerClient),
LOADER-SOUND-MIXIN (routed to the Loader repository, external),
BATTLE-STEEL-ARMOUR-OBSCURES-TEAM (owned: LedgerClient, server-side dyed accent
pieces; the live front/rear frame pair is a separate graphical gate).

This file did not exist before; the previous reference to
`receipts/D153/ROW-OWNERS.md` in session notes was wrong. Lane ownership lives
here and in the row's own `source`/`owner` note, not in a second list.
