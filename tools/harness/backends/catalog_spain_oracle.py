"""Fail-closed checks for Spain's received, per-leg protocol-bot haul receipts.

This checks hauling receipt consistency and endpoint proximity, not the
surveyed route, extraction, production, loaded chunks or TPS. Retain the
original bot report and exact submitted spec.
"""

import math


def _near(position, point):
    return (isinstance(position, (list, tuple)) and len(position) == 3
            and isinstance(point, (list, tuple)) and len(point) == 3
            and all(type(value) in (int, float) and math.isfinite(value) for value in position)
            and all(type(value) is int for value in point)
            and abs(position[0] - point[0] - 0.5) <= 0.35
            and abs(position[1] - point[1]) <= 0.35
            and abs(position[2] - point[2] - 0.5) <= 0.35)

def _route_valid(route):
    return (isinstance(route, list) and len(route) >= 2
            and all(isinstance(point, list) and len(point) == 3
                    and all(type(value) is int for value in point) for point in route)
            and all(abs(a[0] - b[0]) + abs(a[2] - b[2]) == 1 and abs(a[1] - b[1]) <= 1
                    for a, b in zip(route, route[1:])))



def check_hauls(specs, report):
    failures = []
    rows = report.get("hauls")
    if not isinstance(rows, list) or len(rows) != len(specs):
        return ["haul receipt count differs from submitted spec"]
    if not specs:
        return ["no haul work submitted"]
    for index, (spec, row) in enumerate(zip(specs, rows)):
        def fail(message):
            failures.append(f"haul {index}: {message}")

        if not isinstance(row, dict):
            fail("receipt missing")
            continue
        source_route, destination_route = spec.get("source_route"), spec.get("destination_route")
        if (not _route_valid(source_route) or not _route_valid(destination_route)
                or source_route[-1] != destination_route[0]
                or source_route[-1] == destination_route[-1]
                or spec.get("bot") == spec.get("observer_bot")):
            fail("invalid route or independent observer")
        for key in ("bot", "observer_bot", "player_uuid"):
            if key not in spec or row.get(key) != spec[key]:
                fail(f"{key} mismatch")
        if row.get("completed") is not True or row.get("error") is not None:
            fail("haul did not complete cleanly")
        plan, approach = row.get("source_approach_plan"), row.get("source_approach_arrival")
        if plan is not None or approach is not None:
            route = plan.get("route") if isinstance(plan, dict) else None
            source_arrival = row.get("source_arrival")
            if (not isinstance(plan, dict) or not isinstance(approach, dict)
                    or plan.get("bot") != spec.get("bot") or plan.get("observer_bot") != spec.get("observer_bot")
                    or not _route_valid(route) or not source_route or route[-1] != source_route[0]
                    or approach.get("observer_bot") != spec.get("observer_bot")
                    or approach.get("player_uuid") != spec.get("player_uuid")
                    or not _near(approach.get("position"), source_route[0])
                    or type(plan.get("leg_start_version")) is not int
                    or approach.get("leg_start_version") != plan.get("leg_start_version")
                    or type(approach.get("reader_version")) is not int
                    or approach["reader_version"] <= plan["leg_start_version"]
                    or not isinstance(source_arrival, dict)
                    or source_arrival.get("leg_start_version") != approach["reader_version"]):
                fail("source approach lacks a fresh independent start fence")
        for label in ("source_arrival", "destination_arrival"):
            arrival = row.get(label)
            if not isinstance(arrival, dict):
                fail(f"{label} missing")
                continue
            if arrival.get("observer_bot") != spec.get("observer_bot") or arrival.get("player_uuid") != spec.get("player_uuid"):
                fail(f"{label} observer or player mismatch")
            version = arrival.get("reader_version")
            start = arrival.get("leg_start_version")
            if type(version) is not int or type(start) is not int or version <= start:
                fail(f"{label} lacks a fresh observer update")
            route = spec.get("source_route" if label == "source_arrival" else "destination_route")
            if not isinstance(route, list) or not route or not _near(arrival.get("position"), route[-1]):
                fail(f"{label} position does not match the submitted endpoint")
        for label, sign in (("source_transfer", 1), ("destination_transfer", -1)):
            transfer = row.get(label)
            if not isinstance(transfer, dict):
                fail(f"{label} missing")
                continue
            count = spec.get("count")
            if type(count) is not int or count <= 0 or transfer.get("item") != spec.get("item") or transfer.get("count") != count:
                fail(f"{label} item or count mismatch")
                continue
            before = transfer.get("before_container_player")
            after = transfer.get("after_container_player")
            if not (isinstance(before, list) and isinstance(after, list) and len(before) == len(after) == 2 and all(type(x) is int and x >= 0 for x in before + after)):
                fail(f"{label} missing inventory images")
                continue
            if after[0] - before[0] != -sign * count or after[1] - before[1] != sign * count:
                fail(f"{label} does not conserve exact item count")
            if (label == "source_transfer" and before[1] != 0) or (label == "destination_transfer" and after[1] != 0):
                fail(f"{label} violates empty actor item boundary")
        source = row.get("source_transfer")
        destination = row.get("destination_transfer")
        if isinstance(source, dict) and isinstance(destination, dict):
            source_after = source.get("after_container_player")
            destination_before = destination.get("before_container_player")
            if (isinstance(source_after, list) and len(source_after) == 2
                    and isinstance(destination_before, list) and len(destination_before) == 2
                    and source_after[1] != destination_before[1]):
                fail("actor inventory changed between transfer legs")
        times = [row.get(label, {}).get("elapsed_us") if isinstance(row.get(label), dict) else None
                 for label in ("source_arrival", "source_transfer", "destination_arrival", "destination_transfer")]
        if any(type(value) is not int or value < 0 for value in times) or times != sorted(times):
            fail("missing or unordered leg timestamps")
    return failures


def check_farmer_delivery(spec, receipt, drop, collection, collector):
    delivery_spec = spec.get("farmer_delivery")
    delivery = receipt.get("farmer_delivery")
    if not isinstance(delivery_spec, dict) or not isinstance(delivery, dict):
        return ["missing farmer delivery specification or receipt"]
    errors = []
    worker, item, count = delivery_spec.get("farmer_uuid"), spec.get("item"), spec.get("count")
    if delivery.get("completed") is not True or delivery.get("farmer_uuid") != worker:
        errors.append("farmer drop is incomplete or wrong worker")
    withdrawal = delivery.get("withdrawal")
    if not isinstance(withdrawal, dict) or withdrawal.get("item") != item or withdrawal.get("count") != count:
        errors.append("farmer delivery withdrawal missing")
    else:
        before, after = withdrawal.get("before_container_player"), withdrawal.get("after_container_player")
        if not (isinstance(before, list) and isinstance(after, list) and len(before) == len(after) == 2
                and all(type(value) is int and value >= 0 for value in before + after)
                and type(count) is int and count > 0 and before[1] == 0
                and before[0] - after[0] == count and after[1] == count):
            errors.append("farmer delivery withdrawal is not conserved")
    # The farmer pose is last received data, not fresh authoritative state.
    pursuit = delivery.get("pursuit_arrival")
    if (not isinstance(pursuit, dict)
            or pursuit.get("observer_bot") != spec.get("observer_bot")
            or pursuit.get("player_uuid") != spec.get("player_uuid")
            or pursuit.get("farmer_uuid") != worker
            or type(pursuit.get("reader_version")) is not int
            or type(pursuit.get("leg_start_version")) is not int
            or pursuit["reader_version"] <= pursuit["leg_start_version"]
            or type(pursuit.get("farmer_reader_version")) is not int
            or pursuit["farmer_reader_version"] <= 0):
        errors.append("farmer pursuit lacks observed identity or actor fence")
    else:
        actor = pursuit.get("position")
        farmer = pursuit.get("farmer_last_received_position")
        if not (isinstance(actor, list) and isinstance(farmer, list)
                and len(actor) == len(farmer) == 3
                and all(type(value) in (int, float) and math.isfinite(value) for value in actor + farmer)
                and all(abs(a - b) <= 1 for a, b in zip(actor, farmer))):
            errors.append("pursuit actor was not observed near last received farmer pose")
    offer = delivery.get("drop_offer")
    if not isinstance(offer, dict) or offer.get("farmer_uuid") != worker or offer.get("item") != item or offer.get("count") != count:
        errors.append("missing farmer drop offer")
    else:
        actor, farmer = offer.get("actor_position"), offer.get("farmer_pose")
        if not (isinstance(actor, list) and isinstance(farmer, list) and len(actor) == len(farmer) == 3
                and all(type(value) in (int, float) and math.isfinite(value) for value in actor + farmer)
                and all(abs(a - b) <= 1 for a, b in zip(actor, farmer))
                and type(offer.get("farmer_reader_version")) is int):
            errors.append("farmer not observed at drop reach")
        observation = offer.get("villager_observation")
        images = observation.get("villagers") if isinstance(observation, dict) else None
        margin = observation.get("horizontal_competitor_margin") if isinstance(observation, dict) else None
        if not (isinstance(observation, dict) and observation == delivery.get("offer_observation")
                and observation.get("id") == spec.get("id") and observation.get("bot") == spec.get("bot")
                and observation.get("farmer_uuid") == worker and observation.get("blocked") is False
                and observation.get("actor_client_predicted_position") == actor
                and type(observation.get("observation_seq")) is int and observation["observation_seq"] > 0
                and type(margin) in (int, float) and math.isfinite(margin) and margin >= 4
                and isinstance(images, list) and any(isinstance(row, dict) and row.get("uuid") == worker for row in images)):
            errors.append("farmer offer lacks bounded visible-villager observation")
        elif not (isinstance(actor, list) and len(actor) == 3
                  and all(type(value) in (int, float) and math.isfinite(value) for value in actor)):
            errors.append("farmer offer has invalid actor position")
        else:
            for image in images:
                position = image.get("position") if isinstance(image, dict) else None
                if (not isinstance(image, dict) or not image.get("uuid")
                        or type(image.get("reader_version")) is not int or image["reader_version"] <= 0
                        or not isinstance(position, list) or len(position) != 3
                        or any(type(value) not in (int, float) or not math.isfinite(value) for value in position)):
                    errors.append("villager offer image is incomplete")
                    break
                if image["uuid"] != worker and abs(position[0] - actor[0]) <= margin and abs(position[2] - actor[2]) <= margin:
                    errors.append("visible competing villager near wheat offer")
                    break
    observed = delivery.get("drop")
    if not isinstance(drop, dict) or not isinstance(observed, dict) or not isinstance(collection, dict):
        errors.append("missing drop or collection event")
    elif (not drop.get("item_uuid") or observed.get("item_uuid") != drop.get("item_uuid")
          or drop.get("id") != spec.get("id") or drop.get("bot") != spec.get("bot")
          or drop.get("player_uuid") != spec.get("player_uuid") or drop.get("farmer_uuid") != worker
          or drop.get("item") != item or drop.get("count") != count
          or observed.get("item") != item or observed.get("count") != count
          or collection.get("item_uuid") != drop.get("item_uuid")
          or not collector or collection.get("collector_uuid") != collector or collection.get("amount") != count
          or drop.get("retention_truncated", 0) or collection.get("retention_truncated", 0)):
        errors.append("drop and farmer pickup identities or quantities differ")
    return errors


def check_wheat_pickup(before, after, worker, item_id, count):
    """Validate an exact worker carry increase across two native snapshots."""
    if not isinstance(before, dict) or not isinstance(after, dict):
        return ["missing worker carry snapshots"]
    if type(item_id) is not int or item_id < 0:
        return ["invalid wheat registry identity"]
    errors = []
    if before.get("worker") != worker or after.get("worker") != worker:
        errors.append("worker identity mismatch")
    if type(before.get("tick")) is not int or type(after.get("tick")) is not int or before["tick"] >= after["tick"]:
        errors.append("worker pickup snapshots are not tick ordered")
    if not isinstance(before.get("fence"), dict) or not isinstance(after.get("fence"), dict) or before["fence"] == after["fence"]:
        errors.append("worker pickup lacks distinct native fences")
    def wheat(snapshot):
        slots = snapshot.get("slots")
        if not isinstance(slots, list) or len(slots) != 8:
            raise ValueError("carry snapshot must contain eight slots")
        total = 0
        for slot in slots:
            if slot is None:
                continue
            stack = slot.get("stack") if isinstance(slot, dict) else None
            if not isinstance(stack, dict) or type(stack.get("item_id")) is not int or type(stack.get("count")) is not int or stack["count"] <= 0:
                raise ValueError("invalid native carry slot")
            if stack["item_id"] == item_id:
                total += stack["count"]
        return total
    try:
        if type(count) is not int or count <= 0 or wheat(after) - wheat(before) != count:
            errors.append("worker carry does not show exact wheat pickup")
    except ValueError as error:
        errors.append(str(error))
    return errors


def check_bread_job(before, after, job, worker, job_id, wheat_id, bread_id):
    """Check one real bread portion against two owner-authorized native snapshots.

    This does not establish how wheat entered the worker's inventory.
    The item IDs must come from the matching server's registry.
    """
    errors = []
    if not all(isinstance(value, dict) for value in (before, after, job)):
        return ["missing carry or job response"]
    if type(wheat_id) is not int or type(bread_id) is not int or wheat_id < 0 or bread_id < 0 or wheat_id == bread_id:
        return ["invalid bread recipe registry identities"]
    if before.get("worker") != worker or after.get("worker") != worker or job.get("worker") != worker:
        errors.append("worker identity mismatch")
    if job.get("id") != job_id or job.get("recipe") != "krasnokamen-settlements:bread":
        errors.append("job identity or recipe mismatch")
    if job.get("count") != 1 or job.get("done") != 1 or job.get("cancelled") is not False:
        errors.append("single bread portion did not complete")
    if type(before.get("tick")) is not int or type(after.get("tick")) is not int or after["tick"] <= before["tick"]:
        errors.append("carry observations are not ordered by server tick")
    if not isinstance(before.get("fence"), dict) or not isinstance(after.get("fence"), dict) or before.get("fence") == after.get("fence"):
        errors.append("carry change lacks distinct native fences")

    def totals(snapshot):
        slots = snapshot.get("slots")
        if not isinstance(slots, list) or len(slots) != 8:
            raise ValueError("carry snapshot must contain eight slots")
        result = {}
        for slot in slots:
            if slot is None:
                continue
            stack = slot.get("stack") if isinstance(slot, dict) else None
            if not isinstance(stack, dict) or type(stack.get("item_id")) is not int or type(stack.get("count")) is not int or stack["count"] <= 0:
                raise ValueError("invalid native carry slot")
            result[stack["item_id"]] = result.get(stack["item_id"], 0) + stack["count"]
        return result

    try:
        first, last = totals(before), totals(after)
        if last.get(wheat_id, 0) != first.get(wheat_id, 0) - 3 or last.get(bread_id, 0) != first.get(bread_id, 0) + 1:
            errors.append("native carry does not show three wheat consumed and one bread produced")
    except ValueError as error:
        errors.append(str(error))
    receipts = job.get("receipts")
    if not isinstance(receipts, list) or len(receipts) != 1 or not isinstance(receipts[0], dict):
        errors.append("missing exact single-portion native receipt")
        return errors
    receipt = receipts[0]
    if receipt.get("job") != job_id or receipt.get("sequence") != 1 or receipt.get("recipe") != job.get("recipe") or receipt.get("recipe_revision") != job.get("recipe_revision") or not job.get("recipe_revision"):
        errors.append("portion receipt identity mismatch")
    for name, item, count in (("inputs", wheat_id, 3), ("outputs", bread_id, 1)):
        stacks = receipt.get(name)
        if not isinstance(stacks, list) or not stacks or any(not isinstance(stack, dict) or stack.get("item_id") != item or type(stack.get("count")) is not int or stack["count"] <= 0 for stack in stacks) or sum(stack["count"] for stack in stacks) != count:
            errors.append(f"portion receipt {name} mismatch")
    return errors


def check_persisted_job(before, after):
    """Compare immutable completed-job identity and exact receipts across restart."""
    if not isinstance(before, dict) or not isinstance(after, dict):
        return ["missing before or after restart job"]
    keys = ("id", "worker", "recipe", "recipe_revision", "count", "done", "cancelled", "receipts")
    if before.get("done") != 1 or not before.get("receipts"):
        return ["pre-restart job lacks a completed receipt"]
    if any(key not in before or after.get(key) != before[key] for key in keys):
        return ["completed job or receipts changed across restart"]
    return []


def check_persisted_carry(before, after):
    """Compare actual worker carry images at the two restart boundaries."""
    if not isinstance(before, dict) or not isinstance(after, dict):
        return ["missing before or after restart carry"]
    slots, fence = before.get("slots"), before.get("fence")
    if (not before.get("worker") or before.get("worker") != after.get("worker")
            or not isinstance(slots, list) or len(slots) != 8 or after.get("slots") != slots
            or not isinstance(fence, dict) or type(fence.get("revision")) is not int
            or not isinstance(fence.get("snapshot_hash"), str) or not fence["snapshot_hash"]
            or after.get("fence") != fence):
        return ["worker carry identity, slots or native fence changed across restart"]
    return []
