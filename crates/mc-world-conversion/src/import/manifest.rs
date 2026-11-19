//! Checkpoint manifest images and upgrade/deferral receipts.

use super::*;

pub(super) fn image(
    manifest: &mut CheckpointManifest,
    dimension: Dimension,
) -> &mut CheckpointManifest {
    if dimension == Dimension::Overworld {
        manifest
    } else {
        manifest
            .dimensions
            .get_mut(&dimension)
            .expect("initialized child")
    }
}

pub(super) fn ticks(
    chunk: &mut Chunk,
    manifest: &mut CheckpointManifest,
    registry: &mc_core::registry::CoreRegistry,
) -> Result<(), CoreError> {
    let key = [chunk.pos.x, chunk.pos.z];
    let mut block = Vec::with_capacity(chunk.scheduled_block_ticks.len());
    for tick in &mut chunk.scheduled_block_ticks {
        if registry.blocks.block(&tick.block).is_none() {
            return Err(fail(format!(
                "chunk {key:?}: unknown scheduled block {}",
                tick.block
            )));
        }
        let sequence = manifest.block_ticks.next_sequence;
        manifest.block_ticks.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| fail("block tick sequence overflow"))?;
        block.push(mc_core::block_ticks::BlockTickImage {
            pos: [tick.pos.x, tick.pos.y, tick.pos.z],
            block: tick.block.clone(),
            trigger_tick: tick.trigger_tick,
            priority: tick.priority,
            sequence,
        });
        *tick = mc_world::ScheduledBlockTick::from_storage(
            tick.pos,
            tick.block.clone(),
            tick.trigger_tick,
            tick.priority,
            sequence,
        );
    }
    manifest
        .block_ticks
        .chunks
        .push(mc_core::block_ticks::BlockTickChunkImage {
            chunk: key,
            ticks: block,
        });
    manifest.fluid_ticks.covered.push(key);
    for tick in &mut chunk.scheduled_fluid_ticks {
        if mc_world::ScheduledFluidTick::fluid_kind(&tick.fluid).is_none() {
            return Err(fail(format!("unknown scheduled fluid {}", tick.fluid)));
        }
        let sequence = manifest.fluid_ticks.next_sequence;
        manifest.fluid_ticks.next_sequence = sequence
            .checked_add(1)
            .ok_or_else(|| fail("fluid tick sequence overflow"))?;
        manifest
            .fluid_ticks
            .due
            .push(mc_core::fluids::FluidTickImage {
                pos: [tick.pos.x, tick.pos.y, tick.pos.z],
                fluid: tick.fluid.clone(),
                deadline: tick.trigger_tick,
                priority: tick.priority,
                sequence,
            });
        *tick = mc_world::ScheduledFluidTick::from_storage(
            tick.pos,
            tick.fluid.clone(),
            tick.trigger_tick,
            tick.priority,
            sequence,
        );
    }
    Ok(())
}

pub(super) fn record_deferred(
    stats: &mut ConversionStats,
    record: String,
    destination: &Path,
    kind: &str,
    version: i32,
) {
    stats.upgrades.push(RecordUpgradeReceipt {
        record,
        kind: kind.into(),
        from: version,
        to: version,
        steps: Vec::new(),
        preserved: vec![super::super::UpgradePreservationReceipt {
            path: destination.to_string_lossy().into_owned(),
            reason: "original file bytes and DataVersion retained without execution; saved-data NBT transformation is deferred to vanilla load, not a vanilla DFU execution or gameplay-parity claim".into(),
        }],
        saved_data: Vec::new(),
    });
}

pub(super) fn record_upgrade(
    stats: &mut ConversionStats,
    record: String,
    kind: RecordKind,
    report: &UpgradeReport,
    mapping: Option<String>,
) {
    let mut receipt = RecordUpgradeReceipt::from_report(record, kind, report);
    if let Some(reason) = mapping {
        receipt
            .preserved
            .push(super::super::UpgradePreservationReceipt {
                path: receipt.record.clone(),
                reason,
            });
    }
    stats.upgrades.push(receipt);
}
