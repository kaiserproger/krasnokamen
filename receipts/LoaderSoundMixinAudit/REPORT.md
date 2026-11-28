# LOADER-SOUND-MIXIN is not a defect of the shipped Loader

Date: 2026-11-28 ~18:05 host. Repo `/run/media/kaiserroman/games/solaris-work/solaris-loader`
(HEAD `d508697`).

## Claim

The row records a Fabric 26.1.2 / API 0.155.2 startup crash caused by
`ScreenSoundEngineMixin` redirecting `getStream` first, which broke Fabric's
`required 0/1` injection. Its own source line already said the class exists
"only on the unmerged loader branch `monitor-fabric`, where `432d29d` removed
it; loader main never had it".

## Evidence

| check | command | result |
|---|---|---|
| the removal commit exists | `git cat-file -t 432d29d` | `commit` |
| which branches contain it | `git branch -a --contains 432d29d` | `monitor-fabric` only |
| is it an ancestor of main | `git merge-base --is-ancestor 432d29d main` | exit 1 — **not** an ancestor |
| the mixin file's history | `git log --all --oneline -1 -- '**/ScreenSoundEngineMixin.java'` | `432d29d` only |
| the class on main | `git grep -l ScreenSoundEngineMixin HEAD` | no match (exit 1) |
| what main ships instead | `git ls-tree -r --name-only HEAD \| grep -i sound` | `LoaderSoundCommand`, `LoaderSoundDefinition`, `LoaderSoundPayload`, `ScreenAudioSound` plus `LoaderSoundCommandTest` and `ScreenAudioSoundTest` |

So on the Loader that ships, the offending mixin does not exist: `432d29d`
introduces the sound handling without it, and that commit reached only
`monitor-fabric`. A `loader:432d29d` reversal on main would refuse anyway,
because the checkout does not contain the fix.

## Independent live evidence

`harness:loader-live:loader-fabric-screen-admission` passed on Loader
`d50869792617` with the core server SHA256
`ec650c260bfa18ca1be375e6f25535fe44e640363178a16320987b11b17c8425`
(receipt `receipts/D145Client/LOADER-ZOMBIE-SHIELD-MIXIN/candidate-result.json`,
2026-11-28): a real Fabric client reaches Play on the shipped Loader. That is
the class of failure this row describes (startup crash before join), and it does
not occur.

## Disposition

Row deleted: it describes a defect of an unmerged branch, cannot recur on the
shipped Loader, and its fix is the commit that removes the mixin — which travels
with `monitor-fabric` if that branch ever lands. Keeping it in the queue would
count a non-existent blocker against the release.

Limit: this audit proves absence and a passing admission run on 26.3. It does
not run a 26.1.2 client; if a 26.1.2 Fabric admission is required for A8, that
belongs to A8's own gate, not to this row.
