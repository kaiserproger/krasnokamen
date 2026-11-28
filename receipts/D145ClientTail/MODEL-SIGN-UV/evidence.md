# MODEL-SIGN-UV

Disposition: open; no symptom-level control or current graphical reproduction.

On client tree `530556e864c25bbf23a940aa13e81665442b7d3b`, `git -C /run/media/kaiserroman/games/solaris-work/wt/CTailClient show --name-status --format=fuller 7df8dd6e -- 'assets/solaris/models/entity/*sign*'` reports deletion of `sign_standing.json` and `sign_wall.json`. `git -C /run/media/kaiserroman/games/solaris-work/wt/CTailClient ls-tree -r --name-only HEAD -- assets/solaris/models/entity | grep -E '(^|/)(hanging_sign_ceiling|hanging_sign_wall|sign_standing|sign_wall)\.json$'` returned no paths. The locally installed 26.3 `oak_sign_rot_0.json` and `oak_wall_sign.json` inherit `minecraft:block/template_sign_rot_0` and `minecraft:block/template_wall_sign` respectively.

Restoring unused old files is not a real control of the current renderer. Neither red nor green graphical runs were performed; the original visual symptom remains unverified on the current client.
