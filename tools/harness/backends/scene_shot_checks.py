"""Scene-shot picture checks: what a retained frame must show (entity rigs, item bars,
clock faces, player head, dynamic context) and the pixel comparisons behind them.

The client/report seams live on ``scene_shot`` (the tests patch them there), so this
module reaches them as ``_qa.<name>`` and never binds them.
"""

from __future__ import annotations

import json
import math
import shutil
from pathlib import Path
from typing import Any

from .probe_steps import ProbeSteps
from . import scene_shot as _qa
from .scene_shot_client import WINDOW


def check_263_entity_rigs(probe: ProbeSteps, artifact: Path) -> None:
    kinds = ("minecraft:sulfur_cube", "minecraft:cushion",
             "minecraft:poplar_boat", "minecraft:poplar_chest_boat")
    received = json.loads((artifact / "entities.json").read_text())
    for kind in kinds:
        probe.check(f"received_{kind}", sum(row.get("entity_type") == kind
                                            for row in received["entities"]) == 1)
    frame = json.loads((artifact / "screenshots/entity-rigs.json").read_text())
    before_frame = json.loads((artifact / "screenshots/before-rigs.json").read_text())
    probe.check("rig_frames_gpu_complete", before_frame["gpu_completion"] is True and frame["gpu_completion"] is True)
    probe.check("rig_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(before_frame["camera_position"], frame["camera_position"])
    ) and abs(before_frame["yaw"] - frame["yaw"]) < 0.01
      and abs(before_frame["pitch"] - frame["pitch"]) < 0.01)
    unrendered = frame["entity_extraction"]["unrendered"]
    for kind in kinds:
        probe.check(f"rendered_{kind}", kind not in unrendered)
        row = next(row for row in received["entities"] if row.get("entity_type") == kind)
        probe.check(f"resolved_{kind}", row.get("cached_rig_status") == "resolved")
    probe.check("four_entities_submitted", frame["entity_extraction"]["submitted"] >= 4)
    from PIL import Image
    with Image.open(artifact / "screenshots/before-rigs.png") as before_image, \
            Image.open(artifact / "screenshots/entity-rigs.png") as after_image:
        before, after = before_image.convert("RGB"), after_image.convert("RGB")
        probe.check("matched_rig_frames", before.size == after.size == (1280, 720))
        # Fixed camera, flat platform and separate boxes isolate each received entity.
        # The two pale foreground models must brighten the platform, not merely cast a shadow.
        regions = {
            "cushion": ((430, 520, 580, 560), True),
            "sulfur_cube": ((740, 495, 805, 545), True),
            "poplar_chest_boat": ((410, 415, 550, 480), False),
            "poplar_boat": ((720, 415, 860, 480), False),
        }
        for name, (box, bright) in regions.items():
            old = before.crop(box).getdata()
            new = after.crop(box).getdata()
            pixels = sum(
                all(y > x + 25 for x, y in zip(a, b)) if bright
                else max(abs(x - y) for x, y in zip(a, b)) > 25
                for a, b in zip(old, new)
            )
            probe.report[f"{name}_changed_pixels"] = pixels
            probe.check(f"visible_pixels_{name}", pixels >= 200)
    probe.report["visual_review_required"] = ["Inspect all four models in entity-rigs.png; pixel changes alone do not prove model parity."]


def check_shelf_clock(probe: ProbeSteps, artifact: Path) -> None:
    frames = [
        json.loads((artifact / f"screenshots/clocks-{phase}.json").read_text())
        for phase in ("day", "night")
    ]
    day, night = frames
    probe.check("clock_frames_gpu_complete", all(frame["gpu_completion"] is True for frame in frames))
    probe.check("clock_camera_at_fixture", all(
        abs(a - b) < 0.01 for a, b in zip(day["camera_position"], (1000.5, 161.62, 1000.5))
    ) and abs(day["yaw"]) < 0.01 and abs(day["pitch"]) < 0.01)
    probe.check("clock_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(day["camera_position"], night["camera_position"])
    ) and abs(day["yaw"] - night["yaw"]) < 0.01 and abs(day["pitch"] - night["pitch"]) < 0.01)
    probe.check("clock_received_time_changed",
                day["environment"]["time_of_day"] == "1000"
                and night["environment"]["time_of_day"] == "13000")
    from PIL import Image
    masks = []
    for phase in ("day", "night"):
        with Image.open(artifact / f"screenshots/clocks-{phase}.png") as image:
            probe.check(f"clock_{phase}_image_size", image.size == (1280, 720))
            pixels = image.convert("RGB").crop((435, 457, 453, 481)).getdata()
            masks.append([b > r * 1.35 and b > g * 1.15 and b > 15 for r, g, b in pixels])
    probe.report["clock_blue_pixels"] = [sum(mask) for mask in masks]
    changes = sum(a != b for a, b in zip(*masks))
    probe.report["clock_changed_blue_pixels"] = changes
    probe.check("clock_face_visible", all(sum(mask) >= 20 for mask in masks))
    probe.check("clock_face_changed", changes >= 20)


def _load_shots(probe: ProbeSteps, artifact: Path, names: tuple[str, ...]) -> dict:
    from PIL import Image
    shots = {}
    for name in names:
        frame = json.loads((artifact / f"screenshots/{name}.json").read_text())
        probe.check(f"{name}_gpu_complete", frame.get("gpu_completion") is True)
        with Image.open(artifact / f"screenshots/{name}.png") as image:
            probe.check(f"{name}_image_size", image.size == (1280, 720))
            shots[name] = (frame, image.convert("RGB"))
    return shots


def _changed(a, b, box) -> int:
    return sum(
        max(abs(x - y) for x, y in zip(p, q)) > 25
        for p, q in zip(a.crop(box).getdata(), b.crop(box).getdata())
    )


def _count(image, box, pred) -> int:
    return sum(1 for p in image.crop(box).getdata() if pred(*p))


def check_entity_visuals(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-OPUS-12, MODEL-OPUS-16, MODEL-ENTITY-FIRE, MODEL-ENTITY-LEASH."""
    names = ("empty", "vindicator", "ghastling", "burning", "plain", "leashed")
    shots = _qa._load_shots(probe, artifact, names)
    ref = shots["empty"][0]
    for name in names[1:]:
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    maroon = lambda r, g, b: r > g + 25 and r > b + 25
    teal = lambda r, g, b: g > r + 15 and b > r + 15
    pink = lambda r, g, b: r - b > 40 and r > 120
    white = lambda r, g, b: r > 170 and g > 170 and b > 170 and max(r, g, b) - min(r, g, b) < 35
    orange = lambda r, g, b: r > 150 and 60 < g < 210 and b < 110
    rope = lambda r, g, b: 60 < r < 140 and 40 < g < 100 and b < 70 and r > g > b
    vindicator = shots["vindicator"][1]
    probe.check("vindicator_visible", _changed(shots["vindicator"][1], shots["empty"][1], (560, 280, 780, 560)) > 500)
    probe.check("vindicator_torso_not_maroon", _count(vindicator, (600, 380, 680, 450), maroon) < 50)
    arm_pixels = sum(_count(vindicator, box, teal) for box in ((590, 390, 615, 470), (665, 390, 700, 470)))
    probe.report["vindicator_teal_arm_pixels"] = arm_pixels
    probe.check("vindicator_arms_not_teal", arm_pixels < 50)
    leg_teal = _count(vindicator, (600, 460, 680, 540), teal)
    probe.report["vindicator_teal_leg_pixels"] = leg_teal
    probe.check("vindicator_legs_teal", leg_teal > 150)
    ghast_box = (560, 260, 780, 470)
    pink_n = _count(shots["ghastling"][1], ghast_box, pink)
    white_excess = _count(shots["ghastling"][1], ghast_box, white) - _count(shots["empty"][1], ghast_box, white)
    probe.report["ghastling_pink_pixels"] = pink_n
    probe.report["ghastling_white_excess"] = white_excess
    probe.check("ghastling_not_pink", pink_n < 500)
    probe.check("ghastling_body_white", white_excess > 3000)
    fire_box = (560, 300, 760, 520)
    burning_orange = _count(shots["burning"][1], fire_box, orange)
    plain_orange = _count(shots["plain"][1], fire_box, orange)
    probe.report["fire_orange_pixels"] = [plain_orange, burning_orange]
    probe.check("fire_overlay_only_when_burning", burning_orange - plain_orange > 4000)
    probe.check("fire_overlay_present", burning_orange > 5000)
    rope_n = _count(shots["leashed"][1], (560, 400, 760, 500), rope)
    probe.report["leash_rope_pixels"] = rope_n
    probe.check("leash_rope_drawn", rope_n > 300)
    probe.report["visual_review_required"] = [
        "Inspect vindicator/ghastling/burning/plain/leashed frames; pixel predicates do not prove full model parity."
    ]


def check_toasts(probe: ProbeSteps, artifact: Path) -> None:
    """NATIVE-TOASTS-MISSING: the join-time unsecure-server toast is drawn, then clears.

    The client raises it on the Play login packet (10 s visible). The scene teleports, fills the
    platform, teleports back, waits 40 ticks and shoots `toast-early`, then `toast-late` about
    16 s later. The player stands still, and the ROI is the panel's own
    `gui/sprites/toast/system` rectangle - `ToastManager` puts x = gui_width - width and y = 0,
    which at GUI scale 3 on a 1280x720 frame is x 621..1280, y 0..168 - so a dark navy panel and
    its yellow title there can only be the toast. Clouds drift between the two frames, so the
    assertion is the panel's presence and later absence, never a whole-frame diff (calibrated on
    20261123T215339-native-toast-nmjj7q5f: early panel 89445, late 0; title 3195 vs 0)."""
    shots = _qa._load_shots(probe, artifact, ("toast-early", "toast-late"))
    early_f, late_f = shots["toast-early"][0], shots["toast-late"][0]
    probe.check("toast_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(early_f["camera_position"], late_f["camera_position"])
    ) and abs(early_f["yaw"] - late_f["yaw"]) < 0.01 and abs(early_f["pitch"] - late_f["pitch"]) < 0.01)
    box = (621, 0, 1280, 168)
    panel = lambda r, g, b: max(r, g, b) < 90
    # `AdvancementToast`/`SystemToast` title ink: 0xffff00 on the navy panel.
    title = lambda r, g, b: r > 170 and g > 170 and b < 140
    early, late = shots["toast-early"][1], shots["toast-late"][1]
    early_panel, late_panel = _count(early, box, panel), _count(late, box, panel)
    early_title, late_title = _count(early, box, title), _count(late, box, title)
    probe.report["toast_roi"] = list(box)
    probe.report["toast_pixels"] = {
        "early_panel": early_panel,
        "late_panel": late_panel,
        "early_title": early_title,
        "late_title": late_title,
    }
    probe.check("toast_panel_appeared", early_panel > 1500 and late_panel < 50)
    probe.check("toast_title_drawn", early_title > 800 and late_title < 50)
    probe.report["visual_review_required"] = [
        "Inspect toast-early/toast-late; the pixels prove a toast panel was drawn, not the vanilla layout."
    ]


def check_item_bars(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-ITEM-BARS: durability bar and bundle fullness bar in the GUI hotbar."""
    shots = _qa._load_shots(probe, artifact, ("bars-damaged", "bars-clean"))
    ref = shots["bars-damaged"][0]
    frame = shots["bars-clean"][0]
    probe.check("bars_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
    ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    yellow = lambda r, g, b: r > 70 and g > 70 and b < r - 20 and b < g - 10
    blue = lambda r, g, b: b > r + 25 and b > g + 10 and b > 70
    damaged, clean = shots["bars-damaged"][1], shots["bars-clean"][1]
    slot0, slot2 = (376, 664, 420, 712), (420, 664, 464, 712)
    probe.report["pickaxe_slot_changed_pixels"] = _changed(damaged, clean, slot0)
    probe.report["bundle_slot_changed_pixels"] = _changed(damaged, clean, slot2)
    probe.check("pickaxe_slot_bar_changes", _changed(damaged, clean, slot0) > 50)
    probe.check("bundle_slot_bar_changes", _changed(damaged, clean, slot2) > 50)
    probe.report["durability_bar_pixels"] = _count(damaged, (376, 698, 420, 710), yellow)
    probe.report["fullness_bar_pixels"] = _count(damaged, (420, 698, 464, 710), blue)
    probe.check("durability_bar_drawn", _count(damaged, (376, 698, 420, 710), yellow) > 20)
    probe.check("fullness_bar_drawn", _count(damaged, (420, 698, 464, 710), blue) > 15)


def check_shade_override(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-SHADE-OVERRIDE: a face named by the model's shade override takes the up weight.

    `template_torch` carries `shade_direction_override: "up"` on its only element and its
    four side faces share one texture region, so two frames of the same torch — from the
    north and from the east — must read the same luminance: 26.3's override replaces each
    quad's own cardinal weight with the named one (up, 1.0). Without the override the
    mesher shades each side by its face (north 0.8, east 0.6 in the overworld table), so
    the ratio is 4:3 and the row's symptom is present. Noon keeps the sun term equal on
    both vertical faces; the torch's own light is identical in both frames.
    """
    shots = _qa._load_shots(probe, artifact, ("torch-north", "torch-east"))
    north_f, east_f = shots["torch-north"][0], shots["torch-east"][0]
    probe.check("torch_north_camera", all(
        abs(a - b) < 0.01 for a, b in zip(north_f["camera_position"], (1000.5, 161.62, 1002.5))
    ) and abs(north_f["yaw"]) < 0.01 and abs(north_f["pitch"] - 33.0) < 0.01)
    probe.check("torch_east_camera", all(
        abs(a - b) < 0.01 for a, b in zip(east_f["camera_position"], (1002.5, 161.62, 1004.5))
    ) and abs(east_f["yaw"] - 90.0) < 0.01 and abs(east_f["pitch"] - 33.0) < 0.01)
    probe.check("torch_frames_at_noon",
                north_f["environment"]["time_of_day"] == "6000"
                and east_f["environment"]["time_of_day"] == "6000")
    # The torch's warm texels against the grey stone and blue sky behind it.
    torch = lambda r, g, b: r - b >= 12 and r >= 25
    box = (600, 290, 680, 445)
    lums, counts = [], []
    for name in ("torch-north", "torch-east"):
        pixels = [p for p in shots[name][1].crop(box).getdata() if torch(*p)]
        counts.append(len(pixels))
        lums.append(sum(sum(p) for p in pixels) / len(pixels) / 3 if pixels else 0.0)
    probe.report["torch_face_roi"] = list(box)
    probe.report["torch_face_pixels"] = counts
    probe.report["torch_face_mean_luminance"] = [round(v, 2) for v in lums]
    probe.check("torch_face_visible_both_views", all(c > 1500 for c in counts))
    ratio = lums[0] / lums[1] if lums[1] else 0.0
    probe.report["torch_face_luminance_ratio"] = round(ratio, 3)
    probe.check("torch_faces_same_shade", 0.94 <= ratio <= 1.06)


def check_decorated_pot(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-PLACED-POT-DECORATIONS: a placed pot's sides read the received sherds.

    26.3 `PotDecorations` is a record of optional `back`/`left`/`right`/`front` templates, and
    `ItemStackTemplate.CODEC` accepts either `{id, ...}` or a bare item id — the server copies the
    placing item's component verbatim into the block entity's `sherds`, so a pot placed from a
    bare-form item reaches the client as bare strings. The fixture sets the same block twice at the
    same position and camera: plain, then with the angler sherd on all four sides. A renderer that
    reads only `template["id"]` draws brick in both frames (the row's symptom); one that also
    accepts the bare id draws the pottery pattern in the second frame.
    """
    shots = _qa._load_shots(probe, artifact, ("pot-plain", "pot-sherds"))
    plain_f, sherds_f = shots["pot-plain"][0], shots["pot-sherds"][0]
    probe.check("pot_camera_at_fixture", all(
        abs(a - b) < 0.01 for a, b in zip(plain_f["camera_position"], (1000.5, 161.62, 1002.0))
    ) and abs(plain_f["yaw"]) < 0.01 and abs(plain_f["pitch"] - 20.0) < 0.01)
    probe.check("pot_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(plain_f["camera_position"], sherds_f["camera_position"])
    ) and abs(plain_f["yaw"] - sherds_f["yaw"]) < 0.01
      and abs(plain_f["pitch"] - sherds_f["pitch"]) < 0.01)
    box = (500, 280, 780, 540)
    plain, sherds = shots["pot-plain"][1].crop(box), shots["pot-sherds"][1].crop(box)
    # The pot body against the stone/sky: terracotta browns, not grey or blue.
    pot = lambda r, g, b: r > g + 12 and r > 60 and b < r
    visible = [sum(1 for pixel in image.getdata() if pot(*pixel)) for image in (plain, sherds)]
    changed = _changed(shots["pot-plain"][1], shots["pot-sherds"][1], box)
    probe.report["decorated_pot_roi"] = list(box)
    probe.report["decorated_pot_pixels"] = visible
    probe.report["decorated_pot_changed_pixels"] = changed
    probe.check("pot_visible_both_frames", all(count > 2000 for count in visible))
    probe.check("pot_side_pattern_changed", changed > 300)


def check_structure_box(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-BLOCK-ENTITY-DRAW: a spectator's structure block draws its received wire box.

    26.3 draws the structure block's transform box only for a viewer allowed to see the game-master
    overlay (a spectator, or a level-2 creative operator) and only from the block entity's received
    `mode`/`posX…`/`sizeX…`/`showboundingbox` fields. The fixture holds one structure block at a
    fixed position and camera, re-set at the same cell in LOAD mode twice: first with
    `showboundingbox` off (block model only), then with it on (block model plus the 4x3x3 box), so
    the two frames may differ only by the overlay wireframe.
    """
    shots = _qa._load_shots(probe, artifact, ("box-hidden", "box-shown"))
    hidden_f, shown_f = shots["box-hidden"][0], shots["box-shown"][0]
    # Standing on the pad the eye reads 161.60 (the player settles a hair into its top face), so the
    # height carries a 0.05 band; x/z stay tight and both frames must agree exactly.
    probe.check("structure_camera_at_fixture", all(
        abs(a - b) < 0.05 for a, b in zip(hidden_f["camera_position"], (1000.5, 161.62, 1002.0))
    ) and abs(hidden_f["yaw"]) < 0.01 and abs(hidden_f["pitch"]) < 0.01)
    probe.check("structure_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(hidden_f["camera_position"], shown_f["camera_position"])
    ) and abs(hidden_f["yaw"] - shown_f["yaw"]) < 0.01
      and abs(hidden_f["pitch"] - shown_f["pitch"]) < 0.01)
    # The box's near face projects to roughly x 211..897, y 123..638 at three blocks; the far face
    # to x 426..768, y 242..499 at six. One ROI over the whole projected box keeps every edge.
    box = (200, 110, 910, 650)
    # `wire` draws 0.9-neutral segments: bright grey, not the stone pad or the sky.
    line = lambda r, g, b: r > 190 and g > 190 and b > 190 and max(r, g, b) - min(r, g, b) <= 16
    lines = [_count(shots[name][1], box, line) for name in ("box-hidden", "box-shown")]
    changed = _changed(shots["box-hidden"][1], shots["box-shown"][1], box)
    probe.report["structure_box_roi"] = list(box)
    probe.report["structure_box_line_pixels"] = lines
    probe.report["structure_box_changed_pixels"] = changed
    probe.check("structure_box_lines_drawn", lines[1] > 300)
    probe.check("structure_box_changed", changed > 300)


def check_element_emission(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-ELEMENT-EMISSION: a model element with `light_emission` draws full-bright at night.

    26.3 `cross_emissive` (the parent of `firefly_bush` and `open_eyeblossom`) carries two extra
    elements with `light_emission: 15` over a separate emissive texture, so at night the overlay's
    texels must ignore the world light. `firefly_bush_emissive.png` has 9 texels with every channel
    above 200 and the base texture has none, so a near-white pixel in the bush's region can only be
    the emissive overlay: a baker that drops the per-element emission shades those texels with the
    night light and the frame holds none.
    """
    shots = _qa._load_shots(probe, artifact, ("bush-night",))
    frame = shots["bush-night"][0]
    probe.check("bush_camera_at_fixture", all(
        abs(a - b) < 0.01 for a, b in zip(frame["camera_position"], (1000.5, 161.62, 1002.3))
    ) and abs(frame["yaw"]) < 0.01 and abs(frame["pitch"] - 27.0) < 0.01)
    probe.check("bush_frame_at_night", frame["environment"]["time_of_day"] == "18000")
    # Light delivery probe. `minecraft_read_block` reports the mesher's own source light, so the
    # bush cell reads 7 only after the client received this section's block-light layer; a section
    # meshed before that keeps the `[-1.; 2]` unknown sample, whose vertices interpolate to a dim
    # overlay under load (the measured false red this probe removes). The scene lights a torch
    # behind the camera, waits for the section's remesh, reads the cell, removes the torch, waits
    # again and reads it back, so the frame that is asserted is drawn from a mesh built after the
    # layer was proven present and dark.
    def light(name: str) -> tuple[int, int]:
        row = json.loads((artifact / f"{name}.json").read_text())
        probe.report[name.replace("-", "_")] = {
            "sky_light": row["sky_light"], "block_light": row["block_light"],
        }
        return int(row["sky_light"]), int(row["block_light"])
    before_probe = light("bush-cell-before-probe")
    under_torch = light("bush-cell-under-torch")
    before_shot = light("bush-cell-before-shot")
    probe.check("bush_cell_block_light_received", under_torch[1] > 0)
    probe.check("bush_cell_dark_before_shot", before_shot[0] == 0 and before_shot[1] == 0)
    # The bush's crossed planes against the night floor and sky.
    box = (520, 240, 760, 480)
    image = shots["bush-night"][1].crop(box)
    bright = sum(1 for r, g, b in image.getdata() if min(r, g, b) > 200)
    probe.report["element_emission_roi"] = list(box)
    probe.report["element_emission_bright_pixels"] = bright
    # Measured 306 near-white pixels on the fixed client and 0 on the control; the pre-probe read
    # is evidence only (unknown and dark block light both report 0 to `read_block`).
    probe.report["element_emission_light_before_probe"] = list(before_probe)
    probe.check("emissive_overlay_full_bright", bright > 60)


def check_pig_saddle(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-OPUS-3: a pig with a saddle in equipment slot 7 draws its saddle layer.

    The server sends the saddle as its own equipment slot (`equipment:{saddle:{…}}` on
    `minecraft:pig` is the mount slot 7, which `command_summon` accepts and
    `entity_appearance` forwards), so the fixture needs no second client: one pig stands at a
    fixed position and camera, is captured plain, then is replaced by the same pig with the
    saddle. A client that resolves only the horse/wolf animal overlays draws two identical
    pigs (the row's symptom); one whose animal overlay table carries the pig draws the saddle
    in the second frame. Both frames must show the pig, so a missing entity cannot pass.
    """
    shots = _qa._load_shots(probe, artifact, ("pig-plain", "pig-saddled"))
    plain_f, saddled_f = shots["pig-plain"][0], shots["pig-saddled"][0]
    probe.check("pig_camera_at_fixture", all(
        abs(a - b) < 0.01 for a, b in zip(plain_f["camera_position"], (1000.5, 161.62, 1002.0))
    ) and abs(plain_f["yaw"]) < 0.01 and abs(plain_f["pitch"] - 20.0) < 0.01)
    probe.check("pig_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(plain_f["camera_position"], saddled_f["camera_position"])
    ) and abs(plain_f["yaw"] - saddled_f["yaw"]) < 0.01
      and abs(plain_f["pitch"] - saddled_f["pitch"]) < 0.01)
    # The body against the stone floor and sky: pig skin is a desaturated pink, and the saddle's
    # leather is the only brown in the frame (the stone is grey, the sky blue). The two frames are
    # the same pig one summon apart, so an animation or shading difference between them stays pink
    # and cannot pass this count; measured 0 on the control and 3810 brown pixels on the re-land.
    box = (500, 260, 780, 560)
    pig = lambda r, g, b: r > 120 and r > g + 25 and r > b + 25
    leather = lambda r, g, b: r > 70 and r > g + 35 and g > b + 15 and b < 90
    bodies = [sum(1 for pixel in shots[name][1].crop(box).getdata() if pig(*pixel))
              for name in ("pig-plain", "pig-saddled")]
    saddles = [sum(1 for pixel in shots[name][1].crop(box).getdata() if leather(*pixel))
               for name in ("pig-plain", "pig-saddled")]
    changed = _changed(shots["pig-plain"][1], shots["pig-saddled"][1], box)
    probe.report["pig_saddle_roi"] = list(box)
    probe.report["pig_visible_pixels"] = bodies
    probe.report["pig_saddle_leather_pixels"] = saddles
    probe.report["pig_saddle_changed_pixels"] = changed
    probe.check("pig_visible_both_frames", all(count > 1500 for count in bodies))
    probe.check("pig_saddle_drawn", saddles[0] < 100 and saddles[1] > 300)


def player_head_skin_pack(artifact: Path) -> Path:
    """An authored offline Steve skin: cyan base, opaque magenta hat with a face window."""
    from PIL import Image, ImageDraw

    pack = artifact / "head-skin-pack"
    skin = Image.new("RGBA", (64, 64), (0, 220, 220, 255))
    draw = ImageDraw.Draw(skin)
    draw.rectangle((32, 0, 63, 15), fill=(240, 0, 240, 255))
    draw.rectangle((42, 10, 45, 13), fill=(0, 0, 0, 0))
    # The pinned entity facts select slim/steve for unresolved placed heads
    # (default_skins[6]); the special-item path selects wide/steve.
    for model in ("slim", "wide"):
        texture = pack / f"assets/minecraft/textures/entity/player/{model}/steve.png"
        texture.parent.mkdir(parents=True)
        skin.save(texture)
    (pack / "pack.mcmeta").write_text(json.dumps({
        "pack": {"description": "Authored player-head hat regression skin", "pack_format": 84},
    }) + "\n")
    return pack


def check_gait_spider(probe: ProbeSteps, artifact: Path) -> None:
    """The fixed gait camera must show the spider, not the dying zombie's body."""
    from PIL import Image

    sample = json.loads((artifact / "spider-phase-walk.json").read_text())
    image = sample["attempts"][-1]["image"]
    with Image.open(image) as source:
        frame = source.convert("RGB")
    if frame.size != WINDOW:
        raise ValueError("gait spider oracle requires a 1280x720 frame")
    # Matched gait fixture: below the horizon, above chat/HUD, excluding the hand.
    # The zombie's death-tinted skin/trousers are red/purple; the spider is dark
    # with small red eyes. The original obstructed frame contains 5513 such pixels,
    # versus 0..75 across the three retained unobstructed runs.
    pixels = list(frame.crop((540, 390, 800, 500)).getdata())
    obstruction = sum(r > 50 and r > g * 1.4 for r, g, b in pixels)
    spider = sum(max(p) < 65 and max(p) - min(p) < 30 for p in pixels)
    probe.report["spider_capture_pixels"] = {"body": spider, "zombie": obstruction, "image": image}
    probe.check("spider_body_visible", spider > 1000)
    probe.check("spider_unobstructed", obstruction <= 100)


def check_use_pose(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-USE-POSE: third-person use poses are per item, not one generic raise.

    The own avatar is captured in third person (back view) holding a bow, a spyglass and a
    trident, idle and with the use action held. 26.3 gives each item its own pose: the trident
    raise is `-PI` overhead, so it leaves the torso silhouette beside/above the head, and the
    drawn bow and the using spyglass are different poses. The pre-83994480 client raised the
    using hand by one generic `-1.35`, which from behind stays inside the torso, so neither
    assertion can pass on the control. Measured 2026-11-25: the trident's above-shoulder change
    reads 1667 px on the fix and 125 on the control, and the drawn-bow vs using-spyglass frames
    differ by 5179 vs 713.
    """
    names = ("bow-idle", "bow-draw-start", "bow-draw-hold", "spyglass-idle", "spyglass-use-hold",
             "trident-idle", "trident-hold-start", "trident-hold-hold")
    shots = _qa._load_shots(probe, artifact, names)
    ref = shots["bow-idle"][0]
    for name in names[1:]:
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    box = (540, 280, 740, 500)
    halves = {"left": (540, 280, 640, 500), "right": (640, 280, 740, 500)}
    idle = shots["bow-idle"][1]
    drawn = shots["bow-draw-hold"][1]
    spy = shots["spyglass-use-hold"][1]
    # The trident pose raises the arm overhead (`-PI`), which leaves the torso silhouette; the
    # boxes beside and above the head hold only sky while the arm hangs.
    above = {"left": (545, 250, 615, 350), "right": (665, 250, 735, 350)}
    trident_idle = shots["trident-idle"][1]
    trident_held = shots["trident-hold-hold"][1]
    probe.report["use_pose_roi"] = list(box)
    probe.report["use_pose_arm_changed"] = {
        half: _changed(idle, drawn, region) for half, region in halves.items()
    }
    probe.report["use_pose_spyglass_changed"] = {
        half: _changed(shots["spyglass-idle"][1], spy, region) for half, region in halves.items()
    }
    probe.report["use_pose_drawn_vs_spyglass_changed"] = _changed(drawn, spy, box)
    raised = {half: _changed(trident_idle, trident_held, region) for half, region in above.items()}
    probe.report["use_pose_trident_raised_changed"] = raised
    probe.check("trident_use_pose_raised", max(raised.values()) > 800)
    probe.check("use_poses_differ_by_item", probe.report["use_pose_drawn_vs_spyglass_changed"] > 2500)
    probe.report["visual_review_required"] = [
        "Inspect trident-idle / trident-hold-hold for the overhead pose and bow-draw-hold / "
        "spyglass-use-hold for the per-item difference."
    ]


def check_elytra(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-OPUS-15: a worn elytra draws its wings on the third-person avatar's back."""
    shots = _qa._load_shots(probe, artifact, ("back-plain", "back-elytra", "back-none", "back-chestplate"))
    ref = shots["back-plain"][0]
    for name in ("back-elytra", "back-none", "back-chestplate"):
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    # The avatar's upper back at the fixed third-person camera. `back-none` is the same player
    # with an empty chest slot, so only the worn layer differs; the same comparison against
    # `back-plain` bounds the idle arm motion (`idle_back_motion_bounded`, under 2500 px).
    box = (556, 296, 724, 476)
    elytra = _changed(shots["back-elytra"][1], shots["back-none"][1], box)
    idle = _changed(shots["back-plain"][1], shots["back-none"][1], box)
    chestplate = _changed(shots["back-chestplate"][1], shots["back-none"][1], box)
    probe.report["elytra_back_roi"] = list(box)
    probe.report["elytra_changed_pixels"] = {"elytra": elytra, "idle": idle, "chestplate": chestplate}
    probe.check("chestplate_back_roi", chestplate > 4000)
    probe.check("worn_elytra_drawn", elytra > 4000)
    probe.check("idle_back_motion_bounded", idle < 2500)
    probe.report["visual_review_required"] = [
        "Inspect back-plain/back-elytra/back-chestplate; pixel change proves a layer was drawn, not vanilla wing shape."
    ]


def check_placed_head(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-PLACED-HEAD-PROFILE: an unresolved placed head keeps the default skin.

    The same scene as `native-player-head`, judged only on the placed block: the worn
    assertions belong to WORN-PLAYER-HEAD-ON-BACK and must not gate this row's restore."""
    shots = _qa._load_shots(probe, artifact, ("head-empty", "head-block"))
    empty_f, block_f = shots["head-empty"][0], shots["head-block"][0]
    probe.check("placed_head_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(empty_f["camera_position"], block_f["camera_position"])
    ) and abs(empty_f["yaw"] - block_f["yaw"]) < 0.01 and abs(empty_f["pitch"] - block_f["pitch"]) < 0.01)
    head_box = (580, 520, 700, 655)
    placed = _changed(shots["head-block"][1], shots["head-empty"][1], head_box)
    magenta = lambda r, g, b: r > 60 and b > 60 and r > g * 2 and b > g * 2
    hat = _count(shots["head-block"][1], head_box, magenta)
    probe.report["placed_head_roi"] = list(head_box)
    probe.report["placed_head_changed_pixels"] = placed
    probe.report["placed_head_hat_layer_pixels"] = hat
    probe.check("placed_head_renders", placed > 100)
    probe.check("placed_head_hat_layer_pixels", hat > 100)


def check_player_head(probe: ProbeSteps, artifact: Path) -> None:
    """Placed/worn offline heads and the contrasting authored hat layer."""
    shots = _qa._load_shots(probe, artifact, ("head-empty", "head-block", "head-worn", "head-none"))
    empty_f, block_f = shots["head-empty"][0], shots["head-block"][0]
    probe.check("head_front_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(empty_f["camera_position"], block_f["camera_position"])
    ) and abs(empty_f["yaw"] - block_f["yaw"]) < 0.01 and abs(empty_f["pitch"] - block_f["pitch"]) < 0.01)
    worn_f, none_f = shots["head-worn"][0], shots["head-none"][0]
    probe.check("head_back_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(worn_f["camera_position"], none_f["camera_position"])
    ) and abs(worn_f["yaw"] - none_f["yaw"]) < 0.01 and abs(worn_f["pitch"] - none_f["pitch"]) < 0.01)
    # player_head_scene: head at (1000.5,160.25,1004.5), camera
    # (1000.5,161.62,1001.5), yaw/pitch 0, vertical FOV 70, 1280x720.
    # The projected half-block head is x593..687/y534..641 (hat expands it).
    head_box = (580, 520, 700, 655)
    placed = _changed(shots["head-block"][1], shots["head-empty"][1], head_box)
    worn = _changed(shots["head-worn"][1], shots["head-none"][1], (560, 280, 760, 420))
    probe.report["placed_head_changed_pixels"] = placed
    probe.report["worn_head_changed_pixels"] = worn
    magenta = lambda r, g, b: r > 60 and b > 60 and r > g * 2 and b > g * 2
    worn_hat = _count(shots["head-worn"][1], (600, 280, 680, 375), magenta)
    probe.report["worn_head_hat_layer_pixels"] = worn_hat
    hat = _count(shots["head-block"][1], head_box, magenta)
    probe.report["placed_head_hat_layer_pixels"] = hat
    probe.report["placed_head_roi"] = list(head_box)
    # The authored skull's magenta hat is distinct from the avatar's brown
    # torso. Raw frame differences also count idle body/arm motion, so compare
    # hat pixels instead; the bare frame accounts for the held item's edge.
    back_box = (600, 400, 680, 510)
    back_changed = _changed(shots["head-worn"][1], shots["head-none"][1], back_box)
    back_hat = _count(shots["head-worn"][1], back_box, magenta)
    bare_back_hat = _count(shots["head-none"][1], back_box, magenta)
    added_back_hat = max(0, back_hat - bare_back_hat)
    probe.report["worn_head_back_roi"] = list(back_box)
    probe.report["worn_head_back_changed_pixels"] = back_changed
    probe.report["worn_head_back_hat_pixels"] = back_hat
    probe.report["bare_head_back_hat_pixels"] = bare_back_hat
    probe.report["worn_head_added_back_hat_pixels"] = added_back_hat
    probe.check("placed_head_renders", placed > 100)
    probe.check("placed_head_hat_layer_pixels", hat > 100)
    probe.check("worn_head_renders", worn > 500 and worn_hat > 100)
    probe.check("worn_head_back_unchanged", added_back_hat <= 100)


def check_player_head_registry(probe: ProbeSteps, artifact: Path) -> None:
    """PLAYER-HEAD-FAILED-PROFILE-REGISTRY-EXHAUSTION: 261 malformed profiles must not deny a
    newly demanded valid one.

    Both heads are worn by the same avatar in the same fixed third-person frame, demanded after
    the flood, so the only difference between the frames is the profile: `flood-steve` carries an
    offline name (default skin) and `flood-valid` the public Notch texture property. A client whose
    head registry keeps failed keys at its cap refuses the valid key and renders the same default
    skin; the fixed client evicts a failed entry and publishes Notch, changing the avatar's head.
    The promotion receipts measure 51 changed pixels on the pre-fix client (mean colour unchanged:
    the same default skin) against 6765 on the fixed one, so the threshold sits between them.

    The texture fetch is a real network read, so the same run demands a **second** valid key
    (`pre-valid`, the same document re-serialized) before the flood: a broken fetch reds
    `pre_flood_valid_head_renders` instead of leaving the row's red ambiguous.
    """
    shots = _qa._load_shots(probe, artifact, ("empty", "pre-valid", "flood-steve", "flood-valid"))
    ref = shots["empty"][0]
    for name in ("pre-valid", "flood-steve", "flood-valid"):
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    # The avatar's worn head at the fixed third-person camera; the same ROI `check_player_head`
    # samples for `worn_head_renders`.
    head_box = (600, 280, 680, 375)
    pre = _changed(shots["pre-valid"][1], shots["empty"][1], head_box)
    steve = _changed(shots["flood-steve"][1], shots["empty"][1], head_box)
    valid = _changed(shots["flood-valid"][1], shots["empty"][1], head_box)
    profile = _changed(shots["flood-valid"][1], shots["flood-steve"][1], head_box)
    probe.report["flood_head_roi"] = list(head_box)
    probe.report["pre_flood_valid_changed_pixels"] = pre
    probe.report["flood_steve_changed_pixels"] = steve
    probe.report["flood_valid_changed_pixels"] = valid
    probe.report["flood_profile_changed_pixels"] = profile
    probe.check("pre_flood_valid_head_renders", pre > 2000)
    probe.check("flood_steve_head_renders", steve > 100)
    probe.check("flood_valid_head_renders", valid > 100)
    probe.check("valid_head_after_flood", profile > 2000)
    probe.report["visual_review_required"] = [
        "Inspect flood-valid against flood-steve: the pixels prove a different skin was published, "
        "not the exact Notch face."
    ]


def _clock_face_samples(image: Any, box: tuple[int, ...]) -> tuple[int, list, int, int]:
    """Sample the gold-enclosed face, including its pose-independent sky colours."""
    crop = image.crop(box)
    gold = lambda r, g, b: r > b * 1.25 and g > b * 1.15 and g > 20 and r > g * 0.9
    rows = []
    for y in range(crop.height):
        ring = [x for x in range(crop.width) if gold(*crop.getpixel((x, y)))]
        if len(ring) >= 2 and ring[-1] - ring[0] >= 12:
            rows.append((y, ring[0] + 3, ring[-1] - 2))
    pixels = sum(x1 - x0 for _, x0, x1 in rows)
    cells = [[] for _ in range(64)]
    blue_pixels = dark_pixels = 0
    if rows:
        top, bottom = rows[0][0], rows[-1][0] + 1
        # Normalize between rim edges so yaw/bob cannot move the background
        # into the oracle. Time changes the pattern, not its total blue area.
        for y, x0, x1 in rows:
            for x in range(x0, x1):
                r, g, b = crop.getpixel((x, y))
                index = min(7, (y - top) * 8 // (bottom - top)) * 8 + min(7, (x - x0) * 8 // (x1 - x0))
                blue = b > r * 1.35 and b > g * 1.15 and b > 15
                dark = max(r, g, b) < 85 and max(r, g, b) - min(r, g, b) <= max(r, g, b) * 0.25
                cells[index].append(blue)
                blue_pixels += blue
                dark_pixels += dark
    return pixels, [sum(cell) / len(cell) if cell else None for cell in cells], blue_pixels, dark_pixels


def _capture_dropped_clock(probe: ProbeSteps, artifact: Path, name: str) -> str:
    # Item entities spin. Select on face coverage alone, never on animation
    # success: the unchanged day/night oracle must still reject a static clock.
    attempts = []
    selected = None
    for index in range(32):
        if index:
            probe.call("minecraft_wait_ticks", {"ticks": 4})
        sample = f"{name}-attempt-{index + 1}"
        image = _qa.capture_native(probe, artifact, sample)
        _, frame = _qa._load_shots(probe, artifact, (sample,))[sample]
        pixels, grid, _, _ = _clock_face_samples(frame, (300, 260, 460, 420))
        occupied = sum(cell is not None for cell in grid)
        attempts.append({"image": image, "face_pixels": pixels, "sampled_cells": occupied})
        if pixels >= 100 and occupied == 64:
            selected = Path(image)
            break
    (artifact / f"{name}-clock-capture.json").write_text(
        json.dumps({"attempts": attempts, "selected": str(selected) if selected else None}, indent=2) + "\n")
    probe.check(f"{name}_clock_face_captured", selected is not None)
    # Keep the selected bytes and their original same-frame sidecar; do not
    # take another screenshot after the item has advanced to a different yaw.
    target = artifact / f"screenshots/{name}.png"
    shutil.copyfile(selected, target)
    shutil.copyfile(selected.with_suffix(".json"), target.with_suffix(".json"))
    return str(target)


def check_clock_face(probe: ProbeSteps, day: Any, night: Any, name: str, box: tuple[int, ...]) -> None:
    """Require both changed face cells and a changed blue/dark sky proportion."""
    grids = []
    blue_fractions = []
    for phase, image in (("day", day), ("night", night)):
        pixels, grid, blue, dark = _clock_face_samples(image, box)
        probe.report[f"{name}_face_pixels_{phase}"] = pixels
        probe.check(f"{name}_face_{phase}", pixels >= 100)
        grids.append(grid)
        probe.report[f"{name}_face_blue_dark_pixels_{phase}"] = [blue, dark]
        probe.check(f"{name}_face_colours_{phase}", blue + dark >= 20)
        blue_fractions.append(blue / (blue + dark))
    changes = [abs(a - b) for a, b in zip(*grids) if a is not None and b is not None]
    probe.report[f"{name}_face_blue_grid"] = grids
    probe.report[f"{name}_face_changed_cells"] = sum(delta > 0.35 for delta in changes)
    probe.check(f"{name}_face_sampled", len(changes) >= 48)
    # Bob/clipping/yaw can move unchanged blue pixels between normalized cells.
    # Blue versus neutral-dark sky is a material proportion within the same
    # face: projection scales both, while noon/midnight swap those sky colours.
    # Excluding the gold rim/sun also avoids counting a clipped lower bezel.
    probe.report[f"{name}_face_blue_fractions"] = blue_fractions
    probe.check(f"{name}_face_changed", sum(delta > 0.35 for delta in changes) >= 4
                and abs(blue_fractions[0] - blue_fractions[1]) >= 0.35)


def check_dynamic_context(probe: ProbeSteps, artifact: Path) -> None:
    """MODEL-DYNAMIC-CONTEXT: shelf, dropped and held clocks plus two shelf
    compasses at different positions all receive world context."""
    import math
    shots = _qa._load_shots(probe, artifact, ("ctx-day", "ctx-night", "compass-day", "held-day", "held-night"))
    ref = shots["ctx-day"][0]
    for name in ("ctx-night", "compass-day"):
        frame = shots[name][0]
        probe.check(f"{name}_camera_unchanged", all(
            abs(a - b) < 0.01 for a, b in zip(ref["camera_position"], frame["camera_position"])
        ) and abs(ref["yaw"] - frame["yaw"]) < 0.01 and abs(ref["pitch"] - frame["pitch"]) < 0.01)
    probe.check("ctx_received_time_changed",
                shots["ctx-day"][0]["environment"]["time_of_day"] == "6000"
                and shots["ctx-night"][0]["environment"]["time_of_day"] == "18000"
                and shots["compass-day"][0]["environment"]["time_of_day"] == "6000")
    def axis_angle(points):
        cx = sum(p[0] for p in points) / len(points)
        cy = sum(p[1] for p in points) / len(points)
        sxx = sum((x - cx) ** 2 for x, _ in points) / len(points)
        syy = sum((y - cy) ** 2 for _, y in points) / len(points)
        sxy = sum((x - cx) * (y - cy) for x, y in points) / len(points)
        return math.degrees(0.5 * math.atan2(2 * sxy, sxx - syy))

    def axis_diff(a, b):
        d = abs(a - b) % 180.0
        return min(d, 180.0 - d)

    blue = lambda r, g, b: b > r * 1.35 and b > g * 1.15 and b > 15
    day, night = shots["ctx-day"][1], shots["ctx-night"][1]
    for name, box, threshold in (
        ("shelf_clock_left", (385, 480, 410, 505), 20),
        ("shelf_clock_right", (948, 480, 982, 515), 20),
    ):
        m1 = [blue(*p) for p in day.crop(box).getdata()]
        m2 = [blue(*p) for p in night.crop(box).getdata()]
        changed = sum(a != b for a, b in zip(m1, m2))
        probe.report[f"{name}_blue_pixels"] = [sum(m1), sum(m2)]
        probe.report[f"{name}_changed"] = changed
        probe.check(f"{name}_face_changed", changed >= threshold)
    check_clock_face(probe, day, night, "dropped_clock", (300, 260, 460, 420))
    held_day, held_night = shots["held-day"], shots["held-night"]
    probe.check("held_camera_unchanged", all(
        abs(a - b) < 0.01 for a, b in zip(held_day[0]["camera_position"], held_night[0]["camera_position"])
    ) and abs(held_day[0]["yaw"] - held_night[0]["yaw"]) < 0.01
      and abs(held_day[0]["pitch"] - held_night[0]["pitch"]) < 0.01)
    probe.check("held_received_time_changed", held_day[0]["environment"]["time_of_day"] == "6000"
                and held_night[0]["environment"]["time_of_day"] == "18000")
    check_clock_face(probe, held_day[1], held_night[1], "held_clock", (570, 300, 710, 440))
    red = lambda r, g, b: r > g * 1.25 and r > b * 1.25 and r > 50
    comp = shots["compass-day"][1]
    angles = []
    for name, box in (("compass_left", (425, 485, 450, 510)), ("compass_right", (1000, 480, 1025, 505))):
        crop = comp.crop(box)
        w, h = crop.size
        px = [(x, y) for y in range(h) for x in range(w) if red(*crop.getpixel((x, y)))]
        probe.report[f"{name}_needle_pixels"] = len(px)
        probe.check(f"{name}_needle_visible", len(px) >= 50)
        angles.append(axis_angle(px))
    probe.report["compass_needle_axis_angles"] = [round(a, 1) for a in angles]
    probe.check("compass_needle_follows_position", axis_diff(angles[0], angles[1]) > 30.0)
    probe.report["visual_review_required"] = [
        "Inspect ctx-day/ctx-night/compass-day frames; mask predicates do not prove vanilla needle parity."
    ]
