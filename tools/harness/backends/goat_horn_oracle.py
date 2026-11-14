"""Pixel check for the fixed 1280x720 goat front parity scene."""
from pathlib import Path


def horn_left_edges(image: Path) -> list[int]:
    from PIL import Image

    with Image.open(image) as source:
        frame = source.convert("RGB")
        if frame.size != (1280, 720):
            raise ValueError("goat oracle requires the matched 1280x720 frame")
        edges = []
        # The horn is isolated against the blue sky in the declared front pose.
        # Sampling the dark silhouette avoids depending on interior horn texture.
        for y in range(365, 396, 5):
            dark = [x for x in range(450, 491) if max(frame.getpixel((x, y))) < 180]
            if len(dark) < 4:
                raise ValueError(f"goat horn silhouette missing at y={y}")
            edges.append(dark[0])
        return edges


def upright(edges: list[int]) -> bool:
    # Java 26.3 reference is vertical over these rows (zero-pixel edge span).
    # A one-pixel tolerance admits rasterization but rejects the previous lean.
    return max(edges) - min(edges) <= 1


def body_left_edges(image: Path) -> list[int]:
    from PIL import Image

    with Image.open(image) as source:
        frame = source.convert("RGB")
        if frame.size != (1280, 720):
            raise ValueError("goat oracle requires the matched 1280x720 frame")
        edges = []
        for y in (450, 470, 490, 510):
            pale = [x for x in range(410, 590)
                    if min(rgb := frame.getpixel((x, y))) > 95 and max(rgb) - min(rgb) < 32]
            if len(pale) < 10:
                raise ValueError(f"goat body silhouette missing at y={y}")
            edges.append(pale[0])
        return edges


def body_aligned(edges: list[int]) -> bool:
    # The matching 26.3 oracle has its front body edge at x=447 in this pose.
    return all(446 <= x <= 448 for x in edges)


def ear_lower_coverage(image: Path) -> int:
    from PIL import Image

    with Image.open(image) as source:
        frame = source.convert("RGB")
        if frame.size != (1280, 720):
            raise ValueError("goat oracle requires the matched 1280x720 frame")
        # Lower left-ear area in the matched Java front view; sky/grass are not grey.
        return sum(
            1 for y in range(420, 425) for x in range(442, 450)
            if min(rgb := frame.getpixel((x, y))) > 80 and max(rgb) - min(rgb) < 20
        )
