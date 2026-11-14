//! Live block data edits: preflight once, then publish through resident owners.
use super::*;
use mc_data::item_nbt::field;

pub(crate) fn update(
    state: &mut TickState,
    pos: BlockPos,
    before: &Tag,
    candidate: &Tag,
) -> Result<(), String> {
    let (Tag::Compound(before), Tag::Compound(candidate)) = (before, candidate) else {
        return Err("Block entity data must be a compound".into());
    };
    if candidate
        .iter()
        .any(|(key, _)| key.starts_with("krasnokamen") || key.starts_with("Krasnokamen"))
    {
        return Err("Internal block entity fields cannot be edited".into());
    }
    let kind = state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .and_then(|block| BlockEntityKind::for_block(block.block.id.path()))
        .ok_or("The target block is not a block entity")?;
    let changed = |key: &str| field(before, key) != field(candidate, key);
    let cell = (pos.x, pos.y, pos.z);
    let mut image = prepare_fields(state, kind, candidate.clone())?;
    // Unopened imported chests keep their typed loot source until first open.
    // A metadata-only edit must not manufacture an empty inventory or replace it.
    let preserve_unopened_loot = !state.block_entities.contains_key(&cell)
        && ["Items", "LootTable", "LootTableSeed"]
            .iter()
            .all(|key| !changed(key))
        && state
            .world
            .chunk(crate::boundary::ChunkPos {
                x: pos.x.div_euclid(16),
                z: pos.z.div_euclid(16),
            })
            .and_then(|chunk| chunk.chests.get(&pos))
            .is_some_and(|chest| chest.loot_table.is_some());
    if preserve_unopened_loot {
        image.storage = None;
    }
    if kind == BlockEntityKind::CommandBlock {
        for (name, value) in &image.fields {
            let valid = match name.as_str() {
                "Command" => matches!(value, Tag::String(command) if command.len() <= 32767),
                "auto" | "powered" | "conditionMet" | "TrackOutput" | "UpdateLastExecution" => {
                    matches!(value, Tag::Byte(0 | 1))
                }
                "SuccessCount" => matches!(value, Tag::Int(_)),
                "LastExecution" => matches!(value, Tag::Long(_)),
                _ => true,
            };
            if !valid {
                return Err(format!("Invalid command block field {name}"));
            }
        }
    }
    image.fields.extend([
        ("x".into(), Tag::Int(pos.x)),
        ("y".into(), Tag::Int(pos.y)),
        ("z".into(), Tag::Int(pos.z)),
    ]);
    if let Some(bytes) = state
        .world
        .chunk(crate::boundary::ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        })
        .and_then(|chunk| chunk.block_entities.get(&pos))
    {
        let Tag::Compound(fields) =
            mc_nbt::read_network(&mut bytes.as_slice()).map_err(|error| error.to_string())?
        else {
            return Err("Invalid resident block entity image".into());
        };
        image.fields.extend(fields.into_iter().filter(|(name, _)| {
            name.starts_with("krasnokamen") || name.starts_with("Krasnokamen")
        }));
    }
    let native = prepare_native(
        state,
        pos,
        kind,
        &mut image,
        changed("Delay"),
        changed("listener"),
    )?;
    if let Some(storage) = &image.storage
        && crate::plugin_inventories::has_reservations_for_block(state, pos)
        && crate::plugin_inventories::below_block_floors(state, pos, &storage.slots)
    {
        return Err("Item is reserved by a plugin".into());
    }
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &Tag::Compound(image.fields))
        .map_err(|error| error.to_string())?;

    // No fallible operation follows. Do not use placement installation: it
    // clears resident caches and discards live clocks and inventory ownership.
    if let Some(storage) = image.storage {
        if let Some(current) = state.block_entities.get(&cell) {
            let slots_changed = current.slots != storage.slots;
            let furnace_changed = current.furnace != storage.furnace;
            if slots_changed || furnace_changed {
                let current = state
                    .block_entities
                    .get_mut(&cell)
                    .expect("admitted live inventory");
                if slots_changed {
                    current.slots = storage.slots;
                }
                if furnace_changed {
                    current.furnace = storage.furnace;
                }
            }
        } else {
            state.block_entities.insert(cell, storage);
        }
    }
    match native {
        Native::None => {}
        Native::Sculk(prepared) => crate::sculk::apply_data(state, pos, prepared),
        Native::Shrieker(prepared) => crate::shrieker::apply_data(state, pos, prepared),
        Native::Hopper(ticks) => state
            .redstone_containers
            .apply_data_cooldown(pos, ticks, state.tick),
        Native::Gateway(age) => {
            state.gateway_ages.insert(
                pos,
                crate::end_portals::GatewayAge {
                    age,
                    stored_age: age,
                },
            );
        }
        Native::Brewing(stand) => crate::brewing::apply_data(state, pos, stand),
        Native::Crafter(settings) => {
            state.crafters.entry(cell).or_default().settings = settings;
        }
        Native::Lectern(lectern) => crate::lectern::apply_data(state, pos, lectern),
        Native::Campfire(campfire) => {
            state.campfires.insert(cell, campfire);
        }
        Native::Hive(hive) => {
            state.hives.insert(cell, hive);
        }
        Native::Jukebox(jukebox) => crate::jukebox::apply_data(state, pos, jukebox),
        Native::MonsterSpawner(spawner) => {
            state.monster_spawners.replace_data(pos, spawner);
        }
        Native::TrialSpawner(spawner) => {
            state.trial_spawners.insert(cell, spawner);
        }
        Native::Vault(vault) => {
            state.vaults.insert(cell, vault);
        }
    }
    if preserve_unopened_loot {
        state
            .world
            .set_opaque_block_entity(pos, Some(bytes.clone()));
    } else {
        state.world.install_command_block_entity(pos, bytes.clone());
    }
    state.opaque_changes.push((pos, Some(bytes)));
    state.block_entities.mark_dirty(cell);
    crate::redstone::containers::inventory_changed(&mut state.redstone_containers, pos);
    state.world.mark_block_entity_visible(pos);
    if kind == BlockEntityKind::CommandBlock {
        crate::command_blocks::data_updated(
            state,
            pos,
            matches!(field(before, "auto"), Some(Tag::Byte(value)) if *value != 0),
        );
    }
    crate::tick::refresh_container_viewers(state, pos);
    publish(state, pos);
    Ok(())
}

enum Native {
    None,
    Sculk(crate::sculk::PreparedData),
    Shrieker(crate::shrieker::PreparedData),
    Hopper(i32),
    Gateway(i64),
    Brewing(crate::brewing::BrewingState),
    Crafter(mc_data::storage_blocks::CrafterSettings),
    Lectern(crate::lectern::Lectern),
    Campfire(crate::campfire::CampfireState),
    Hive(crate::bees::Hive),
    Jukebox(crate::jukebox::Jukebox),
    MonsterSpawner(crate::monster_spawner::Spawner),
    TrialSpawner(crate::trial_spawner::Spawner),
    Vault(mc_data::trial_nbt::VaultData),
}

fn prepare_native(
    state: &TickState,
    pos: BlockPos,
    kind: BlockEntityKind,
    image: &mut Prepared,
    delay_changed: bool,
    listener_changed: bool,
) -> Result<Native, String> {
    let cell = (pos.x, pos.y, pos.z);
    // These codecs own byte admission already. Other resident owners admit
    // fields directly, without serializing and reparsing an intermediate image.
    let mut bytes = Vec::new();
    if matches!(
        kind,
        BlockEntityKind::Beehive
            | BlockEntityKind::Jukebox
            | BlockEntityKind::MobSpawner
            | BlockEntityKind::TrialSpawner
    ) {
        mc_nbt::write_network(&mut bytes, &Tag::Compound(image.fields.clone()))
            .map_err(|error| error.to_string())?;
    }
    Ok(match kind {
        BlockEntityKind::SculkSensor
        | BlockEntityKind::CalibratedSculkSensor
        | BlockEntityKind::SculkCatalyst => Native::Sculk(crate::sculk::prepare_data(
            state,
            pos,
            &mut image.fields,
            listener_changed,
        )?),
        BlockEntityKind::SculkShrieker => Native::Shrieker(crate::shrieker::prepare_data(
            state,
            pos,
            &mut image.fields,
            listener_changed,
        )?),
        BlockEntityKind::Hopper => {
            let ticks = match field(&image.fields, "TransferCooldown") {
                None => 0,
                Some(Tag::Int(ticks)) => (*ticks).clamp(0, 8),
                _ => return Err("TransferCooldown: expected Int".into()),
            };
            mc_data::trial_nbt::set(&mut image.fields, "TransferCooldown", Tag::Int(ticks));
            mc_data::trial_nbt::set(
                &mut image.fields,
                "krasnokamen:TransferCooldownUntil",
                Tag::Long(state.tick.saturating_add(ticks as u64) as i64),
            );
            Native::Hopper(ticks)
        }
        BlockEntityKind::EndGateway => {
            let gateway = crate::end_portals::gateway_from_fields(pos, &image.fields)?;
            Native::Gateway(gateway.age)
        }
        BlockEntityKind::BrewingStand => {
            let slots = &image
                .storage
                .as_ref()
                .expect("brewing inventory admitted")
                .slots;
            let previous = if let Some(stand) = state.brewing.get(&cell) {
                stand.clone()
            } else if let Some(bytes) = state
                .world
                .chunk(crate::boundary::ChunkPos {
                    x: pos.x.div_euclid(16),
                    z: pos.z.div_euclid(16),
                })
                .and_then(|chunk| chunk.block_entities.get(&pos))
            {
                crate::brewing::BrewingState::decode(bytes, &state.registry.items)
                    .ok_or("Invalid resident brewing stand")?
                    .0
            } else {
                crate::brewing::BrewingState::default()
            };
            let stand = previous.with_data(&image.fields, slots)?;
            mc_data::trial_nbt::set(&mut image.fields, "BrewTime", Tag::Short(stand.brew_time));
            mc_data::trial_nbt::set(&mut image.fields, "Fuel", Tag::Byte(stand.fuel as i8));
            stand.retain_ingredient(&mut image.fields, &state.registry.items);
            Native::Brewing(stand)
        }
        BlockEntityKind::Crafter => {
            let (settings, slots) =
                mc_data::storage_blocks::decode_crafter(&image.fields, &state.registry.items)?;
            image.storage = Some(BlockEntityState {
                slots,
                furnace: None,
            });
            Native::Crafter(settings)
        }
        BlockEntityKind::Lectern => {
            let (lectern, book) =
                crate::lectern::Lectern::from_fields(&image.fields, &state.registry.items)?;
            mc_data::trial_nbt::set(&mut image.fields, "Page", Tag::Int(lectern.page()));
            image.storage = Some(BlockEntityState {
                slots: vec![book],
                furnace: None,
            });
            Native::Lectern(lectern)
        }
        BlockEntityKind::Campfire => Native::Campfire(crate::campfire::CampfireState::from_fields(
            &image.fields,
            &state.registry.items,
        )?),
        BlockEntityKind::Beehive => {
            Native::Hive(crate::bees::Hive::decode(&bytes).map_err(|error| error.to_string())?)
        }
        BlockEntityKind::Jukebox => {
            let jukebox = crate::jukebox::Jukebox::decode(&bytes, &state.registry.items)
                .map_err(|error| error.to_string())?;
            image.storage = Some(BlockEntityState {
                slots: vec![jukebox.record().clone()],
                furnace: None,
            });
            Native::Jukebox(jukebox)
        }
        BlockEntityKind::MobSpawner => {
            let decoded = crate::monster_spawner::Spawner::decode(&bytes, &state.registry)?;
            let spawner =
                state
                    .monster_spawners
                    .prepare_data(pos, decoded, delay_changed, state.tick);
            image.fields = spawner.data_fields().clone();
            Native::MonsterSpawner(spawner)
        }
        BlockEntityKind::TrialSpawner => {
            let mut spawner = crate::trial_spawner::Spawner::decode(&bytes, &state.registry.items)?;
            if let Some(old) = state.trial_spawners.get(&cell) {
                old.preserve_pending(&mut spawner);
            }
            image.fields = spawner.fields(&state.registry.items);
            Native::TrialSpawner(spawner)
        }
        BlockEntityKind::Vault => {
            let vault = mc_data::trial_nbt::VaultData::from_fields(
                image.fields.clone(),
                &state.registry.items,
            )?;
            image.fields = vault.to_fields(&state.registry.items);
            Native::Vault(vault)
        }
        _ => Native::None,
    })
}
