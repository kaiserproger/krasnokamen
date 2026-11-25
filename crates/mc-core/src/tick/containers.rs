//! Container menus, viewer refresh and furnace ticking.
//! Moved verbatim from `tick.rs`.

use super::*;
use mc_data::block_entity_kinds::BlockEntityKind;

/// Whether `actor`'s open block menu names a cell whose chunk is not resident.
///
/// The cold overlay still answers [`crate::world::ChunkWorld::block_at`] for a
/// non-resident chunk, so the three click inputs that pick a handler from the
/// menu coordinate — `container_click`, `container_slot_state` and
/// `container_button` in `tick/input/inventory.rs` — ask residency first: a
/// stale coordinate must not select a handler that reads or writes the overlay,
/// and a cold source closes the menu (cursor custody intact) exactly as a
/// destroyed container does. `damage_anvil` repeats the residency check where it
/// writes the stored cell.
///
/// Three menu inputs are deliberately not gated here: `place_recipe`'s cell
/// test answers from the cold overlay, so it cannot serve as a residency gate;
/// its slot snapshot comes from resident `block_entities` records, and a cold
/// cell has none, so the menu's slot-count check closes it before any write.
/// `select_bundle_item` mutates only resident `block_entities` records, and
/// `beacon::configure` reads the recorded image and materializes its chunk
/// instead of writing the overlay.
pub(super) fn menu_source_cold(state: &TickState, actor: EntityId) -> bool {
    state
        .open_containers
        .menu_at(&actor)
        .is_some_and(|&(x, _, z)| !state.world.has_chunk(x.div_euclid(16), z.div_euclid(16)))
}

/// Whether the cell a menu is anchored on has left residency.
fn anchor_chunk_cold(state: &TickState, open: &(i32, i32, i32)) -> bool {
    !state
        .world
        .has_chunk(open.0.div_euclid(16), open.2.div_euclid(16))
}

fn animated_block(state: &TickState, pos: BlockPos) -> Option<(BlockEntityKind, u32, u32)> {
    let id = state.world.block_at(pos.x, pos.y, pos.z)?;
    let block = state.registry.blocks.by_id(mc_world::BlockStateId(id))?;
    let kind = BlockEntityKind::for_block(block.block.id.path())?;
    if !matches!(
        kind,
        BlockEntityKind::Chest | BlockEntityKind::TrappedChest | BlockEntityKind::ShulkerBox
    ) {
        return None;
    }
    Some((kind, u32::try_from(block.block.raw_id).ok()?, id))
}

pub(super) fn expire_native_openers(state: &mut TickState) {
    let now = state.tick_control.game_ticks(state.tick);
    let mut expired = std::mem::take(&mut state.expired_container_openers);
    let entities = &state.entities;
    let world = &state.world;
    let blocks = &state.registry.blocks;
    state
        .open_containers
        .retain_native(|entity, (x, y, z), until| {
            let valid = now < until
                && entities
                    .slot_of(entity)
                    .is_some_and(|slot| entities.body(slot).alive)
                && world
                    .block_at(x, y, z)
                    .and_then(|id| blocks.by_id(mc_world::BlockStateId(id)))
                    .is_some_and(|block| {
                        matches!(
                            BlockEntityKind::for_block(block.block.id.path()),
                            Some(BlockEntityKind::Chest | BlockEntityKind::TrappedChest)
                        )
                    });
            if !valid {
                expired.push(entity);
            }
            valid
        });
    for entity in expired.drain(..) {
        crate::copper_golem::finish_container_interaction(state, entity);
    }
    state.expired_container_openers = expired;
}

/// Seal opener transitions after destruction/menus and before outbox publication.
pub(super) fn animate_openers(state: &mut TickState) {
    let mut dirty = state.open_containers.take_animation_dirty();
    if dirty.is_empty() {
        state.open_containers.recycle_animation_dirty(dirty);
        return;
    }
    let changed = dirty.len();
    for index in 0..changed {
        let (x, y, z) = dirty[index];
        let pos = BlockPos { x, y, z };
        if matches!(
            animated_block(state, pos),
            Some((BlockEntityKind::ShulkerBox, _, _))
        ) {
            continue;
        }
        if let Some((_, partner)) =
            crate::chests::storage_partner(&state.registry.blocks, pos, &|p| {
                state.world.block_at(p.x, p.y, p.z)
            })
        {
            dirty.push((partner.x, partner.y, partner.z));
        }
        // Removal or a shape change may leave the previous half unpaired.
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            let (Some(x), Some(z)) = (x.checked_add(dx), z.checked_add(dz)) else {
                continue;
            };
            let neighbour = BlockPos { x, y, z };
            if matches!(
                animated_block(state, neighbour),
                Some((BlockEntityKind::Chest | BlockEntityKind::TrappedChest, _, _))
            ) {
                dirty.push((x, y, z));
            }
        }
    }
    dirty.sort_unstable();
    dirty.dedup();
    let mut recipients = std::mem::take(&mut state.container_animation_recipients);
    for &(x, y, z) in &dirty {
        let pos = BlockPos { x, y, z };
        let Some((_, block, block_state)) = animated_block(state, pos) else {
            continue;
        };
        let partner = crate::chests::storage_partner(&state.registry.blocks, pos, &|p| {
            state.world.block_at(p.x, p.y, p.z)
        })
        .map(|(_, p)| (p.x, p.y, p.z));
        let parameter = u8::from(state.open_containers.count_at((x, y, z), partner) != 0);
        recipients.clear();
        recipients.extend(state.players.iter().filter_map(|(entity, player)| {
            let slot = state.entities.slot_of(*entity)?;
            let p = state.entities.pos(slot);
            let distance = (p[0] - f64::from(x)).powi(2)
                + (p[1] - f64::from(y)).powi(2)
                + (p[2] - f64::from(z)).powi(2);
            (distance < 64.0 * 64.0).then_some(player.connection)
        }));
        for &connection in &recipients {
            state.push_player(
                connection,
                PlayerChange::BlockEvent {
                    position: pos,
                    block,
                    state: block_state,
                    action: 1,
                    parameter,
                },
            );
        }
    }
    recipients.clear();
    state.container_animation_recipients = recipients;
    state.open_containers.recycle_animation_dirty(dirty);
}

/// Container menu for a block; both halves share one double-chest menu.
pub(super) fn container_menu_for(
    state: &TickState,
    x: i32,
    y: i32,
    z: i32,
) -> Option<(crate::inventory::MenuKind, String, usize)> {
    let current = state.world.block_at(x, y, z)?;
    let menu = container_kind_for_state(state, current)?;
    if container_cells(state, (x, y, z)).len() == 2 {
        return Some((
            crate::inventory::MenuKind::Chest { rows: 6 },
            "Large Chest".to_owned(),
            54,
        ));
    }
    Some(menu)
}

/// Storage cells behind the container at `pos`, in menu order: one, or the
/// two halves of a double chest.
pub(super) fn container_cells(state: &TickState, pos: (i32, i32, i32)) -> Vec<(i32, i32, i32)> {
    let at = BlockPos {
        x: pos.0,
        y: pos.1,
        z: pos.2,
    };
    crate::chests::storage_cells(&state.registry.blocks, at, &|cell: BlockPos| {
        state.world.block_at(cell.x, cell.y, cell.z)
    })
    .into_iter()
    .map(|cell| (cell.x, cell.y, cell.z))
    .collect()
}

/// The contents of `cells` back to back (menu order).
pub(super) fn container_slots(
    state: &TickState,
    cells: &[(i32, i32, i32)],
) -> Vec<mc_data::ItemStack> {
    cells
        .iter()
        .filter_map(|cell| state.block_entities.get(cell))
        .flat_map(|record| record.slots.iter().cloned())
        .collect()
}

/// Snapshot the actual backing storage of the issuing player's current menu.
pub(crate) fn debug_menu_snapshot(
    state: &TickState,
    entity: EntityId,
) -> Result<serde_json::Value, &'static str> {
    let player = state.players.get(&entity).ok_or("player not found")?;
    let menu = player.open.as_ref().unwrap_or(&player.inventory_menu);
    use std::borrow::Cow;
    let container: Cow<'_, [mc_data::ItemStack]> = if menu.kind.container_size() == 0 {
        Cow::Borrowed(&[])
    } else if let Some((_, icons)) = state
        .settlement_menus
        .get(&player.connection)
        .map(|open| open.snapshot())
        .filter(|(window, _)| *window == menu.window)
    {
        Cow::Borrowed(icons)
    } else if crate::vehicles::is_open(state, entity) {
        Cow::Borrowed(
            crate::vehicles::container_slots_for_player(state, entity)
                .ok_or("vehicle menu storage is unavailable")?,
        )
    } else if let crate::inventory::MenuKind::Mount { entity: mount, .. } = menu.kind {
        let slot = state
            .entities
            .slot_of(mount)
            .ok_or("mount is unavailable")?;
        if crate::mounts::menus::kind(state, slot) != Some(menu.kind) {
            return Err("mount menu is stale");
        }
        Cow::Owned(crate::mounts::menus::slots(state, slot))
    } else if crate::ender_chest::is_open(state, entity) {
        Cow::Owned(
            crate::ender_chest::contents(&player.opaque_player_fields, &state.registry.items)
                .map_err(|_| "ender chest storage is invalid")?,
        )
    } else if let Some(position) = state.open_containers.menu_at(&entity) {
        let cells = container_cells(state, *position);
        let mut slots = Vec::with_capacity(menu.kind.container_size());
        for cell in cells {
            let record = state
                .block_entities
                .get(&cell)
                .ok_or("container storage is unavailable")?;
            slots.extend(record.slots.iter().cloned());
        }
        Cow::Owned(slots)
    } else {
        return Err("menu backing storage is unavailable");
    };
    if container.len() < menu.kind.container_size() {
        return Err("menu backing storage is incomplete");
    }
    let stack = |item: &mc_data::ItemStack| -> Result<serde_json::Value, &'static str> {
        if item.is_empty() {
            return Ok(serde_json::json!({"item": null, "count": 0, "components": null}));
        }
        let name = state
            .registry
            .items
            .name_of(item.item_id)
            .ok_or("unknown item in menu")?;
        Ok(serde_json::json!({"item": name.as_str(), "count": item.count, "components": item}))
    };
    let slots = (0..menu.kind.slot_count())
        .map(|index| {
            use crate::inventory::SlotRef;
            let item = match menu.kind.slot(index) {
                Some(SlotRef::Result) => &menu.result,
                Some(SlotRef::Grid(i)) => &menu.grid[i],
                Some(SlotRef::Inventory(i)) => &player.inventory.slots[i],
                Some(SlotRef::Container(i)) => &container[i],
                None => &mc_data::ItemStack::EMPTY,
            };
            stack(item)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(serde_json::json!({
        "container_id": menu.window,
        "carried": stack(&menu.carried)?,
        "slots": slots,
    }))
}

/// Resync the real current menu after refusing a native inventory transition.
/// A vehicle or block menu must never be imaged with placeholder empty cargo.
/// Every caller is a live player's own input path (pick item, swap hands,
/// creative slot, drop, book edit, interact), so `players[&entity]` is an
/// invariant here rather than a checked precondition.
pub(crate) fn inventory_transition_refused(
    state: &mut TickState,
    entity: EntityId,
) -> crate::player::PlayerEffects {
    if let Some(slots) = crate::vehicles::container_slots_for_player(state, entity) {
        return state.players[&entity].inventory_transition_refused(slots);
    }
    if crate::ender_chest::is_open(state, entity) {
        let slots = crate::ender_chest::contents(
            &state.players[&entity].opaque_player_fields,
            &state.registry.items,
        );
        return slots
            .map(|slots| state.players[&entity].inventory_transition_refused(&slots))
            .unwrap_or_default();
    }
    // A block menu is only resynced when every slot it names is readable: its
    // retained shape must match the reconstructed storage and every block must
    // be resident (the same check the click and recipe paths make). A cold
    // half, whichever one it is, would otherwise be imaged as empty slots.
    // Menus with no `open_containers` record (Mount, plugin, settlement) have no
    // cells, so a non-zero retained size fails the shape check and they close
    // fail-closed here instead of being imaged with an empty placeholder.
    let open_cells = state
        .open_containers
        .menu_at(&entity)
        .map(|open| container_cells(state, *open))
        .unwrap_or_default();
    let container = container_slots(state, &open_cells);
    let cold = state
        .players
        .get(&entity)
        .and_then(|player| player.open.as_ref())
        .is_some_and(|menu| menu.kind.container_size() != container.len())
        || open_cells
            .iter()
            .any(|&(x, y, z)| state.world.block_at(x, y, z).is_none());
    if cold {
        let connection = state.players[&entity].connection;
        super::players::close_player_menu(state, connection);
        // Window-0 resync after the close: the client's predicted stack, cursor
        // or book edit rolls back against the authoritative inventory.
        return state.players[&entity].inventory_transition_refused(&[]);
    }
    state.players[&entity].inventory_transition_refused(&container)
}

/// Publish the current authoritative menu image to every viewer after external
/// native custody mutation, with one fresh revision per affected menu.
pub(crate) fn refresh_container_viewers(state: &mut TickState, position: BlockPos) {
    let cells = container_cells(state, (position.x, position.y, position.z));
    // A viewer anchored on a half that is now cold is stale by definition: a
    // cold half resolves to itself alone, so a refresh of the resident partner
    // never matches it, and the menu would keep the old image until the first
    // click closed it through the residency check (a refresh of another
    // container could never serve it either). Close it on any custody refresh.
    let viewers: Vec<_> = state
        .open_containers
        .menu_viewers()
        .filter(|(_, open)| {
            anchor_chunk_cold(state, *open) || container_cells(state, **open) == cells
        })
        .filter_map(|(viewer, _)| {
            state
                .connections
                .entity_connection(*viewer)
                .map(|connection| (*viewer, connection))
        })
        .collect();
    if viewers.is_empty() {
        return;
    }
    let contents = container_slots(state, &cells);
    // The same custody check the click, recipe and refusal paths make: a menu
    // is only republished when every slot it names is readable. A cold half
    // would otherwise be imaged as its real slots plus empty ones (or from
    // overlay records of the cold half), telling the viewer a lie about
    // custody; the viewer's menu closes instead, with a window-0 resync so
    // predicted stack and cursor edits roll back.
    let cold = cells
        .iter()
        .any(|&(x, y, z)| state.world.block_at(x, y, z).is_none());
    for (viewer, connection) in viewers {
        let anchor_cold = state
            .open_containers
            .menu_at(&viewer)
            .is_some_and(|open| anchor_chunk_cold(state, open));
        let shape_mismatch = state
            .players
            .get(&viewer)
            .and_then(|player| player.open.as_ref())
            .is_some_and(|menu| menu.kind.container_size() != contents.len());
        if anchor_cold || cold || shape_mismatch {
            super::players::close_player_menu(state, connection);
            let changes = state
                .players
                .get(&viewer)
                .map(|player| player.inventory_transition_refused(&[]).changes)
                .unwrap_or_default();
            for change in changes {
                state.push_player(connection, change);
            }
            continue;
        }
        let Some(player) = state.players.get_mut(&viewer) else {
            continue;
        };
        if let Some(menu) = player.open.as_mut() {
            menu.state_id = (menu.state_id + 1) & 0x7fff;
        }
        for change in player.sync_container_view(&contents) {
            state.push_player(connection, change);
        }
    }
}

/// Write menu-order `slots` back over `cells`; a half whose contents
/// changed is marked dirty.
pub(super) fn store_container_slots(
    state: &mut TickState,
    cells: &[(i32, i32, i32)],
    slots: &[mc_data::ItemStack],
) {
    let mut rest = slots;
    for cell in cells {
        let Some(record) = state.block_entities.get_mut(cell) else {
            continue;
        };
        let (mine, tail) = rest.split_at(record.slots.len().min(rest.len()));
        rest = tail;
        if record.slots != mine {
            record.slots.clear();
            record.slots.extend_from_slice(mine);
            crate::brewing::slots_replaced(&mut state.brewing, *cell, &record.slots);
            let position = BlockPos {
                x: cell.0,
                y: cell.1,
                z: cell.2,
            };
            crate::redstone::containers::inventory_changed(
                &mut state.redstone_containers,
                position,
            );
            state.world.mark_block_entity_dirty(position);
        } else {
            for (target, source) in record.slots.iter_mut().zip(mine) {
                crate::inventory::bundle::copy_selection(target, source);
            }
        }
    }
}

/// Container menu for a block state id.
pub(super) fn container_kind_for_state(
    state: &TickState,
    state_id: u32,
) -> Option<(crate::inventory::MenuKind, String, usize)> {
    let block_state = state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(state_id))?;
    let id = block_state.block.id.as_str();
    if let Some(menu) = stations::craft_menu_for_block(id) {
        return Some(menu);
    }
    if crate::copper::chest(block_state.block.id.path()) {
        return Some((
            crate::inventory::MenuKind::Chest { rows: 3 },
            "Copper Chest".into(),
            27,
        ));
    }
    if crate::shulker::is_block(block_state.block.id.path()) {
        return Some((
            crate::inventory::MenuKind::ShulkerBox,
            "Shulker Box".into(),
            27,
        ));
    }
    match id {
        "minecraft:chest" | "minecraft:trapped_chest" => Some((
            crate::inventory::MenuKind::Chest { rows: 3 },
            "Chest".to_owned(),
            27,
        )),
        "minecraft:hopper" => Some((crate::inventory::MenuKind::Hopper, "Hopper".into(), 5)),
        "minecraft:crafter" => Some((crate::inventory::MenuKind::Crafter, "Crafter".into(), 9)),
        "minecraft:brewing_stand" => Some((
            crate::inventory::MenuKind::BrewingStand,
            "Brewing Stand".into(),
            5,
        )),
        "minecraft:dispenser" | "minecraft:dropper" => Some((
            crate::inventory::MenuKind::Dispenser,
            if id == "minecraft:dropper" {
                "Dropper"
            } else {
                "Dispenser"
            }
            .into(),
            9,
        )),
        "minecraft:barrel" => Some((
            crate::inventory::MenuKind::Chest { rows: 3 },
            "Barrel".to_owned(),
            27,
        )),
        "minecraft:crafting_table" => Some((
            crate::inventory::MenuKind::CraftingTable,
            "Crafting Table".to_owned(),
            // Grid/result custody belongs to this player's Menu, not the block.
            0,
        )),
        "minecraft:beacon" => Some((crate::inventory::MenuKind::Beacon, "Beacon".into(), 0)),
        "minecraft:lectern" => Some((crate::inventory::MenuKind::Lectern, "Lectern".into(), 1)),
        "minecraft:enchanting_table" => Some((
            crate::inventory::MenuKind::EnchantingTable,
            "Enchant".into(),
            0,
        )),
        "minecraft:anvil" | "minecraft:chipped_anvil" | "minecraft:damaged_anvil" => {
            Some((crate::inventory::MenuKind::Anvil, "Repair & Name".into(), 0))
        }
        "minecraft:grindstone" => Some((
            crate::inventory::MenuKind::Grindstone,
            "Repair & Disenchant".into(),
            0,
        )),
        "minecraft:furnace" => Some((
            crate::inventory::MenuKind::Furnace(mc_data::recipes::CookingKind::Furnace),
            "Furnace".to_owned(),
            3,
        )),
        "minecraft:smoker" => Some((
            crate::inventory::MenuKind::Furnace(mc_data::recipes::CookingKind::Smoker),
            "Smoker".to_owned(),
            3,
        )),
        "minecraft:blast_furnace" => Some((
            crate::inventory::MenuKind::Furnace(mc_data::recipes::CookingKind::BlastFurnace),
            "Blast Furnace".to_owned(),
            3,
        )),
        _ => None,
    }
}

/// Tick every furnace block entity: cooking advances, slots mutate in
/// place. `lit` block property follows the burn state when resolvable.
pub(crate) fn tick_furnaces(
    state: &mut TickState,
    block_writes: &mut Vec<(BlockPos, u32)>,
    catch_up: Option<(ChunkPos, u64)>,
) {
    // Independent borrow of the registry (Arc): the storage pass below needs
    // `&mut state`.
    let registry = std::sync::Arc::clone(&state.registry);
    let rules = registry.item_rules();
    // Collect positions first: the tick mutates storage per entity.
    let positions: Vec<(i32, i32, i32)> = state.block_entities.keys().copied().collect();
    for pos in positions {
        let chunk = ChunkPos {
            x: pos.0.div_euclid(16),
            z: pos.2.div_euclid(16),
        };
        if catch_up.is_some_and(|(target, _)| target != chunk)
            || (catch_up.is_none() && state.world.simulation_debt(chunk) > 0)
        {
            continue;
        }
        if !state
            .world
            .is_block_active(pos.0.div_euclid(16), pos.2.div_euclid(16))
        {
            continue;
        }
        let Some(current) = state.world.block_at(pos.0, pos.1, pos.2) else {
            continue;
        };
        let kind = match furnace_kind_for_state(&registry, current) {
            Some(kind) => kind,
            None => continue,
        };
        if catch_up.is_some() {
            let expect = Footprint {
                block_expect: vec![(
                    BlockPos {
                        x: pos.0,
                        y: pos.1,
                        z: pos.2,
                    },
                    current,
                )],
                ..Default::default()
            };
            if !state.footprint_valid(&expect) {
                continue;
            }
        }
        let position = BlockPos {
            x: pos.0,
            y: pos.1,
            z: pos.2,
        };
        let candidate = if crate::plugin_inventories::has_reservations_for_block(state, position) {
            let Some(record) = state.block_entities.get(&pos) else {
                continue;
            };
            let mut slots = record.slots.clone();
            if slots.len() < 3 {
                slots.resize(3, mc_data::ItemStack::EMPTY);
            }
            let mut progress = record.furnace.clone().unwrap_or_default();
            let before = progress.properties();
            let outcome = match catch_up {
                Some((_, elapsed)) => progress.catch_up(&rules, kind, &mut slots, elapsed),
                None => progress.tick(&rules, kind, &mut slots),
            };
            let after = progress.properties();
            if crate::plugin_inventories::below_block_floors(state, position, &slots) {
                continue;
            }
            Some((slots, progress, before, outcome, after))
        } else {
            None
        };
        let record = if catch_up.is_some() {
            state.block_entities.get_mut_catch_up(&pos)
        } else {
            state.block_entities.get_mut(&pos)
        };
        let Some(record) = record else {
            continue;
        };
        let (before, outcome, after) = if let Some((slots, progress, before, outcome, after)) =
            candidate
        {
            record.slots = slots;
            record.furnace = Some(progress);
            (before, outcome, after)
        } else {
            if record.slots.len() < 3 {
                record.slots.resize(3, mc_data::ItemStack::EMPTY);
            }
            let progress = record.furnace.get_or_insert_with(Default::default);
            let before = progress.properties();
            let outcome = match catch_up {
                Some((_, elapsed)) => progress.catch_up(&rules, kind, &mut record.slots, elapsed),
                None => progress.tick(&rules, kind, &mut record.slots),
            };
            (before, outcome, progress.properties())
        };
        if !outcome.changed && before == after {
            continue;
        }
        for (viewer, open) in state.open_containers.menu_viewers() {
            if *open != pos {
                continue;
            }
            let Some(player) = state.players.get_mut(viewer) else {
                continue;
            };
            let Some(menu) = player.open.as_mut() else {
                continue;
            };
            let connection = player.connection;
            let window = menu.window;
            let queue = state.outbox.entry(connection).or_default();
            if outcome.changed {
                menu.state_id = (menu.state_id + 1) & 0x7FFF;
                queue.extend(player.sync_container_view(&record.slots));
            }
            for (property, (old, value)) in before.into_iter().zip(after).enumerate() {
                if old != value {
                    queue.push(PlayerChange::MenuProperty {
                        window,
                        property: property as i16,
                        value: value as i16,
                    });
                }
            }
        }
        if outcome.changed {
            crate::redstone::containers::inventory_changed(
                &mut state.redstone_containers,
                position,
            );
        }
        if let Some(lit) = outcome.lit {
            set_lit_prop(state, pos, lit, block_writes, catch_up.is_some());
        }
    }
}

/// Cooking kind for a furnace-family block state.
pub(super) fn furnace_kind_for_state(
    registry: &CoreRegistry,
    state_id: u32,
) -> Option<mc_data::recipes::CookingKind> {
    let block_state = registry.blocks.by_id(mc_world::BlockStateId(state_id))?;
    match block_state.block.id.as_str() {
        "minecraft:furnace" => Some(mc_data::recipes::CookingKind::Furnace),
        "minecraft:smoker" => Some(mc_data::recipes::CookingKind::Smoker),
        "minecraft:blast_furnace" => Some(mc_data::recipes::CookingKind::BlastFurnace),
        _ => None,
    }
}

/// Set the `lit` property on the block at `pos` (no-op when unresolvable).
pub(super) fn set_lit_prop(
    state: &mut TickState,
    pos: (i32, i32, i32),
    lit: bool,
    block_writes: &mut Vec<(BlockPos, u32)>,
    catch_up: bool,
) {
    let Some(current) = state.world.block_at(pos.0, pos.1, pos.2) else {
        return;
    };
    let Some(block_state) = state.registry.blocks.by_id(mc_world::BlockStateId(current)) else {
        return;
    };
    let mut properties = block_state.properties.clone();
    let Some((_, value)) = properties.iter_mut().find(|(key, _)| key == "lit") else {
        return;
    };
    *value = if lit { "true" } else { "false" }.to_owned();
    if let Some(next) = state
        .registry
        .blocks
        .by_name_and_props(&block_state.block.id, &properties)
    {
        if catch_up {
            state.world.set_block_catch_up(pos.0, pos.1, pos.2, next.0);
        } else {
            state.world.set_block(pos.0, pos.1, pos.2, next.0);
        }
        block_writes.push((
            BlockPos {
                x: pos.0,
                y: pos.1,
                z: pos.2,
            },
            next.0,
        ));
    }
}
