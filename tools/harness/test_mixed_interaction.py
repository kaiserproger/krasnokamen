import tempfile
import unittest
from pathlib import Path

from .mixed_interaction import admitted_protocols


class AdmittedProtocolTests(unittest.TestCase):
    def test_exact_named_connections_required(self):
        first = 'INFO login admitted peer=127.0.0.1:1 connection=4 name=GateFabric protocol=777 release="26.3"\n'
        second = 'INFO login admitted peer=127.0.0.1:2 connection=5 name=MixedVersion protocol=775 release="26.1.2"\n'
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "server.log"
            log.write_text(first + second)
            self.assertEqual(admitted_protocols(log)["MixedVersion"]["protocol"], "775")
            for bad in (first, first + second.replace("protocol=775", "protocol=777"),
                        first + second.replace("connection=5", "connection=4"), first + second + second):
                log.write_text(bad)
                with self.assertRaises(AssertionError):
                    admitted_protocols(log)
