"""Silhouette check for the frozen zombie death-tilt scene (tools/harness/death_tilt_scene.json).

The scene films one zombie from behind against a plain backdrop from a fixed pose: an empty
frame, an alive frame, then one frame per stepped tick `t00`..`t18` after the kill under
/tick freeze. Vanilla 26.3 `LivingEntityRenderer.setupRotations` turns the body about its
feet, so it tips to the frame's left and lies on the ground; a client that turns it about
another point leaves the body floating at a height Java never shows, and one on another
deathTime curve is off by whole frames early in the fall.

JAVA holds the top row and left column of the silhouette in the Java 26.3 client's frame at
each step (receipts/ClientEntityParity2, `runs-java-3` and `runs-java-4`, same pixels).
Java renders a frozen entity at partial tick 1.0 (`TickRateManager.isEntityFrozen`); the
native client renders it at partial 0, so its frame at step `s` equals Java's at step
`s - 1` pixel for pixel (measured on all 19 steps). A shot matches when it is within
TOLERANCE of the Java frame of its own step or of the step before, so the gate also holds
once the native client takes the vanilla frozen partial.
"""
from pathlib import Path

FRAME = (1280, 720)
# Frame region around the zombie: in front of the backdrop, above the chat and hotbar, left of
# the held hand. Only the zombie and its shadow differ from the empty frame here.
REGION = (0, 0, 900, 640)
# A pixel belongs to the silhouette when one channel differs from the empty frame by more.
THRESHOLD = 40
# Java 26.3 silhouette edges (top row, left column) per stepped tick after the kill.
JAVA: dict[str, tuple[int, int]] = {
    "t00": (159, 548),
    "t01": (173, 410),
    "t02": (203, 357),
    "t03": (235, 324),
    "t04": (263, 304),
    "t05": (288, 290),
    "t06": (312, 281),
    "t07": (335, 276),
    "t08": (357, 274),
    "t09": (373, 275),
    "t10": (385, 277),
    "t11": (397, 276),
    "t12": (408, 275),
    "t13": (414, 274),
    "t14": (414, 274),
    "t15": (414, 274),
    "t16": (414, 274),
    "t17": (414, 274),
    "t18": (414, 274),
}
# Pixel tolerance for the matched frame: covers rasterization (the native upright frame is one
# column off), rejects a moved pivot or a neighbouring curve step early in the fall.
TOLERANCE = 2


def edges(empty: Path, frame: Path) -> tuple[int, int]:
    """Top row and left column of what differs from the empty frame in REGION."""
    from PIL import Image, ImageChops

    with Image.open(empty) as before, Image.open(frame) as after:
        if before.size != FRAME or after.size != FRAME:
            raise ValueError("death-tilt oracle requires the matched 1280x720 frames")
        diff = ImageChops.difference(before.convert("RGB").crop(REGION), after.convert("RGB").crop(REGION))
    red, green, blue = diff.split()
    mask = ImageChops.lighter(ImageChops.lighter(red, green), blue).point(lambda v: 255 if v > THRESHOLD else 0)
    box = mask.getbbox()
    if box is None:
        raise ValueError(f"death-tilt silhouette missing in {frame}")
    return REGION[1] + box[1], REGION[0] + box[0]


def references(shot: str) -> list[tuple[int, int]]:
    """Java frames a shot may equal: its own step and the step before it."""
    step = int(shot[1:])
    return [JAVA[name] for name in (shot, f"t{step - 1:02d}") if name in JAVA]


def matches(shot: str, measured: tuple[int, int]) -> bool:
    return any(all(abs(a - b) <= TOLERANCE for a, b in zip(measured, reference, strict=True))
               for reference in references(shot))
