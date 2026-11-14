#!/usr/bin/env python3
"""Generate reproducible synthetic input data (not live battle metrics)."""
import json
import pathlib
import sys

out = pathlib.Path(sys.argv[1])
out.mkdir(parents=True, exist_ok=False)
with (out/'telemetry.jsonl').open('w') as f:
    for tick in range(200):
        host = None
        if tick % 20 == 0:
            host = {'sampled_unix_ns':1_000_000_000+tick*50_000_000,
                    'cgroup_memory_current_bytes':3_000_000_000+tick*1_000_000,
                    'cgroup_cpu_usage_usec':1_000_000+tick*100_000,
                    'cpu_total_ticks':10000+tick*40,'cpu_steal_ticks':20+tick//20}
        s = {'tick':tick,'wall_start_unix_ns':1_000_000_000+tick*50_000_000,
             'wall_us':25000+(tick%50)*600,'total_us':20000,
             'phase_us':{'entity_us':12000},'host':host,
             'battle':{'units':20000,'alive':{'1':10000-tick,'2':10000-tick//2}}}
        f.write(json.dumps(s)+'\n')
with (out/'frames.jsonl').open('w') as f:
    for frame in range(600):
        f.write(json.dumps({'frame':frame,'last_applied_tick':frame//3})+'\n')
(out/'title.txt').write_text('SYNTHETIC SAMPLE - NOT BATTLE FOOTAGE\n')
