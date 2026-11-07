# Project status

Where Solaris stands and what to work on next. Update this page when a release
ships or a known 1.0 gap opens or closes; day-to-day progress belongs in commits
and the [bug ledger](bugs/README.md), not here. Last updated 2026-11-07.

## Release

- Published: **1.0.0-rc.5 "Pauline"** (core tag `v1.0.0-rc.5`, native client
  release with matching server and plugins). Notes: [RC5](releases/v1.0.0-rc.5.md);
  all tags: [release notes](releases/README.md).
- RC5 checks on the published assets: checksums match; a cold Linux install of
  the client entered a new world, relaunched and re-entered it. On a 4-core
  Windows VM without a GPU the first two Play attempts timed out
  (`SP-START-TIMEOUT-SLOW-MACHINE`); walking in the world and disconnecting
  were not checked there. No macOS client or package run exists (core lib tests
  only).

## Priorities

From the [owner contract](development/owner-contract.md), in order:

1. Ordinary vanilla-client gameplay and multiplayer.
2. The production plugin API and adapters.
3. Measured optimization, ECS, ownership and autoscaling.
4. Rare edges.

A lower item moves up only when it blocks ordinary play, saves or plugins.

## Work queue

The owner's open GitHub issues are the queue:

```sh
gh issue list -R kaiserproger/solaris --author kaiserproger --state open
```

Known bugs, with their status and regression checks, are rows in
[`bugs/ledger.toml`](bugs/README.md).

## Known gaps before 1.0

From the independent whole-core review of RC5 (2026-11-07). Each needs either a
fix or an owner decision recorded in the release notes.

| Gap | State |
|---|---|
| A scoreboard with about 500 or more scores disconnected every joining player (per-tick outbox cap 512) | Fixed after RC5 (`fix: batch scoreboard updates for reliable delivery`); not released yet |
| The bug ledger is not a reliable 1.0 gate: rows fixed in code are still `open`, some fixes have no registered regression check, and `regressions-client` has not run for the RC5 cycle | Open: reconcile the ledger and run `regressions-client` before 1.0 |
| Phantoms and pillager patrols never spawned in survival in RC5 | Fixed after RC5 (`feat(core): phantoms, patrols, ...`); not released yet |
| The plugin API ships as `solaris:plugin@0.8.0` with no stability promise | Owner decision |
| A world whose RC4 first start was stopped (database without checkpoint, no first-start marker) refuses to open forever; the world never had a player | Documented in the RC5 notes; no tool offers a fix |

Unconfirmed risks from the same review, each with the check that would settle it:

- Windows: a briefly locked WAL batch file (antivirus, sync tools) during the
  rename stops the world. Check: a 1 h or longer Windows soak, singleplayer and
  hosted, with Defender on and the world in an indexed or OneDrive folder.
- A function that sends more than about 500 chat lines in one tick may
  disconnect every recipient (same outbox cap). Check: 600 `tellraw @a` lines
  with two players online.
- Conduit and shrieker still panic on a bad block entity if some write path
  skips chunk validation (piston, structure, plugin).
- After a power cut the journal directory may miss a batch (directory fsync
  every 10 batches), so recovery refuses the world. Check: a power-cut test on
  ext4 and xfs.
