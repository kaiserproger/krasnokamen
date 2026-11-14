"""Subjects part 2: menus, inventory, crafting, items, combat, death, world systems."""
from subjects import D, domain

# Menu oracle text keyed by menu subject; MenuKind/menu_type ids are extracted from
# core:inventory.rs and block->menu mapping from core:tick.rs container_kind_for_state.
MENUS = {
    "player_inventory_2x2": ("MenuKind::Player", "open inventory (E), craft 2x2 planks, move armor/offhand", "window 0; 2x2 result consumes inputs; armor slots accept only matching armor"),
    "crafting_table": ("MenuKind::CraftingTable", "open crafting table, craft shaped recipe, shift-click result", "3x3 grid; result recomputed per click; shift-craft crafts max; grid returned on close"),
    "chest": ("MenuKind::Chest{rows:3} chest", "open chest, deposit/withdraw/quick-move", "27 slots; contents persist; viewers see live updates"),
    "trapped_chest": ("MenuKind::Chest{rows:3} trapped_chest", "open trapped chest wired to lamp", "chest menu; redstone power = viewer count while open"),
    "double_chest": ("MenuKind::Chest{rows:6}", "open double chest from each half", "54 slots, one shared menu, halves consistent"),
    "barrel": ("MenuKind::Chest{rows:3} barrel", "open barrel", "27 slots; open state property toggles"),
    "copper_chest": ("MenuKind::Chest{rows:3} copper chest", "open copper chest", "27 slots like chest; copper golem may sort into it"),
    "ender_chest": ("MenuKind::Chest{rows:3} ender", "open ender chest", "per-player 27-slot storage"),
    "shulker_box": ("MenuKind::ShulkerBox", "open shulker box, try inserting a shulker box", "27 slots; shulker boxes refused inside"),
    "hopper": ("MenuKind::Hopper", "open hopper; insert items", "5 slots; transfers into facing container every 8 ticks"),
    "dispenser": ("MenuKind::Dispenser dispenser", "open dispenser, insert items, power it", "9 slots; dispenses per item behaviour"),
    "dropper": ("MenuKind::Dispenser dropper", "open dropper, insert, power into chest", "9 slots; drops/transfers one item per pulse"),
    "crafter": ("MenuKind::Crafter", "open crafter, toggle disabled slots, power", "3x3 + slot-state; one craft per rising edge"),
    "furnace": ("MenuKind::Furnace(Furnace)", "smelt iron ore with coal", "progress/fuel properties tick; output and XP on take"),
    "smoker": ("MenuKind::Furnace(Smoker)", "cook food", "2x speed, food only"),
    "blast_furnace": ("MenuKind::Furnace(BlastFurnace)", "smelt ore/armor", "2x speed, ores/metal only"),
    "brewing_stand": ("MenuKind::BrewingStand", "brew awkward potion with blaze powder fuel", "fuel 20 brews; 400-tick brew; ingredient consumed"),
    "enchanting_table": ("MenuKind::EnchantingTable", "enchant with lapis + 15 bookshelves, pick offer", "three offers by seed; levels and lapis debited; seed rerolls"),
    "anvil": ("MenuKind::Anvil", "repair/combine/rename item", "cost shown; XP debited; 'Too Expensive' >= 40; anvil may degrade"),
    "grindstone": ("MenuKind::Grindstone", "disenchant / repair-combine", "enchantments removed except curses; XP returned"),
    "smithing_table": ("MenuKind::Smithing", "netherite upgrade and armor trim", "template+base+addition consumed; components kept"),
    "loom": ("MenuKind::Loom", "apply banner pattern", "pattern layer added; dye consumed"),
    "stonecutter": ("MenuKind::Stonecutter", "pick a stone recipe button, shift-take", "recipe list for input; one input per output batch"),
    "cartography_table": ("MenuKind::Cartography", "zoom, clone, lock map", "map variants per input; paper/glass pane consumed"),
    "beacon": ("MenuKind::Beacon", "pay iron ingot and select primary/secondary", "payment consumed; effects by pyramid tier applied to players in range"),
    "merchant_villager": ("MenuKind::Merchant villager", "open villager trade and buy", "offers list; payment slots; uses/restock; villager XP"),
    "merchant_wandering_trader": ("MenuKind::Merchant trader", "open wandering trader and buy", "offers; no levels; trades lock after uses"),
    "chest_boat": ("vehicles/containers.rs chest_boat", "open chest boat while riding/adjacent", "27 slots bound to the entity; break drops contents once"),
    "chest_minecart": ("vehicles/containers.rs chest_minecart", "open chest minecart", "27 slots; contents move with cart"),
    "hopper_minecart": ("vehicles/containers.rs hopper_minecart", "hopper minecart over hopper/chest", "5 slots; pulls items while enabled"),
    "plugin_chest_menu": ("plugin_player.rs MenuKind::Chest{rows:n}", "open basic-economy /economy shop menu", "plugin-owned rows; clicks reported to plugin; no item leak into player inventory"),
    "mount_inventory": ("(none)", "open donkey/llama/horse inventory (sneak-use or E while riding)", "vanilla: horse screen with saddle/armor/chest slots", ),
    "lectern": ("(none)", "put book on lectern, open, take book", "vanilla: lectern menu page turning + take button"),
}
MENU_STATUS = {"mount_inventory": "server_missing", "lectern": "server_missing"}
MENU_SRC = {
    "ender_chest": ["core:ender_chest.rs"], "shulker_box": ["core:shulker.rs"], "crafter": ["core:crafter.rs"],
    "brewing_stand": ["core:brewing.rs"], "enchanting_table": ["core:inventory/enchanting.rs"], "anvil": ["core:inventory/anvil.rs"],
    "grindstone": ["core:inventory/grindstone.rs"], "furnace": ["core:inventory/furnace.rs"], "smoker": ["core:inventory/furnace.rs"],
    "blast_furnace": ["core:inventory/furnace.rs"], "beacon": ["core:beacon.rs"], "merchant_villager": ["core:tick/merchant.rs", "core:villager/trades.rs"],
    "merchant_wandering_trader": ["core:tick/merchant.rs", "core:mob_lanes_trades.rs"], "chest_boat": ["core:vehicles/containers.rs"],
    "chest_minecart": ["core:vehicles/containers.rs"], "hopper_minecart": ["core:vehicles/containers.rs", "core:redstone/containers.rs"],
    "plugin_chest_menu": ["core:plugin_player.rs", "core:plugin_inventories.rs"], "hopper": ["core:redstone/containers.rs"],
    "dispenser": ["core:redstone/dispenser_blocks.rs"], "dropper": ["core:redstone/dispenser_blocks.rs"], "copper_chest": ["core:copper.rs"],
    "cartography_table": ["core:maps/cartography.rs"], "loom": ["core:inventory/stations.rs"], "stonecutter": ["core:inventory/stations.rs"],
    "smithing_table": ["core:inventory/stations.rs"], "crafting_table": ["core:inventory/crafting.rs"],
}
MENU_CAPS = {
    "enchanting_table": ["container_button"], "stonecutter": ["container_button"], "loom": ["container_button"],
    "anvil": ["text_input_screen"], "beacon": ["set_beacon"], "merchant_villager": ["select_trade", "interact_entity"],
    "merchant_wandering_trader": ["select_trade", "interact_entity"], "crafter": ["crafter_slot_toggle"],
    "chest_boat": ["interact_entity"], "chest_minecart": ["interact_entity"], "hopper_minecart": ["interact_entity"],
    "plugin_chest_menu": ["plugin_deploy"], "mount_inventory": ["interact_entity"],
}
domain("containers", ["core:inventory.rs", "core:inventory/click.rs", "core:tick/container_access.rs", "core:tick.rs"],
       ["open_container", "click_pickup_quickmove", "read_world_state"], [
           (k, f"{v[1]} ({v[0]})", v[2], {"menu": True, "custody": True,
                                           **({"status": MENU_STATUS[k]} if k in MENU_STATUS else {}),
                                           **({"src": MENU_SRC[k]} if k in MENU_SRC else {}),
                                           **({"caps": MENU_CAPS[k]} if k in MENU_CAPS else {}),
                                           **({"plugins": ["basic-economy"]} if k == "plugin_chest_menu" else {})})
           for k, v in MENUS.items()])

# ---------------------------------------------------------------- inventory operations (click modes are code-enumerated)
domain("inventory", ["core:inventory/click.rs", "core:boundary.rs", "core:player_reservations.rs"], ["open_container", "read_world_state"], [
    ("click_pickup", "ClickMode::Pickup: left/right click take, half-split, place one, swap different items, merge", "vanilla cursor semantics; totals conserved", {"menu": True, "custody": True, "caps": ["click_pickup_quickmove"]}),
    ("click_quick_move", "ClickMode::QuickMove: shift-click between inventory and container, full target", "moves to first fitting slots; remainder stays; no loss", {"menu": True, "custody": True, "caps": ["click_pickup_quickmove"]}),
    ("click_swap", "ClickMode::Swap: number key 1-9 and F (offhand) over a slot", "slot<->hotbar/offhand swap exactly", {"menu": True, "custody": True, "caps": ["click_advanced_modes"]}),
    ("click_clone", "ClickMode::Clone: middle-click in creative/survival", "creative: full stack copied to cursor; survival: no-op", {"menu": True, "custody": True, "caps": ["click_advanced_modes"]}),
    ("click_throw", "ClickMode::Throw: Q / ctrl+Q over slot, click outside window", "one / whole stack thrown as item entity; debited once", {"menu": True, "custody": True, "caps": ["click_advanced_modes"]}),
    ("click_quick_craft", "ClickMode::QuickCraft: left/right/middle drag across slots", "even split / one each / creative fill; remainder on cursor", {"menu": True, "custody": True, "caps": ["click_advanced_modes"]}),
    ("click_pickup_all", "ClickMode::PickupAll: double-click to gather same items", "collects up to max stack from menu then inventory", {"menu": True, "custody": True, "caps": ["click_advanced_modes"]}),
    ("stale_state_id", "click with an old state_id (raw)", "vanilla 26.1.2 handleContainerClick still applies the click once (menu.clicked), then sends a full-state resync because the state id was stale; item totals conserved, applied once (javap evidence: scenarios/runs/DomItems/facts/ServerGamePacketListenerImpl.javap:5717-5775)", {"menu": True, "custody": True, "caps": ["malformed_packet_injection"]}),
    ("cursor_on_close", "close menu while carrying a stack; full inventory variant", "carried stack returned to inventory; dropped if full; never duplicated", {"menu": True, "custody": True, "caps": ["click_pickup_quickmove"]}),
    ("hotbar_select", "select slots 0-8 and scroll; then drop/use the held stack", "actor: the server tracks the selected slot (SetCarriedItem 0-8) and the next drop/use acts on that slot's stack; observers: receive the actor's main-hand equipment update (SetEquipment) showing the new held item", {"caps": ["select_hotbar"]}),
    ("swap_hands", "press F with items in both hands", "main/offhand swapped", {"caps": ["swap_hands_offhand"], "custody": True}),
    ("drop_item", "Q drop one, ctrl-Q drop stack", "item entity with pickup delay; inventory debited once", {"caps": ["drop_item"], "custody": True}),
    ("pickup_item_entity", "walk over items; full/partial inventory", "picks up up to free space; remainder stays; pickup delay respected", {"src": ["core:item_motion.rs"], "custody": True}),
    ("creative_inventory", "creative tab take/delete/set slots", "SetCreativeModeSlot accepted only in creative; survival refused", {"caps": ["creative_slot_set"], "custody": True}),
    ("armor_equip", "right-click armor/elytra/pumpkin; shift-click into armor slots", "equips to matching slot or swaps; curse of binding blocks removal", {"src": ["core:mob_equipment.rs"], "custody": True}),
    ("recipe_book_place", "click recipe in recipe book (make one / make all)", "ingredients moved from inventory into grid; missing ingredients ghost only", {"caps": ["recipe_book_place"], "menu": True, "custody": True, "src": ["core:inventory/recipe_book.rs"]}),
    ("item_entity_merge", "drop many identical stacks nearby", "item entities merge up to max stack; count conserved", {"src": ["core:item_motion.rs"]}),
    ("xp_orb_pickup", "collect XP orbs with mending gear", "XP added or repairs mending item first; orbs merge", {"src": ["core:experience_orbs.rs"]}),
])

# ---------------------------------------------------------------- crafting / processing
domain("crafting", ["core:inventory/crafting.rs", "core:inventory/craft_stations.rs", "core:inventory/stations.rs"], ["open_container", "click_pickup_quickmove"], [
    ("shaped", "craft pickaxe/stairs (shaped, mirrored)", "result matches recipe incl. mirrored patterns", {"menu": True, "custody": True}),
    ("shapeless", "craft planks/dye/book", "order-independent", {"menu": True, "custody": True}),
    ("craft_max_shift", "shift-click result with many inputs", "crafts until inputs or space run out; exact debits", {"menu": True, "custody": True}),
    ("special_recipes", "firework rocket/star, armor dye, banner duplicate, map extend, book clone, suspicious stew", "special recipe outputs per vanilla", {"menu": True, "custody": True}),
    ("limited_crafting", "doLimitedCrafting true with locked recipe", "unknown recipes not craftable", {"menu": True, "op": 2}),
    ("smelting_fuel_xp", "smelt ores with coal/lava bucket/wood; take output", "burn times per fuel; XP granted on take; bucket returned", {"menu": True, "custody": True, "src": ["core:inventory/furnace.rs"]}),
    ("campfire_passive", "campfire cooking 4 items", "each cooks 600 ticks; drops", {"src": ["core:campfire.rs"]}),
    ("stonecutting", "stonecutter recipes", "1->n outputs per recipe", {"menu": True, "custody": True, "caps": ["container_button"]}),
    ("smithing_netherite", "netherite upgrade of diamond tools/armor", "keeps enchants/name/damage", {"menu": True, "custody": True}),
    ("smithing_trim", "apply armor trim template + material to armor", "minecraft:trim component (pattern+material) added; template/material consumed", {"menu": True, "custody": True, "src": ["core:inventory/click.rs", "crates/mc-data/src/stations.rs"]}),
    ("brewing_chain", "water->awkward->strength->splash->lingering; redstone/glowstone modifiers", "potion components per vanilla brewing tree", {"menu": True, "custody": True, "src": ["core:brewing.rs"]}),
    ("enchanting_offers", "enchant with 0/8/15 bookshelves", "level ranges and seeds per vanilla; lapis/levels consumed", {"menu": True, "custody": True, "caps": ["container_button"], "src": ["core:inventory/enchanting.rs"]}),
    ("anvil_repair_combine_rename", "repair with material, combine books, rename", "cost formula, prior-work penalty, rename cost 1", {"menu": True, "custody": True, "caps": ["text_input_screen"], "src": ["core:inventory/anvil.rs"]}),
    ("grindstone_disenchant", "grindstone enchanted gear", "XP returned; curses stay", {"menu": True, "custody": True, "src": ["core:inventory/grindstone.rs"]}),
    ("loom_patterns", "loom with pattern items", "pattern available only with item where required", {"menu": True, "custody": True, "caps": ["container_button"]}),
    ("cartography_ops", "zoom/clone/lock maps", "scale+1 / copy / locked component", {"menu": True, "custody": True, "src": ["core:maps/cartography.rs"]}),
    ("crafter_automation", "crafter with disabled slots + comparator + pulse", "crafts once per pulse; comparator reads filled slots", {"menu": True, "custody": True, "src": ["core:crafter.rs"], "caps": ["crafter_slot_toggle"]}),
])

# ---------------------------------------------------------------- items / components
domain("items", ["core:consumables.rs", "core:command_items.rs"], ["use_item_air", "read_world_state"], [
    ("eat_food", "eat each food class while hungry (incl. golden apples, chorus, rotten flesh, pufferfish)", "hunger/saturation restored; effects per item; chorus teleports", {"src": ["core:effects.rs", "core:chorus.rs"], "caps": ["release_use_item"]}),
    ("drink_potion_milk_honey", "drink potions, milk bucket, honey bottle, ominous bottle", "effects applied/cleared; bottle/bucket returned", {"src": ["core:effects.rs"], "custody": True}),
    ("throwables", "throw snowball, egg, XP bottle, splash/lingering potion, ender pearl", "projectile spawned; item debited (not creative); impact effect", {"src": ["core:projectiles/weapons.rs"]}),
    ("tool_durability_unbreaking_mending", "use tools to break; unbreaking III; mending with XP", "durability per use with unbreaking chance; mending repairs", {"src": ["core:enchantments.rs"]}),
    ("shield", "raise shield vs melee/arrows; axe hit disables", "frontal damage blocked; 5s disable by axe", {"src": ["core:physical_defense.rs"], "caps": ["release_use_item"]}),
    ("bundle", "insert/extract items in bundle", "weight limit 64; order LIFO", {"menu": True, "custody": True}),
    ("books", "write/sign book and quill; read written book", "pages/title stored; signed book immutable", {"src": ["core:books.rs"], "caps": ["book_edit"]}),
    ("maps", "create map, explore, locate player marker", "map fills where held; markers and scale", {"src": ["core:maps.rs", "core:maps/runtime.rs"]}),
    ("name_tag", "rename tag in anvil, apply to mob", "custom name shown; mob persistent", {"src": ["core:leash.rs"]}),
    ("fishing", "cast, wait bite, reel; early reel", "loot by fishing table; early reel gives nothing; rod durability", {"src": ["core:projectiles/fishing.rs"], "caps": ["release_use_item"]}),
    ("spawn_eggs", "use spawn eggs on ground/water", "mob spawned; egg debited", {"src": ["core:spawn_egg.rs"]}),
    ("item_components_roundtrip", "give items with custom_name, enchantments, dyed_color, damage, lore", "components preserved through inventory/containers/drops/restart", {"src": ["core:command_items.rs"], "caps": ["read_world_state"]}),
    ("item_despawn", "drop item and wait 5 minutes (6000 ticks)", "item entity removed at age 6000", {"src": ["core:item_motion.rs"], "inflight": True}),
    ("item_hazards", "drop items into lava, fire, cactus; explosions", "items destroyed per vanilla (netherite survives fire/lava)", {"src": ["core:item_hazards.rs"]}),
    ("firework_rocket", "launch firework from ground and while gliding", "rocket flies/explodes with stars; boosts elytra", {"src": ["core:projectiles/weapons.rs"]}),
    ("totem_of_undying", "die holding totem of undying", "vanilla: death prevented, health 1, regen/absorption/fire res, totem consumed", {"status": "no_code_hit"}),
    ("spyglass_goat_horn", "use spyglass; blow goat horn", "vanilla: zoom (client) / horn sound + cooldown", {"status": "no_code_hit"}),
    ("recovery_compass_lodestone", "recovery compass after death; lodestone compass", "vanilla: points to last death / lodestone", {"status": "no_code_hit"}),
])
