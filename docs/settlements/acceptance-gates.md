# Settlement real-client acceptance

These profiles exercise settlement behavior through a native client, with different fixtures and limits on what a pass demonstrates.

See [construction](construction.md), [production](production.md), and the [real-client gate procedure](../development/real-client-gates.md#village-autonomy-qa).

## Autonomous village

`village-autonomy-qa` checks an unowned generated village, production without player-supplied items, and client-verified Storage L1. It also requires another authored building and an authored L2 or higher building before claim. It then checks claiming through the dialog, refusal of a second claim, an owner order, and project resume after a real server and client restart.

The primary player is not an operator. In the isolated QA world, spawn protection is disabled; the second player receives operator permission for a teleport fixture. This does not make that player's claim an operator-only test. The restart allows up to 90 seconds for server shutdown and records `restart_shutdown_seconds`; a nonzero exit fails the clean-stop check.

## Owner order

`village-owner-order-qa` uses a fresh seed-100 world. It claims the village and checks an unfunded House L1 order and its missing-material dialog before and after restart. The harness then uses bounded console `item replace block` fixtures in the founding chest to fund construction, checks the new start notification and project status, and checks cancellation of a second queued order. This supplied-material scenario is not proof of autonomous material production.

## Civilization observation

`civilization-seed-qa` claims a generated village as an ordinary player, certifies the founding storage, and grants the player a founding stock for transfer through the chest inventory:

- 32 oak logs and 16 bread;
- 8 raw iron, 8 coal, and 32 cobblestone.

After claim and transfer, the observing player enters Creative mode. The observation concerns residents, not the player's hunger or combat survival. Generated buildings, beds, and residents are the baseline, not proof of later construction or births.

Snapshots cover project status, buildings, storage, ecology, and client views. During the first 3,000 accelerated ticks the harness observes at 300-tick intervals; thereafter it observes at 3,000-tick intervals, issuing 300-tick sprint commands. Unchanged project status, project UI, and buildings for 24,000 accelerated ticks ends the run as a stall. Reaching 96,000 ticks without a sustained stall also ends the bounded observation with an error, not a civilization pass.

## Completion and production observations

The autonomy completion oracle derives legal origins and rotations from certificate bounds, using the maximum width and depth across all authored levels as the rotation frame. It compares every authored non-air cell and stable rotated properties, including cells outside the certificate bounds. Oak-authored components must match one wood species throughout; normal mutable gameplay states are allowed. For a project completed after restart, the derived origin must match the saved project origin, which need not be the certificate minimum. Symmetric layouts can match more than one rotation.

For production observation, the harness includes the founding chest and chests, trapped chests, and barrels in certified Storage bounds. The founding chest is opened through the client, with a 15-second menu wait; other container contents are read using bounded, read-only server-console `data get block` queries. A connected double chest is counted once, reading each physical half once when using console queries. A failed container read does not produce a partial total.
