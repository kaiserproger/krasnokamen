# civ2 gate run — failed at the birth snapshot, and why

Artifact: `wt/W1Freeze/.analysis/gates/civ2/20261128T111053-civilization-seed-qa-oigfe27s`
(`result.json` → `status: failed`). Started 18:10, ended ~19:04 after ~55 minutes of live scene time.

## Symptom

```
[harness] error: native birth snapshot failed: …/birth-snapshot.log
$ tail birth-snapshot.log
error: unexpected argument '--addr' found
Usage: krasnokamen [OPTIONS] [COMMAND]
```

## Cause — my launch, not the product

`village_autonomy_qa.py:705-733` runs the birth snapshot through the **bots** binary:
`<binary> --addr 127.0.0.1:1 --clients 1 --view-distance 1 --native-snapshot <world>
--mechanic-spec <spec> --report <json>`, and it hashes that binary against
`KRASNOKAMEN_SNAPSHOT_BINARY` before running. My launch set
`KRASNOKAMEN_SNAPSHOT_BINARY=$PIN` where `$PIN` was the pinned
`krasnokamen` **server**, which has no `--addr`; the check passed because it only
compares the hash of whatever path it is given, so the mismatch surfaced 55 minutes
later at the snapshot stage instead of at preflight.

The gate itself was healthy until that step: the scene ran, the world was written
(journal batches up to tick ~77584), and the run was killed by this command only.

## What this changes

- The failure stays in the record as a failed run; it is not a product defect and it
  does not license a retry of the same launch.
- The next civ launch sets `KRASNOKAMEN_SNAPSHOT_BINARY` and `KRASNOKAMEN_BOTS_BINARY`
  to a `pin-release --bots` binary built from the same tree as the server, and the
  bots binary is checked for `--native-snapshot` **before** a scene slot is taken.
- The snapshot command is exercised against this run's retained stopped world first;
  a reader that cannot read a stopped world is not worth another 55-minute slot.

## Follow-up worth a harness change (not done here)

`_birth_snapshot`'s preflight hash check cannot tell a server binary from a bots
binary. A `--help`-level capability probe (does the binary accept `--native-snapshot`?)
at preflight would have failed this launch in seconds instead of an hour.

## The reader works — tested before booking another slot (19:10)

`player-scale-bots` from the existing clean pin `3511c5e815e8` (built 15:52 today, same
INPUT_PATHS as the server tree) reads this run's retained stopped world:

```
player-scale-bots --addr 127.0.0.1:1 --clients 1 --view-distance 1 \
  --native-snapshot <civ2>/world \
  --mechanic-spec <civ2>/birth-snapshot-spec.json \
  --report /tmp/civsnap/out2.json
exit 0
{"event":"final","mechanics":{ … , "checkpoint_tick": 85267, "entities": [ … ] }}
success True
```

First attempt failed with `Error: invalid native record: identity mismatch` because I wrote
a hand-made spec with `native_seed: 100`; the run's own `birth-snapshot-spec.json` says
`native_seed: 24, native_worldgen_revision: 33`. With the run's exact spec the reader is
green, so the snapshot stage is not a second defect: only the launch variable was wrong.

Consequence for the rerun: a civ gate on current main needs a server pin **and** a bots pin
from the same tree, and the bots binary is the one in `KRASNOKAMEN_SNAPSHOT_BINARY`.
