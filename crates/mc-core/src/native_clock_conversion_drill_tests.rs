//! Legacy checkpoint plus published tail converted at one recovered boundary.
use super::*;
use crate::dimensions::Dimension;
use crate::weather::GameClockImage;

#[test]
fn legacy_game_clock_conversion_folds_tail_anchors_active_and_drops_phantom_debt() {
    let directory = tempfile::tempdir().unwrap();
    let mut drill = fixture(directory.path());
    drill.state.tick_control.set_frozen(true);
    drill.run(&[]).unwrap();
    drill.checkpoint();
    let config = drill.config.clone();
    let mut legacy = drill.manifest.clone();
    let checkpoint = legacy.tick;
    let game = drill.state.tick_control.game_ticks(checkpoint);
    let climate = legacy.climate.as_mut().unwrap();
    climate.sim_debt_anchored_since = None;
    climate.game_clock = GameClockImage::Held {
        game_ticks: game,
        held_since: None,
    };
    for debt in &mut legacy.debt {
        debt.held_anchor = None;
    }
    for dimension in legacy.dimensions.values_mut() {
        for debt in &mut dimension.debt {
            debt.held_anchor = None;
        }
    }
    let tick = checkpoint + 400;
    let tail_climate = legacy.climate.unwrap();
    let villager = crate::villager::VillagerState {
        brain_clock: true,
        activity: crate::villager::Activity::Work,
        last_work: game.saturating_sub(1) as i64,
        sleeping: true,
        home: Some(mc_world::BlockPos {
            x: 34,
            y: 200,
            z: 40,
        }),
        last_slept: game.checked_sub(1),
        panic_until: 999,
        ..Default::default()
    };
    let entries = vec![
        JournalEntry::SimDebt {
            image: crate::journal::SimDebtImage {
                chunk: [CHUNK.x, CHUNK.z],
                tick,
                last_sim: checkpoint,
                held_anchor: None,
                remaining: 350,
                active: false,
            },
        },
        JournalEntry::SimDebt {
            image: crate::journal::SimDebtImage {
                chunk: [9, 9],
                tick,
                last_sim: tick,
                held_anchor: None,
                remaining: 0,
                active: true,
            },
        },
        JournalEntry::Entity {
            id: 70000,
            uuid: uuid::Uuid::from_u128(70000).to_string(),
            type_id: mc_data::entity_types::entity_type_contract_26_1_2_by_name(
                "minecraft:villager",
            )
            .unwrap()
            .protocol_id,
            health: 20.0,
            alive: true,
            age: 0,
            breed_cooldown: 0,
            pickup_delay: 0,
            item: None,
            appearance: Default::default(),
            active_effects: Default::default(),
            villager: Some(Box::new(villager)),
            state: Default::default(),
            projectile: None,
            tick,
            pos: [34.5, 200.0, 40.5],
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
        },
        JournalEntry::Climate {
            value: tail_climate,
        },
    ];
    drill
        .writer
        .as_mut()
        .unwrap()
        .append(checkpoint + 1, tick, &entries)
        .unwrap();
    drill.writer.as_mut().unwrap().flush().unwrap();
    drop(drill);
    crate::native_checkpoint::tests::write_manifest(&config, &legacy).unwrap();
    let tail = config
        .world_dir
        .join("journal")
        .join(format!("batch-{:020}-{tick:020}.bin", checkpoint + 1));
    let original_tail = std::fs::read(&tail).unwrap();
    assert!(crate::native_checkpoint::recover_offline(&config).is_err());
    let store = fixture_store(&config).unwrap();
    let report = crate::native_checkpoint::game_clock_upgrade::write_converted_checkpoint(
        &config,
        Arc::clone(&store),
    )
    .unwrap();
    assert_eq!(report.tick, tick);
    assert_eq!(report.held_ticks, tick - game);
    assert_eq!(report.held_since, Some(tick));
    assert_eq!(report.checkpoint_tick, checkpoint);
    assert_eq!(report.tail_held_ticks, 400);
    assert_eq!(report.dropped_debt, 350);
    assert_eq!((report.work_normalized, report.sleep_normalized), (1, 1));
    assert_eq!(
        std::fs::read(&tail).unwrap(),
        original_tail,
        "checkpoint must not rewrite the published WAL"
    );
    drop(store);
    let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
    let root = &recovered.dimensions[&Dimension::Overworld];
    assert_eq!(root.climate.unwrap().sim_debt_anchored_since, Some(tick));
    let active = root.sim_debt[&[9, 9]];
    assert_eq!(
        (active.last_sim, active.held_anchor),
        (tick, Some(tick - game))
    );
    let inactive = root.sim_debt[&[CHUNK.x, CHUNK.z]];
    assert_eq!(inactive.remaining, 0);
    assert_eq!(inactive.held_anchor, Some(checkpoint - game));
    let villager = root.entities[&70000].villager.as_ref().unwrap();
    assert_eq!(villager.last_work, (tick - 2) as i64);
    assert_eq!(villager.last_slept, Some(tick - 2));
    assert_eq!(villager.panic_until, 999);
    let extras = crate::native_checkpoint::decode_tag(
        &recovered.checkpoint.globals["conversion_extras"],
        "conversion_extras",
    )
    .unwrap();
    let mc_nbt::Tag::Compound(fields) = extras else {
        panic!("extras compound")
    };
    assert!(fields.iter().any(
        |(key, value)| key == "data/krasnokamen-game-clock-upgrade.json"
            && matches!(value, mc_nbt::Tag::ByteArray(_))
    ));
    drop(recovered);
    let mut resumed = recovered_world(&config);
    resumed
        .world
        .ensure_blocking(&[(CHUNK, 0)].into_iter().collect());
    resumed
        .tickets
        .insert("conversion-unfreeze".into(), (CHUNK, 0));
    let item = resumed
        .registry
        .items
        .id_of(&mc_data::Identifier::parse("minecraft:egg").unwrap())
        .unwrap();
    let id = crate::tick::spawn_item_entity(
        &mut resumed,
        [34.5, 200.0, 40.5],
        mc_data::ItemStack::new(item, 4),
    );
    let slot = resumed.entities.slot_of(id).unwrap();
    resumed.entities.row_mut(slot).age = 5854;
    resumed.tick_control.set_frozen(false);
    crate::tick::run_tick(&mut resumed, &[]);
    let slot = resumed
        .entities
        .slot_of(id)
        .expect("no phantom 350-tick catch-up after unfreeze");
    assert_eq!(resumed.entities.row(slot).age, 5855);
    assert_eq!(
        resumed
            .entities
            .row(slot)
            .item
            .as_ref()
            .unwrap()
            .full_stack()
            .count,
        4
    );
}

#[test]
fn legacy_game_clock_conversion_held_tail_preserves_checkpoint_items_and_fresh_memories() {
    const HELD_TAIL: u64 = 400;
    let age = 6000 - HELD_TAIL as u32 + 1;
    for checkpoint_held in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut drill = fixture(directory.path());
        if checkpoint_held {
            drill.state.tick_control.set_frozen(true);
            drill.run(&[]).unwrap();
        }
        let checkpoint = drill.state.tick;
        let game = drill.state.tick_control.game_ticks(checkpoint);
        let item = drill
            .state
            .registry
            .items
            .id_of(&mc_data::Identifier::parse("minecraft:egg").unwrap())
            .unwrap();
        let item_id = crate::tick::spawn_item_entity(
            &mut drill.state,
            [34.5, 200.0, 40.5],
            mc_data::ItemStack::new(item, 4),
        );
        let slot = drill.state.entities.slot_of(item_id).unwrap();
        drill.state.entities.row_mut(slot).age = age;
        let villager_type =
            mc_data::entity_types::entity_type_contract_26_1_2_by_name("minecraft:villager")
                .unwrap()
                .protocol_id;
        let villager_id = drill
            .state
            .spawn_mob(villager_type, [34.5, 200.0, 40.5], 20.0);
        let slot = drill.state.entities.slot_of(villager_id).unwrap();
        let last_running = checkpoint - u64::from(checkpoint_held);
        drill.state.entities.row_mut(slot).villager =
            Some(Box::new(crate::villager::VillagerState {
                brain_clock: true,
                activity: crate::villager::Activity::Work,
                last_work: last_running.saturating_sub(1) as i64,
                sleeping: true,
                home: Some(mc_world::BlockPos {
                    x: 34,
                    y: 200,
                    z: 40,
                }),
                last_slept: last_running.checked_sub(1),
                ..Default::default()
            }));
        drill.checkpoint();
        let config = drill.config.clone();
        let mut legacy = drill.manifest.clone();
        assert!(
            legacy
                .debt
                .iter()
                .any(|debt| { debt.chunk == [CHUNK.x, CHUNK.z] && debt.active })
        );
        let climate = legacy.climate.as_mut().unwrap();
        climate.sim_debt_anchored_since = None;
        if checkpoint_held {
            climate.game_clock = GameClockImage::Held {
                game_ticks: game,
                held_since: None,
            };
        }
        for debt in &mut legacy.debt {
            debt.held_anchor = None;
        }
        for dimension in legacy.dimensions.values_mut() {
            for debt in &mut dimension.debt {
                debt.held_anchor = None;
            }
        }
        let tick = checkpoint + HELD_TAIL;
        let mut tail_climate = legacy.climate.unwrap();
        tail_climate.game_clock = GameClockImage::Held {
            game_ticks: game,
            held_since: None,
        };
        drill
            .writer
            .as_mut()
            .unwrap()
            .append(
                checkpoint + 1,
                tick,
                &[JournalEntry::Climate {
                    value: tail_climate,
                }],
            )
            .unwrap();
        drill.writer.as_mut().unwrap().flush().unwrap();
        drop(drill);
        crate::native_checkpoint::tests::write_manifest(&config, &legacy).unwrap();
        let store = fixture_store(&config).unwrap();
        let report = crate::native_checkpoint::game_clock_upgrade::write_converted_checkpoint(
            &config,
            Arc::clone(&store),
        )
        .unwrap();
        assert_eq!(report.checkpoint_tick, checkpoint);
        assert_eq!(report.tail_held_ticks, HELD_TAIL);
        assert_eq!((report.work_normalized, report.sleep_normalized), (1, 1));
        drop(store);
        let recovered = crate::native_checkpoint::recover_offline(&config).unwrap();
        let root = &recovered.dimensions[&Dimension::Overworld];
        let item = root
            .entities
            .get(&item_id.0)
            .expect("held time cannot expire an item");
        assert_eq!(item.age, age);
        assert_eq!(item.item.as_ref().unwrap().count, 4);
        let villager = root.entities[&villager_id.0].villager.as_ref().unwrap();
        assert_eq!(villager.last_work, (tick - 2) as i64);
        assert_eq!(villager.last_slept, Some(tick - 2));
    }
}
