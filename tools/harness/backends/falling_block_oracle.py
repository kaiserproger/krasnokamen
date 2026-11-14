"""Checks for the fixed-camera, dark-backdrop falling sand scene."""
from pathlib import Path


def same_packet(before: dict, after: dict) -> bool:
    return (
        type(before.get("observed_tick")) is int
        and before.get("observed_tick") == after.get("observed_tick")
        and type(before.get("sample_tick")) is int
        and type(after.get("sample_tick")) is int
        and before["sample_tick"] < after["sample_tick"]
        and isinstance(before.get("received_position"), list)
        and len(before["received_position"]) == 3
        and before["received_position"] == after.get("received_position")
        and isinstance(before.get("position"), list)
        and len(before["position"]) == 3
        and isinstance(after.get("position"), list)
        and len(after["position"]) == 3
    )


def sand_bounds(path: Path) -> tuple[int, int, int, int] | None:
    from PIL import Image

    with Image.open(path) as source:
        image = source.convert("RGB")
        if image.size != (1280, 720):
            raise ValueError("falling sand oracle requires a 1280x720 frame")
        points = [(x, y) for y in range(140, 600) for x in range(400, 880)
                  if (lambda c: c[0] > 110 and c[0] >= c[1] > 85
                      and c[2] < c[1] * 0.85)(image.getpixel((x, y)))]
        if len(points) < 50:
            return None
        return (min(x for x, _ in points), min(y for _, y in points),
                max(x for x, _ in points), max(y for _, y in points))
