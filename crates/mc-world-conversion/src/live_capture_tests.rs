//! Portable checkpoint round trips and Linux abrupt-exit checkpoint + WAL tests.
//! Every mutation is queued through Core; no TickState or capture API is exposed.
use crate::test_support::Running;
use mc_core::boundary::{ClickMode, ConnectionId, EntityId, Hand, PlayerChange, PlayerInput};
use mc_core::dimensions::Dimension;
use mc_core::native_checkpoint as bridge;
use mc_core::villager::{VillagerProfession, VillagerState, VillagerTradeOffer};
use mc_data::{Identifier, ItemStack};
use mc_nbt::Tag;
use mc_world::native::NativeCheckpoint;
use mc_world::{BlockStateId, Chunk, ChunkPos};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;

const CONNECTION: ConnectionId = ConnectionId(1);
const PLAYER: uuid::Uuid = uuid::Uuid::from_u128(0x57414c);
const TRADER: uuid::Uuid = uuid::Uuid::from_u128(0x761);
const VILLAGER: uuid::Uuid = uuid::Uuid::from_u128(0x706f69);

#[derive(Clone, Copy, Serialize, Deserialize)]
enum Domain {
    Trader,
    Warning,
    Claim,
    Inhabited,
}

#[derive(Serialize, Deserialize)]
struct Evidence {
    checkpoint_tick: u64,
    through_tick: u64,
    expected: Expected,
}

#[derive(Serialize, Deserialize)]
enum Expected {
    Trader,
    Warning { since: u32 },
    Claim { checkpoint_position: [f64; 3] },
    Inhabited { checkpoint_value: i64, value: i64 },
}

fn config(root: &Path) -> mc_core::CoreConfig {
    let mut config = crate::test_support::config(root.to_path_buf(), 42);
    config.view_distance = 0;
    config.simulation_distance = 0;
    config.checkpoint_every_ticks = u64::MAX;
    config.journal_fsync_every_batches = 1;
    // One bounded sprint can publish 1200 ticks before this observer is scheduled.
    config.broadcast_cap = 2048;
    config
}

fn fixture(root: &Path, domain: Domain) {
    let config = config(root);
    let registry = Arc::new(crate::test_support::registry().unwrap());
    let store = crate::test_support::fixture_store(&config).unwrap();
    // Populate the complete sim0/view0 residency margin before starting Core.
    let mut chunks = Vec::new();
    for x in -1..=1 {
        for z in -1..=1 {
            let mut chunk = Chunk::empty(
                ChunkPos { x, z },
                BlockStateId(registry.states.air),
                Identifier::parse("minecraft:plains").unwrap(),
            );
            for bx in 0..16 {
                for bz in 0..16 {
                    chunk
                        .set_block(bx, 79, bz, BlockStateId(registry.states.stone))
                        .unwrap();
                }
            }
            chunks.push(chunk);
        }
    }
    store
        .fill_generated(0, &chunks, &registry.blocks, &registry.items)
        .unwrap();
    let mut recovered = bridge::recover_with_reader(
        &config,
        Arc::clone(&registry),
        store.checkpoint_reader().unwrap(),
    )
    .unwrap();
    for replay in recovered.dimensions.values_mut() {
        replay.tick = 24_000;
    }
    let replay = recovered.dimensions.get_mut(&Dimension::Overworld).unwrap();
    let entities = match domain {
        Domain::Trader => vec![
            (1, "minecraft:wandering_trader", TRADER, [2.0, 80.0, 2.0]),
            (
                2,
                "minecraft:trader_llama",
                uuid::Uuid::from_u128(0x762),
                [3.0, 80.0, 2.0],
            ),
            (
                3,
                "minecraft:trader_llama",
                uuid::Uuid::from_u128(0x763),
                [4.0, 80.0, 2.0],
            ),
        ],
        Domain::Claim => vec![(
            1,
            "minecraft:villager",
            VILLAGER,
            [3.322834405812458, 80.0, 3.057086014531166],
        )],
        _ => Vec::new(),
    };
    for (wire_id, id, uuid, pos) in entities {
        let entity = mc_data::entity_nbt::VanillaEntity {
            id: id.into(),
            uuid,
            wire_id,
            pos,
            health: 20.0,
            extra_fields: if matches!(domain, Domain::Trader) {
                vec![("NoAI".into(), Tag::Byte(1))]
            } else {
                Vec::new()
            },
            ..Default::default()
        };
        let payload =
            Tag::Compound(mc_data::entity_nbt::entity_to_fields(&registry.items, &entity).unwrap());
        bridge::hydrate_entity(&registry, replay, &payload, 24_000).unwrap();
    }
    if matches!(domain, Domain::Trader) {
        let item = |name| {
            registry
                .items
                .id_of(&Identifier::parse(name).unwrap())
                .unwrap()
        };
        let mut trader = VillagerState::default();
        trader.wandering = true;
        trader.data_finalized = true;
        trader.despawn_at = Some(72_000);
        trader.offers.push(VillagerTradeOffer {
            cost_a: ItemStack::new(item("minecraft:emerald"), 5),
            cost_b: None,
            result: ItemStack::new(item("minecraft:blue_ice"), 1),
            uses: 0,
            max_uses: 6,
            reward_exp: false,
            xp: 1,
            special_price: 0,
            demand: 0,
            price_multiplier: 0.05,
        });
        replay.entities.get_mut(&1).unwrap().villager = Some(Box::new(trader));
        for id in [2, 3] {
            replay
                .entities
                .get_mut(&id)
                .unwrap()
                .state
                .entity_life
                .leash = Some(mc_core::leash::Holder::Entity(TRADER));
        }
        let clock = replay.wandering_trader.get_or_insert_with(Default::default);
        clock.delay = 12_345;
        clock.chance = 50;
    }
    let image = bridge::encode_checkpoint(&recovered).unwrap();
    store
        .commit_checkpoint(
            NativeCheckpoint {
                tick: 24_000,
                previous_tick: 0,
                chunks: &[],
                entities: &image.entities,
                players: &image.players,
                globals: &image.globals,
            },
            &registry.blocks,
            &registry.items,
        )
        .unwrap();
}

async fn command(live: &Running, text: impl Into<String>) -> u64 {
    let (command, completed) = mc_core::command_console::ConsoleCommand::new(text.into());
    assert!(live.core.submit(PlayerInput::ConsoleCommand(command)));
    tokio::time::timeout(std::time::Duration::from_secs(60), completed)
        .await
        .expect("console execution deadline")
        .expect("live command owner")
}

async fn sprint(live: &mut Running, ticks: u32) -> u64 {
    let started = command(live, format!("tick sprint {ticks}")).await;
    live.until(|output, batches| {
        (output.tick >= started
            && batches
                .iter()
                .flat_map(|(_, changes)| changes)
                .any(|change| matches!(change, PlayerChange::TickingState { frozen: true, .. })))
        .then_some(output.tick)
    })
    .await
}

async fn game_mode(live: &mut Running, name: &str, id: u8) -> u64 {
    let acknowledged = command(live, format!("gamemode {name} Capture")).await;
    live.until(|output, batches| {
        (output.tick >= acknowledged
            && batches.iter().flat_map(|(_, changes)| changes).any(|change| {
                matches!(change, PlayerChange::GameModeChanged { game_mode } if *game_mode == id)
            }))
        .then_some(output.tick)
    })
    .await
}

async fn join(live: &mut Running) -> EntityId {
    assert!(live.core.submit(PlayerInput::Join {
        connection: CONNECTION,
        uuid: PLAYER,
        name: "Capture".into(),
        permission_level: 4,
    }));
    live.until(|_, batches| {
        batches
            .iter()
            .filter(|(id, _)| *id == CONNECTION)
            .flat_map(|(_, changes)| changes)
            .find_map(|change| match change {
                PlayerChange::Joined { entity, .. } => Some(*entity),
                _ => None,
            })
    })
    .await
}

async fn leave(live: &mut Running, actor: EntityId) -> u64 {
    assert!(live.core.submit(PlayerInput::Leave {
        connection: CONNECTION
    }));
    live.until(|output, _| {
        output
            .changes
            .entities_removed
            .contains(&actor)
            .then_some(output.tick)
    })
    .await
}

async fn trade(live: &mut Running) -> u64 {
    assert!(live.core.submit(PlayerInput::InteractEntity {
        connection: CONNECTION,
        target: EntityId(1),
        hand: Hand::Main,
        location: [0.0; 3],
    }));
    live.until(|_, batches| {
        batches
            .iter()
            .flat_map(|(_, changes)| changes)
            .find_map(|change| match change {
                PlayerChange::MerchantOffers { offers, .. } => {
                    assert_eq!(offers[0].uses, 0);
                    Some(())
                }
                _ => None,
            })
    })
    .await;
    assert!(live.core.submit(PlayerInput::SelectTrade {
        connection: CONNECTION,
        offer: 0
    }));
    let registry = crate::test_support::registry().unwrap();
    let ice = registry
        .items
        .id_of(&Identifier::parse("minecraft:blue_ice").unwrap())
        .unwrap();
    let (window, state_id) = live
        .until(|_, batches| {
            batches
                .iter()
                .flat_map(|(_, changes)| changes)
                .find_map(|change| match change {
                    PlayerChange::MenuContents {
                        window,
                        state_id,
                        slots,
                        ..
                    } if *window != 0 && slots.get(2) == Some(&Some(ItemStack::new(ice, 1))) => {
                        Some((*window, *state_id))
                    }
                    _ => None,
                })
        })
        .await;
    assert!(live.core.submit(PlayerInput::ContainerClick {
        connection: CONNECTION,
        window,
        state_id,
        slot: 2,
        button: 0,
        mode: ClickMode::QuickMove,
    }));
    live.until(|output, batches| {
        batches
            .iter()
            .flat_map(|(_, changes)| changes)
            .any(|change| {
                matches!(change, PlayerChange::MenuContents { window: w, slots, .. }
            if *w == window && slots.get(2) == Some(&None)
                && slots.iter().skip(3).flatten().filter(|stack| stack.item_id == ice)
                    .map(|stack| stack.count).sum::<i32>() == 1)
            })
            .then_some(output.tick)
    })
    .await
}

async fn warn(live: &mut Running) -> u64 {
    command(live, "tp Capture 4.5 80 4.5").await;
    let started = command(live, "tick step 1").await;
    let tick = live
        .until(|output, batches| {
            (output.tick >= started
                && batches
                    .iter()
                    .flat_map(|(_, changes)| changes)
                    .any(|change| {
                        matches!(
                            change,
                            PlayerChange::ProjectileLevelEvent { event: 3007, .. }
                        )
                    }))
            .then_some(output.tick)
        })
        .await;
    command(live, "tp Capture 0.5 80 0.5").await;
    tick
}

async fn scenario(root: &Path, domain: Domain) -> (Running, Evidence) {
    fixture(root, domain);
    let mut live = Running::open(config(root));
    command(&live, "tick freeze").await;
    let actor = join(&mut live).await;
    command(&live, "tp Capture 0.5 80 0.5").await;
    for text in [
        "gamerule spawn_mobs false",
        "gamerule advance_time false",
        "gamerule advance_weather false",
        "difficulty normal",
    ] {
        command(&live, text).await;
    }
    match domain {
        Domain::Trader => {
            command(&live, "give Capture minecraft:emerald 5").await;
        }
        Domain::Warning => {
            command(
                &live,
                "setblock 4 80 4 minecraft:sculk_shrieker[can_summon=true]",
            )
            .await;
        }
        Domain::Claim => {
            command(&live, "time set 0").await;
        }
        Domain::Inhabited => {
            sprint(&mut live, 12).await;
        }
    }
    // The backup is a real Core checkpoint plus an immutable offline-readable
    // copy. Its checkpoint, not its optional newer WAL, defines the baseline.
    let baseline = root.parent().unwrap().join("baseline");
    std::fs::create_dir(&baseline).unwrap();
    let backup = live.core.backup(&baseline).unwrap();
    let baseline_config = config(&baseline);
    let store = crate::test_support::fixture_store(&baseline_config).unwrap();
    let manifest = crate::test_support::read_manifest(&store).unwrap();
    assert_eq!(manifest.tick, backup.checkpoint_tick);
    let checkpoint_tick = manifest.tick;
    let checkpoint_value = manifest
        .inhabited_time
        .iter()
        .find(|image| image.chunk == [0, 0])
        .map_or(0, |image| {
            assert_eq!(image.tick, checkpoint_tick);
            image.value
        });
    drop(store);
    let (through_tick, expected) = match domain {
        Domain::Trader => {
            trade(&mut live).await;
            sprint(&mut live, 1200).await;
            (leave(&mut live, actor).await, Expected::Trader)
        }
        Domain::Warning => {
            warn(&mut live).await;
            sprint(&mut live, 201).await;
            warn(&mut live).await;
            sprint(&mut live, 201).await;
            let warned_at = warn(&mut live).await;
            // Bounded sprint batches keep publication consumption active even
            // on a contended host; all time is advanced by the real tick owner.
            for _ in 0..12 {
                sprint(&mut live, 1000).await;
            }
            let aged_at = sprint(&mut live, 1).await;
            let elapsed = aged_at - warned_at;
            assert!(
                (12001..24002).contains(&elapsed),
                "one natural warning decay: {elapsed}"
            );
            (
                leave(&mut live, actor).await,
                Expected::Warning {
                    since: (elapsed % 12001) as u32,
                },
            )
        }
        Domain::Claim => {
            let checkpoint = bridge::recover_offline(&baseline_config).unwrap();
            let checkpoint_position = checkpoint.dimensions[&Dimension::Overworld].entities[&1].pos;
            command(&live, "setblock 4 80 4 minecraft:composter").await;
            command(&live, "time set 2000").await;
            let tick = sprint(&mut live, 100).await;
            (
                tick,
                Expected::Claim {
                    checkpoint_position,
                },
            )
        }
        Domain::Inhabited => {
            assert!(checkpoint_value > 0, "checkpoint must carry inhabited time");
            let stopped = game_mode(&mut live, "spectator", 3).await;
            sprint(&mut live, 4).await;
            let resumed = game_mode(&mut live, "creative", 1).await;
            sprint(&mut live, 6).await;
            let stopped_again = game_mode(&mut live, "spectator", 3).await;
            // This multi-dimension Core queues console commands for
            // dimensions::travel::finish, after local inhabited accumulation.
            // A stop publication still counts; a resume publication does not.
            let value = checkpoint_value
                + (stopped - checkpoint_tick) as i64
                + (stopped_again - resumed) as i64;
            assert!(value > checkpoint_value);
            // Observe the next local phase closing the final active interval
            // before asking the crash child to wait for its WAL publication.
            let closed = live
                .until(|output, _| (output.tick > stopped_again).then_some(output.tick))
                .await;
            (
                closed,
                Expected::Inhabited {
                    checkpoint_value,
                    value,
                },
            )
        }
    };
    assert!(through_tick > checkpoint_tick);
    (
        live,
        Evidence {
            checkpoint_tick,
            through_tick,
            expected,
        },
    )
}

fn assert_export(root: &Path, evidence: &Evidence, wal_tail: bool) {
    let output = root.parent().unwrap().join("anvil");
    let receipt = crate::export_anvil(
        &config(root),
        root,
        &output,
        &root.parent().unwrap().join("export.json"),
    )
    .unwrap();
    if wal_tail {
        assert_eq!(
            receipt.checkpoint_tick,
            Some(evidence.checkpoint_tick),
            "crash must not checkpoint the tail"
        );
        assert!(receipt.replay_tick.unwrap() >= evidence.through_tick);
        assert!(receipt.replay_tick > receipt.checkpoint_tick);
    } else {
        assert!(receipt.checkpoint_tick.unwrap() >= evidence.through_tick);
    }
    let registry = crate::test_support::registry().unwrap();
    match &evidence.expected {
        Expected::Trader => {
            let imported = root.parent().unwrap().join("imported");
            crate::import_anvil(
                &config(&imported),
                &output,
                &imported,
                &root.parent().unwrap().join("import.json"),
            )
            .unwrap();
            let recovered = bridge::recover_offline(&config(&imported)).unwrap();
            let replay = &recovered.dimensions[&Dimension::Overworld];
            let clock = replay.wandering_trader.unwrap();
            assert_eq!((clock.delay, clock.chance), (11_145, 50));
            let trader = replay
                .entities
                .values()
                .find(|entity| entity.uuid == TRADER.to_string())
                .unwrap();
            let trader = trader.villager.as_ref().unwrap();
            assert!(trader.wandering);
            assert_eq!(trader.offers.len(), 1);
            assert_eq!(
                trader.offers[0].uses, 1,
                "actual merchant result extraction must persist"
            );
            for uuid in [uuid::Uuid::from_u128(0x762), uuid::Uuid::from_u128(0x763)] {
                let llama = replay
                    .entities
                    .values()
                    .find(|entity| entity.uuid == uuid.to_string())
                    .unwrap();
                assert_eq!(
                    llama.state.entity_life.leash,
                    Some(mc_core::leash::Holder::Entity(TRADER))
                );
            }
            let player = &replay.players[&PLAYER.to_string()];
            let ice = registry
                .items
                .id_of(&Identifier::parse("minecraft:blue_ice").unwrap())
                .unwrap();
            let emerald = registry
                .items
                .id_of(&Identifier::parse("minecraft:emerald").unwrap())
                .unwrap();
            assert_eq!(
                player
                    .inventory
                    .iter()
                    .filter(|stack| stack.item_id == ice)
                    .map(|stack| stack.count)
                    .sum::<i32>(),
                1
            );
            assert_eq!(
                player
                    .inventory
                    .iter()
                    .filter(|stack| stack.item_id == emerald)
                    .map(|stack| stack.count)
                    .sum::<i32>(),
                0
            );
        }
        Expected::Warning { since } => {
            let tag = crate::read_nbt(&output.join(format!("players/data/{PLAYER}.dat"))).unwrap();
            let Tag::Compound(fields) = &tag else {
                panic!("player compound")
            };
            assert!(!fields.iter().any(|(key, _)| key == "KrasnokamenWardenWarning"));
            let player = mc_core::journal::player_from_tag(&tag, &registry.items).unwrap();
            let warning = player.warden_warning;
            assert_eq!(
                (
                    warning.warning_level(),
                    warning.cooldown_ticks(),
                    warning.ticks_since_last_warning()
                ),
                (2, 0, *since)
            );
        }
        Expected::Claim {
            checkpoint_position,
        } => {
            let regions = mc_world::anvil::read_entity_regions(
                &Dimension::Overworld.directory(&output).join("entities"),
                &registry.items,
            )
            .unwrap();
            let villagers: Vec<_> = regions
                .values()
                .flat_map(|chunk| &chunk.entities)
                .filter(|entity| entity.uuid == VILLAGER)
                .collect();
            assert_eq!(villagers.len(), 1);
            let entity = villagers[0];
            if wal_tail {
                assert_eq!(
                    entity.pos, *checkpoint_position,
                    "ordinary motion stays checkpoint durable"
                );
            }
            let Some(Tag::Compound(data)) =
                mc_data::item_nbt::field(&entity.extra_fields, "VillagerData")
            else {
                panic!("VillagerData")
            };
            assert!(data.contains(&("profession".into(), Tag::String("minecraft:farmer".into()))));
            let Some(Tag::Compound(brain)) =
                mc_data::item_nbt::field(&entity.extra_fields, "Brain")
            else {
                panic!("Brain")
            };
            let Some(Tag::Compound(memories)) = mc_data::item_nbt::field(brain, "memories") else {
                panic!("memories")
            };
            assert_eq!(
                memories
                    .iter()
                    .filter(|(key, _)| key == "minecraft:job_site")
                    .count(),
                1
            );
            let imported = root.parent().unwrap().join("imported");
            crate::import_anvil(
                &config(&imported),
                &output,
                &imported,
                &root.parent().unwrap().join("import.json"),
            )
            .unwrap();
            let recovered = bridge::recover_offline(&config(&imported)).unwrap();
            let entity = recovered.dimensions[&Dimension::Overworld]
                .entities
                .values()
                .find(|entity| entity.uuid == VILLAGER.to_string())
                .unwrap();
            let villager = entity.villager.as_ref().unwrap();
            assert_eq!(villager.data.profession, VillagerProfession::Farmer);
            assert_eq!(
                villager.job_site,
                Some(mc_core::boundary::BlockPos { x: 4, y: 80, z: 4 })
            );
            assert_eq!(villager.job_site_dimension, "minecraft:overworld");
        }
        Expected::Inhabited {
            checkpoint_value,
            value,
        } => {
            assert!(*checkpoint_value > 0 && value > checkpoint_value);
            let chunk = crate::export_tests::read_exported_chunk(
                &output,
                Dimension::Overworld,
                ChunkPos { x: 0, z: 0 },
                &registry,
            );
            assert_eq!(
                chunk.inhabited_time(),
                *value,
                "nonzero checkpoint plus active WAL intervals, excluding spectator ticks"
            );
        }
    }
}

async fn checkpoint_roundtrip(domain: Domain) {
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path().join("source");
    let (live, evidence) = scenario(&root, domain).await;
    assert!(live.core.checkpoint());
    drop(live);
    assert_export(&root, &evidence, false);
}

#[tokio::test]
async fn wandering_trader_live_checkpoint_roundtrip() {
    checkpoint_roundtrip(Domain::Trader).await;
}
#[tokio::test]
async fn warning_clock_live_checkpoint_roundtrip() {
    checkpoint_roundtrip(Domain::Warning).await;
}
#[tokio::test]
async fn village_claim_live_checkpoint_roundtrip() {
    checkpoint_roundtrip(Domain::Claim).await;
}
#[tokio::test]
async fn inhabited_time_live_checkpoint_roundtrip() {
    checkpoint_roundtrip(Domain::Inhabited).await;
}

// Linux only: the child exits without destructors after observing tail publication;
// the parent verifies the complete, stable WAL before export.
#[cfg(target_os = "linux")]
#[path = "live_capture_crash_tests.rs"]
mod crash;
