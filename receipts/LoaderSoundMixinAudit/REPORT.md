# LOADER-SOUND-MIXIN: the class is absent from the shipped Loader, the 26.1.2 repro is unrun

Date: 2026-11-28 ~18:10 host. Loader repo `/run/media/kaiserroman/games/solaris-work/solaris-loader`
(HEAD `d508697`). Row `docs/bugs/rows/LOADER-SOUND-MIXIN.toml`.

## What the row claims

A real Fabric 26.1.2 / API 0.155.2 startup crashed before join because
`ScreenSoundEngineMixin` redirected `getStream` first, breaking Fabric's
`required 0/1` injection.

## What the Loader tree says (measured now)

| check | command | result |
|---|---|---|
| the removal commit exists | `git cat-file -t 432d29d` | `commit` |
| which branches contain it | `git branch -a --contains 432d29d` | `monitor-fabric` only |
| is it an ancestor of main | `git merge-base --is-ancestor 432d29d main` | exit 1 — **not** an ancestor |
| the class on main | `git grep -l ScreenSoundEngineMixin HEAD` | no match (exit 1) |
| the file's only history | `git log --all --oneline -1 -- '**/ScreenSoundEngineMixin.java'` | `432d29d` |
| what main ships instead | `git ls-tree -r --name-only HEAD \| grep -i sound` | `LoaderSoundCommand`, `LoaderSoundDefinition`, `LoaderSoundPayload`, `ScreenAudioSound`, plus `LoaderSoundCommandTest` and `ScreenAudioSoundTest` |

So the offending mixin exists only on `monitor-fabric`, and `432d29d` — the
commit that replaces it with the sound API stream hook — removes it. On the
Loader that ships, the code path the row blames is absent.

## Why the row stays open

- The original repro is a **26.1.2** Fabric startup. It has not been run here,
  and `docs/RULES.md` keeps a failure alive until its original repro passes.
  What is established is narrower: the defect cannot occur on Loader main.
- The ledger cannot promote it either: `loader:432d29d` would have to reverse a
  commit that is not in the checkout, and the commit deletes
  `ScreenSoundEngineMixin.java`, which the loader control rule refuses as a
  control file (`docs/bugs/README.md`, Loader fixes).
- The 2026-11-25 audit in the row already recorded "NOT a Fabric-launch
  blocker"; the severity now says `major` to match that finding.

## Consequence

Not a known failure of Loader `main`. To verify the original failure, run
Fabric 26.1.2 with API 0.155.2 through startup and entry to Play on the Loader
fix and its pre-image. A8's mixed-version check counts toward this proof only
if it uses that Fabric/API pair. A passing run is evidence, not a ledger
promotion; the row remains open.

The 26.3 counterpart (`harness:loader-live:loader-fabric-screen-admission` on
Loader `d50869792617`) passes and is recorded on
`LOADER-ZOMBIE-SHIELD-MIXIN`; it is not cited here as proof of the 26.1.2 case.
