"""Negative-control unit tests for the wave-1 native promotion probes."""
from __future__ import annotations

import types
import unittest
from unittest import mock

from tools.harness.backends import native_promotion_wave1 as wave1


class Steps:
    def __init__(self, rows=(), chat=(), health=20.0, screen_open=False, screen_class="none"):
        self.report = {"assertions": {}, "fixture_commands": []}
        self.rows = list(rows)
        self.chat = list(chat)
        self.health = health
        self.screen_open = screen_open
        self.screen_class = screen_class
        self.calls = []
        self.client = types.SimpleNamespace(p4_window=object())

    def fixture(self, command):
        self.report["fixture_commands"].append(command)

    def call(self, name, arguments=None):
        self.calls.append((name, arguments))
        if name == "minecraft_respawn":
            self.screen_open = False
        return {}

    def observe(self, **kwargs):
        return {"recent_chat": list(self.chat),
                "player": {"health": self.health},
                "screen": {"open": self.screen_open, "class": self.screen_class},
                "in_play": not self.screen_open,
                "inventory": []}
    def check(self, name, condition):
        self.report["assertions"][name] = bool(condition)
        if not condition:
            raise RuntimeError(name)

    def wait(self, predicate, label, **kwargs):
        state = self.observe()
        if not predicate(state):
            raise RuntimeError(label)
        return state

    def remaining(self, **kwargs):
        return 60.0


def _row(kind, x, z, **extra):
    row = {"entity_id": abs(hash((kind, x, z))) % 1000, "entity_uuid": f"{kind}-{x}-{z}",
           "entity_type": kind, "x": x, "z": z, "health": extra.pop("health", 20.0),
           "bounding_box": {"min_y": 0.0, "max_y": 2.0}}
    row.update(extra)
    return row


class GolemChatSteps(Steps):
    """A native-client double whose `recent_chat` is bounded and newest first, like the real one.

    The merge replies, the premise marker echo and the `data get` answer are appended to the
    live chat window in that order, so a reply newer than a marker sits at a *smaller* index.
    """

    def __init__(self, chat=(), reply_newest=True):
        super().__init__()
        self.chat = list(chat) or ["HarnessEntities joined the game"]
        self.time = {"game_time": 5_000}
        self.reply_newest = reply_newest

    def _say(self, line):
        self.chat.insert(0, line)
        del self.chat[100:]

    def fixture(self, command):
        super().fixture(command)
        self._say(f"applied {command}")

    def call(self, name, arguments=None):
        message = str((arguments or {}).get("message", ""))
        if name == "minecraft_send_chat":
            if message.startswith("data merge entity "):
                self._say("Modified entity data of Villager")
            elif message.startswith("data get entity "):
                reply = ('Villager has the following entity data: {memories: '
                         f'{{"minecraft:last_slept": {{value: {self.time["game_time"] - 100}L}}}}}}')
                if self.reply_newest:
                    self._say(reply)
                else:
                    # The pre-merge shape: the answer is the oldest line in the window.
                    self.chat.append(reply)
            elif message.startswith("golem premise "):
                self._say(f"<HarnessEntities> {message}")
        return super().call(name, arguments)

    def observe(self, **kwargs):
        state = super().observe(**kwargs)
        state["time"] = dict(self.time)
        return state

    def wait(self, predicate, label, timeout_seconds=None, **kwargs):
        state = self.observe()
        if not predicate(state):
            raise TimeoutError(f"timed out waiting for {label}")
        return state


AGE_KINDS = ("minecraft:squid", "minecraft:glow_squid", "minecraft:skeleton_horse", "minecraft:zombie_horse")
BABY_KINDS = ("minecraft:zombie", "minecraft:piglin", "minecraft:zoglin")


def _summon_rows(baby=True):
    rows = []
    for i, kind in enumerate(AGE_KINDS):
        rows.append(_row(kind, .5 + i, 2.5, received_baby=baby))
    for i, kind in enumerate(BABY_KINDS):
        rows.append(_row(kind, .5 + i, 5.5, received_baby=baby))
    return rows


class Wave1Tests(unittest.TestCase):
    def _patch_common(self, steps, rows):
        return mock.patch.multiple(
            wave1,
            _entity_at=mock.Mock(side_effect=lambda _s, kind, x, z: next(
                r for r in rows if r["entity_type"] == kind and abs(r["x"] - x) < .6 and abs(r["z"] - z) < .6)),
            _rows=mock.Mock(side_effect=lambda _s, radius=48: rows),
            _echo=mock.Mock(side_effect=lambda _s, label: _s.observe()),
            _capture=mock.Mock(), _give=mock.Mock(), _set=mock.Mock(),
            _use=mock.Mock(), _aim=mock.Mock(), _interact=mock.Mock(),
        )

    def test_summon_age_requires_age_and_baby_flags(self):
        chat = ["Squid has the following entity data: -23781"]
        with self._patch_common(Steps(chat=chat), _summon_rows(True)):
            wave1._summon_age(Steps(chat=chat), (0, 0, 0))
        rows = _summon_rows(True)
        rows[3]["received_baby"] = False  # zombie_horse
        steps = Steps(chat=chat)
        with self._patch_common(steps, rows):
            with self.assertRaisesRegex(RuntimeError, "minecraft:zombie_horse_age_syncs_baby_flag"):
                wave1._summon_age(steps, (0, 0, 0))
        steps = Steps(chat=["Squid has the following entity data: 6000"])  # age not applied
        with self._patch_common(steps, _summon_rows(True)):
            with self.assertRaisesRegex(RuntimeError, "minecraft:squid_age_is_negative"):
                wave1._summon_age(steps, (0, 0, 0))

    def test_death_messages_require_death_screen_despite_suppression(self):
        steps = Steps(chat=["Killed HarnessEntities"], screen_open=True,
                      screen_class="net.minecraft.client.gui.screens.DeathScreen")
        with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe()),
                                 _capture=mock.Mock()):
            wave1._death_messages(steps, (0, 0, 0))
        steps = Steps(chat=["Killed HarnessEntities"], screen_open=False)
        with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe()),
                                 _capture=mock.Mock()):
            with self.assertRaisesRegex(RuntimeError, "victim_gets_death_screen_despite_show_death_messages_false"):
                wave1._death_messages(steps, (0, 0, 0))

    def test_score_food_requires_auto_update(self):
        full = "foodfull" + "a" * 32
        steps = Steps(chat=["Created objective hunger", full, "HarnessEntities has 19 (hunger)"])
        with mock.patch.object(wave1.uuid, "uuid4", return_value=types.SimpleNamespace(hex="a" * 32)):
            with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe())):
                wave1._score_food(steps, (0, 0, 0))
        steps = Steps(chat=["Created objective hunger", full, "HarnessEntities has 20 (hunger)"])
        with mock.patch.object(wave1.uuid, "uuid4", return_value=types.SimpleNamespace(hex="a" * 32)):
            with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe())):
                with self.assertRaisesRegex(RuntimeError, "food_score_auto_updated_below_20"):
                    wave1._score_food(steps, (0, 0, 0))

    def test_golem_repair_requires_exact_65_not_overheal(self):
        villager = _row("minecraft:villager", 1.0, -3.0)
        far = _row("minecraft:iron_golem", 15.0, 11.0)
        golem = _row("minecraft:iron_golem", .5, 2.5, health=40.0)
        rows = [villager, far, golem]
        steps = Steps(rows=rows, chat=["Summoned new Iron Golem", "Villager has the following entity data: 1b"], health=20.0)

        def interact(_s, ident):
            golem["health"] = 100.0  # overheal bug: 40 + 25 clamped at max

        with self._patch_common(steps, rows):
            with mock.patch.object(wave1, "_interact", side_effect=interact), \
                 mock.patch.object(wave1, "_same_row", side_effect=lambda s, ident: next(
                     (r for r in rows if r["entity_uuid"] == ident["entity_uuid"]), None)), \
                 mock.patch.object(wave1.uuid, "uuid4", return_value=types.SimpleNamespace(hex="c" * 32)):
                with self.assertRaisesRegex(RuntimeError, "golem_repaired_exactly_25_to_65"):
                    wave1._golem(steps, (0, 0, 0))

    def test_axolotl_requires_play_dead_metadata_19(self):
        ax = _row("minecraft:axolotl", 1.5, 3.5, health=6.0,
                  received_metadata={"19": {"kind": "bool", "value": True}})
        steps = Steps(rows=[ax], chat=["minecraft:regeneration"])
        with self._patch_common(steps, [ax]):
            with mock.patch.object(wave1, "_same_row", side_effect=lambda s, ident: ax):
                wave1._axolotl(steps, (0, 0, 0))
        ax2 = _row("minecraft:axolotl", 1.5, 3.5, health=6.0,
                   received_metadata={"19": {"kind": "bool", "value": False}})
        steps = Steps(rows=[ax2], chat=[])
        with self._patch_common(steps, [ax2]):
            with mock.patch.object(wave1, "_same_row", side_effect=lambda s, ident: ax2):
                with self.assertRaisesRegex(RuntimeError, "axolotl_play_dead_metadata_19"):
                    wave1._axolotl(steps, (0, 0, 0))

    def test_spawner_egg_requires_configured_spawn_and_later_cow(self):
        cow = _row("minecraft:cow", 1.5, 3.5, health=10.0)
        chat = ['id: "minecraft:cow"']
        state = {"calls": 0}

        def rows_fn(_s, radius=48):
            state["calls"] += 1
            return [cow] if state["calls"] > 2 else []  # spawner spawns only later

        steps = Steps(chat=chat)
        with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe()),
                                 _capture=mock.Mock(), _give=mock.Mock(), _set=mock.Mock(),
                                 _block=mock.Mock(), _use=mock.Mock(), _rows=rows_fn):
            wave1._spawner_egg(steps, (0, 0, 0))
        steps = Steps(chat=chat)
        with mock.patch.multiple(wave1, _echo=mock.Mock(side_effect=lambda s, l: s.observe()),
                                 _capture=mock.Mock(), _give=mock.Mock(), _set=mock.Mock(),
                                 _block=mock.Mock(), _use=mock.Mock(), _rows=mock.Mock(return_value=[])):
            with self.assertRaisesRegex(RuntimeError, "spawner_spawned_configured_cow_on_peaceful"):
                wave1._spawner_egg(steps, (0, 0, 0))

    def test_golem_premise_reads_a_reply_newer_than_its_marker(self):
        # 20261124T094235: the merges landed ("Modified entity data of Villager" x5) and the
        # `data get` answer was in the chat, but the old after-the-marker slice looked at the
        # older half of a newest-first window and the wait sat on the scenario deadline.
        steps = GolemChatSteps()
        villagers = [_row("minecraft:villager", .5 + (index % 2) * 2, .5 + (index // 2) * 2)
                     for index in range(5)]
        golem = _row("minecraft:iron_golem", .5, 3.5)

        def wanderer(_s, _kind, x, z, exclude=()):
            return next(v for v in villagers if v["x"] == x and v["z"] == z)

        with mock.patch.multiple(
                wave1,
                _set=mock.Mock(),
                _capture=mock.Mock(),
                _echo=mock.Mock(side_effect=lambda s, label: s.observe()),
                _wanderer_at=mock.Mock(side_effect=wanderer),
                _rows=mock.Mock(side_effect=lambda _s, radius=48: villagers + [golem]),
        ):
            wave1._golem_gossip(steps, (0, 0, 0))
        assertions = steps.report["assertions"]
        self.assertTrue(assertions["golem_memory_premise_written"])
        self.assertTrue(assertions["five_sleeping_villagers_summoned_golem"])
        premise = steps.report["golem_memory_premise"]
        self.assertEqual(len(premise), 5)
        for lines in premise.values():
            self.assertIn("last_slept", lines[0])

    def test_golem_premise_rejects_a_reply_older_than_its_marker(self):
        # The old after-the-marker slice passed only this shape, which a newest-first window
        # never produces: the answer older than its marker is not evidence of this merge.
        steps = GolemChatSteps(reply_newest=False)
        villagers = [_row("minecraft:villager", .5 + (index % 2) * 2, .5 + (index // 2) * 2)
                     for index in range(5)]

        def wanderer(_s, _kind, x, z, exclude=()):
            return next(v for v in villagers if v["x"] == x and v["z"] == z)

        with mock.patch.multiple(
                wave1,
                _set=mock.Mock(),
                _capture=mock.Mock(),
                _echo=mock.Mock(side_effect=lambda s, label: s.observe()),
                _wanderer_at=mock.Mock(side_effect=wanderer),
                # A golem is already in reach, so a code path that accepts the stale reply runs
                # to a green summon verdict instead of stopping on a later assertion.
                _rows=mock.Mock(side_effect=lambda _s, radius=48: villagers + [
                    _row("minecraft:iron_golem", .5, 3.5)]),
        ):
            with self.assertRaisesRegex(TimeoutError, "golem_premise_memories"):
                wave1._golem_gossip(steps, (0, 0, 0))
        # No premise line was accepted, so no golem verdict could be read on it.
        self.assertNotIn("golem_memory_premise", steps.report)

    def test_dead_player_is_respawned_before_the_next_golem_reading(self):
        dead = Steps(health=0.0, screen_open=True,
                     screen_class="net.minecraft.client.gui.screens.DeathScreen")
        original_observe = dead.observe

        def observe(**kwargs):
            state = original_observe(**kwargs)
            if not dead.screen_open:
                state["player"]["health"] = 20.0
            return state
        dead.observe = observe
        wave1._revive_if_dead(dead, "normal_player_respawned")
        self.assertEqual([name for name, _ in dead.calls], ["minecraft_respawn"])
        alive = Steps(health=20.0)
        wave1._revive_if_dead(alive, "normal_player_respawned")
        self.assertEqual(alive.calls, [])


if __name__ == "__main__":
    unittest.main()
