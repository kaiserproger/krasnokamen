"""Buildings-qa constants: the selectable blueprints, their levels, menu slots,
fixture geometry and the verdict labels the dialogs print.
"""

from __future__ import annotations



USERNAME = "BuildQA"


SEED = 100


VILLAGE_CENTER = (120, 70, 7)


DIALOG_CLASS = "net.minecraft.client.gui.screens.dialog.MultiButtonDialogScreen"


CONTAINER_CLASS = "net.minecraft.client.gui.screens.inventory.ContainerScreen"


# Civil Projects menu (ui_menus.rs): the Status icon, and the active project's icon on the Construction page.
PROJECT_STATUS_SLOT, PROJECT_ICON_SLOT = 52, 4


# The Construction page's status line (slot 10) says whether the project is active or queued.
PROJECT_STATUS_LINE_SLOT = 10


# The Construction page's map icon (slot 14) carries the project origin as its lore.
PROJECT_MAP_SLOT = 14


# House L1 at origin + [0,1,0] holds `oak_log[axis=y]` (a corner post) for every rotation; a
# survival player breaks it mid-build (house.broken_cell_replaced).
BROKEN_AUTHORED = "minecraft:oak_log"


BLUEPRINTS = ("house", "storage", "farm", "mason", "lumber_camp", "mine",
              "barracks", "wall", "watch_tower", "gate", "armorer", "butcher",
              "cartographer", "cleric", "fisherman", "fletcher", "leatherworker",
              "librarian", "toolsmith", "weaponsmith")


LEVELS = {"house": 3, "storage": 2, "farm": 2, "mason": 2, "lumber_camp": 2,
          "mine": 2, "barracks": 3, "wall": 3, "watch_tower": 3, "gate": 3,
          "armorer": 2, "butcher": 2, "cartographer": 2, "cleric": 2,
          "fisherman": 2, "fletcher": 2, "leatherworker": 2, "librarian": 2,
          "toolsmith": 2, "weaponsmith": 2}


# The settlement's own perimeter wall, gates and well: a select of its own, not a blueprint (perimeter_qa.py).
PERIMETER = "perimeter"


# Only server/client-observed product outcomes can classify an unrecorded
# building-level verdict as a negative control. Setup and races fail closed.
NEGATIVE_CONTROL_STEPS = frozenset({
    "order_available", "upgrade_available", "project", "built", "blocks",
})


FILL_LIMIT = 32768


def _chunked_fills(x0: int, y0: int, z0: int, x1: int, y1: int, z1: int, block: str) -> list[str]:
    """Fill commands covering the box, split along z so each fits FILL_LIMIT.
    Unsliced fills over the limit are silently rejected, leaving terrain."""
    rows = max(1, FILL_LIMIT // ((x1 - x0 + 1) * (y1 - y0 + 1)))
    return [f"fill {x0} {y0} {z} {x1} {y1} {min(z + rows - 1, z1)} {block}"
            for z in range(z0, z1 + 1, rows)]


# Storage L1 base needs (must arrive via worker production, no /give).
STORAGE_L1_NEEDS = {
    "minecraft:cobblestone": 50,
    "minecraft:oak_log": 41,
    "minecraft:coal": 1,
}


# Dialog kind labels (ui.rs kind_label) + profession per blueprint.
KIND_LABEL = {"house": "House", "storage": "Storage", "farm": "Farmer workshop",
              "mason": "Builder workshop", "lumber_camp": "Lumber camp",
              "mine": "Mine", "barracks": "Barracks", "wall": "Wall section",
              "watch_tower": "Tower"}


# ui.rs blueprint_label (English): the `<label> L<level>` project icon.
PROJECT_LABEL = {"house": "House", "storage": "Storage", "farm": "Farm",
                 "mason": "Mason's workshop", "lumber_camp": "Lumber camp", "mine": "Mine",
                 "barracks": "Barracks", "wall": "Wall", "watch_tower": "Watchtower",
                 "gate": "Gate",
                 **{kind: f"{kind.capitalize()}'s workshop" for kind in
                    ("armorer", "butcher", "cartographer", "cleric", "fisherman", "fletcher",
                     "leatherworker", "librarian", "toolsmith", "weaponsmith")}}


KIND_PROFESSION = {"farm": "Farmer", "mason": "Builder",
                    "lumber_camp": "Lumberjack", "mine": "Miner"}


# Parallel layout: one founded settlement per blueprint, centers 400 apart on
# a grid (radius 64 each: 64+64=128 < 400, no overlap, no village dependency).
# Each site gets: founded settlement (owner=BuildQA) -> hand shell -> 6
# villagers -> order -> fund -> Builder builds. All sites run concurrently:
# the planner ticks one settlement per 20 ticks, builders work in parallel.
PARALLEL_ORIGIN = (0, 70, 0)


PARALLEL_STEP = 400


PARALLEL_RADIUS = 64


PARALLEL_SITES = {
    "house": (0, 0), "storage": (1, 0), "farm": (2, 0),
    "mason": (0, 1), "lumber_camp": (1, 1), "mine": (2, 1),
    "barracks": (0, 2), "wall": (1, 2), "watch_tower": (2, 2),
    "gate": (0, 3), "armorer": (1, 3), "butcher": (2, 3), "cartographer": (0, 4),
    "cleric": (1, 4), "fisherman": (2, 4), "fletcher": (0, 5), "leatherworker": (1, 5),
    "librarian": (2, 5), "toolsmith": (0, 6), "weaponsmith": (1, 6),
}


# Answers that end the question: another identical order cannot change them.
_ORDER_TERMINAL = ("No blueprint upgrade available", "Unknown blueprint.", "Busy.",
                   "not enough:", "Blueprint has no first level.")
