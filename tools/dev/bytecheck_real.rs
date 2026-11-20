//! Above-core real-image byte probe. The runner supplies a separate disposable
//! input for each operation; no operation opens the retained source image.
use mc_core::native_checkpoint::{encode_checkpoint, recover_offline};
use mc_core::{Core, CoreConfig};
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let mode = &args[1];
    let input = PathBuf::from(&args[2]);
    let out = PathBuf::from(&args[3]);
    let mut config = CoreConfig::new(input.clone(), args[4].parse()?);
    config.vanilla_data_dir = Some(PathBuf::from(&args[5]));
    config.plugin_directory = Some(PathBuf::from(&args[6]));
    config.start_frozen = true;
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    config.worldgen_gpu = false;
    match mode.as_str() {
        "repair" => {
            // Base pin only, on the runner's disposable raw copy. This is
            // redb's supported crash repair, not a tick or a checkpoint fold.
            drop(mc_world::native::NativeStore::open(
                &input.join("native/state.redb"),
                mc_core::native_checkpoint::native_identity(&config),
            )?);
        }
        "fold" => {
            let world = recover_offline(&config)?;
            let image = encode_checkpoint(&world)?;
            assert!(
                image.tick > world.manifest.tick,
                "real image needs a WAL tail"
            );
            let changed = image
                .globals
                .iter()
                .filter(|(key, row)| {
                    key.starts_with("settlement/")
                        && world.checkpoint.globals.get(*key) != Some(*row)
                })
                .count();
            assert!(
                image
                    .globals
                    .keys()
                    .any(|key| key.starts_with("settlement/")),
                "real image must contain settlement rows"
            );
            let clocks: BTreeMap<_, _> = world
                .dimensions
                .iter()
                .map(|(dimension, replay)| (dimension.name(), &replay.climate))
                .collect();
            fs::write(out.join("game-clocks.txt"), serde_json::to_vec(&clocks)?)?;
            fs::write(
                out.join("scope.json"),
                format!(
                    "{{\"checkpoint_tick\":{},\"durable_tick\":{},\"changed_settlement_rows\":{}}}\n",
                    world.manifest.tick, image.tick, changed
                ),
            )?;
            let mut rows = BTreeMap::new();
            for (key, row) in image.globals {
                rows.insert(format!("global/{key}"), format!("{row:?}"));
            }
            for player in image.players {
                rows.insert(
                    format!("player/{:02x?}", player.uuid),
                    format!("{:?}", player.record),
                );
            }
            for entity in image.entities {
                let payload = entity.payload.as_ref().map(|tag| {
                    let mut bytes = Vec::new();
                    mc_nbt::write_named(&mut bytes, "", tag).unwrap();
                    bytes
                });
                rows.insert(
                    format!("entity/{:02x?}", entity.identity),
                    format!(
                        "{:?}",
                        (
                            entity.journal_lsn,
                            payload,
                            entity.dimension,
                            [entity.position.x, entity.position.z],
                            entity.generated_slot
                        )
                    ),
                );
            }
            fs::write(out.join("fold.txt"), format!("{rows:#?}"))?;
        }
        "export" => {
            let receipt = mc_world_conversion::export_anvil(
                &config,
                &input,
                &out.join("export"),
                &out.join("export-receipt.json"),
            )?;
            assert_eq!(receipt.status, "PASS");
        }
        "custody" => {
            let receipt = mc_world_conversion::custody_census(&config);
            assert_eq!(receipt.status, "PASS", "{:?}", receipt.error);
            fs::write(out.join("custody.txt"), format!("{receipt:#?}"))?;
        }
        "crash" => {
            let receipt = mc_world_conversion::crash_census(&config)?;
            fs::write(out.join("crash.txt"), serde_json::to_vec(&receipt)?)?;
        }
        "census" => {
            // The ordinary census intentionally refuses a WAL tail. Use the
            // normal frozen Core startup/shutdown checkpoint, not compact.
            let core = Core::open(config.clone())?;
            core.shutdown();
            drop(core);
            let receipt = mc_world_conversion::census_native(
                &config,
                &input,
                &out.join("census-receipt.json"),
            )?;
            assert_eq!(receipt.status, "PASS");
            // Backing-file CRCs and the absolute source path are provenance,
            // not row bytes. Preserve the complete receipt alongside this
            // deterministic census projection, including its checkpoint tick.
            fs::write(
                out.join("census.txt"),
                format!(
                    "{:#?}",
                    (
                        receipt.checkpoint_tick,
                        receipt.native_schema,
                        receipt.counts,
                        receipt.dimensions,
                        receipt.settlement_details
                    )
                ),
            )?;
        }
        _ => panic!("unknown real-image operation"),
    }
    Ok(())
}
