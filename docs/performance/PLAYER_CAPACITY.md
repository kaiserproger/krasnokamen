# Player capacity: how the bots account for work

How `player-scale-bots` and the capacity ladder count offered, admitted and
completed work. The runnable recipe (the ladder command, its options, workloads,
windows and operator thresholds) is in
[BENCHMARKS](BENCHMARKS.md#player-capacity-ladder). This page states no capacity
number: no current measurement is published, and results of older runs are not
evidence for the current build. A capacity claim needs a fresh ladder run on that
build with the lane, envelope and binaries recorded.

## Bot accounting

Assisted bots select landings only on received walkable full-height support and
check their received ground again before Ready. Missing ground fails readiness.
For an edit offer, the bot inspects four candidate directions; it does not replay
missed slots or count packet writes as completed world edits.

Final open-loop reports include per-bot `edit_windows` with offered, admitted,
`writes_completed`, observed `completed`, and dropped counts. The accompanying
`edit_unavailable` distinguishes dead/respawning, edits disabled, not creative,
no eligible candidate, and an already pending cell; capacity and late drops are
separate. Edit completion means a matching received block update, not exclusive
attribution or a full-chunk readback. Movement has no completion oracle. Analyze
each bot as well as totals: aggregate throughput can hide a starved client.

## Reading a ladder result

- When a run loses a connection the bots can end early, so a shed run yields only
  partial windows; the window is measured up to the last completed tick. Short
  partial windows make p99 and sustained TPS weak evidence.
- The lane isolates CPUs, not L3/memory bandwidth or the shared disk: record the
  host load at the start and end of every run. Single runs carry no variance
  estimate.
- `fight` is hostile-versus-player only; the bots never attack and survival edits
  are not driven.
- A perf capture that does not overlap the stall cannot blame a symbol for it;
  time the capture to the event and keep the call graph.
