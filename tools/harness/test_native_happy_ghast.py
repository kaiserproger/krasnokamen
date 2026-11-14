import json
import tempfile
import unittest
from pathlib import Path

from tools.harness.backends.native_happy_ghast import PassengerReceipt


class HappyGhastPassengerReceiptTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        report = self.root / "packets.json"
        report.write_text(json.dumps({"play": {"clientbound": {
            "minecraft:set_passengers": {"protocol_id": 109}}}}))
        self.receipt = PassengerReceipt(report, self.root)

    def test_reads_four_distinct_passengers_without_changing_packets(self):
        payload = bytes([7, 4, 1, 2, 3, 4])
        self.receipt.server_packet(None, "play", 109, payload)
        self.assertEqual(self.receipt.snapshot(None), [{"vehicle": 7, "passengers": [1, 2, 3, 4]}])
        self.assertEqual(payload, bytes([7, 4, 1, 2, 3, 4]))

    def test_refuses_unbounded_count_and_trailing_payload(self):
        for payload in [bytes([7, 5, 1, 2, 3, 4, 5]), bytes([7, 1, 1, 0])]:
            with self.subTest(payload=payload), self.assertRaises(ValueError):
                self.receipt.server_packet(None, "play", 109, payload)
        self.assertEqual(self.receipt.snapshot(None), [])

    def test_ignores_other_phase_and_packet(self):
        self.receipt.server_packet(None, "configuration", 109, b"")
        self.receipt.server_packet(None, "play", 108, b"")
        self.assertEqual(self.receipt.snapshot(None), [])
