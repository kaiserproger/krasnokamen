# CmdBlocks — command/block and settlement-site rows

Base `bbf5b71e5`. Worktree `/run/media/kaiserroman/games/solaris-work/wt/CmdBlocks`.

Scope check: `ledger list --area gameplay` has no other open block-state or
command-defect row in these modules; the remaining open rows near them are the
CIV-SEED24 / ISSUE-1 civ-gate rows owned by other lanes.

Owned ids and handoff state:

| row | handoff | control |
|---|---|---|
| DATAPACK-LIST-HOVER | returned | no acceptable control shape (recorded 2026-11-25); real-client receipt exists |
| DATAPACK-GENERAL-TERRAIN-CUSTOMIZATION-UNSUPPORTED | returned | feature gap: generator must consume custom biomes/dimension types/noise settings/features |
| DATAPACK-NEW-PACK-WORLDGEN-SKIP | fixed-ready | patch reverses at HEAD; red/green below |
| DATAPACK-WORLDGEN-OVERRIDES | fixed-ready | patch reverses at HEAD; red/green below |
| SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF | fixed-ready | HEAD-equivalent patch (plan clamp + stand level search; single commit masked by the stand rule); red/green below |
| SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR | fixed-ready | HEAD-equivalent patch (old commit hunk no longer reverses); red/green below |
| SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN | fixed-ready | HEAD-equivalent patch (old promotion patch stale); red/green below |
| SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES | fixed-ready | patch (const hunk dropped, class b) reverses at HEAD; red/green below |
| CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION | fixed-ready | patch reverses at HEAD; red/green below |

## Evidence

Green for all cargo refs of the eight checkable rows on this tree:
`.analysis/validation/20261128T092429-test-07wzg06v` (8/8 PASS).

| row | red control artifact |
|---|---|
| SETTLEMENT-HARVEST-REOPENS-FARM-REPAIR | `20261128T085243-test-5m5d7c8q` |
| SETTLEMENT-NATURAL-LUMBER-ZONE-ONE-COLUMN | `20261128T085525-test-h0jp3cnb` |
| SETTLEMENT-FURNACE-SITE-ON-ANOTHER-STRUCTURES-ROOF | `20261128T090043-test-6ydslkb4` |
| SETTLEMENT-NATURAL-MINE-ZONE-NEVER-CERTIFIES | `20261128T091846-test-a6mgtipl` |
| CIVILIZATION-NATURAL-FOREST-SITE-PREPARATION | `20261128T092013-test-2rpjhn8p` |
| DATAPACK-WORLDGEN-OVERRIDES | `20261128T092127-test-afysgogm` |
| DATAPACK-NEW-PACK-WORLDGEN-SKIP | `20261128T092226-test-wdggxjsd` |

Per-row notes: `receipts/CmdBlocks/<ID>/`.
