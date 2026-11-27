//! Held-use archaeology. Progress is transient; revealed loot is journaled once.
use crate::boundary::BlockPos;
use crate::tick::TickState;
use mc_data::item_nbt::{field, stack_from_fields, stack_to_fields};
use mc_data::loot::chest_26_1_2::ChestRng;
use mc_data::{Identifier, ItemStack};
use mc_domain::{EntityId, Hand};
use mc_nbt::Tag;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct Brushing {
    progress: HashMap<[i32; 3], Progress>,
}
struct Progress {
    count: u8,
    cooldown: u64,
    reset: u64,
}
fn brushable(state: &TickState, pos: BlockPos) -> Option<bool> {
    match state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(
            state.world.block_at(pos.x, pos.y, pos.z)?,
        ))?
        .block
        .id
        .path()
    {
        "suspicious_sand" => Some(true),
        "suspicious_gravel" => Some(false),
        _ => None,
    }
}
pub(crate) fn use_on(state: &mut TickState, actor: EntityId, hand: Hand) -> bool {
    let Some(player) = state.players.get_mut(&actor) else {
        return false;
    };
    let held = &player.inventory.slots[crate::tick::held_slot_index(player, hand)];
    if held.is_empty()
        || !state
            .registry
            .items
            .name_of(held.item_id)
            .is_some_and(|id| id.as_str() == "minecraft:brush")
    {
        return false;
    }
    player.use_item(&state.registry.item_rules(), hand);
    true
}
fn set_dusted(state: &mut TickState, pos: BlockPos, count: u8) {
    let Some(block) = state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
    else {
        return;
    };
    let dust = match count {
        0 => "0",
        1..=2 => "1",
        3..=5 => "2",
        _ => "3",
    };
    let mut props = block.properties.clone();
    let Some((_, value)) = props.iter_mut().find(|(key, _)| key == "dusted") else {
        return;
    };
    if value == dust {
        return;
    }
    *value = dust.into();
    if let Some(next) = state
        .registry
        .blocks
        .by_name_and_props(&block.block.id, &props)
    {
        state.write_block(pos, next.0);
    }
}
fn loot(state: &mut TickState, pos: BlockPos, face: [i32; 3], reveal: bool) -> Option<ItemStack> {
    let mut fields = state
        .world
        .chunk(crate::boundary::ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        })
        .and_then(|chunk| chunk.block_entities.get(&pos))
        .and_then(|bytes| mc_nbt::read_network(&mut bytes.as_slice()).ok())
        .and_then(|tag| {
            if let Tag::Compound(fields) = tag {
                Some(fields)
            } else {
                None
            }
        })
        .unwrap_or_default();
    let stack = if let Some(Tag::String(table)) = field(&fields, "LootTable") {
        let table = state
            .registry
            .chest_loot
            .get(&Identifier::parse(table).ok()?)?;
        let seed = match field(&fields, "LootTableSeed") {
            Some(Tag::Long(seed)) if *seed != 0 => *seed as u64,
            _ => state.tick ^ (pos.x as u64).rotate_left(32) ^ pos.z as u64,
        };
        let Ok(items) = table.roll(&mut ChestRng::new(seed), &state.registry.items, 0.0) else {
            return None;
        };
        items.into_iter().next().unwrap_or(ItemStack::EMPTY)
    } else if let Some(Tag::Compound(item)) = field(&fields, "item") {
        stack_from_fields(&state.registry.items, item)
            .ok()?
            .unwrap_or(ItemStack::EMPTY)
    } else {
        ItemStack::EMPTY
    };
    if !reveal {
        return Some(stack);
    }
    for (key, value) in [
        ("id", Tag::String("minecraft:brushable_block".into())),
        ("x", Tag::Int(pos.x)),
        ("y", Tag::Int(pos.y)),
        ("z", Tag::Int(pos.z)),
    ] {
        if field(&fields, key).is_none() {
            fields.push((key.into(), value));
        }
    }
    fields.retain(|(key, _)| {
        !matches!(
            key.as_str(),
            "LootTable" | "LootTableSeed" | "item" | "hit_direction"
        )
    });
    if !stack.is_empty() {
        fields.push((
            "item".into(),
            Tag::Compound(stack_to_fields(&state.registry.items, &stack).ok()?),
        ));
    }
    let direction = match face {
        [0, -1, 0] => 0,
        [0, 1, 0] => 1,
        [0, 0, -1] => 2,
        [0, 0, 1] => 3,
        [-1, 0, 0] => 4,
        _ => 5,
    };
    fields.push(("hit_direction".into(), Tag::Int(direction)));
    let mut bytes = Vec::new();
    let nbt = Tag::Compound(fields);
    mc_nbt::write_network(&mut bytes, &nbt).expect("brush loot NBT");
    state
        .world
        .set_opaque_block_entity(pos, Some(bytes.clone()));
    state.opaque_changes.push((pos, Some(bytes)));
    state.world.mark_block_entity_visible(pos);
    let nbt = mc_world::wire::brushable_update_tag(nbt);
    let viewers: Vec<_> = state
        .players
        .values()
        .map(|player| player.connection)
        .collect();
    for connection in viewers {
        state.push_player(
            connection,
            crate::boundary::PlayerChange::BlockEntityData {
                position: pos,
                block_entity_type: mc_data::block_entity_kinds::BlockEntityKind::BrushableBlock
                    .protocol_id(),
                nbt: nbt.clone(),
            },
        );
    }
    Some(stack)
}
pub(crate) fn run(state: &mut TickState) {
    let strokes: Vec<_> = state
        .players
        .iter()
        .filter_map(|(&actor, p)| {
            p.brush_use(&state.registry.item_rules())
                .filter(|(_, elapsed)| elapsed % 10 == 5)
                .map(|(hand, _)| (actor, hand))
        })
        .collect();
    for (actor, hand) in strokes {
        let Some(slot) = state.entities.slot_of(actor) else {
            continue;
        };
        let mut eye = state.entities.pos(slot);
        eye[1] += crate::tick::player_eye_height(state.entities.row(slot).appearance.pose);
        let [yaw, pitch, _] = state.entities.rot(slot);
        let Some(hit) = crate::pov_ray::clip(state, eye, yaw, pitch, false) else {
            state
                .players
                .get_mut(&actor)
                .expect("brush user")
                .release_use_item();
            continue;
        };
        let pos = hit.pos;
        let key = [pos.x, pos.y, pos.z];
        let Some(sand) = brushable(state, pos) else {
            continue;
        };
        if !crate::plugin_player::allows_block(state, actor, pos)
            || crate::spawn_protection::protected(state, actor, pos)
        {
            continue;
        }
        let progress = state.brushing.progress.entry(key).or_insert(Progress {
            count: 0,
            cooldown: 0,
            reset: 0,
        });
        progress.reset = state.tick + 40;
        if progress.cooldown > state.tick {
            continue;
        }
        let count = progress.count;
        if count == 9 && crate::copper::prepare_tool_wear(state, actor, hand).is_err() {
            continue;
        }
        let item = if count == 0 || count == 9 {
            let Some(item) = loot(state, pos, hit.face, count == 0) else {
                continue;
            };
            item
        } else {
            ItemStack::EMPTY
        };
        let progress = state
            .brushing
            .progress
            .get_mut(&key)
            .expect("brush progress");
        progress.cooldown = state.tick + 10;
        progress.count += 1;
        let count = progress.count;
        if count < 10 {
            set_dusted(state, pos, count);
            continue;
        }
        let Ok(wear) = crate::copper::prepare_tool_wear(state, actor, hand) else {
            continue;
        };
        let next = state
            .registry
            .blocks
            .block(
                &Identifier::parse(if sand {
                    "minecraft:sand"
                } else {
                    "minecraft:gravel"
                })
                .unwrap(),
            )
            .expect("brush result")
            .default
            .0;
        state.write_block(pos, next);
        state.world.set_opaque_block_entity(pos, None);
        state.opaque_changes.push((pos, None));
        state.brushing.progress.remove(&key);
        if !item.is_empty() {
            let point = [
                f64::from(pos.x + hit.face[0]) + 0.5,
                f64::from(pos.y + hit.face[1]) + 0.5,
                f64::from(pos.z + hit.face[2]) + 0.5,
            ];
            let id = crate::tick::spawn_item_entity(state, point, item);
            state.hostile.spawned.push(id);
        }
        crate::copper::apply_tool_wear(state, actor, wear);
    }
    let mut progress = std::mem::take(&mut state.brushing.progress);
    progress.retain(|key, entry| {
        let pos = BlockPos {
            x: key[0],
            y: key[1],
            z: key[2],
        };
        // A non-resident cell is paused, not reset: `block_at` would answer from
        // the cold overlay and `set_dusted` would decrement and write into it.
        if !state
            .world
            .has_chunk(pos.x.div_euclid(16), pos.z.div_euclid(16))
        {
            return true;
        }
        if brushable(state, pos).is_none() {
            return false;
        }
        if state.tick >= entry.reset {
            entry.count = entry.count.saturating_sub(2);
            entry.reset = state.tick + 4;
            set_dusted(state, pos, entry.count);
        }
        entry.count != 0
    });
    state.brushing.progress = progress;
}

#[cfg(test)]
mod cold_residency_tests {
    //! Brush reset residency: a non-resident in-progress cell is paused, not
    //! reread through the cold overlay and decremented there.

    use super::*;
    use crate::boundary::{ChunkPos, PlayerInput};
    use crate::tick::{TickState, run_tick};
    use crate::world::ChunkWorld;
    use mc_data::Identifier;
    use mc_domain::ConnectionId;
    use std::sync::Arc;

    struct Empty(u32);
    impl mc_world::ChunkGenerator for Empty {
        fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
            mc_world::Chunk::empty(
                pos,
                mc_world::BlockStateId(self.0),
                Identifier::parse("minecraft:plains").unwrap(),
            )
        }
    }

    fn state_id(state: &TickState, name: &str, dusted: &str) -> u32 {
        let id = Identifier::parse(format!("minecraft:{name}")).expect("block id");
        let props = [("dusted".to_owned(), dusted.to_owned())];
        state
            .registry
            .blocks
            .by_name_and_props(&id, &props)
            .unwrap_or_else(|| panic!("registered {name} dusted={dusted}"))
            .0
    }

    fn fixture() -> (TickState, BlockPos) {
        let registry = Arc::new(crate::tick_tests::full_registry());
        let world = ChunkWorld::new(
            42,
            Arc::new(Empty(registry.states.air)),
            &registry,
            mc_world::BlockStateId(registry.states.air),
            [8, 65, 8],
            mc_world::OVERWORLD_GEOMETRY,
        );
        let mut state = TickState::new(registry, world, 0, 512);
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(1),
                uuid: uuid::Uuid::new_v4(),
                name: "brush".to_owned(),
                permission_level: 0,
            }],
        );
        state.world.ensure_blocking(&state.desired_chunks());
        (state, BlockPos { x: 8, y: 64, z: 8 })
    }

    #[test]
    fn brush_reset_pauses_on_a_cold_cell_instead_of_decrementing_its_overlay() {
        let (mut state, pos) = fixture();
        let clean = state_id(&state, "suspicious_sand", "0");
        let dusted = state_id(&state, "suspicious_sand", "1");
        state.world.set_block(pos.x, pos.y, pos.z, clean);
        state.world.fold_overlay();
        let key = [pos.x, pos.y, pos.z];
        let entry = || Progress {
            count: 5,
            cooldown: 0,
            reset: 0,
        };
        state.brushing.progress.insert(key, entry());
        // Resident cell: the due reset decrements and writes the dusted stage.
        run(&mut state);
        assert_eq!(state.brushing.progress[&key].count, 3);
        assert_eq!(
            state.world.block_at(pos.x, pos.y, pos.z),
            Some(state_id(&state, "suspicious_sand", "2"))
        );
        // Cold cell: the same due reset must pause and leave the overlay alone.
        state.brushing.progress.insert(key, entry());
        let chunk = ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        };
        state.world.finish_inhabited_tick(state.tick + 1);
        state
            .world
            .set_active_chunks(Default::default(), state.tick + 1);
        state.world.capture_dirty(&Default::default());
        state
            .world
            .mark_flushed(&[(chunk, state.world.storage_revision(chunk))]);
        assert!(
            state
                .world
                .evict_outside(&Default::default())
                .contains(&chunk)
        );
        assert!(!state.world.has_chunk(chunk.x, chunk.z));
        state.world.set_block(pos.x, pos.y, pos.z, dusted);
        for _ in 0..3 {
            state.tick += 4;
            run(&mut state);
            assert_eq!(
                state.brushing.progress[&key].count, 5,
                "a cold cell keeps its progress"
            );
            assert_eq!(
                state.world.block_at(pos.x, pos.y, pos.z),
                Some(dusted),
                "no dusted stage is written into the cold overlay"
            );
        }
    }
}
