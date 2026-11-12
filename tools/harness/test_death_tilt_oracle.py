"""Silhouette check contract; the death-tilt scene supplies the parity evidence."""
import tempfile
import unittest
from pathlib import Path

from PIL import Image, ImageDraw

from .backends import death_tilt_oracle as oracle


def frame(path: Path, body=None, hud=False) -> Path:
    image = Image.new("RGB", oracle.FRAME, (120, 160, 90))
    draw = ImageDraw.Draw(image)
    if hud:
        # Chat, hotbar and hand changes outside the region never count.
        left, top, right, bottom = oracle.REGION
        draw.rectangle((0, bottom, oracle.FRAME[0] - 1, oracle.FRAME[1] - 1), fill=(255, 255, 255))
        draw.rectangle((right, 0, oracle.FRAME[0] - 1, oracle.FRAME[1] - 1), fill=(255, 255, 255))
    if body is not None:
        draw.rectangle(body, fill=(40, 80, 160))
    image.save(path)
    return path


class DeathTiltOracleTests(unittest.TestCase):
    def test_java_edges_match_and_a_floating_body_does_not(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            empty = frame(root / "empty.png")
            for shot, (top, left) in oracle.JAVA.items():
                body = (left, top, left + 40, top + 30)
                measured = oracle.edges(empty, frame(root / f"{shot}.png", body, hud=True))
                self.assertEqual(measured, (top, left))
                self.assertTrue(oracle.matches(shot, measured))
                raised = (top - oracle.TOLERANCE - 1, left)
                self.assertFalse(oracle.matches(shot, raised))

    def test_a_frame_matches_its_own_step_or_the_step_before_and_no_other(self):
        self.assertTrue(oracle.matches("t05", oracle.JAVA["t05"]))
        self.assertTrue(oracle.matches("t05", oracle.JAVA["t04"]))
        self.assertTrue(oracle.matches("t05", tuple(v + oracle.TOLERANCE for v in oracle.JAVA["t04"])))
        self.assertFalse(oracle.matches("t05", oracle.JAVA["t03"]))
        self.assertFalse(oracle.matches("t05", oracle.JAVA["t06"]))
        self.assertTrue(oracle.matches("t00", oracle.JAVA["t00"]))
        self.assertFalse(oracle.matches("t00", oracle.JAVA["t01"]))

    def test_missing_silhouette_and_unmatched_frames_are_errors(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            empty = frame(root / "empty.png")
            with self.assertRaisesRegex(ValueError, "silhouette missing"):
                oracle.edges(empty, frame(root / "same.png", hud=True))
            Image.new("RGB", (854, 480)).save(root / "small.png")
            with self.assertRaisesRegex(ValueError, "1280x720"):
                oracle.edges(empty, root / "small.png")


if __name__ == "__main__":
    unittest.main()
