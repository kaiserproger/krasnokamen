use super::*;
use crate::datapacks::{PackSet, Selection};

#[test]
fn atm_container_vehicle_pack_removed_open_hopper_and_death_use_empty_once() {
    for operation in ["open", "pull", "hopper_tick", "death"] {
        for removed in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let root = directory.path().join("a");
            std::fs::create_dir_all(root.join("data/demo/loot_table")).unwrap();
            std::fs::write(
                root.join("pack.mcmeta"),
                r#"{"pack":{"pack_format":121,"description":"vehicle reload test"}}"#,
            )
            .unwrap();
            std::fs::write(root.join("data/demo/loot_table/vehicle.json"),
                r#"{"type":"minecraft:chest","pools":[{"rolls":1,"entries":[{"type":"minecraft:item","name":"minecraft:iron_ingot","modifier":[{"type":"minecraft:set_count","count":7}]}]}]}"#).unwrap();
            let mut state = fixture();
            let actor = player(&mut state, [0.5, 65.0, 0.5]);
            let base = Arc::clone(&state.registry);
            let mut registry = base.prepare_content();
            PackSet::load(directory.path(), Selection::default())
                .unwrap()
                .install_content(&mut registry)
                .unwrap();
            state.registry = Arc::new(registry);
            let name = if operation == "hopper_tick" {
                "hopper_minecart"
            } else {
                "chest_minecart"
            };
            let cart = mob(&mut state, name, [0.5, 65.0, 0.5]);
            let slot = state.entities.slot_of(cart).unwrap();
            let vehicle = state
                .entities
                .row_mut(slot)
                .entity_life
                .vehicle
                .as_mut()
                .unwrap();
            vehicle.loot_table = Some(Identifier::parse("demo:vehicle").unwrap());
            vehicle.loot_seed = -17;
            loot::validate_items(vehicle, &state.registry.chest_loot, &state.registry.items)
                .unwrap();
            if removed {
                // Delete the pack after the unopened vehicle passed admission.
                std::fs::remove_dir_all(&root).unwrap();
                let mut registry = base.prepare_content();
                PackSet::load(directory.path(), Selection::default())
                    .unwrap()
                    .install_content(&mut registry)
                    .unwrap();
                assert!(
                    registry
                        .chest_loot
                        .get(&Identifier::parse("demo:vehicle").unwrap())
                        .is_none()
                );
                state.registry = Arc::new(registry);
            }
            state.block_entities.insert(
                (0, 64, 0),
                crate::tick::BlockEntityState {
                    slots: vec![ItemStack::EMPTY; 5],
                    furnace: None,
                },
            );
            let log = tempfile::NamedTempFile::new().unwrap();
            let writer = log.reopen().unwrap();
            let subscriber = tracing_subscriber::fmt()
                .without_time()
                .with_ansi(false)
                .with_writer(move || writer.try_clone().unwrap())
                .finish();
            let mut spawned = Vec::new();
            tracing::subscriber::with_default(subscriber, || {
                match operation {
                    "open" => {
                        containers::open(&mut state, actor, cart, &mut Default::default());
                        assert!(state.players[&actor].open.is_some());
                    }
                    "pull" => assert_eq!(
                        pull_into_hopper(
                            &mut state,
                            BlockPos { x: 0, y: 65, z: 0 },
                            BlockPos { x: 0, y: 64, z: 0 }
                        ),
                        !removed
                    ),
                    "hopper_tick" => {
                        containers::tick_hopper(&mut state, slot, &mut Default::default())
                    }
                    "death" => {
                        assert!(death(&mut state, slot, &mut spawned));
                        assert!(state.entities.row(slot).loot.death_handled);
                        assert_eq!(spawned.len(), if removed { 1 } else { 2 });
                        let before = spawned.len();
                        assert!(death(&mut state, slot, &mut spawned));
                        assert_eq!(spawned.len(), before);
                    }
                    _ => unreachable!(),
                }
                let vehicle = state
                    .entities
                    .row(slot)
                    .entity_life
                    .vehicle
                    .as_ref()
                    .unwrap();
                assert!(
                    vehicle.loot_table.is_none(),
                    "{operation}, removed={removed}"
                );
                assert_eq!(vehicle.loot_seed, 0);
                let slots = vehicle.slots.clone();
                loot::materialize(&mut state, slot);
                assert_eq!(
                    state
                        .entities
                        .row(slot)
                        .entity_life
                        .vehicle
                        .as_ref()
                        .unwrap()
                        .slots,
                    slots
                );
            });
            let log = std::fs::read_to_string(log.path()).unwrap();
            assert_eq!(
                log.matches("Missing container loot table").count(),
                usize::from(removed),
                "{operation}: {log}"
            );
            if removed {
                assert!(log.contains("demo:vehicle"), "{log}");
            }
            let iron = item(&state, "iron_ingot");
            let vehicle = state
                .entities
                .row(slot)
                .entity_life
                .vehicle
                .as_ref()
                .unwrap();
            let stored: i32 = vehicle
                .slots
                .iter()
                .chain(state.block_entities[&(0, 64, 0)].slots.iter())
                .filter(|stack| stack.item_id == iron)
                .map(|stack| stack.count)
                .sum();
            let dropped: i32 = spawned
                .iter()
                .filter_map(|id| state.entities.slot_of(*id))
                .filter_map(|slot| state.entities.row(slot).item.as_ref())
                .filter(|stack| stack.item_id == iron)
                .map(|stack| stack.count)
                .sum();
            assert_eq!(stored + dropped, if removed { 0 } else { 7 }, "{operation}");
        }
    }
}
