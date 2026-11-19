#!/usr/bin/env python3
import json
import pathlib
import tempfile
import unittest
from unittest import mock
from tools.battle.video import overlay


def sample(tick, wall, host=None):
    return {'tick':tick,'wall_start_unix_ns':wall,'wall_us':30000+tick*100,
            'total_us':25000,'phase_us':{'entity_us':12000},'host':host,
            'battle':{'units':20000,'alive':{'1':9999,'2':9998}}}


class OverlayTests(unittest.TestCase):
    def test_host_deltas_and_missing(self):
        host1 = {'sampled_unix_ns':1_000_000_000,'cgroup_memory_current_bytes':3*2**30,'process_rss_bytes':2**30,
                 'cgroup_cpu_usage_usec':1000,'cpu_total_ticks':100,'cpu_steal_ticks':10}
        host2 = dict(host1, sampled_unix_ns=2_000_000_000,cgroup_cpu_usage_usec=501000,
                     cpu_total_ticks=200,cpu_steal_ticks=15)
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'t.jsonl'
            p.write_text('\n'.join(json.dumps(x) for x in [sample(1,1_000_000_000,host1), sample(2,2_000_000_000,host2), sample(3,3_000_000_000)])+'\n')
            data=overlay.load(p)
            # RSS and the cgroup total, which includes page cache, are distinct.
            self.assertEqual(data[1][2:5],(None,None,1.0))
            self.assertEqual(data[2][2:5],(50.0,5.0,1.0))
            self.assertEqual(data[3][2:5],(50.0,5.0,1.0))

    def test_memory_labels_use_distinct_sources(self):
        host = {'sampled_unix_ns':1_000_000_000,'cgroup_memory_current_bytes':3*2**30,'process_rss_bytes':2**30,
                'cgroup_cpu_usage_usec':1000,'cpu_total_ticks':100,'cpu_steal_ticks':10}
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d)/'t.jsonl'
            p.write_text(json.dumps(sample(1,1_000_000_000,host))+'\n')
            data = overlay.load(p)[1]
            self.assertEqual((data[4], data[7]), (1.0, 3.0))
            with mock.patch.object(overlay, 'text', wraps=overlay.text) as rendered:
                overlay.render(*data)
            labels = [call.args[3] for call in rendered.call_args_list]
            self.assertIn('SERVER RSS 1.0 GIB  CPU N/A%', labels)
            self.assertIn('CGROUP TOTAL (INCL. PAGE CACHE) 3.0 GIB', labels)
            self.assertFalse(any('4 GIB' in label or 'LIMIT' in label for label in labels))

    def test_memory_never_falls_back_to_cgroup(self):
        host = {'sampled_unix_ns':1_000_000_000,'cgroup_memory_current_bytes':3*2**30,'process_rss_bytes':None,
                'cgroup_cpu_usage_usec':1000,'cpu_total_ticks':100,'cpu_steal_ticks':10}
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'t.jsonl'
            p.write_text(json.dumps(sample(1,1_000_000_000,host))+'\n')
            data=overlay.load(p)[1]
            self.assertIsNone(data[4])
            self.assertEqual(data[7], 3.0)
            self.assertEqual(data[5],((1,None),))

    def test_rss_graph_expires_old_samples_without_new_host_data(self):
        host = {'sampled_unix_ns':1_000_000_000,'cgroup_memory_current_bytes':3*2**30,'process_rss_bytes':2**30,
                'cgroup_cpu_usage_usec':1000,'cpu_total_ticks':100,'cpu_steal_ticks':10}
        with tempfile.TemporaryDirectory() as d:
            p = pathlib.Path(d)/'t.jsonl'
            p.write_text('\n'.join(json.dumps(x) for x in [
                sample(1, 1_000_000_000, host), sample(1201, 61_000_000_000)])+'\n')
            self.assertEqual(overlay.load(p)[1201][5], ())

    def test_missing_tick_rejected(self):
        import subprocess, sys
        with tempfile.TemporaryDirectory() as d:
            d=pathlib.Path(d)
            (d/'t').write_text(json.dumps(sample(1,1_000_000_000))+'\n')
            (d/'f').write_text(json.dumps({'frame':0,'last_applied_tick':2})+'\n')
            r=subprocess.run([sys.executable,str(pathlib.Path(overlay.__file__)), '--telemetry',str(d/'t'), '--frames',str(d/'f'), '--out',str(d/'out')],capture_output=True,text=True)
            self.assertNotEqual(r.returncode,0)
            self.assertIn('tick 2 missing',r.stderr)

if __name__ == '__main__': unittest.main()
