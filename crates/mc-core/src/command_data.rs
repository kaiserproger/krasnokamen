//! Operator data access through native entity, block-entity and storage owners.
use crate::boundary::{BlockPos, ChunkPos, ConnectionId, EntityId};
use crate::command_context::Context;
use crate::command_nbt_path::{argument, get, number, result, snbt};
use crate::tick::TickState;
use mc_nbt::{ListTag, Tag, tag_type};

fn put(fields: &mut Vec<(String, Tag)>, key: &str, value: Tag) {
    if let Some((_, old)) = fields.iter_mut().find(|(name, _)| name == key) {
        *old = value;
    } else {
        fields.push((key.into(), value));
    }
}
fn list(elements: Vec<Tag>, element_type: u8) -> Tag {
    Tag::List(ListTag {
        element_type,
        elements,
    })
}
fn public_fields(fields: &mut Vec<(String, Tag)>) {
    fields.retain(|(key, _)| {
        !key.starts_with("solaris") && !key.starts_with("Solaris") && key != "DataVersion"
    });
}

pub(crate) fn entity(state: &TickState, id: EntityId) -> Result<Tag, String> {
    let slot = state
        .entities
        .slot_of_any(id)
        .ok_or("No entity was found")?;
    let row = state.entities.row(slot);
    let mut fields = if let Some(player) = state.players.get(&id) {
        let Tag::Compound(mut fields) =
            crate::core::player_data_tag(state, id).map_err(|e| e.to_string())?
        else {
            unreachable!()
        };
        let uuid = row.uuid.as_u128();
        put(
            &mut fields,
            "UUID",
            Tag::IntArray([96, 64, 32, 0].map(|shift| (uuid >> shift) as i32).to_vec()),
        );
        put(
            &mut fields,
            "Motion",
            list(
                state
                    .entities
                    .vel(slot)
                    .map(|v| Tag::Double(v / 20.0))
                    .to_vec(),
                tag_type::DOUBLE,
            ),
        );
        put(
            &mut fields,
            "OnGround",
            Tag::Byte(i8::from(state.entities.ground(slot))),
        );
        if !fields.iter().any(|(key, _)| key == "EnderItems") {
            fields.push(("EnderItems".into(), list(Vec::new(), tag_type::COMPOUND)));
        }
        if let Some(stack) = player
            .inventory
            .slots
            .get(usize::from(player.inventory.selected))
            .filter(|stack| !stack.is_empty())
        {
            put(
                &mut fields,
                "SelectedItem",
                Tag::Compound(
                    mc_data::item_nbt::stack_to_fields(&state.registry.items, stack)
                        .map_err(|e| e.to_string())?,
                ),
            );
        }
        fields
    } else {
        let record =
            crate::core::entity_data_record(state, slot).ok_or("Entity has no vanilla data")?;
        mc_data::entity_nbt::entity_to_fields(&state.registry.items, &record)
            .map_err(|e| e.to_string())?
    };
    public_fields(&mut fields);
    // EntityDataAccessor uses NbtPredicate -> saveWithoutId, not a disk entity root.
    fields.retain(|(key, _)| key != "id");
    Ok(Tag::Compound(fields))
}

pub(crate) fn block(state: &TickState, pos: BlockPos) -> Result<Tag, String> {
    use mc_data::block_entity_kinds::BlockEntityKind;
    let chunk = state
        .world
        .chunk(ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        })
        .ok_or("That position is not loaded")?;
    let kind = state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .and_then(|block| BlockEntityKind::for_block(block.block.id.path()))
        .ok_or("The target block is not a block entity")?;
    let hive = if kind == BlockEntityKind::Beehive {
        crate::bees::loot_image(state, pos)
    } else {
        None
    };
    let tag = if let Some(bytes) = hive {
        Some(mc_nbt::read_network(&mut bytes.as_slice()).map_err(|e| e.to_string())?)
    } else {
        mc_world::anvil::chunk_nbt::block_entity_to_tag(
            chunk,
            pos,
            &state.registry.blocks,
            &state.registry.items,
        )
        .map_err(|e| e.to_string())?
    };
    let mut fields = match tag {
        Some(Tag::Compound(fields)) => fields,
        None => Vec::new(),
        _ => return Err("Invalid block entity NBT".into()),
    };
    for (key, value) in [
        ("id", Tag::String(kind.vanilla_id().into())),
        ("x", Tag::Int(pos.x)),
        ("y", Tag::Int(pos.y)),
        ("z", Tag::Int(pos.z)),
    ] {
        put(&mut fields, key, value);
    }
    let key = (pos.x, pos.y, pos.z);
    if kind == BlockEntityKind::Hopper
        && let Some(ticks) = state.redstone_containers.data_cooldown(pos, state.tick) {
        put(&mut fields, "TransferCooldown", Tag::Int(ticks));
    }
    if let Some(clock) = state.gateway_ages.get(&pos) {
        put(&mut fields, "Age", Tag::Long(clock.age));
    }
    if let Some(live) = state.block_entities.get(&key) {
        if (kind.inventory_size().is_some() || kind == BlockEntityKind::ChiseledBookshelf)
            && !fields.iter().any(|(key, _)| key == "LootTable") {
            let mut items = Vec::new();
            for (slot, stack) in live
                .slots
                .iter()
                .enumerate()
                .filter(|(_, stack)| !stack.is_empty())
            {
                let mut item = mc_data::item_nbt::stack_to_fields(&state.registry.items, stack)
                    .map_err(|e| e.to_string())?;
                item.push(("Slot".into(), Tag::Byte(slot as i8)));
                items.push(Tag::Compound(item));
            }
            put(&mut fields, "Items", list(items, tag_type::COMPOUND));
        }
        if kind == BlockEntityKind::DecoratedPot {
            fields.retain(|(name, _)| name != "item");
            if let Some(stack) = live.slots.first().filter(|stack| !stack.is_empty()) {
                put(&mut fields, "item", Tag::Compound(
                    mc_data::item_nbt::stack_to_fields(&state.registry.items, stack)
                        .map_err(|error| error.to_string())?));
            }
        }
        if let Some(furnace) = &live.furnace {
            for (key, value) in [
                ("lit_time_remaining", furnace.lit_remaining),
                ("lit_total_time", furnace.lit_total),
                ("cooking_time_spent", furnace.cook_progress),
                ("cooking_total_time", furnace.cook_total),
            ] {
                put(&mut fields, key, Tag::Short(value as i16));
            }
            put(
                &mut fields,
                "RecipesUsed",
                Tag::Compound(
                    furnace
                        .recipes_used
                        .iter()
                        .map(|(key, value)| (key.clone(), Tag::Int(*value)))
                        .collect(),
                ),
            );
        }
    }
    if let Some(stand) = state.brewing.get(&key) {
        put(&mut fields, "BrewTime", Tag::Short(stand.brew_time));
        put(&mut fields, "Fuel", Tag::Byte(stand.fuel as i8));
    }
    if let Some(jukebox) = state.jukeboxes.get(&key) {
        fields.retain(|(key, _)| key != "RecordItem" && key != "ticks_since_song_started");
        fields.extend(jukebox.data_fields(&state.registry.items));
    }
    if let Some(campfire) = state.campfires.get(&key) {
        for (name, value) in campfire.data_fields(&state.registry.items) {
            put(&mut fields, &name, value);
        }
    }
    if let Some(crafter) = state.crafters.get(&key) {
        mc_data::storage_blocks::encode_crafter_fields(&mut fields, crafter.settings);
    }
    if let Some(lectern) = state.lecterns.get(&key) {
        fields.retain(|(name, _)| name != "Book" && name != "Page");
        if let Some(book) = state.block_entities.get(&key)
            .and_then(|container| container.slots.first()).filter(|book| !book.is_empty()) {
            put(&mut fields, "Book", Tag::Compound(
                mc_data::item_nbt::stack_to_fields(&state.registry.items, book)
                    .map_err(|error| error.to_string())?));
            put(&mut fields, "Page", Tag::Int(lectern.page()));
        }
    }
    if let Some(spawner) = state.monster_spawners.data_fields(pos, state.tick) {
        for (name, value) in spawner { put(&mut fields, &name, value); }
    }
    if let Some(spawner) = state.trial_spawners.get(&key) {
        for (name, value) in spawner.fields(&state.registry.items) {
            put(&mut fields, &name, value);
        }
    }
    if let Some(vault) = state.vaults.get(&key) {
        for (name, value) in vault.to_fields(&state.registry.items) {
            put(&mut fields, &name, value);
        }
    }
    if let Some(sculk) = crate::sculk::data_fields(state, pos) {
        for (name, value) in sculk { put(&mut fields, &name, value); }
    }
    if let Some(shrieker) = crate::shrieker::data_fields(state, pos) {
        for (name, value) in shrieker { put(&mut fields, &name, value); }
    }
    public_fields(&mut fields);
    Ok(Tag::Compound(fields))
}

/// A command resolves its accessor once, before preparing any mutation.
pub(crate) enum Accessor {
    Storage(String),
    Entity(crate::dimensions::Dimension, EntityId),
    Block(crate::dimensions::Dimension, BlockPos),
}

impl Accessor {
    pub(crate) fn parse(
        state: &TickState,
        context: Context,
        input: &mut &str,
    ) -> Result<Self, String> {
        match argument(input)? {
            "storage" => {
                let name = mc_data::Identifier::parse(argument(input)?)
                    .map_err(|error| error.to_string())?;
                Ok(Self::Storage(name.to_string()))
            }
            "entity" => {
                let target = crate::commands::target_of(argument(input)?)
                    .map_err(|error| error.message())?;
                let multiple = match &target {
                    crate::commands::Target::AllPlayers => true,
                    crate::commands::Target::Selector(selector) => {
                        selector.limit.unwrap_or(
                            if matches!(
                                selector.kind,
                                crate::commands::SelectorKind::AllPlayers
                                    | crate::commands::SelectorKind::Entities
                            ) {
                                usize::MAX
                            } else {
                                1
                            },
                        ) > 1
                    }
                    _ => false,
                };
                if multiple {
                    return Err("Only one entity is allowed, but the provided selector allows more than one".into());
                }
                let selected = crate::dimensions::travel::context_targets(state, context, &target)?;
                let [id] = selected.as_slice() else {
                    return Err(if selected.is_empty() {
                        "No entity was found"
                    } else {
                        "Only one entity is allowed, but the provided selector allows more than one"
                    }.into());
                };
                let (dimension, _) = crate::dimensions::travel::locate(state, *id)
                    .ok_or("No entity was found")?;
                Ok(Self::Entity(dimension, *id))
            }
            "block" => {
                let coords = [argument(input)?, argument(input)?, argument(input)?];
                let coords = crate::commands::coords(&coords).ok_or("Invalid block coordinates")?;
                let position = crate::commands::resolve_coords(
                    coords,
                    context.coordinate_origin(coords[0].local),
                    context.rotation,
                );
                if position.iter().any(|value| {
                    !value.is_finite() || *value < i32::MIN as f64 || *value > i32::MAX as f64
                }) {
                    return Err("Invalid block coordinates".into());
                }
                let [x, y, z] = position.map(|value| value.floor() as i32);
                Ok(Self::Block(context.dimension, BlockPos { x, y, z }))
            }
            _ => Err("Expected storage, entity or block".into()),
        }
    }

    pub(crate) fn read(&self, state: &TickState) -> Result<Tag, String> {
        match self {
            Self::Storage(name) => Ok(state.command_storage.get(name).cloned()
                .unwrap_or_else(|| Tag::Compound(Vec::new()))),
            Self::Entity(dimension, id) => entity(
                crate::dimensions::travel::world(state, *dimension)
                    .ok_or("Entity dimension unavailable")?, *id),
            Self::Block(dimension, pos) => block(
                crate::dimensions::travel::world(state, *dimension)
                    .ok_or("Unknown dimension")?, *pos),
        }
    }

    fn feedback(&self, state: &TickState) -> Result<(&'static str, Vec<Tag>), String> {
        Ok(match self {
            Self::Storage(name) => ("storage", vec![text(name)]),
            Self::Entity(dimension, id) => (
                "entity",
                vec![crate::command_execution::entity_name(
                    crate::dimensions::travel::world(state, *dimension)
                        .ok_or("Entity dimension unavailable")?, *id)],
            ),
            Self::Block(_, BlockPos { x, y, z }) => (
                "block", vec![text(x), text(y), text(z)],
            ),
        })
    }
}

fn text(value: impl ToString) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(value.to_string()))])
}

/// Returns the Brigadier result; errors remain failures even when observed by execute store.
pub(crate) fn query(
    state: &mut TickState,
    context: Context,
    mut input: &str,
) -> Result<i32, String> {
    if context.permission < 2 {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    if argument(&mut input)? != "data" || argument(&mut input)? != "get" {
        return Err("Expected data get entity|block".into());
    }
    let target = Accessor::parse(state, context, &mut input)?;
    let root = target.read(state)?;
    let (kind, mut args) = target.feedback(state)?;
    let (key, value) = if input.trim().is_empty() {
        args.push(text(snbt(&root)));
        (format!("commands.data.{kind}.query"), 1)
    } else {
        let path = argument(&mut input)?;
        let value = get(&root, path)?;
        if input.trim().is_empty() {
            args.push(text(snbt(&value)));
            (format!("commands.data.{kind}.query"), result(&value))
        } else {
            let scale = argument(&mut input)?
                .parse::<f64>()
                .map_err(|_| "Invalid scale")?;
            if !scale.is_finite() || !input.trim().is_empty() {
                return Err("Invalid scale or trailing arguments".into());
            }
            let value = (number(&value)
                .ok_or_else(|| format!("Can't get {path}; only numeric tags are allowed"))?
                * scale)
                .floor() as i32;
            args.insert(0, text(path));
            args.push(text(format!("{scale:.2}")));
            args.push(text(value));
            (format!("commands.data.{kind}.get"), value)
        }
    };
    crate::command_execution::success_feedback(state, context.connection, &key, args);
    Ok(value)
}

pub(crate) fn mutate(
    state: &mut TickState,
    context: Context,
    mut input: &str,
) -> Result<i32, String> {
    if context.permission < 2 {
        return Err(crate::commands::CommandError::Unknown.message());
    }
    if argument(&mut input)? != "data" { return Err("Expected data".into()); }
    let action = argument(&mut input)?;
    let target = Accessor::parse(state, context, &mut input)?;
    let (kind, args) = target.feedback(state)?;
    let count = match target {
        Accessor::Storage(name) => {
            let empty = Tag::Compound(Vec::new());
            let previous = state.command_storage.get(&name).unwrap_or(&empty);
            let (count, next) = crate::command_storage::prepare(state, context, previous, action, input)?;
            state.command_storage.insert(name, next);
            state.command_storage_dirty = true;
            count
        }
        target => {
            let previous = target.read(state)?;
            let (count, next) = crate::command_storage::prepare(state, context, &previous, action, input)?;
            let dimension = match target {
                Accessor::Entity(dimension, _) | Accessor::Block(dimension, _) => dimension,
                Accessor::Storage(_) => unreachable!(),
            };
            let world = if state.dimension == dimension { &mut *state } else {
                state.other_dimensions.get_mut(&dimension).map(|world| &mut **world)
                    .ok_or("Target dimension unavailable")?
            };
            match target {
                Accessor::Entity(_, id) => crate::command_summon::data::update(world, id, &previous, &next)?,
                Accessor::Block(_, pos) => crate::command_block_entity::data::update(world, pos, &previous, &next)?,
                Accessor::Storage(_) => unreachable!(),
            }
            count
        }
    };
    crate::command_execution::success_feedback(
        state, context.connection, &format!("commands.data.{kind}.modified"), args);
    Ok(count)
}

pub(crate) fn command(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    input: &str,
) -> crate::command_execution::CommandResult {
    use crate::command_execution::CommandResult::{Failed, Success};
    let context = state
        .command_context
        .current
        .unwrap_or_else(|| Context::source(state, connection, executor, 2));
    let result = if input.split_whitespace().nth(1) == Some("get") {
        query(state, context, input)
    } else {
        mutate(state, context, input)
    };
    match result {
        Ok(value) => Success(value),
        Err(message) => {
            state.push_player(connection, crate::boundary::PlayerChange::Chat { message });
            Failed
        }
    }
}

#[cfg(test)]
#[path = "command_data_tests.rs"]
mod tests;
