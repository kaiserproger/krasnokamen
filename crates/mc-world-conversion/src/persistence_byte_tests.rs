//! Above-core consumers of the D65 fixture. Each input is a fresh BASE copy;
//! no Core::open, tick loop, compact or reverse dev-dependency is involved.
use super::*;
use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint::{encode_checkpoint, recover_offline};
use serde_json::json;

fn config(path: PathBuf) -> CoreConfig {
    let mut config = crate::test_support::config(path, 42);
    config.view_distance = 1;
    config.simulation_distance = 0;
    config
}

#[test]
#[ignore = "requires the base-world copies supplied by tools/dev/bytecheck.sh"]
fn persistence_byte_consumers() {
    let out =
        PathBuf::from(std::env::var_os("CORE_UNLOAD_OUT").expect("bytecheck output directory"));
    let offline_config = config(out.join("offline-input"));
    let world = recover_offline(&offline_config).unwrap();
    assert_eq!(world.manifest.tick, 1);
    assert_eq!(world.dimensions[&Dimension::Overworld].tick, 3);
    let image = encode_checkpoint(&world).unwrap();
    let stable_uuid: uuid::Uuid =
        serde_json::from_slice(&fs::read(out.join("reopen-transit.json")).unwrap()).unwrap();
    let stable_key = format!("transit/{stable_uuid}");
    assert!(image.globals.contains_key(&stable_key));
    assert_eq!(
        image.globals[&stable_key].payload,
        world.checkpoint.globals[&stable_key].payload
    );
    let mut rows = serde_json::Map::new();
    for (key, record) in image.globals {
        rows.insert(format!("global/{key}"),json!({"version":record.version,"captured_tick":record.captured_tick,"payload":record.payload}));
    }
    assert!(image.players.is_empty());
    assert!(!image.entities.is_empty());
    for entity in image.entities {
        let mut payload = Vec::new();
        mc_nbt::write_named(&mut payload, "", entity.payload.as_ref().unwrap()).unwrap();
        rows.insert(format!("entity/{}",uuid::Uuid::from_bytes(entity.identity)),json!({
            "version":1,"captured_tick":entity.journal_lsn,"payload":payload,
            "dimension":entity.dimension,"position":[entity.position.x,entity.position.z],"generated_slot":entity.generated_slot
        }));
    }
    fs::write(
        out.join("C-offline.json"),
        serde_json::to_vec(&rows).unwrap(),
    )
    .unwrap();
    drop(world);
    let custody = crate::custody_census(&config(out.join("custody-input")));
    assert_eq!(custody.status, "PASS", "{:?}", custody.error);
    assert_eq!(custody.checkpoint_tick, Some(1));
    assert_eq!(custody.durable_tick, Some(3));
    assert!(!custody.villagers.is_empty());
    assert_eq!(custody.transits.len(), 1);
    assert_eq!(
        custody.transits[&stable_uuid.to_string()].members,
        vec![stable_uuid.to_string()]
    );
    fs::write(
        out.join("D-custody.json"),
        serde_json::to_vec(&custody).unwrap(),
    )
    .unwrap();
    let crash = crate::crash_census(&config(out.join("crash-input"))).unwrap();
    assert_eq!(crash.checkpoint_tick, 1);
    assert_eq!(crash.durable_tick, 3);
    assert!(!crash.entities.is_empty());
    fs::write(
        out.join("D-crash.json"),
        serde_json::to_vec(&crash).unwrap(),
    )
    .unwrap();
    let export = out.join("D-export");
    fs::create_dir(&export).unwrap();
    let export_config = config(out.join("export-input"));
    let stats = crate::export::convert(&export_config, &export_config.world_dir, &export).unwrap();
    assert_eq!(stats.tick, 3);
    assert!(export.join("level.dat").is_file());
}
