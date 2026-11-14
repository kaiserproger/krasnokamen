import unittest

from tools.harness.backends.catalog_spain_oracle import check_bread_job, check_farmer_delivery, check_hauls, check_persisted_carry, check_persisted_job, check_wheat_pickup


class HaulOracleTests(unittest.TestCase):
    def setUp(self):
        self.spec = {"bot": 0, "observer_bot": 1, "player_uuid": "actor", "item": "minecraft:wheat", "count": 3,
                     "source_route": [[0, 2, 3], [1, 2, 3]], "destination_route": [[1, 2, 3], [2, 2, 3]]}
        arrival = {"observer_bot": 1, "player_uuid": "actor", "position": [1.5, 2, 3.5], "reader_version": 2, "leg_start_version": 1, "elapsed_us": 1}
        destination_arrival = {**arrival, "position": [2.5, 2, 3.5], "elapsed_us": 3}
        self.row = {"bot": 0, "observer_bot": 1, "player_uuid": "actor", "completed": True, "error": None,
                    "source_arrival": arrival, "destination_arrival": destination_arrival,
                    "source_transfer": {"item": "minecraft:wheat", "count": 3, "before_container_player": [9, 0], "after_container_player": [6, 3], "elapsed_us": 2},
                    "destination_transfer": {"item": "minecraft:wheat", "count": 3, "before_container_player": [1, 3], "after_container_player": [4, 0], "elapsed_us": 4}}

    def test_exact_images_and_fresh_observer(self):
        self.assertEqual(check_hauls([self.spec], {"hauls": [self.row]}), [])

    def test_wrong_endpoint_is_not_arrival(self):
        self.row["source_arrival"]["position"] = [20.5, 2, 3.5]
        self.assertTrue(any("endpoint" in failure for failure in check_hauls([self.spec], {"hauls": [self.row]})))

    def test_source_approach_needs_new_observer_at_route_start(self):
        self.row["source_approach_plan"] = {"bot": 0, "observer_bot": 1, "leg_start_version": 0,
                                             "actor_observed_position": [-0.5, 2, 3.5],
                                             "route": [[-1, 2, 3], [0, 2, 3]]}
        self.row["source_approach_arrival"] = {"observer_bot": 1, "player_uuid": "actor",
                                                "position": [0.5, 2, 3.5],
                                                "reader_version": 1, "leg_start_version": 0}
        self.assertEqual(check_hauls([self.spec], {"hauls": [self.row]}), [])
        self.row["source_approach_arrival"]["reader_version"] = 0
        self.assertTrue(check_hauls([self.spec], {"hauls": [self.row]}))

    def test_missing_and_stale_are_not_success(self):
        self.row["source_arrival"]["reader_version"] = 1
        self.row["destination_transfer"] = None
        self.assertTrue(check_hauls([self.spec], {"hauls": [self.row]}))
        self.assertTrue(check_hauls([self.spec], {"hauls": []}))
        self.assertTrue(check_hauls([], {"hauls": []}))

    def test_wrong_count_and_actor_fail(self):
        self.row["player_uuid"] = "other"
        self.row["source_transfer"]["after_container_player"] = [7, 3]
        self.assertEqual(len(check_hauls([self.spec], {"hauls": [self.row]})), 2)

    def test_cross_leg_change_and_bad_order_fail(self):
        self.row["destination_transfer"]["before_container_player"] = [1, 4]
        self.row["destination_transfer"]["after_container_player"] = [4, 1]
        self.row["destination_arrival"]["elapsed_us"] = 0
        failures = check_hauls([self.spec], {"hauls": [self.row]})
        self.assertTrue(any("between transfer legs" in failure for failure in failures))
        self.assertTrue(any("timestamps" in failure for failure in failures))


class FarmerDeliveryOracleTests(unittest.TestCase):
    def test_delivery_requires_same_observed_drop_and_farmer_collection(self):
        spec = {"id": "h", "bot": 0, "observer_bot": 1, "player_uuid": "actor",
                "item": "minecraft:wheat", "count": 3,
                "destination_route": [[1, 2, 3], [2, 2, 3]],
                "farmer_delivery": {"farmer_uuid": "farmer"}}
        receipt = {"farmer_delivery": {
            "completed": True, "farmer_uuid": "farmer",
            "withdrawal": {"item": "minecraft:wheat", "count": 3,
                           "before_container_player": [3, 0], "after_container_player": [0, 3]},
            "drop_offer": {"farmer_uuid": "farmer", "item": "minecraft:wheat", "count": 3,
                           "actor_position": [3.5, 2, 3.5], "farmer_pose": [4, 2, 3.5],
                           "farmer_reader_version": 2},
            "drop": {"item_uuid": "wheat", "item": "minecraft:wheat", "count": 3}}}
        drop = {"id": "h", "bot": 0, "player_uuid": "actor", "farmer_uuid": "farmer",
                "item_uuid": "wheat", "item": "minecraft:wheat", "count": 3}
        collection = {"item_uuid": "wheat", "collector_uuid": "farmer", "amount": 3}
        observation = {"event": "farmer_offer_observation", "id": "h", "bot": 0,
                       "farmer_uuid": "farmer", "actor_client_predicted_position": [3.5, 2, 3.5],
                       "observation_seq": 1, "horizontal_competitor_margin": 4.0, "blocked": False,
                       "villagers": [{"uuid": "farmer", "entity_id": 5, "position": [4, 2, 3.5],
                                      "reader_version": 2, "within_competitor_margin": False}]}
        receipt["farmer_delivery"]["offer_observation"] = observation
        receipt["farmer_delivery"]["drop_offer"]["villager_observation"] = observation
        self.assertTrue(check_farmer_delivery(spec, receipt, drop, collection, "farmer"))
        delivery = receipt["farmer_delivery"]
        delivery["pursuit_arrival"] = {"observer_bot": 1, "player_uuid": "actor",
                                       "position": [3.5, 2, 3.5], "reader_version": 4,
                                       "leg_start_version": 3, "farmer_uuid": "farmer",
                                       "farmer_last_received_position": [4, 2, 3.5],
                                       "farmer_reader_version": 5}
        self.assertEqual(check_farmer_delivery(spec, receipt, drop, collection, "farmer"), [])
        other_collection = {**collection, "collector_uuid": "other"}
        self.assertEqual(check_farmer_delivery(spec, receipt, drop, other_collection, "other"), [])
        for bad_collection in ({**collection, "item_uuid": "other"},
                               {**collection, "collector_uuid": "other"},
                               {**collection, "amount": 2},
                               {**collection, "retention_truncated": 1}):
            self.assertTrue(check_farmer_delivery(spec, receipt, drop, bad_collection, "farmer"))
        observation["villagers"].append({"uuid": "other", "entity_id": 6, "position": [4, 2, 4],
                                         "reader_version": 1, "within_competitor_margin": True})
        self.assertTrue(check_farmer_delivery(spec, receipt, drop, collection, "farmer"))
        observation["villagers"].pop()
        delivery["pursuit_arrival"]["reader_version"] = 3
        self.assertTrue(check_farmer_delivery(spec, receipt, drop, collection, "farmer"))
        delivery["pursuit_arrival"]["reader_version"] = 4
        delivery["pursuit_arrival"]["farmer_last_received_position"] = [20, 2, 3]
        self.assertTrue(check_farmer_delivery(spec, receipt, drop, collection, "farmer"))
        del delivery["pursuit_arrival"]


class BreadOracleTests(unittest.TestCase):
    def test_exact_native_carry_and_receipt(self):
        before = {"worker": "w", "tick": 10, "fence": {"revision": 1},
                  "slots": [{"stack": {"item_id": 1, "count": 3}}] + [None] * 7}
        after = {"worker": "w", "tick": 20, "fence": {"revision": 2},
                 "slots": [{"stack": {"item_id": 2, "count": 1}}] + [None] * 7}
        job = {"id": "j", "worker": "w", "recipe": "krasnokamen-settlements:bread",
               "recipe_revision": "r", "count": 1, "done": 1, "cancelled": False,
               "receipts": [{"job": "j", "sequence": 1, "recipe": "krasnokamen-settlements:bread", "recipe_revision": "r",
                             "inputs": [{"item_id": 1, "count": 3}], "outputs": [{"item_id": 2, "count": 1}]}]}
        self.assertEqual(check_bread_job(before, after, job, "w", "j", 1, 2), [])
        after["tick"] = 10
        job["receipts"][0]["inputs"][0]["count"] = 2
        failures = check_bread_job(before, after, job, "w", "j", 1, 2)
        self.assertTrue(any("ordered" in failure for failure in failures))
        self.assertTrue(any("inputs" in failure for failure in failures))

    def test_wheat_pickup_needs_exact_new_carry(self):
        before = {"worker": "w", "tick": 10, "fence": {"revision": 1}, "slots": [None] * 8}
        after = {"worker": "w", "tick": 11, "fence": {"revision": 2},
                 "slots": [{"stack": {"item_id": 1, "count": 3}}] + [None] * 7}
        self.assertEqual(check_wheat_pickup(before, after, "w", 1, 3), [])
        after["fence"] = before["fence"]
        self.assertTrue(check_wheat_pickup(before, after, "w", 1, 3))

    def test_persisted_job_must_retain_exact_completed_receipt(self):
        before = {"id": "j", "worker": "w", "recipe": "bread", "recipe_revision": "1",
                  "count": 1, "done": 1, "cancelled": False, "receipts": [{"sequence": 1, "inputs": [3]}]}
        self.assertEqual(check_persisted_job(before, dict(before)), [])
        after = {**before, "receipts": []}
        self.assertTrue(check_persisted_job(before, after))

    def test_restart_carry_needs_same_worker_and_slots(self):
        before = {"worker": "w", "slots": [None] * 8, "fence": {"revision": 1, "snapshot_hash": "hash"}}
        self.assertEqual(check_persisted_carry(before, dict(before)), [])
        self.assertTrue(check_persisted_carry(before, {**before, "worker": "other"}))
        self.assertTrue(check_persisted_carry(before, {**before, "slots": [None] * 7}))
        self.assertTrue(check_persisted_carry(before, {**before, "fence": {"revision": 2, "snapshot_hash": "hash"}}))


if __name__ == "__main__":
    unittest.main()
