# D145Harness throughput

Strict counts per 30-minute window. `fixed-ready` = a row whose fix, guard refs
and graphical ref are all in the promoted-tree candidate and whose real-client
receipt passes the row's verdict (ready for `ledger promote`). `in flight` = a
gate run or patch build started but not yet returned. `returned` = back to the
pool with a recorded reason. Zero `fixed-ready` in a window means the stuck rows
go back to the pool.

| window (UTC) | fixed-ready | in flight | returned | notes |
|---|---|---|---|---|
| 2026-11-28 08:30–09:00 | 0 | 5 | 0 | D149 scope set; guard refs + graphical refs wired; candidate pin building |
