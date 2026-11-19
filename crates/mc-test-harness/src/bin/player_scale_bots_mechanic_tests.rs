fn mechanic_test_args() -> Result<Args> {
    let mut args = Args::try_parse_from([
        "player-scale-bots",
        "--addr",
        "127.0.0.1:1",
        "--clients",
        "2",
        "--view-distance",
        "2",
        "--report",
        "/unused.json",
    ])?;
    args.schedule = Arc::new(serde_json::from_value(json!({"windows":[
        {"name":"steady","duration_ms":1000},
        {"name":"burst","duration_ms":1000},
        {"name":"recovery","duration_ms":1000,"recovery":true}
    ]}))?);
    Ok(args)
}
fn mechanic_test_state(extra: Value, args: &Args) -> Result<Mechanics> {
    let mut value = json!({"schema_version":1});
    for (key, item) in extra.as_object().expect("fixture object") {
        value[key] = item.clone();
    }
    Mechanics::new(serde_json::from_value(value)?, args)
}

#[tokio::test]
async fn runtime_command_requires_marker_before_writing_and_receiving_reply() -> Result<()> {
    let mut args = mechanic_test_args()?;
    let mut state = mechanic_test_state(json!({}), &args)?;
    state.begin_measurement()?;
    state.runtime_enqueue(
        0,
        "job1".into(),
        "settlement status".into(),
        "status ".into(),
        "scale_0000",
        17,
    )?;
    assert!(
        state
            .runtime_enqueue(
                0,
                "job2".into(),
                "settlement status".into(),
                "status ".into(),
                "scale_0000",
                18
            )
            .is_err()
    );
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    let (mut client, _peer, _buffer) = connection().await?;
    bot.runtime_command_poll(&mut client, Duration::ZERO)
        .await?;
    bot.mechanic_frame(&system_chat_frame("status old replay")?, Some("steady"))?;
    assert!(matches!(
        shared.lock().runtime_commands[0].as_ref().unwrap().stage,
        RuntimeCommandStage::WaitingMarker
    ));
    let marker = shared.lock().runtime_commands[0]
        .as_ref()
        .unwrap()
        .marker
        .clone();
    bot.mechanic_frame(&system_chat_frame(&marker)?, Some("steady"))?;
    bot.mechanic_frame(&system_chat_frame("status before command")?, Some("steady"))?;
    assert!(matches!(
        shared.lock().runtime_commands[0].as_ref().unwrap().stage,
        RuntimeCommandStage::MarkerObserved
    ));
    bot.runtime_command_poll(&mut client, Duration::ZERO)
        .await?;
    bot.mechanic_frame(
        &system_chat_frame("status {\"worker\":42}")?,
        Some("steady"),
    )?;
    let state = shared.lock();
    assert!(state.runtime_commands[0].is_none());
    let reply = state
        .runtime_events
        .iter()
        .find(|event| event["event"] == "runtime_command_reply")
        .unwrap();
    assert_eq!(reply["reply"], "status {\"worker\":42}");
    assert!(reply["receive_seq"].as_u64() > reply["write_receive_seq"].as_u64());
    Ok(())
}

#[test]
fn haul_routes_require_cardinal_edges_and_transfer_balance() {
    assert!(haul_route_valid(&[[0, 64, 0], [1, 64, 0], [1, 65, 1]]));
    assert!(!haul_route_valid(&[[0, 64, 0], [2, 64, 0]]));
    assert!(!haul_route_valid(&[[0, 64, 0], [1, 64, 1]]));
    assert!(haul_balanced((10, 0), (6, 4), 4, true));
    assert!(!haul_balanced((10, 0), (6, 3), 4, true));
    assert!(haul_balanced((3, 4), (7, 0), 4, false));
    assert!(!haul_balanced((3, 4), (7, 1), 4, false));
}

#[test]
fn repeated_haul_requires_explicit_ordered_request_after_completion() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let base = json!({"id":"first","bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
        "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
        "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],"timeout_ms":5000});
    let mut second = base.clone();
    second["id"] = json!("second");
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[base,second]
        }),
        &args,
    )?;
    assert!(state.request_haul(0, "first").is_err());
    state.spec.hauls[0].defer_start = true;
    assert_eq!(state.request_haul(0, "first")?, 0);
    assert!(state.request_haul(0, "second").is_err());
    state.haul_requests[0] = None;
    assert!(state.request_haul(0, "second").is_err());
    state.haul_reports[0]["status"] = json!("finished");
    assert_eq!(state.request_haul(0, "second")?, 1);
    assert!(state.request_haul(0, "second").is_err());
    assert_eq!(state.haul_reports[1]["status"], "requested");
    assert_eq!(state.haul_requests[0], Some(1));
    let failures = state.fail_hauls_for_dead_bot(1, [2.0, 64.0, 0.5]);
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0]["id"], "second");
    assert_eq!(state.haul_reports[0]["status"], "finished");
    assert_eq!(state.haul_reports[1]["status"], "failed");
    assert!(state.haul_requests[0].is_none());
    Ok(())
}

#[test]
fn observed_observer_death_fails_active_haul_immediately() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"id":"work","bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],"timeout_ms":5000}]
        }),
        &args,
    )?;
    state.haul_reports[0]["status"] = json!("running");
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let mut observer = Bot::new(
        1,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    observer.mechanic_frame(
        &mechanic_test_frame(&ClientboundSetHealth {
            health: 0.0,
            food: 20,
            saturation: 5.0,
        })?,
        Some("steady"),
    )?;
    let state = args.mechanics.as_ref().unwrap().lock();
    assert_eq!(state.haul_reports[0]["status"], "failed");
    assert_eq!(state.haul_reports[0]["error"], "observer died");
    assert_eq!(state.haul_reports[0]["completed"], false);
    Ok(())
}

#[test]
fn farmer_delivery_requires_exact_received_hotbar_stack() {
    let mut menu = MenuReceipt {
        final_slots: Some(vec![
            SlotImage {
                slot: 0,
                item_id: None,
                name: None,
                count: 0,
                plain: true
            };
            27
        ]),
        final_player_inventory: Some(vec![SlotImage {
            slot: 54,
            item_id: Some(7),
            name: Some("minecraft:wheat".into()),
            count: 4,
            plain: true,
        }]),
        ..Default::default()
    };
    assert_eq!(haul_hotbar_slot(&menu, "minecraft:wheat", 4), Some((0, 7)));
    assert_eq!(haul_hotbar_slot(&menu, "minecraft:wheat", 5), None);
    menu.final_player_inventory.as_mut().unwrap()[0].slot = 53;
    assert_eq!(haul_hotbar_slot(&menu, "minecraft:wheat", 4), None);
}

#[tokio::test]
async fn farmer_drop_requires_received_debit_and_one_new_item_identity() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let farmer_uuid = Uuid::from_u128(42);
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true},
                {"kind":"minecraft:villager","count":1,"reader_images":true},
                {"kind":"minecraft:item","count":1}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],
                "farmer_delivery":{"farmer_uuid":farmer_uuid},"timeout_ms":5000}]
        }),
        &args,
    )?;
    let item_id = *state
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:wheat")
        .unwrap()
        .0;
    let mut farmer = mechanic_test_entity(&state, "minecraft:villager", 42);
    farmer.x = 3.5;
    farmer.z = 1.0;
    state.spawn(0, &farmer, None);
    let mut competitor = mechanic_test_entity(&state, "minecraft:villager", 44);
    competitor.x = 4.0;
    competitor.z = 0.5;
    state.spawn(0, &competitor, None);
    let (_, blocked) = state.farmer_offer_images(0, farmer_uuid, [3.5, 64.0, 0.5])?;
    assert!(blocked);
    state
        .entities
        .get_mut(&competitor.uuid)
        .unwrap()
        .reader_positions[0] = None;
    assert!(
        state
            .farmer_offer_images(0, farmer_uuid, [3.5, 64.0, 0.5])
            .is_err()
    );
    state
        .entities
        .get_mut(&competitor.uuid)
        .unwrap()
        .reader_positions[0] = Some([4.0, 64.0, 0.5]);
    state.capacity_errors = 1;
    assert!(
        state
            .farmer_offer_images(0, farmer_uuid, [3.5, 64.0, 0.5])
            .is_err()
    );
    state.capacity_errors = 0;
    state.movement(0, 44, [10.0, 0.0, 0.0], true, None);
    assert!(
        !state
            .farmer_offer_images(0, farmer_uuid, [3.5, 64.0, 0.5])?
            .1
    );
    state.movement(0, 44, [-10.0, 0.0, 0.0], true, None);
    state.remove(0, 44, None);
    assert!(
        !state
            .farmer_offer_images(0, farmer_uuid, [3.5, 64.0, 0.5])?
            .1
    );
    state.spawn(0, &competitor, None);
    let mut inventory: Vec<_> = (0..45)
        .map(|slot| state.slot(slot, &ItemStack::EMPTY))
        .collect();
    inventory[36] = state.slot(36, &ItemStack::new(item_id, 4));
    state.players[0].final_inventory = Some(inventory);
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 3.5;
    bot.y = 64.0;
    bot.z = 0.5;
    bot.mechanic_bot.haul = Some(HaulState {
        spec_index: 0,
        phase: HaulPhase::FarmerDrop,
        waypoint: 1,
        leg_version: 0,
        started: Instant::now(),
        menu_index: 0,
        image_before: 0,
        counts_before: (0, 0),
        hotbar_slot: 0,
        drop_item_id: item_id,
        drop_position: [0.0; 3],
        prior_items: HashSet::new(),
        approach_route: Vec::new(),
        pursuit_route: Vec::new(),
        pursuit_target: [0.0; 3],
        pursuit_replans: 0,
        pursuit_wait_version: 0,
        observer_wait_resume_walk: false,
        pursuit_reused_fence: false,
        pursuit_retained: false,
        offer_observation_seq: None,
    });
    bot.mechanic_bot.haul.as_mut().unwrap().phase = HaulPhase::FarmerPursuitWalk;
    bot.trace_farmer_refusal(4.0, 0.5, "missing_column")?;
    bot.trace_farmer_refusal(5.0, 0.5, "unwalkable_column")?;
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["first_route_refusal"]["reason"],
        "missing_column"
    );
    bot.mechanic_bot.haul.as_mut().unwrap().phase = HaulPhase::FarmerDrop;
    let (mut client, _peer, _buffer) = connection().await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerDrop
    ));
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["drop_written"].is_null());
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["completed"],
        false
    );
    let snapshot = shared.lock().haul_reports[0]["farmer_delivery"]["offer_observation"].clone();
    assert_eq!(snapshot["blocked"], true);
    assert_eq!(snapshot["villagers"].as_array().unwrap().len(), 2);
    assert!(
        snapshot["villagers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["uuid"] == competitor.uuid.to_string()
                && entry["within_competitor_margin"] == true
                && entry["reader_version"]
                    .as_u64()
                    .is_some_and(|version| version > 0))
    );
    let seq = bot
        .mechanic_bot
        .haul
        .as_ref()
        .unwrap()
        .offer_observation_seq;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(
        bot.mechanic_bot
            .haul
            .as_ref()
            .unwrap()
            .offer_observation_seq,
        seq
    );
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["drop_written"].is_null());
    shared
        .lock()
        .movement(0, 44, [10.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["drop_offer"]["villager_observation"]["blocked"],
        false
    );
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["completed"],
        false
    );
    {
        let mut state = shared.lock();
        let empty = state.slot(36, &ItemStack::EMPTY);
        state.players[0].final_inventory.as_mut().unwrap()[36] = empty;
    }
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["completed"],
        false
    );
    let item_uuid = {
        let mut state = shared.lock();
        let mut item = mechanic_test_entity(&state, "minecraft:item", 43);
        item.x = 3.5;
        item.z = 0.5;
        state.spawn(0, &item, Some("steady"));
        let image = state.slot(0, &ItemStack::new(item_id, 4));
        let entity = state.entities.get_mut(&item.uuid).unwrap();
        entity.item = Some(image);
        entity.item_observer = Some(0);
        item.uuid
    };
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let state = shared.lock();
    assert_eq!(
        state.haul_reports[0]["farmer_delivery"]["drop"]["item_uuid"],
        item_uuid.to_string()
    );
    assert_eq!(state.haul_reports[0]["farmer_delivery"]["completed"], true);
    assert_eq!(state.item_collections_received, 0);
    Ok(())
}

#[test]
fn haul_observer_follows_only_received_actor_pose_and_fails_on_conflicting_work() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],
                "timeout_ms":5000}]
        }),
        &args,
    )?;
    let mut actor = mechanic_test_entity(&state, "minecraft:player", 41);
    actor.x = 8.5;
    actor.z = 0.5;
    state.spawn(1, &actor, None);
    state.haul_reports[0]["status"] = json!("running");
    let shared = Arc::new(MechanicMutex::new(state));
    let mut observer = terrain_bot(0.5, 64, 0.5, 0);
    observer.index = 1;
    observer.mechanics = Some(Arc::clone(&shared));
    observer.observer_follow_poll()?;
    let step = observer.observer_follow_step(0.1).unwrap();
    assert!(step.0 > 0.0 || step.1.abs() > 0.0);
    assert!(shared.lock().haul_reports[0]["source_arrival"].is_null());
    shared.lock().remove(1, 41, None);
    observer.observer_follow_poll()?;
    assert!(observer.observer_follow_step(0.1).is_some());
    assert!(shared.lock().haul_reports[0]["source_arrival"].is_null());
    shared
        .lock()
        .spec
        .actions
        .push(serde_json::from_value(json!({"bot":1,"kind":"command"}))?);
    assert!(observer.observer_follow_poll().is_err());
    shared.lock().haul_reports[0]["status"] = json!("finished");
    observer.observer_follow_poll()?;
    assert!(observer.observer_follow_step(0.1).is_none());
    Ok(())
}

#[tokio::test]
async fn farmer_pursuit_tolerates_small_move_replans_material_move_and_requires_actor_observer()
-> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true},
                {"kind":"minecraft:villager","count":1,"reader_images":true},
                {"kind":"minecraft:item","count":1}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],
                "farmer_delivery":{"farmer_uuid":Uuid::from_u128(42)},"timeout_ms":5000}]
        }),
        &args,
    )?;
    let mut actor = mechanic_test_entity(&state, "minecraft:player", 41);
    actor.x = 0.5;
    actor.z = 0.5;
    state.spawn(1, &actor, None);
    let mut farmer = mechanic_test_entity(&state, "minecraft:villager", 42);
    farmer.x = 3.5;
    farmer.z = 0.5;
    state.spawn(0, &farmer, None);
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.entity_id = 41;
    bot.mechanics = Some(Arc::clone(&shared));
    bot.schedule = Arc::clone(&args.schedule);
    let mut haul = HaulState::new(0);
    haul.phase = HaulPhase::FarmerPursuitPlan;
    bot.mechanic_bot.haul = Some(haul);
    let (mut client, _peer, _buffer) = connection().await?;
    shared.lock().remove(1, 41, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerObserverWait
    ));
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    shared.lock().spawn(1, &actor, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitPlan
    ));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let waypoint = bot.mechanic_bot.haul.as_ref().unwrap().pursuit_route[1];
    bot.x = f64::from(waypoint[0]) + 0.5;
    bot.z = f64::from(waypoint[2]) + 0.5;
    shared.lock().remove(1, 41, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerObserverWait
    ));
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    shared.lock().spawn(1, &actor, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWalk
    ));
    shared
        .lock()
        .movement(0, 42, [1.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWalk
    ));
    assert_eq!(bot.mechanic_bot.haul.as_ref().unwrap().pursuit_replans, 1);
    shared
        .lock()
        .movement(0, 42, [5.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitPlan
    ));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let haul = bot.mechanic_bot.haul.as_mut().unwrap();
    assert_eq!(haul.pursuit_replans, 2);
    assert_eq!(haul.pursuit_target, [9.5, 64.0, 0.5]);
    let endpoint = *haul.pursuit_route.last().unwrap();
    haul.waypoint = haul.pursuit_route.len() - 1;
    bot.x = f64::from(endpoint[0]) + 0.5;
    bot.y = f64::from(endpoint[1]);
    bot.z = f64::from(endpoint[2]) + 0.5;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    shared
        .lock()
        .movement(1, 41, [bot.x, bot.y, bot.z], false, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let state = shared.lock();
    let arrival = &state.haul_reports[0]["farmer_delivery"]["pursuit_arrival"];
    assert_eq!(arrival["farmer_uuid"], farmer.uuid.to_string());
    assert!(arrival["reader_version"].as_u64() > arrival["leg_start_version"].as_u64());
    assert_eq!(state.haul_reports[0]["farmer_delivery"]["completed"], false);
    Ok(())
}

#[tokio::test]
async fn farmer_failed_replan_retains_safe_progress_without_false_arrival() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true},
                {"kind":"minecraft:villager","count":1,"reader_images":true},
                {"kind":"minecraft:item","count":1}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"bot":0,"observer_bot":1,"player_uuid":Uuid::from_u128(41),
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],"destination_route":[[1,64,0],[2,64,0]],
                "farmer_delivery":{"farmer_uuid":Uuid::from_u128(42)},"timeout_ms":5000}]
        }),
        &args,
    )?;
    let mut actor = mechanic_test_entity(&state, "minecraft:player", 41);
    actor.x = 0.5;
    actor.z = 0.5;
    state.spawn(1, &actor, None);
    let mut farmer = mechanic_test_entity(&state, "minecraft:villager", 42);
    farmer.x = 5.5;
    farmer.z = 0.5;
    state.spawn(0, &farmer, None);
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.entity_id = 41;
    bot.mechanics = Some(Arc::clone(&shared));
    bot.schedule = Arc::clone(&args.schedule);
    let mut haul = HaulState::new(0);
    haul.phase = HaulPhase::FarmerPursuitPlan;
    bot.mechanic_bot.haul = Some(haul);
    let (mut client, _peer, _buffer) = connection().await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let waypoint = bot.mechanic_bot.haul.as_ref().unwrap().pursuit_route[1];
    bot.x = f64::from(waypoint[0]) + 0.5;
    bot.z = f64::from(waypoint[2]) + 0.5;
    // Block all candidate goal cells for both target positions, including
    // the neighboring positive-X and negative-Z chunks.
    for z in -1_i32..=1 {
        for x in 13_i32..=16 {
            let chunk = (x.div_euclid(16), z.div_euclid(16));
            let index = (z.rem_euclid(16) * 16 + x.rem_euclid(16)) as usize;
            bot.surfaces.get_mut(&chunk).unwrap().columns[index].walkable = false;
        }
    }
    shared
        .lock()
        .movement(0, 42, [9.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let haul = bot.mechanic_bot.haul.as_ref().unwrap();
    assert!(matches!(haul.phase, HaulPhase::FarmerPursuitWalk));
    assert_eq!(haul.pursuit_replans, 2);
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_search_failure"]["continued_previous_route"],
        true
    );
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["completed"],
        false
    );
    shared
        .lock()
        .movement(0, 42, [1.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWalk
    ));
    assert_eq!(bot.mechanic_bot.haul.as_ref().unwrap().pursuit_replans, 2);
    let step = bot.haul_step(0.1).unwrap();
    assert!(step.0 > 0.0 || step.1.abs() > 0.0);
    let endpoint = *bot
        .mechanic_bot
        .haul
        .as_ref()
        .unwrap()
        .pursuit_route
        .last()
        .unwrap();
    let haul = bot.mechanic_bot.haul.as_mut().unwrap();
    haul.waypoint = haul.pursuit_route.len() - 1;
    bot.x = f64::from(endpoint[0]) + 0.5;
    bot.y = f64::from(endpoint[1]);
    bot.z = f64::from(endpoint[2]) + 0.5;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitPlan
    ));
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWait
    ));
    assert_eq!(bot.mechanic_bot.haul.as_ref().unwrap().pursuit_replans, 3);
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_search_failure"]["waiting_for_farmer_update"],
        true
    );
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(bot.mechanic_bot.haul.as_ref().unwrap().pursuit_replans, 3);
    shared
        .lock()
        .movement(0, 42, [0.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWait
    ));
    shared
        .lock()
        .movement(0, 42, [-1.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitPlan
    ));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitWait
    ));
    assert_eq!(bot.mechanic_bot.haul.as_ref().unwrap().pursuit_replans, 4);
    assert!(shared.lock().haul_reports[0]["farmer_delivery"]["pursuit_arrival"].is_null());
    assert_eq!(
        shared.lock().haul_reports[0]["farmer_delivery"]["completed"],
        false
    );
    let haul = bot.mechanic_bot.haul.as_mut().unwrap();
    haul.started = Instant::now() - Duration::from_secs(6);
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(shared.lock().haul_reports[0]["status"], "failed");
    assert_eq!(
        shared.lock().haul_reports[0]["error"],
        "haul deadline exceeded"
    );
    Ok(())
}

#[tokio::test]
async fn haul_approaches_exact_source_start_and_requires_fresh_observer_fence() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let uuid = Uuid::from_u128(41);
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true}],
            "containers":[{"name":"source","bot":0,"position":[4,64,3],"slots":27},
                {"name":"destination","bot":0,"position":[5,64,3],"slots":27}],
            "hauls":[{"bot":0,"observer_bot":1,"player_uuid":uuid,
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[3,64,2],[4,64,2]],
                "destination_route":[[4,64,2],[5,64,2]],"timeout_ms":5000}]
        }),
        &args,
    )?;
    let mut actor = mechanic_test_entity(&state, "minecraft:player", 41);
    actor.x = 0.5;
    actor.z = 0.5;
    state.spawn(1, &actor, None);
    let shared = Arc::new(MechanicMutex::new(state));
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.entity_id = 41;
    bot.mechanics = Some(Arc::clone(&shared));
    bot.schedule = Arc::clone(&args.schedule);
    let (mut client, _peer, _buffer) = connection().await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let haul = bot.mechanic_bot.haul.as_ref().unwrap();
    assert!(matches!(haul.phase, HaulPhase::SourceApproach));
    assert_eq!(haul.approach_route.last(), Some(&[3, 64, 2]));
    let step = bot.haul_step(0.2).unwrap();
    assert!(step.0.abs() + step.1.abs() > 0.0);
    let haul = bot.mechanic_bot.haul.as_mut().unwrap();
    haul.waypoint = haul.approach_route.len() - 1;
    bot.x = 3.5;
    bot.z = 2.5;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::SourceApproach
    ));
    assert!(shared.lock().haul_reports[0]["source_approach_arrival"].is_null());
    assert!(shared.lock().haul_reports[0]["source_arrival"].is_null());
    shared
        .lock()
        .movement(1, 41, [3.5, 64.0, 2.5], false, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::SourceRoute
    ));
    assert!(shared.lock().haul_reports[0]["source_approach_arrival"]["reader_version"].is_number());
    assert!(shared.lock().haul_reports[0]["source_arrival"].is_null());
    Ok(())
}

#[tokio::test]
async fn haul_arrival_requires_fresh_observer_packet_for_the_actor() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let uuid = Uuid::from_u128(41);
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:player","count":1,"reader_images":true},
                {"kind":"minecraft:villager","count":1,"reader_images":true},
                {"kind":"minecraft:item","count":1}],
            "containers":[{"name":"source","bot":0,"position":[1,64,1],"slots":27},
                          {"name":"destination","bot":0,"position":[2,64,1],"slots":27}],
            "hauls":[{"id":"first","defer_start":true,"bot":0,"observer_bot":1,"player_uuid":uuid,
                "source":"source","destination":"destination","item":"minecraft:wheat","count":4,
                "source_route":[[0,64,0],[1,64,0]],
                "destination_route":[[1,64,0],[2,64,0]],
                "farmer_delivery":{"farmer_uuid":Uuid::from_u128(42)},"timeout_ms":5000}]
        }),
        &args,
    )?;
    let mut actor = mechanic_test_entity(&state, "minecraft:player", 41);
    actor.x = 0.5;
    actor.z = 0.5;
    state.spawn(1, &actor, None);
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.entity_id = 42;
    sync_to_origin(&mut bot);
    let (mut client, _peer, _buffer) = connection().await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::AwaitTrigger
    ));
    assert_eq!(bot.haul_step(0.2), Some((0.0, 0.0, bot.yaw)));
    assert_eq!(shared.lock().haul_reports[0]["status"], "pending");
    shared.lock().request_haul(0, "first")?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::AwaitStart
    ));
    bot.entity_id = 41;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    bot.x = 1.5;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(shared.lock().haul_reports[0]["source_arrival"].is_null());
    shared
        .lock()
        .movement(1, 41, [1.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    {
        let state = shared.lock();
        assert_eq!(state.haul_reports[0]["source_arrival"]["observer_bot"], 1);
        assert_eq!(
            state.haul_reports[0]["source_arrival"]["player_uuid"],
            uuid.to_string()
        );
        assert_eq!(state.haul_reports[0]["completed"], false);
    }
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let item = *shared
        .lock()
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:wheat")
        .unwrap()
        .0;
    let mut source = vec![ItemStack::EMPTY; 63];
    source[0] = ItemStack::new(item, 4);
    shared
        .lock()
        .menu_image(0, 1, &source, &ItemStack::EMPTY, Some("steady"))?;
    bot.mechanic_bot.menu = Some((1, 0, 1));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    shared
        .lock()
        .menu_slot(0, 2, 0, &ItemStack::EMPTY, Some("steady"))?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(shared.lock().haul_reports[0]["source_transfer"].is_null());
    shared
        .lock()
        .menu_slot(0, 3, 27, &ItemStack::new(item, 4), Some("steady"))?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(shared.lock().haul_reports[0]["source_transfer"]["count"], 4);
    bot.x = 2.5;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(shared.lock().haul_reports[0]["destination_arrival"].is_null());
    shared
        .lock()
        .movement(1, 41, [1.0, 0.0, 0.0], true, Some("steady"));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let mut destination = vec![ItemStack::EMPTY; 63];
    destination[27] = ItemStack::new(item, 4);
    shared
        .lock()
        .menu_image(1, 1, &destination, &ItemStack::EMPTY, Some("steady"))?;
    bot.mechanic_bot.menu = Some((2, 1, 1));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    shared
        .lock()
        .menu_slot(1, 2, 27, &ItemStack::EMPTY, Some("steady"))?;
    shared
        .lock()
        .menu_slot(1, 3, 0, &ItemStack::new(item, 4), Some("steady"))?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert_eq!(shared.lock().haul_reports[0]["completed"], true);
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::WithdrawOpen
    ));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    let mut stocked = vec![ItemStack::EMPTY; 63];
    stocked[0] = ItemStack::new(item, 4);
    shared
        .lock()
        .menu_image(1, 4, &stocked, &ItemStack::EMPTY, Some("steady"))?;
    bot.mechanic_bot.menu = Some((3, 1, 4));
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    shared
        .lock()
        .menu_slot(1, 5, 0, &ItemStack::EMPTY, Some("steady"))?;
    shared
        .lock()
        .menu_slot(1, 6, 54, &ItemStack::new(item, 4), Some("steady"))?;
    bot.haul_poll(&mut client, Duration::ZERO).await?;
    assert!(matches!(
        bot.mechanic_bot.haul.as_ref().unwrap().phase,
        HaulPhase::FarmerPursuitPlan
    ));
    Ok(())
}

#[tokio::test]
async fn survival_destroy_sends_start_and_finish_and_requires_block_update() -> Result<()> {
    let mut args = mechanic_test_args()?;
    let state = mechanic_test_state(
        json!({"blocks":[{"name":"stone","position":[0,64,0],"reader_images":true}],
        "actions":[
            {"bot":0,"kind":"start_destroy_block","position":[0,64,0],"at_ms":0},
            {"bot":0,"kind":"finish_destroy_block","position":[0,64,0],"at_ms":100}
        ]}),
        &args,
    )?;
    let stone = state
        .registry
        .iter()
        .position(|entry| {
            entry
                .as_ref()
                .is_some_and(|block| block.name == "minecraft:stone")
        })
        .unwrap() as i32;
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.creative = false;
    sync_to_origin(&mut bot);
    let (mut client, mut peer, mut buffer) = connection().await?;
    assert!(!bot.mechanic_action(&mut client, 0).await?);
    assert_eq!(
        bot.mechanic_bot.last_unavailable_reason,
        Some("target_cell_not_received")
    );
    shared.lock().cell(0, 0, stone, true, None);
    assert!(bot.mechanic_action(&mut client, 0).await?);
    assert!(bot.mechanic_action(&mut client, 1).await?);
    bot.complete_mechanics(Some("steady"));
    assert_eq!(shared.lock().actions[1].completed, 0);
    for expected in [
        PlayerActionKind::StartDestroyBlock,
        PlayerActionKind::StopDestroyBlock,
    ] {
        let mut frame = loop {
            if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                break frame;
            }
            peer.read_buf(&mut buffer).await?;
        };
        assert_eq!(frame.id, ServerboundPlayerAction::ID);
        let packet = ServerboundPlayerAction::decode(&mut frame.body)?;
        assert_eq!(packet.action, expected);
        assert_eq!(packet.position, pack_block_pos(0, 64, 0));
    }
    shared.lock().cell(0, 0, 0, false, Some("steady"));
    bot.complete_mechanics(Some("steady"));
    assert_eq!(shared.lock().actions[1].completed, 1);
    Ok(())
}

#[test]
fn command_offers_exclude_the_window_end_boundary() -> Result<()> {
    let schedule: OpenLoopSchedule = serde_json::from_value(json!({"windows":[
        {"name":"warmup","duration_ms":2000,"command_interval_ms":500,"commands":["say x"]}
    ]}))?;
    let window = &schedule.windows[0];
    assert_eq!(window.offered_commands(Duration::from_millis(1999)), 3);
    assert_eq!(window.offered_commands(Duration::from_millis(2000)), 3);
    assert_eq!(window.offered_commands(Duration::from_millis(3000)), 3);
    Ok(())
}

#[test]
fn anchor_chunk_landing_checks_the_final_stride_position() {
    assert_eq!(landing_step_in_chunk(2104, 8, 0, -1, 12, Some((131, 0))), None);
    assert_eq!(landing_step_in_chunk(2104, 12, 0, -1, 4, Some((131, 0))), Some((2104, 8)));
    assert_eq!(landing_step_in_chunk(2104, 8, 0, -1, 12, None), Some((2104, -4)));
    assert_eq!(landing_step_in_chunk(-1, -1, -1, 0, 1, Some((-1, -1))), Some((-2, -1)));
}

fn mechanic_test_entity(state: &Mechanics, kind: &str, id: i32) -> AddEntity {
    AddEntity {
        entity_id: id,
        uuid: Uuid::from_u128(id as u128),
        entity_type_id: *state
            .kinds
            .iter()
            .find(|(_, name)| name.as_str() == kind)
            .expect("declared kind")
            .0,
        x: 1.0,
        y: 64.0,
        z: 1.0,
        movement: EntityVec3::ZERO,
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        data: 0,
    }
}
#[test]
fn collection_receipt_only_resolves_entities_visible_to_observer() -> Result<()> {
    let mut args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[{"kind":"minecraft:item","count":1},
                          {"kind":"minecraft:villager","count":1}]}),
        &args,
    )?;
    let item = mechanic_test_entity(&state, "minecraft:item", 41);
    let villager = mechanic_test_entity(&state, "minecraft:villager", 42);
    state.spawn(0, &item, None);
    state.spawn(1, &villager, None);
    state.entities.get_mut(&item.uuid).unwrap().item = Some(SlotImage {
        slot: 0,
        item_id: Some(5),
        name: Some("minecraft:wheat".into()),
        count: 3,
        plain: true,
    });
    state.entities.get_mut(&item.uuid).unwrap().item_observer = Some(0);
    state.begin_measurement()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(state)));
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.mechanic_frame(
        &mechanic_test_frame(&ClientboundTakeItemEntity {
            item_entity_id: 41,
            player_entity_id: 42,
            amount: 2,
        })?,
        Some("steady"),
    )?;
    let state = args.mechanics.as_ref().unwrap().lock();
    let report = state.json(Some(Duration::from_secs(1)), false);
    let event = &report["item_collections"][0];
    assert_eq!(event["observer_bot"], 0);
    assert_eq!(event["item_entity_id"], 41);
    assert_eq!(event["collector_entity_id"], 42);
    assert_eq!(event["item_uuid"], item.uuid.to_string());
    assert!(event["collector_uuid"].is_null());
    assert_eq!(event["item"]["name"], "minecraft:wheat");
    assert_eq!(event["amount"], 2);
    assert_eq!(report["item_collections_truncated"], 0);
    Ok(())
}

#[test]
fn first_self_damage_keeps_received_source_and_labels_predicted_victim_pose() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[
            {"kind":"minecraft:zombie","count":1,"reader_images":true}
        ]}),
        &args,
    )?;
    let attacker = mechanic_test_entity(&state, "minecraft:zombie", 41);
    state.spawn(0, &attacker, None);
    state.begin_measurement()?;
    let packet = ClientboundDamageEvent {
        entity_id: 77,
        source_type_id: 1,
        source_cause_id: 41,
        source_direct_id: 41,
        source_position: None,
    };
    let event = state
        .first_self_damage(0, &packet, [3.0, 64.0, 5.0], Some("steady"))
        .unwrap();
    assert_eq!(
        event["victim_client_predicted_position"],
        json!([3.0, 64.0, 5.0])
    );
    assert_eq!(event["cause"]["uuid"], attacker.uuid.to_string());
    assert_eq!(event["cause"]["kind"], "minecraft:zombie");
    assert_eq!(event["cause"]["received_position"], json!([1.0, 64.0, 1.0]));
    assert!(
        state
            .first_self_damage(0, &packet, [4.0, 64.0, 5.0], Some("steady"))
            .is_none()
    );
    state.remove(0, 41, Some("steady"));
    assert!(state.observed_damage_source(0, 41).is_null());
    Ok(())
}

#[tokio::test]
async fn setup_world_fence_waits_for_late_geometry_after_chat_echo() -> Result<()> {
    let mut bot = terrain_bot(0.5, 67, 0.5, 0);
    bot.surfaces.get_mut(&(0, 0)).unwrap().setup_cells =
        Some(vec![0; HEIGHT * 256].into_boxed_slice());
    bot.update(pack_block_pos(0, 63, 0), 1, "test", None);
    bot.update(pack_block_pos(0, 66, 0), 1, "test", None);
    assert_eq!(bot.landing_height(0, 0), Some(67));
    let (mut client, mut peer, mut buffer) = connection().await?;
    let server = tokio::spawn(async move {
        let mut request = loop {
            if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                break frame;
            }
            ensure!(
                peer.read_buf(&mut buffer).await? != 0,
                "world fence socket closed"
            );
        };
        assert_eq!(request.id, ServerboundCustomPayload::ID);
        let request = ServerboundCustomPayload::decode(&mut request.body)?;
        let echo = system_chat_frame("<scale_0000> setup echo")?;
        peer.write_all(&mc_protocol::frame::encode_frame(
            echo.id,
            &echo.body,
            Compression::Disabled,
        )?)
        .await?;
        let mut body = Vec::new();
        ClientboundKeepAlive { id: 42 }.encode(&mut body)?;
        peer.write_all(&mc_protocol::frame::encode_frame(
            ClientboundKeepAlive::ID,
            &body,
            Compression::Disabled,
        )?)
        .await?;
        let mut reply = loop {
            if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                break frame;
            }
            ensure!(
                peer.read_buf(&mut buffer).await? != 0,
                "world fence heartbeat closed"
            );
        };
        assert_eq!(ServerboundKeepAlive::decode(&mut reply.body)?.id, 42);
        let mut body = Vec::new();
        BlockUpdate {
            position: pack_block_pos(0, 66, 0),
            state_id: 0,
        }
        .encode(&mut body)?;
        peer.write_all(&mc_protocol::frame::encode_frame(
            BlockUpdate::ID,
            &body,
            Compression::Disabled,
        )?)
        .await?;
        let mut body = Vec::new();
        ClientboundCustomPayload {
            payload: request.payload,
        }
        .encode(&mut body)?;
        peer.write_all(&mc_protocol::frame::encode_frame(
            ClientboundCustomPayload::ID,
            &body,
            Compression::Disabled,
        )?)
        .await?;
        Ok::<(), anyhow::Error>(())
    });
    tokio::time::timeout(Duration::from_secs(5), bot.setup_world_fence(&mut client)).await??;
    tokio::time::timeout(Duration::from_secs(5), server).await???;
    assert_eq!(bot.landing_height(0, 0), Some(64));
    Ok(())
}

#[tokio::test]
async fn frame_setup_rejects_missing_save_before_any_packet() -> Result<()> {
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.commands = Arc::new(CommandSchedule {
        setup: vec!["say must_not_be_sent".into()],
        frame_positions: vec![[0, 65, 0]],
        ..Default::default()
    });
    let (mut client, _peer, _buffer) = connection().await?;
    let error = bot.setup_commands(&mut client, true).await.unwrap_err();
    assert!(error.to_string().contains("positions and a final save"));
    assert_eq!(bot.metrics.commands_sent, 0);
    Ok(())
}

#[tokio::test(start_paused = true)]
async fn preparation_fence_uses_total_readiness_deadline_and_keeps_echoing_heartbeats() -> Result<()>
{
    // Real socket readiness must not auto-advance the paused clock to the
    // enclosing timeout before the kernel delivers already-written bytes.
    let clock_guard = tokio::spawn(async {
        loop {
            tokio::task::yield_now().await;
        }
    });
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    let username = bot.username.clone();
    let (mut client, mut peer, mut buffer) = connection().await?;
    let server = tokio::spawn(async move {
        let mut fence = loop {
            if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                break frame;
            }
            ensure!(
                peer.read_buf(&mut buffer).await? != 0,
                "fence socket closed"
            );
        };
        assert_eq!(fence.id, ServerboundChat::ID);
        let chat = ServerboundChat::decode(&mut fence.body)?;
        let mut body = Vec::new();
        ClientboundKeepAlive { id: 42 }.encode(&mut body)?;
        let keepalive = mc_protocol::frame::encode_frame(
            ClientboundKeepAlive::ID,
            &body,
            Compression::Disabled,
        )?;
        peer.write_all(&keepalive).await?;
        let mut reply = loop {
            if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                break frame;
            }
            ensure!(
                peer.read_buf(&mut buffer).await? != 0,
                "heartbeat socket closed"
            );
        };
        assert_eq!(reply.id, ServerboundKeepAlive::ID);
        assert_eq!(ServerboundKeepAlive::decode(&mut reply.body)?.id, 42);
        // A real checkpoint may exceed the former per-fence 60-second timer.
        tokio::time::advance(Duration::from_secs(65)).await;
        let echo = system_chat_frame(&format!("<{username}> {}", chat.message))?;
        peer.write_all(&mc_protocol::frame::encode_frame(
            echo.id,
            &echo.body,
            Compression::Disabled,
        )?)
        .await?;
        Ok::<(), anyhow::Error>(())
    });
    tokio::time::timeout(
        Duration::from_secs(120),
        bot.setup_fence(&mut client, "slow_checkpoint", true),
    )
    .await??;
    server.await??;
    assert_eq!(bot.metrics.fixture_input_fences_completed, 1);
    assert_eq!(bot.metrics.keepalives_replied, 1);

    // The enclosing preparation budget still bounds a server that never echoes.
    let (mut client, _peer, _buffer) = connection().await?;
    let expire = tokio::spawn(async {
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(1)).await;
    });
    assert!(
        tokio::time::timeout(
            Duration::from_secs(1),
            bot.setup_fence(&mut client, "never_echoes", true),
        )
        .await
        .is_err()
    );
    expire.await?;
    clock_guard.abort();
    let _ = clock_guard.await;
    Ok(())
}

fn sync_to_origin(bot: &mut Bot) {
    bot.position_sync_reply(&SynchronizePlayerPosition {
        teleport_id: 7,
        x: 0.5,
        y: 64.0,
        z: 0.5,
        dx: 0.0,
        dy: 0.0,
        dz: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        relative_flags: 0,
    });
}

fn mechanic_test_frame<P: Packet>(packet: &P) -> Result<RawFrame> {
    let mut body = BytesMut::new();
    packet.encode(&mut body)?;
    Ok(RawFrame {
        id: P::ID,
        body: body.freeze(),
    })
}
