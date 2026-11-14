"""Silhouette check contract; real scenes supply the parity evidence."""
import tempfile
import unittest
from pathlib import Path

from PIL import Image

from .backends import goat_horn_oracle


class GoatHornOracleTests(unittest.TestCase):
    def test_upright_and_leaning_silhouettes_are_distinct(self):
        with tempfile.TemporaryDirectory() as directory:
            image = Image.new("RGB", (1280, 720), (190, 220, 255))
            for y in range(360, 396):
                for x in range(469, 481):
                    image.putpixel((x, y), (50, 50, 50))
            path = Path(directory) / "upright.png"
            image.save(path)
            self.assertTrue(goat_horn_oracle.upright(goat_horn_oracle.horn_left_edges(path)))
            for y in range(360, 396):
                for x in range(450, 491):
                    image.putpixel((x, y), (190, 220, 255))
                for x in range(469 - (y - 360) // 10, 481 - (y - 360) // 10):
                    image.putpixel((x, y), (50, 50, 50))
            image.save(path)
            self.assertFalse(goat_horn_oracle.upright(goat_horn_oracle.horn_left_edges(path)))

    def test_body_alignment_rejects_observed_left_shift(self):
        self.assertTrue(goat_horn_oracle.body_aligned([447, 447, 447, 447]))
        self.assertFalse(goat_horn_oracle.body_aligned([443, 443, 443, 443]))

    def test_side_horn_alignment_rejects_observed_offset(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "side.png"
            image = Image.new("RGB", (1280, 720), (190, 220, 255))
            for y in range(360, 376):
                for x in range(701, 723):
                    image.putpixel((x, y), (50, 50, 50))
            image.save(path)
            edges = goat_horn_oracle.side_horn_left_edges(path)
            self.assertTrue(goat_horn_oracle.side_horn_aligned(edges))
            self.assertFalse(goat_horn_oracle.side_horn_aligned([edge + 8 for edge in edges]))
            # The observed raster shift begins the same horn one row lower.
            for x in range(701, 723):
                image.putpixel((x, 360), (190, 220, 255))
                image.putpixel((x, 376), (50, 50, 50))
            image.save(path)
            shifted = goat_horn_oracle.side_horn_left_edges(path)
            self.assertTrue(goat_horn_oracle.side_horn_aligned(shifted))
            self.assertFalse(goat_horn_oracle.side_horn_aligned([edge + 8 for edge in shifted]))

    def test_ear_coverage_distinguishes_grey_ear_from_grass(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "ear.png"
            image = Image.new("RGB", (1280, 720), (70, 110, 54))
            image.save(path)
            self.assertEqual(goat_horn_oracle.ear_lower_coverage(path), 0)
            for y in range(420, 425):
                for x in range(442, 450):
                    image.putpixel((x, y), (177, 177, 175))
            image.save(path)
            self.assertEqual(goat_horn_oracle.ear_lower_coverage(path), 40)
