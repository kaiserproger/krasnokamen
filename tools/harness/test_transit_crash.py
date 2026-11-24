"""Pure checks of the migration transit SIGKILL scenario (no server)."""
from __future__ import annotations

import json
import tempfile
import unittest
import uuid
from pathlib import Path

from . import wal_frame
from .crash_wal import read_published
from .transit_crash import (body_entry, bread_count, census_verdict, custody_verdict, places,
                            publish_batch, published_through, transit_images, uuid_array)

A, B, C = (str(uuid.UUID(int=n)) for n in (0x10, 0x11, 0x12))
RECORD = A
# The world's bindings a forged batch must carry: any 16-byte store, 8-byte branch, 8-byte palette.
IDENTITY = (bytes(range(1, 17)), bytes(range(51, 59)), bytes(range(101, 109)))


def receipt(villagers=None, transits=None, status="PASS"):
    return {"status": status, "error": None, "villagers": villagers or {},
            "transits": transits or {}}


def record(members, status="travelling", household="1-1"):
    return {"dimension": "minecraft:overworld", "source": 1, "destination": 2, "status": status,
            "departed": 5, "arrives": 900, "household": household, "members": members}


def body(settlement=2, household="1-1"):
    return {"dimension": "minecraft:overworld", "wire_id": 7, "settlement": settlement,
            "household": household}


class CustodyVerdictTests(unittest.TestCase):
    def test_departure_needs_every_member_once_in_the_record(self):
        good = receipt(transits={RECORD: record([A, B, C])})
        self.assertEqual(custody_verdict("departure", good, [A, B, C], RECORD, 2, "1-1"), [])
        self.assertEqual(places(good, [A, B, C]), {m: [f"transit:{RECORD}"] for m in (A, B, C)})

    def test_member_both_body_and_record_is_red(self):
        duplicated = receipt(villagers={B: body()}, transits={RECORD: record([A, B, C])})
        failures = custody_verdict("departure", duplicated, [A, B, C], RECORD, 2, "1-1")
        self.assertIn(f"member {B} held 2 times: ['entity', 'transit:{RECORD}']", failures)

    def test_member_in_no_place_is_red(self):
        lost = receipt(transits={})
        failures = custody_verdict("departure", lost, [A, B, C], RECORD, 2, "1-1")
        self.assertIn(f"member {A} held 0 times: []", failures)
        self.assertIn(f"transit record {RECORD} missing at the durable boundary", failures)

    def test_in_transit_wants_awaiting_entry(self):
        travelling = receipt(transits={RECORD: record([A, B, C])})
        self.assertEqual(custody_verdict("in-transit", travelling, [A, B, C], RECORD, 2, "1-1"),
                         ["record status travelling != awaiting_entry"])

    def test_arrival_wants_bodies_bound_to_the_destination(self):
        arrived = receipt(villagers={m: body() for m in (A, B, C)})
        self.assertEqual(custody_verdict("arrival", arrived, [A, B, C], RECORD, 2, "1-1"), [])
        wrong = receipt(villagers={A: body(settlement=1), B: body(), C: body()})
        self.assertEqual(len(custody_verdict("arrival", wrong, [A, B, C], RECORD, 2, "1-1")), 1)

    def test_failed_census_is_red(self):
        failed = receipt(status="FAIL")
        failed["error"] = "member x of transit record y is also a world entity"
        self.assertEqual(custody_verdict("departure", failed, [A], RECORD, 2, "1-1"),
                         ["custody census FAIL: member x of transit record y is also a world entity"])

    def test_stored_census_must_match_the_cut(self):
        census = {"status": "PASS", "counts": {"transit_records": 1, "transit_members": 3,
                                                "settlement_orphan_members": 0},
                  "settlement_details": [{"name": "TransitDest", "members": 0, "household_members": 0}]}
        self.assertEqual(census_verdict("departure", census, "TransitDest"), [])
        self.assertEqual(len(census_verdict("arrival", census, "TransitDest")), 2)


class WalTests(unittest.TestCase):
    def test_forged_batch_is_a_valid_published_frame_with_the_member_body(self):
        member = {"uuid": B, "type_id": 1, "pos": [0.5, 201.0, 0.5], "ground": True, "tick": 3}
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            publish_batch(journal, 5, [{"Transit": {"id": RECORD, "tick": 5,
                                                    "record": json.dumps(record([A, B]))}}], IDENTITY)
            publish_batch(journal, 6, [body_entry(member, 99, 6)], IDENTITY)
            self.assertEqual(published_through(journal, 4), 6)
            self.assertEqual(published_through(journal, 6), 6)
            wal = read_published(journal, 4)
            self.assertEqual(wal["published_through"], 6)
            [image] = transit_images(wal, RECORD)
            self.assertEqual((image["tick"], image["record"]["members"]), (5, [A, B]))
            forged = wal["frames"][1]["entries"][0]["Entity"]
            self.assertEqual((forged["id"], forged["uuid"], forged["tick"]), (99, B, 6))
            self.assertNotIn("ground", forged)
            frame = wal_frame.read(journal / wal_frame.batch_name(6, 6))
            self.assertEqual((frame.identity, frame.lo, frame.hi), (IDENTITY, 6, 6),
                             "the forged batch is the current format, bound to the world it joins")

    def test_published_through_stops_at_a_gap(self):
        with tempfile.TemporaryDirectory() as temp:
            journal = Path(temp)
            publish_batch(journal, 5, [], IDENTITY)
            publish_batch(journal, 7, [], IDENTITY)
            self.assertEqual(published_through(journal, 4), 5)

    def test_uuid_array_matches_vanilla_signed_words(self):
        self.assertEqual(uuid_array("ffffffff-0000-0001-8000-000000000002"),
                         "[I;-1,1,-2147483648,2]")

    def test_bread_count_reads_only_bread_stacks(self):
        reply = ('Villager has the following entity data: [{count: 2, id: "minecraft:bread"}, '
                 '{count: 5, id: "minecraft:carrot"}, {count: 1, id: "minecraft:bread"}]')
        self.assertEqual(bread_count(reply), 3)
        self.assertEqual(bread_count("Found no elements matching Inventory"), 0)


if __name__ == "__main__":
    unittest.main()
