import math
import tempfile
import unittest
from pathlib import Path

from tools.harness.backends import screens_voice_qa as qa


class ScreensVoiceHelperTests(unittest.TestCase):
    def test_facing_puts_the_crosshair_on_the_target(self):
        yaw, pitch = qa.facing((1000.5, 160.0, 1000.5), (1000.5, 161.62, 1004.5))
        self.assertAlmostEqual(yaw, 0.0)
        self.assertAlmostEqual(pitch, 0.0)
        yaw, _ = qa.facing((999.5, 160.0, 1002.0), (1000.5, 161.62, 1002.0))
        self.assertAlmostEqual(yaw, -90.0)  # +x is yaw -90 in vanilla
        _, pitch = qa.facing((0.0, 160.0, 0.0), (0.0, 171.62, 10.0))
        self.assertAlmostEqual(pitch, -45.0)  # looking up is negative pitch

    def test_recording_measures_only_the_tone_that_was_heard(self):
        with tempfile.TemporaryDirectory() as directory:
            recording = qa.Recording(qa.write_tone(Path(directory) / "heard.wav", 440, 0.5))
            window = recording.segment(0, recording.size(), (440, 660))
            self.assertTrue(qa.heard(window, 440))
            self.assertTrue(qa.silent(window, 660))
            empty = recording.segment(recording.size(), recording.size(), (440,))
            self.assertEqual(empty["frames"], 0)
            self.assertTrue(qa.silent(empty, 440))

    def test_a_packetized_tone_is_heard_despite_phase_jumps(self):
        # A relayed stream restarts phase at packet boundaries; it is still the same audible tone.
        rate, packet = qa.RATE, qa.RATE // 50
        mono, phase = [], 0.0
        for index in range(3 * 50):
            phase += 0.37 * index
            mono.extend(0.05 * math.sin(phase + 2 * math.pi * 440 * n / rate) for n in range(packet))
        amplitude = qa.tone_amplitudes(mono, rate, (440, 660))
        self.assertTrue(qa.heard({"amplitude": amplitude}, 440), amplitude)
        self.assertTrue(qa.silent({"amplitude": amplitude}, 660), amplitude)


if __name__ == "__main__":
    unittest.main()
