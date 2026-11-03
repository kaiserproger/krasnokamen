//! One block-state-driven creation boundary for commands and normal placement.
use crate::boundary::{BlockPos, ChunkPos};
use crate::tick::{BlockEntityState, TickState};
use mc_data::ItemStack;
use mc_data::block_entity_kinds::BlockEntityKind;
use mc_nbt::Tag;

/// Accepted placement creates a new owner even if break/re-place folds to the
/// same final block state. Property-only writes never enter this boundary.
pub(crate) fn accepted_placement(state: &mut TickState, pos: BlockPos, written: u32) {
    if state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(written))
        .and_then(|block| BlockEntityKind::for_block(block.block.id.path()))
        .is_some()
    {
        state.stage_container_removal(pos, false);
    }
}

/// Capture the accepted placement stack before its inventory claim is debited.
pub(crate) fn stage_banner(
    state: &mut TickState,
    intent: &crate::intent::Intent,
) -> Result<(), ()> {
    let crate::intent::IntentKind::PlaceBlock { pos, state: block } = intent.kind else {
        return Ok(());
    };
    if state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .and_then(|entry| BlockEntityKind::for_block(entry.block.id.path()))
        != Some(BlockEntityKind::Banner)
    {
        return Ok(());
    }
    let Some(stack) = intent
        .footprint
        .inv
        .iter()
        .find(|claim| claim.owner == intent.actor)
        .and_then(|claim| {
            state
                .players
                .get(&claim.owner)
                .and_then(|player| player.inventory.slots.get(usize::from(claim.slot)))
        })
    else {
        return Ok(());
    };
    let patterns = &stack.extended().banner_patterns;
    if patterns.is_empty() {
        return Ok(());
    }
    let layers = patterns
        .iter()
        .map(|layer| {
            let color = usize::try_from(layer.color)
                .ok()
                .and_then(|index| mc_data::stations::DYE_COLORS.get(index))
                .ok_or(())?;
            Ok(Tag::Compound(vec![
                ("pattern".into(), Tag::String(layer.pattern.as_str().into())),
                ("color".into(), Tag::String((*color).into())),
            ]))
        })
        .collect::<Result<Vec<_>, ()>>()?;
    let image = Tag::Compound(vec![
        (
            "id".into(),
            Tag::String(BlockEntityKind::Banner.vanilla_id().into()),
        ),
        ("x".into(), Tag::Int(pos.x)),
        ("y".into(), Tag::Int(pos.y)),
        ("z".into(), Tag::Int(pos.z)),
        (
            "patterns".into(),
            Tag::List(mc_nbt::ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: layers,
            }),
        ),
    ]);
    let mut bytes = Vec::new();
    mc_nbt::write_network(&mut bytes, &image).map_err(|_| ())?;
    state.merge_banner_placements.push((pos, block, bytes));
    Ok(())
}

pub(crate) fn commit_banners(state: &mut TickState) {
    for (pos, block, bytes) in std::mem::take(&mut state.merge_banner_placements) {
        if state.world.block_at(pos.x, pos.y, pos.z) != Some(block) {
            continue;
        }
        state
            .world
            .set_opaque_block_entity(pos, Some(bytes.clone()));
        state.world.mark_block_entity_visible(pos);
        let Tag::Compound(mut fields) =
            mc_nbt::read_network(&mut bytes.as_slice()).expect("staged banner NBT decodes")
        else {
            unreachable!("staged banner is a compound");
        };
        fields.retain(|(key, _)| !matches!(key.as_str(), "id" | "x" | "y" | "z"));
        let nbt = Tag::Compound(fields);
        let viewers: Vec<_> = state
            .players
            .values()
            .map(|player| player.connection)
            .collect();
        for viewer in viewers {
            // The reliable outbox precedes best-effort world updates. Install
            // the block first so the client can accept its block-entity data.
            state.push_player(
                viewer,
                crate::boundary::PlayerChange::BlockResynced {
                    position: pos,
                    block,
                },
            );
            state.push_player(
                viewer,
                crate::boundary::PlayerChange::BlockEntityData {
                    position: pos,
                    block_entity_type: BlockEntityKind::Banner.protocol_id(),
                    nbt: nbt.clone(),
                },
            );
        }
        state.opaque_changes.push((pos, Some(bytes)));
    }
}

pub(crate) fn after_blocks(state: &mut TickState, writes: &[(BlockPos, u32)]) {
    for &(pos, written) in writes {
        if state.world.block_at(pos.x, pos.y, pos.z) != Some(written) {
            continue;
        }
        let Some(kind) = state
            .registry
            .blocks
            .by_id(mc_world::BlockStateId(written))
            .and_then(|block| BlockEntityKind::for_block(block.block.id.path()))
        else {
            continue;
        };
        let chunk = ChunkPos {
            x: pos.x.div_euclid(16),
            z: pos.z.div_euclid(16),
        };
        if state
            .world
            .chunk(chunk)
            .is_none_or(|chunk| chunk.block_entities.contains_key(&pos))
        {
            continue;
        }
        if let Some(size) = kind.inventory_size() {
            state
                .block_entities
                .entry((pos.x, pos.y, pos.z))
                .or_insert_with(|| BlockEntityState {
                    slots: vec![ItemStack::EMPTY; size],
                    furnace: matches!(
                        kind,
                        BlockEntityKind::Furnace
                            | BlockEntityKind::Smoker
                            | BlockEntityKind::BlastFurnace
                    )
                    .then(Default::default),
                });
        }
        let image = Tag::Compound(vec![
            ("id".into(), Tag::String(kind.vanilla_id().into())),
            ("x".into(), Tag::Int(pos.x)),
            ("y".into(), Tag::Int(pos.y)),
            ("z".into(), Tag::Int(pos.z)),
        ]);
        let mut bytes = Vec::new();
        mc_nbt::write_network(&mut bytes, &image).expect("native block entity defaults encode");
        state
            .world
            .set_opaque_block_entity(pos, Some(bytes.clone()));
        state.opaque_changes.push((pos, Some(bytes)));
    }
}

#[cfg(test)]
#[path = "block_entity_defaults_tests.rs"]
mod tests;
