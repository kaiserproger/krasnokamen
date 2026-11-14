//! Inventory slot replacement for operator commands.
use crate::boundary::{BlockPos, ConnectionId, EntityId, PlayerChange};
use crate::commands::{CommandError, Coord, Target};
use crate::tick::TickState;
use mc_data::ItemStack;
use mc_nbt::Tag;

#[derive(Debug, Clone, PartialEq)]
pub struct ItemCommand {
    destination: Destination,
    source: Source,
}
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Destination {
    Entity(Target, String),
    Block([Coord; 3], String),
}
#[derive(Debug, Clone, PartialEq)]
enum Source {
    With(crate::command_items::ItemArgument, i32),
    From(Destination),
    Unsupported,
}
pub(crate) fn destination(args: &[&str]) -> Result<(Destination, usize), CommandError> {
    match args {
        ["entity", target, slot, ..] => Ok((
            Destination::Entity(crate::commands::target_of(target)?, (*slot).into()),
            3,
        )),
        ["block", x, y, z, slot, ..] => Ok((
            Destination::Block(
                crate::commands::coords(&[x, y, z])
                    .ok_or(CommandError::Usage("item block <pos> <slot>"))?,
                (*slot).into(),
            ),
            5,
        )),
        _ => Err(CommandError::Usage("item <destination>")),
    }
}
pub(crate) fn parse(args: &[&str]) -> Result<ItemCommand, CommandError> {
    let [action, rest @ ..] = args else {
        return Err(CommandError::Usage("item replace ..."));
    };
    if *action == "modify" {
        return Ok(ItemCommand {
            destination: Destination::Entity(Target::Executor, String::new()),
            source: Source::Unsupported,
        });
    }
    if *action != "replace" {
        return Err(CommandError::Usage("item replace ..."));
    }
    let (target, used) = destination(rest)?;
    let source = match &rest[used..] {
        ["with", item] => Source::With(crate::command_items::ItemArgument::parse(item)?, 1),
        ["with", item, count] => {
            let count = count
                .parse::<i32>()
                .ok()
                .filter(|n| (1..=99).contains(n))
                .ok_or_else(|| CommandError::Message("Invalid item count".into()))?;
            Source::With(crate::command_items::ItemArgument::parse(item)?, count)
        }
        ["from", rest @ ..] => {
            let (source, used) = destination(rest)?;
            if used != rest.len() {
                return Err(CommandError::Message(
                    "Item modifiers are not supported".into(),
                ));
            }
            Source::From(source)
        }
        _ => return Err(CommandError::Usage("item replace ...")),
    };
    Ok(ItemCommand {
        destination: target,
        source,
    })
}
pub(crate) fn position(
    state: &TickState,
    executor: EntityId,
    coords: [Coord; 3],
) -> Result<BlockPos, String> {
    let (origin, rotation) =
        crate::command_console::pose(state, executor).ok_or("No command position")?;
    let pos = crate::commands::resolve_coords(coords, origin, rotation);
    Ok(BlockPos {
        x: pos[0].floor() as i32,
        y: pos[1].floor() as i32,
        z: pos[2].floor() as i32,
    })
}
fn slot_index(name: &str, selected: usize, player: bool) -> Option<usize> {
    if let Some(n) = name.strip_prefix("container.") {
        return n.parse().ok();
    }
    if player
        && let Some(n) = name
            .strip_prefix("hotbar.")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n < 9)
    {
        return Some(n);
    }
    if player
        && let Some(n) = name
            .strip_prefix("inventory.")
            .and_then(|n| n.parse::<usize>().ok())
            .filter(|n| *n < 27)
    {
        return Some(n + 9);
    }
    match (name, player) {
        ("weapon" | "weapon.mainhand", true) => Some(selected),
        ("weapon.offhand", true) => Some(40),
        ("armor.feet", true) => Some(36),
        ("armor.legs", true) => Some(37),
        ("armor.chest", true) => Some(38),
        ("armor.head", true) => Some(39),
        ("weapon" | "weapon.mainhand", false) => Some(0),
        ("weapon.offhand", false) => Some(1),
        ("armor.feet", false) => Some(2),
        ("armor.legs", false) => Some(3),
        ("armor.chest", false) => Some(4),
        ("armor.head", false) => Some(5),
        _ => None,
    }
}
pub(crate) fn read_slot(
    state: &TickState,
    executor: EntityId,
    destination: &Destination,
) -> Result<ItemStack, String> {
    match destination {
        Destination::Block(coords, slot) => {
            let pos = position(state, executor, *coords)?;
            let index = slot_index(slot, 0, false).ok_or("Invalid slot")?;
            if !slot.starts_with("container.") {
                return Err("Invalid container slot".into());
            }
            state
                .block_entities
                .get(&(pos.x, pos.y, pos.z))
                .and_then(|b| b.slots.get(index))
                .cloned()
                .ok_or_else(|| "Container or slot not found".into())
        }
        Destination::Entity(target, slot) => {
            let entities = crate::command_execution::resolve_target(state, executor, target)?;
            if slot.starts_with("container.") {
                return Err("Invalid entity slot".into());
            }
            let entity = entities.first().ok_or("Source entity not found")?;
            if let Some(player) = state.players.get(entity) {
                let index = slot_index(slot, usize::from(player.inventory.selected.min(8)), true)
                    .ok_or("Invalid slot")?;
                return player
                    .inventory
                    .slots
                    .get(index)
                    .cloned()
                    .ok_or_else(|| "Invalid slot".into());
            }
            let entity_slot = state.entities.slot_of(*entity).ok_or("Entity not found")?;
            if !matches!(
                state.entities.body(entity_slot).kind,
                crate::entities::EntityKind::Mob { .. }
            ) {
                return Err("Entity has no equipment".into());
            }
            let index = slot_index(slot, 0, false)
                .filter(|n| *n < 6)
                .ok_or("Invalid slot")?;
            Ok(state.entities.row(entity_slot).appearance.equipment[index]
                .clone()
                .unwrap_or(ItemStack::EMPTY))
        }
    }
}
fn slot_span(
    name: &str,
    selected: usize,
    player: bool,
    count: usize,
) -> Option<std::ops::Range<usize>> {
    let start = slot_index(name, selected, player)?;
    let count = if name.starts_with("container.") {
        count
    } else if name.starts_with("hotbar.") {
        (count <= 9 - start).then_some(count)?
    } else if name.starts_with("inventory.") {
        (count <= 36 - start).then_some(count)?
    } else {
        count.min(1)
    };
    Some(start..start.checked_add(count)?)
}

pub(crate) fn write_slots(
    state: &mut TickState,
    executor: EntityId,
    destination: &Destination,
    stacks: &[ItemStack],
    length: usize,
) -> Result<i32, String> {
    if length == 0 {
        return Ok(0);
    }
    let empty = ItemStack::EMPTY;
    let stack_at = |index| stacks.get(index).unwrap_or(&empty);
    match destination {
        Destination::Block(coords, slot) => {
            let pos = position(state, executor, *coords)?;
            if !slot.starts_with("container.") {
                return Err("Invalid container slot".into());
            }
            let range = slot_span(slot, 0, false, length).ok_or("Invalid slot range")?;
            let record = state
                .block_entities
                .get(&(pos.x, pos.y, pos.z))
                .ok_or("Container not found")?;
            if range.end > record.slots.len() {
                return Err("Container or slot not found".into());
            }
            if crate::plugin_inventories::has_reservations_for_block(state, pos) {
                let mut candidate = record.slots.clone();
                for (offset, index) in range.clone().enumerate() {
                    candidate[index] = stack_at(offset).clone();
                }
                if crate::plugin_inventories::below_block_floors(state, pos, &candidate) {
                    return Err("Item is reserved by a plugin".into());
                }
            }
            let count = range.len() as i32;
            let record = state
                .block_entities
                .get_mut(&(pos.x, pos.y, pos.z))
                .expect("admitted container");
            for (offset, index) in range.enumerate() {
                record.slots[index] = stack_at(offset).clone();
            }
            crate::brewing::slots_replaced(
                &mut state.brewing,
                (pos.x, pos.y, pos.z),
                &record.slots,
            );
            crate::redstone::containers::inventory_changed(&mut state.redstone_containers, pos);
            state.world.mark_block_entity_dirty(pos);
            crate::tick::refresh_container_viewers(state, pos);
            Ok(count)
        }
        Destination::Entity(target, slot) => {
            if slot.starts_with("container.") {
                return Err("Invalid entity slot".into());
            }
            let required = match slot.as_str() {
                "armor.feet" => Some("feet"),
                "armor.legs" => Some("legs"),
                "armor.chest" => Some("chest"),
                "armor.head" => Some("head"),
                _ => None,
            };
            let first = stack_at(0);
            if !first.is_empty()
                && let Some(required) = required
                && state
                    .registry
                    .item_facts
                    .facts_for_stack(first, &state.registry.items)
                    .and_then(|facts| facts.equippable_slot.as_deref())
                    != Some(required)
            {
                return Err("Item cannot be placed in the specified armor slot".into());
            }
            enum Write {
                Player(EntityId, std::ops::Range<usize>),
                Equipment(
                    EntityId,
                    crate::entities::EntitySlot,
                    [Option<ItemStack>; 6],
                    crate::plugin_inventories::PreparedEntityCustody,
                ),
            }
            let entities = crate::command_execution::resolve_target(state, executor, target)?;
            let mut prepared = Vec::with_capacity(entities.len());
            let mut count = 0;
            for entity in entities {
                if let Some(player) = state.players.get(&entity) {
                    let Some(range) = slot_span(
                        slot,
                        usize::from(player.inventory.selected.min(8)),
                        true,
                        length,
                    )
                    .filter(|range| range.end <= player.inventory.slots.len()) else {
                        continue;
                    };
                    let replacements: Vec<_> = range
                        .clone()
                        .enumerate()
                        .map(|(offset, index)| (index, stack_at(offset)))
                        .collect();
                    if crate::plugin_inventories::below_player_replacement_floors(
                        state,
                        entity,
                        &replacements,
                    ) {
                        return Err("Item is reserved by a plugin".into());
                    }
                    count += range.len() as i32;
                    prepared.push(Write::Player(entity, range));
                } else if let Some(entity_slot) = state.entities.slot_of(entity) {
                    if !matches!(
                        state.entities.body(entity_slot).kind,
                        crate::entities::EntityKind::Mob { .. }
                    ) {
                        continue;
                    }
                    let Some(range) =
                        slot_span(slot, 0, false, length).filter(|range| range.end <= 6)
                    else {
                        continue;
                    };
                    let mut equipment =
                        state.entities.row(entity_slot).appearance.equipment.clone();
                    for (offset, index) in range.clone().enumerate() {
                        let stack = stack_at(offset);
                        equipment[index] = (!stack.is_empty()).then(|| stack.clone());
                    }
                    let custody = crate::plugin_inventories::prepare_entity_custody(
                        state,
                        entity_slot,
                        &equipment,
                    )
                    .map_err(|_| "Entity inventory custody is unavailable")?;
                    count += range.len() as i32;
                    prepared.push(Write::Equipment(entity, entity_slot, equipment, custody));
                }
            }
            if prepared.is_empty() {
                return Err("No entity has the specified slot".into());
            }
            // Every target and its final reservation footprint is admitted.
            for write in prepared {
                match write {
                    Write::Player(entity, range) => {
                        let player = state.players.get_mut(&entity).expect("admitted player");
                        for (offset, index) in range.enumerate() {
                            player.inventory.slots[index] = stack_at(offset).clone();
                        }
                        crate::projectiles::weapons::sync(state, entity);
                    }
                    Write::Equipment(entity, slot, equipment, custody) => {
                        std::sync::Arc::make_mut(&mut state.entities.row_mut(slot).appearance)
                            .equipment = equipment;
                        crate::entity_loot::restore(state, slot);
                        crate::plugin_inventories::apply_entity_custody(state, custody);
                        state.appearance_changed.push(entity);
                    }
                }
            }
            Ok(count)
        }
    }
}
pub(crate) fn execute(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    command: &ItemCommand,
) -> Result<i32, String> {
    let stack = match &command.source {
        Source::Unsupported => return Err("Item modifiers are not supported".into()),
        Source::From(source) => read_slot(state, executor, source)?,
        Source::With(item, count) => {
            let mut stack = item.stack_with(&state.registry.item_rules())?;
            if *count > state.registry.item_rules().max_stack(&stack) {
                return Err("Item count exceeds stack limit".into());
            }
            if item.item.as_str() == "minecraft:air" {
                ItemStack::EMPTY
            } else {
                stack.count = *count;
                stack
            }
        }
    };
    let count = write_slots(
        state,
        executor,
        &command.destination,
        std::slice::from_ref(&stack),
        1,
    )?;
    if crate::command_execution::accepts_success(state, connection) {
        state.push_player(
            connection,
            PlayerChange::StyledChat {
                content: Tag::Compound(vec![(
                    "text".into(),
                    Tag::String(format!("Replaced item in {count} slot(s)")),
                )]),
            },
        );
    }
    Ok(count)
}
