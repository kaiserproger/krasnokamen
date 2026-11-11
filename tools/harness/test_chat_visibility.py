"""Offline checks for the chat-visibility gate: registration and the ClientInformation rewriter."""
from pathlib import Path
import unittest

from . import profiles, scenarios
from .backends import chat_visibility as gate
from .backends.chat_visibility import ClientInformationRewriter, write_varint


def frame(packet_id: int, body: bytes = b"", *, compressed: bool = False) -> bytes:
    payload = (write_varint(0) if compressed else b"") + write_varint(packet_id) + body
    return write_varint(len(payload)) + payload


def client_information(mode: int) -> bytes:
    # locale "en_us", view distance 8, chat mode, then the rest of the packet.
    return write_varint(5) + b"en_us" + bytes([8]) + write_varint(mode) + b"\x01\x7f\x01\x00\x01\x00"


class ChatVisibilityGateTests(unittest.TestCase):
    def test_scenario_resolves_to_graphical_profile(self) -> None:
        scenario = scenarios.SCENARIOS["native-chat-visibility"]
        self.assertEqual(scenario.profile, "chat-visibility")
        self.assertEqual(scenario.evidence, "graphical")
        self.assertEqual(profiles.PROFILES[scenario.profile]["run"], profiles.run_chat_visibility)

    def test_rejects_non_windowed_or_non_native_clients_before_launch(self) -> None:
        for client, windowed in (("java", True), ("native", False)):
            with self.subTest(client=client, windowed=windowed):
                ctx = profiles.ProfileContext(Path("."), client=client, windowed=windowed)
                with self.assertRaisesRegex(ValueError, "requires --client native --windowed"):
                    gate.run(ctx)

    def stream(self, play_mode: int = 0) -> bytes:
        return (frame(0, b"\x00" * 6) + frame(0, b"name") + frame(3)
                + frame(CONFIG_INFORMATION, client_information(0)) + frame(2, b"x") + frame(3)
                + frame(0, b"\x01") + frame(PLAY_INFORMATION, client_information(play_mode)))

    def test_rewrites_the_chat_mode_in_configuration_and_play_only(self) -> None:
        rewriter = ClientInformationRewriter(gate.HIDDEN)
        rewriter.threshold = None
        original = self.stream()
        rewritten = rewriter.feed(original)
        self.assertEqual(len(rewritten), len(original))
        self.assertEqual(rewriter.rewrites, [{"phase": "configuration", "was": 0, "now": 2},
                                             {"phase": "play", "was": 0, "now": 2}])
        # Only the two chat-mode bytes differ; the Play id-0 teleport confirmation is untouched.
        differences = [i for i, (a, b) in enumerate(zip(original, rewritten)) if a != b]
        self.assertEqual(len(differences), 2)
        self.assertTrue(all(rewritten[i] == gate.HIDDEN for i in differences))

    def test_survives_arbitrary_fragmentation_and_compression_framing(self) -> None:
        for compressed in (False, True):
            with self.subTest(compressed=compressed):
                rewriter = ClientInformationRewriter(gate.COMMANDS_ONLY)
                rewriter.threshold = 256 if compressed else None
                data = b"".join(
                    frame(pid, body, compressed=compressed)
                    for pid, body in ((0, b"\x00" * 6), (0, b"name"), (3, b""),
                                      (CONFIG_INFORMATION, client_information(0)), (3, b""),
                                      (PLAY_INFORMATION, client_information(0))))
                out = b"".join(rewriter.feed(data[i:i + 1]) for i in range(len(data)))
                self.assertEqual(len(out), len(data))
                self.assertEqual([r["now"] for r in rewriter.rewrites], [1, 1])

    def test_a_compressed_frame_is_forwarded_untouched(self) -> None:
        rewriter = ClientInformationRewriter(gate.HIDDEN)
        rewriter.threshold = 256
        data = frame(0) + frame(0) + frame(3)
        payload = write_varint(5) + b"\x00zlib"  # data length 5: compressed
        data += write_varint(len(payload)) + payload
        self.assertEqual(rewriter.feed(data), data)
        self.assertEqual(rewriter.rewrites, [])

    def test_the_planned_modes_cover_every_setting_and_a_second_hidden_join(self) -> None:
        self.assertEqual(gate.MODES, (gate.FULL, gate.COMMANDS_ONLY, gate.HIDDEN, gate.HIDDEN))
        self.assertEqual((gate.FULL, gate.COMMANDS_ONLY, gate.HIDDEN), (0, 1, 2))


CONFIG_INFORMATION = gate.CONFIGURATION_CLIENT_INFORMATION
PLAY_INFORMATION = gate.PLAY_CLIENT_INFORMATION

if __name__ == "__main__":
    unittest.main()
