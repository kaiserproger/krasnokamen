#!/usr/bin/env python3
import hashlib
import json
import pathlib
import struct
import tempfile
import unittest
import zlib
from unittest import mock
import overlay


def sample(tick, wall, host=None):
    return {'tick':tick,'wall_start_unix_ns':wall,'wall_us':30000+tick*100,
            'total_us':25000,'phase_us':{'entity_us':12000},'host':host,
            'battle':{'units':20000,'alive':{'1':9999,'2':9998}}}


class OverlayTests(unittest.TestCase):
    def test_hash_and_budget_line(self):
        with tempfile.TemporaryDirectory() as d:
            path = pathlib.Path(d)/'telemetry.jsonl'
            path.write_text('\n'.join(json.dumps(sample(t, 1_000_000_000+t*50_000_000)) for t in range(3))+'\n')
            data = overlay.load(path)[2]
            image1 = overlay.render(*data)
            image2 = overlay.render(*overlay.load(path)[2])
            self.assertEqual(hashlib.sha256(image1).hexdigest(), hashlib.sha256(image2).hexdigest())
            self.assertNotEqual(image1, overlay.render(*overlay.load(path)[1]))
            self.assertEqual(overlay.graph_y(50,100),122)
            length = struct.unpack('>I',image1[33:37])[0]
            self.assertEqual(image1[37:41],b'IDAT')
            raw = zlib.decompress(image1[41:41+length])
            offset = 122*(overlay.W*4+1)+1+300*4
            self.assertEqual(raw[offset:offset+4],bytes((255,190,65,255)))
            self.assertEqual(data[1][-1][1],20.0)

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

    def test_rss_graph_pixels(self):
        def host(tick, rss):
            return {'sampled_unix_ns':1_000_000_000+tick*50_000_000,'cgroup_memory_current_bytes':3*2**30,'process_rss_bytes':rss,
                    'cgroup_cpu_usage_usec':1000*tick,'cpu_total_ticks':100*tick,'cpu_steal_ticks':0}
        with tempfile.TemporaryDirectory() as d:
            p=pathlib.Path(d)/'t.jsonl'
            # Axis is the whole-file peak (3 GiB) rounded up; tick 1 at 1 GiB, tick 41 at 3 GiB.
            p.write_text('\n'.join(json.dumps(sample(t,1_000_000_000+t*50_000_000,host(t,r) if t in (1,41) else None)) for t,r in [(1,2**30)]+[(t,0) for t in range(2,41)]+[(41,3*2**30)])+'\n')
            data=overlay.load(p)
            self.assertEqual(data[41][6],3)
            self.assertEqual(overlay.rss_y(3,3),overlay.RSS_TOP)
            self.assertEqual(overlay.rss_y(1,3),overlay.RSS_TOP+overlay.RSS_HEIGHT-12)
            image=overlay.render(*data[41])
            length=struct.unpack('>I',image[33:37])[0]
            raw=zlib.decompress(image[41:41+length])
            def pixel(x,y):
                o=y*(overlay.W*4+1)+1+x*4
                return raw[o:o+4]
            end_x=12+(41-(41-overlay.RSS_WINDOW_TICKS))*360//overlay.RSS_WINDOW_TICKS
            self.assertEqual(pixel(end_x,overlay.RSS_TOP),bytes((99,220,255,255)))
            self.assertEqual(overlay.H,324)

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
