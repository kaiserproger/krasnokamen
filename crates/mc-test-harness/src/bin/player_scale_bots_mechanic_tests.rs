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
fn named_click_slot_uses_received_inventory_not_server_slot_assumptions() -> Result<()> {
    let args = mechanic_test_args()?;
    let state = mechanic_test_state(
        json!({"actions":[{"bot":0,"kind":"click_slot","slot_item":"minecraft:oak_log",
                          "window":"steady","at_ms":0}]}),
        &args,
    )?;
    let action = &state.spec.actions[0];
    let mut menu = MenuReceipt::default();
    assert_eq!(action.menu_slot(&menu), None);
    for slot in [10, 38] {
        menu.final_player_inventory = Some(vec![SlotImage {
            slot,
            item_id: Some(134),
            name: Some("minecraft:oak_log".into()),
            count: 1,
            plain: true,
        }]);
        assert_eq!(action.menu_slot(&menu), Some(slot as i16));
        menu.final_player_inventory.as_mut().unwrap()[0].count = 0;
        assert_eq!(action.menu_slot(&menu), None);
    }
    for action in [
        json!({"bot":0,"kind":"click_slot","slot":38,"slot_item":"minecraft:oak_log"}),
        json!({"bot":0,"kind":"select_slot","slot_item":"minecraft:oak_log"}),
    ] {
        assert!(mechanic_test_state(json!({"actions":[action]}), &args).is_err());
    }
    Ok(())
}

#[test]
fn absolute_block_deltas_survive_observer_handoff_without_duplicate_work() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"blocks":[{"name":"observed","position":[0,63,0]}]}),
        &args,
    )?;
    state.cell(0, 0, 0, true, None);
    state.begin_measurement()?;
    state.cell(1, 0, 1, false, Some("steady"));
    assert_eq!(state.cells[0].initial, Some(0));
    assert_eq!(state.cells[0].final_state, Some(1));
    assert_eq!(state.cells[0].update_count, 1);
    state.cell(0, 0, 1, false, Some("steady"));
    assert_eq!(state.cells[0].update_count, 1);
    state.cell(1, 0, 0, true, Some("steady"));
    assert_eq!(state.cells[0].final_state, Some(1));
    assert_eq!(state.cells[0].update_count, 1);
    Ok(())
}

#[test]
fn selected_reader_block_images_do_not_inherit_another_viewers_update() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"blocks":[
            {"name":"selected","position":[0,63,0],"reader_images":true},
            {"name":"shared","position":[1,63,0]}
        ]}),
        &args,
    )?;
    state.cell(0, 0, 0, true, None);
    state.cell(1, 0, 0, true, None);
    state.begin_measurement()?;
    state.cell(0, 0, 1, false, Some("burst"));
    let report = state.json(Some(Duration::from_secs(3)), false);
    assert_eq!(report["players"][0]["final_blocks"]["selected"]["state"], 1);
    assert_eq!(report["players"][1]["final_blocks"]["selected"]["state"], 0);
    assert!(report["players"][1]["final_blocks"].get("shared").is_none());
    assert!(state.cells[1].reader_states.is_empty());
    Ok(())
}

#[test]
fn selected_reader_entity_images_keep_independent_baselines_and_clear_on_remove() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[{"kind":"minecraft:armor_stand","count":1,"reader_images":true}]}),
        &args,
    )?;
    let mut packet = mechanic_test_entity(&state, "minecraft:armor_stand", 41);
    state.spawn(0, &packet, None);
    state.spawn(1, &packet, None);
    state.begin_measurement()?;
    state.movement(0, 41, [8.0, 64.0, 1.0], false, Some("burst"));
    state.movement(1, 41, [1.0, 0.0, 0.0], true, Some("burst"));
    let report = state.json(Some(Duration::from_secs(3)), false);
    assert_eq!(
        report["players"][0]["final_observed_entities"][0]["final_position"],
        json!([8.0, 64.0, 1.0])
    );
    assert_eq!(
        report["players"][1]["final_observed_entities"][0]["final_position"],
        json!([2.0, 64.0, 1.0])
    );
    state.remove(1, 41, Some("recovery"));
    assert_eq!(state.entities[&packet.uuid].reader_positions[1], None);
    packet.x = 9.0;
    state.spawn(1, &packet, Some("recovery"));
    assert_eq!(
        state.entities[&packet.uuid].reader_positions[1],
        Some([9.0, 64.0, 1.0])
    );
    Ok(())
}

#[test]
fn finite_mechanic_slots_use_wall_epoch_and_forbid_recovery() -> Result<()> {
    let args = mechanic_test_args()?;
    let state = mechanic_test_state(
        json!({"actions":[
            {"id":"first","bot":0,"kind":"set_direction","direction":0},
            {"id":"second","bot":0,"kind":"set_direction","direction":2}
        ]}),
        &args,
    )?;
    assert_eq!(
        state
            .actions
            .iter()
            .map(|a| (&a.window, a.at_ms))
            .collect::<Vec<_>>(),
        vec![
            (&"steady".to_owned(), 500),
            (&"burst".to_owned(), 1250),
            (&"burst".to_owned(), 1500)
        ]
    );
    let report = state.json(Some(Duration::from_millis(1600)), false);
    assert_eq!(report["actions"][0]["offered"], 1);
    assert_eq!(report["actions"][0]["admitted"], 0);
    assert_eq!(report["actions"][0]["dropped_late"], 1);
    assert_eq!(report["windows"]["recovery"]["block_updates"], 0);
    let denied = mechanic_test_state(
        json!({"actions":[{"bot":0,"kind":"respawn","window":"recovery","at_ms":0}]}),
        &args,
    );
    assert!(denied.is_err());
    Ok(())
}

#[test]
fn arbitrary_negative_section_cells_ignore_advertised_heightmaps() -> Result<()> {
    use mc_protocol::codec::WriteMc;
    let args = mechanic_test_args()?;
    let shared = Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({"blocks":[
            {"name":"deep","position":[-9,-48,-7]},
            {"name":"top","position":[-9,319,-7]}
        ]}),
        &args,
    )?));
    let mut body = Vec::new();
    WriteMc::write_i32(&mut body, -1);
    WriteMc::write_i32(&mut body, -1);
    body.write_varint(1);
    body.write_varint(5);
    body.write_varint(37);
    for _ in 0..37 {
        body.extend_from_slice(&0_u64.to_be_bytes());
    }
    let mut sections = Vec::new();
    for section in 0..HEIGHT / 16 {
        WriteMc::write_i16(&mut sections, 0);
        WriteMc::write_i16(&mut sections, 0);
        WriteMc::write_u8(&mut sections, 0);
        sections.write_varint(i32::from(section == 1));
        WriteMc::write_u8(&mut sections, 0);
        sections.write_varint(0);
    }
    body.write_varint(i32::try_from(sections.len())?);
    body.extend_from_slice(&sections);
    let facts = Facts::load()?;
    read_surface_mechanics(&body, &facts, Some((&shared, 0, None)), false)?;
    let mut state = shared.lock();
    assert_eq!(state.cells[0].initial, Some(1));
    assert_eq!(state.cells[1].initial, Some(0));
    // Re-streams from another client cannot manufacture another physical transition.
    state.cell(1, 0, 0, true, Some("steady"));
    assert_eq!(state.cells[0].final_state, Some(1));
    state.update_cell(0, pack_block_pos(-9, -48, -7), 0, Some("burst"));
    state.update_cell(0, pack_block_pos(-9, -48, -7), 0, Some("burst"));
    let report = state.json(None, false);
    assert_eq!(report["blocks"]["deep"]["update_count"], 1);
    assert_eq!(
        report["blocks"]["deep"]["final"]["name"],
        state.registry[0].as_ref().unwrap().name
    );
    assert_eq!(report["windows"]["burst"]["block_updates"], 1);
    Ok(())
}

#[test]
fn missing_physical_and_menu_probes_remain_null_not_synthetic_air() -> Result<()> {
    let args = mechanic_test_args()?;
    let state = mechanic_test_state(
        json!({"blocks":[{"name":"absent","position":[0,64,0]}],"containers":[{"name":"unopened","position":[0,64,0],"slots":3}]}),
        &args,
    )?;
    let report = state.json(None, true);
    assert!(report["blocks"]["absent"]["initial"].is_null());
    assert!(report["blocks"]["absent"]["final"].is_null());
    assert!(report["containers"]["unopened"]["initial"].is_null());
    assert_eq!(report["missing_probes"]["blocks"], json!(["absent"]));
    assert_eq!(report["missing_probes"]["containers"], json!(["unopened"]));
    Ok(())
}

#[test]
fn entity_identity_motion_and_ghost_checks_are_received_state() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[{"kind":"minecraft:zombie","count":1}]}),
        &args,
    )?;
    let type_id = *state.kinds.keys().next().unwrap();
    let packet = AddEntity {
        entity_id: 41,
        uuid: Uuid::from_u128(42),
        entity_type_id: type_id,
        x: 1.0,
        y: 64.0,
        z: 1.0,
        movement: EntityVec3::ZERO,
        pitch: 0.0,
        yaw: 0.0,
        head_yaw: 0.0,
        data: 0,
    };
    state.spawn(0, &packet, None);
    state.spawn(1, &packet, None);
    assert_eq!(state.entities.len(), 1);
    assert_eq!(state.duplicate_spawns, 0);
    state.movement(0, 41, [0.0, 0.0, 0.0], true, Some("steady"));
    assert_eq!(
        state.json(None, false)["windows"]["steady"]["entity_moves"],
        0
    );
    state.movement(0, 41, [0.5, 0.0, 0.0], true, Some("burst"));
    assert_eq!(
        state.entities[&packet.uuid].final_position,
        [1.5, 64.0, 1.0]
    );
    assert_eq!(state.entities[&packet.uuid].move_count, 1);
    state.spawn(0, &packet, None);
    assert_eq!(state.duplicate_spawns, 1);
    state.remove(0, 41, Some("burst"));
    state.movement(0, 41, [1.0, 0.0, 0.0], true, Some("recovery"));
    assert_eq!(state.ghost_updates, 1);
    let mut conflict = packet.clone();
    conflict.entity_id = 43;
    state.spawn(0, &conflict, None);
    assert_eq!(state.identity_errors, 1);
    Ok(())
}

#[test]
fn click_completion_requires_conserved_source_debit_and_recipient_credit() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"containers":[{"name":"furnace","position":[0,64,0],"slots":3}]}),
        &args,
    )?;
    let item = state
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:iron_ore")
        .map(|(id, _)| *id)
        .unwrap();
    let mut items = vec![ItemStack::EMPTY; 39];
    items[30] = ItemStack::new(item, 64);
    state.menu_image(0, 1, &items, &ItemStack::EMPTY, None)?;
    let transfer = MenuTransfer::capture(0, &state.menus[0], 30, true).unwrap();
    assert!(!transfer.completed(&state.menus[0]));
    // A source debit alone is not a receipt: the credit must actually arrive.
    state.menu_slot(0, 2, 30, &ItemStack::EMPTY, Some("steady"))?;
    assert!(!transfer.completed(&state.menus[0]));
    state.menu_slot(0, 2, 0, &ItemStack::new(item, 64), Some("steady"))?;
    assert!(transfer.completed(&state.menus[0]));
    assert_eq!(state.menus[0].initial.as_ref().unwrap()[0].count, 0);
    assert_eq!(state.menus[0].final_slots.as_ref().unwrap()[0].count, 64);
    let before = state.menus[0].updates;
    state.menu_slot(0, 2, 0, &ItemStack::new(item, 64), Some("steady"))?;
    assert_eq!(state.menus[0].updates, before);
    Ok(())
}

#[test]
fn net_zero_container_slot_changes_exclude_player_and_carried_updates() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"containers":[{"name":"hopper","position":[0,64,0],"slots":5}]}),
        &args,
    )?;
    let item = *state
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:dirt")
        .unwrap()
        .0;
    let mut items = vec![ItemStack::EMPTY; 41];
    items[0] = ItemStack::new(item, 32);
    state.menu_image(0, 0, &items, &ItemStack::EMPTY, None)?;
    items[5] = ItemStack::new(item, 64);
    state.menu_image(0, 1, &items, &ItemStack::EMPTY, Some("steady"))?;
    assert_eq!(state.menus[0].container_slot_updates, 0);
    items[0] = ItemStack::new(item, 16);
    state.menu_image(0, 2, &items, &ItemStack::EMPTY, Some("steady"))?;
    items[0] = ItemStack::new(item, 32);
    state.menu_image(0, 3, &items, &ItemStack::EMPTY, Some("steady"))?;
    assert_eq!(state.menus[0].container_slot_updates, 2);
    state.menu_slot(0, 4, 5, &ItemStack::EMPTY, Some("burst"))?;
    state.menu_slot(0, 5, -1, &ItemStack::new(item, 1), Some("burst"))?;
    assert_eq!(state.menus[0].container_slot_updates, 2);
    state.menu_slot(0, 6, 0, &ItemStack::new(item, 16), Some("burst"))?;
    state.menu_slot(0, 7, 0, &ItemStack::new(item, 32), Some("burst"))?;
    assert_eq!(state.menus[0].container_slot_updates, 4);
    assert!(state.menus[0].initial == state.menus[0].final_slots);
    assert_eq!(
        state.json(None, false)["containers"]["hopper"]["container_slot_updates"],
        4
    );
    state.begin_measurement()?;
    assert_eq!(state.menus[0].container_slot_updates, 0);
    Ok(())
}

#[test]
fn sparse_command_intervals_preserve_wall_offers_and_recovery_zero() -> Result<()> {
    let schedule: OpenLoopSchedule = serde_json::from_value(json!({"windows":[
        {"name":"steady","duration_ms":25000,"command_interval_ms":6000,"commands":["say load"]},
        {"name":"recovery","duration_ms":1000,"recovery":true}
    ]}))?;
    schedule.validate()?;
    let window = &schedule.windows[0];
    assert_eq!(window.command_slots(Duration::from_millis(5999)), 0);
    assert_eq!(window.command_slots(Duration::from_millis(25000)), 4);
    let mut accounted = 0;
    assert_eq!(
        window.due_command_slot(Duration::from_millis(19000), &mut accounted),
        Some((3, 1_000_000))
    );
    assert_eq!(
        window.due_command_slot(Duration::from_millis(19000), &mut accounted),
        None
    );
    let invalid: OpenLoopSchedule = serde_json::from_value(
        json!({"windows":[{"name":"recovery","duration_ms":1000,"recovery":true,"command_interval_ms":10000,"commands":["save-all flush"]}]}),
    )?;
    assert!(invalid.validate().is_err());
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

#[tokio::test]
async fn fresh_menu_open_is_not_completed_by_an_old_image() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({"containers":[{"name":"furnace","position":[0,64,0],"slots":3}],"actions":[{"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":0}]}),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    shared
        .lock()
        .menu_image(0, 1, &vec![ItemStack::EMPTY; 39], &ItemStack::EMPTY, None)?;
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 0.5;
    bot.y = 64.0;
    bot.z = 0.5;
    let (mut client, _peer, _buffer) = connection().await?;
    assert!(bot.mechanic_action(&mut client, 0).await?);
    bot.complete_mechanics(Some("steady"));
    assert_eq!(shared.lock().actions[0].completed, 0);
    shared.lock().menu_image(
        0,
        2,
        &vec![ItemStack::EMPTY; 39],
        &ItemStack::EMPTY,
        Some("steady"),
    )?;
    bot.complete_mechanics(Some("steady"));
    assert_eq!(shared.lock().actions[0].completed, 1);
    Ok(())
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
fn entity_visibility_union_needs_absolute_primary_rebaseline() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[{"kind":"minecraft:zombie","count":1}]}),
        &args,
    )?;
    let packet = mechanic_test_entity(&state, "minecraft:zombie", 41);
    state.spawn(0, &packet, None);
    state.spawn(1, &packet, None);
    state.remove(0, 41, Some("steady"));
    assert!(state.entities[&packet.uuid].active);
    assert_eq!(state.entities[&packet.uuid].visible_observers, 1);
    state.movement(1, 41, [1.0, 0.0, 0.0], true, Some("steady"));
    assert_eq!(state.position_baseline_misses, 1);
    assert_eq!(
        state.entities[&packet.uuid].final_position,
        [1.0, 64.0, 1.0]
    );
    state.movement(1, 41, [2.0, 64.0, 1.0], false, Some("burst"));
    assert_eq!(state.entities[&packet.uuid].observer_bot, 1);
    assert_eq!(state.entities[&packet.uuid].move_count, 1);
    state.remove(1, 41, Some("recovery"));
    assert!(!state.entities[&packet.uuid].active);
    state.remove(1, 41, Some("recovery"));
    assert_eq!(state.duplicate_removals, 1);
    assert_eq!(state.ghost_updates, 0);
    assert_eq!(
        state.receive_windows["recovery"].get("entity_remove"),
        Some(&1)
    );
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

#[test]
fn own_projectile_hits_keep_causal_uuid_and_count_duplicates() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({"entities":[
            {"kind":"minecraft:skeleton","count":1}, {"kind":"minecraft:arrow","count":1}
        ]}),
        &args,
    )?;
    let owner = mechanic_test_entity(&state, "minecraft:skeleton", 41);
    let arrow = mechanic_test_entity(&state, "minecraft:arrow", 42);
    state.spawn(0, &owner, None);
    state.spawn(0, &arrow, Some("steady"));
    let packet = ClientboundDamageEvent {
        entity_id: 77,
        source_type_id: 1,
        source_cause_id: 41,
        source_direct_id: 42,
        source_position: None,
    };
    state.damage_event(0, 77, &packet, Some("steady"));
    state.damage_event(0, 77, &packet, Some("steady"));
    // Another client's fanout must not manufacture a second victim.
    state.damage_event(1, 88, &packet, Some("steady"));
    assert_eq!(state.projectile_hits.len(), 1);
    assert_eq!(state.projectile_duplicate_hits, 1);
    assert_eq!(state.damage_events_filtered, 1);
    assert_eq!(state.damage_events_received, 3);
    assert_eq!(state.projectile_hits[0].projectile_uuid, arrow.uuid);
    assert_eq!(state.projectile_hits[0].owner_uuid, owner.uuid);
    assert_eq!(state.projectile_hits[0].target_bot, 0);
    let unresolved = ClientboundDamageEvent {
        source_cause_id: 99,
        ..packet
    };
    state.damage_event(0, 77, &unresolved, Some("burst"));
    assert_eq!(state.damage_events_unresolved, 1);
    assert_eq!(state.projectile_hits.len(), 1);
    assert!(state.damage_events.last().unwrap().cause_uuid.is_none());
    Ok(())
}

#[test]
fn literal_command_replies_filter_only_generated_chat_and_stay_bounded() -> Result<()> {
    use mc_nbt::Tag;
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(json!({}), &args)?;
    let literal = Tag::Compound(vec![
        ("text".into(), Tag::String("Saved the game".into())),
        ("bold".into(), Tag::Byte(1)),
    ]);
    assert_eq!(
        received_literal_text(&literal).as_deref(),
        Some("Saved the game")
    );
    assert_eq!(
        received_literal_text(&Tag::Compound(vec![(
            "translate".into(),
            Tag::String("unrendered.key".into())
        ),])),
        None
    );
    assert_eq!(
        received_literal_text(&Tag::String("x".repeat(MECHANIC_REPLY_BYTES + 1))),
        None
    );
    for _ in 0..MECHANIC_REPLY_CAP + 1 {
        state.record_reply(
            0,
            Some("steady"),
            &Tag::String("<scale_1> scale_open_loop_42_1_0_1".into()),
        );
    }
    state.record_reply(
        0,
        None,
        &Tag::String("<scale_0> scale_setup_fence_123_0_batch_1".into()),
    );
    assert!(state.command_replies.is_empty());
    assert_eq!(state.command_replies_truncated, 0);
    assert_eq!(
        state.command_replies_filtered,
        (MECHANIC_REPLY_CAP + 2) as u64
    );
    assert!(!harness_chat_echo("Saved the game"));
    assert!(!harness_chat_echo("<scale_0> scale_open_loop_bad_1_0_1"));
    state.record_reply(0, Some("burst"), &literal);
    assert_eq!(state.command_replies[0].text, "Saved the game");
    assert_eq!(state.command_replies[0].window.as_deref(), Some("burst"));
    for _ in 0..MECHANIC_REPLY_CAP {
        state.record_reply(0, Some("burst"), &literal);
    }
    assert_eq!(state.command_replies.len(), MECHANIC_REPLY_CAP);
    assert_eq!(state.command_replies_truncated, 1);
    Ok(())
}

#[test]
fn survival_transition_offers_are_wall_derived_not_poll_derived() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.survival_on_run = true;
    let mut state = mechanic_test_state(json!({}), &args)?;
    assert_eq!(
        state.json(None, false)["players"][0]["survival_transition_offered"],
        0
    );
    assert_eq!(
        state.json(Some(Duration::ZERO), true)["players"][0]["survival_transition_offered"],
        0
    );
    state.players[0].survival_transition_admitted = 1;
    let report = state.json(Some(Duration::from_secs(3)), false);
    assert_eq!(report["players"][0]["survival_transition_offered"], 1);
    assert_eq!(report["players"][0]["survival_transition_dropped_late"], 0);
    assert_eq!(report["players"][1]["survival_transition_dropped_late"], 1);
    Ok(())
}

#[test]
fn ready_fence_census_is_distinct_clients_not_extra_batch_echoes() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.open_loop = true;
    let snapshots = vec![
        Arc::new(Mutex::new(Counters {
            fixture_input_fences_completed: 65,
            ..Counters::default()
        })),
        Arc::new(Mutex::new(Counters::default())),
    ];
    let report = aggregate(&snapshots, &args, 2, Instant::now(), "ready", None)?;
    assert_eq!(report["ready_input_fenced_clients"], 1);
    assert_eq!(report["ready_input_fences_complete"], false);
    snapshots[1]
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .fixture_input_fences_completed = 1;
    let report = aggregate(&snapshots, &args, 2, Instant::now(), "ready", None)?;
    assert_eq!(report["ready_input_fences_complete"], true);
    Ok(())
}

#[tokio::test]
async fn no_op_setup_batches_finish_on_unique_chat_not_feedback_count() -> Result<()> {
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.commands = Arc::new(CommandSchedule {
        setup: (0..65).map(|_| "clear @s".to_owned()).collect(),
        recurring: Vec::new(),
        ..Default::default()
    });
    let username = bot.username.clone();
    let (mut client, mut peer, mut buffer) = connection().await?;
    let server = tokio::spawn(async move {
        let mut commands = 0;
        let mut fences = 0;
        loop {
            let mut frame = loop {
                if let Some(frame) = try_decode_frame(&mut buffer, Compression::Disabled)? {
                    break frame;
                }
                ensure!(
                    peer.read_buf(&mut buffer).await? != 0,
                    "setup socket closed"
                );
            };
            if frame.id == ServerboundChatCommand::ID {
                assert_eq!(
                    ServerboundChatCommand::decode(&mut frame.body)?.command,
                    "clear @s"
                );
                commands += 1;
            } else {
                assert_eq!(frame.id, ServerboundChat::ID);
                let chat = ServerboundChat::decode(&mut frame.body)?;
                fences += 1;
                assert_eq!(commands, (fences * SETUP_COMMAND_BATCH).min(65));
                let echo = system_chat_frame(&format!("<{username}> {}", chat.message))?;
                let bytes =
                    mc_protocol::frame::encode_frame(echo.id, &echo.body, Compression::Disabled)?;
                peer.write_all(&bytes).await?;
                if fences == 3 {
                    return Ok::<_, anyhow::Error>((commands, fences));
                }
            }
        }
    });
    tokio::time::timeout(
        Duration::from_secs(5),
        bot.setup_commands(&mut client, true),
    )
    .await??;
    assert_eq!(server.await??, (65, 3));
    assert_eq!(bot.metrics.commands_sent, 65);
    assert_eq!(bot.metrics.fixture_input_fences_completed, 3);
    assert_eq!(bot.metrics.chats_sent, 0);
    assert_eq!(bot.metrics.chat_echoes_received, 0);
    assert!(bot.pending_chats.is_empty());
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

#[tokio::test]
async fn read_only_preparation_tracks_real_spawn_center() -> Result<()> {
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.mechanic_read_only = true;
    let (mut client, _peer, _buffer) = connection().await?;
    let mut body = BytesMut::new();
    SetCenterChunk {
        chunk_x: -3,
        chunk_z: 7,
    }
    .encode(&mut body)?;
    bot.frame(
        &mut client,
        RawFrame {
            id: SetCenterChunk::ID,
            body: body.freeze(),
        },
        2,
        true,
    )
    .await?;
    assert_eq!(bot.center, (-3, 7));
    assert!(bot.center_sync);
    Ok(())
}

#[tokio::test]
async fn automatic_respawn_defers_recovery_and_finished_deaths() -> Result<()> {
    let mut bot = terrain_bot(0.5, 64, 0.5, 0);
    bot.respawn = true;
    configure_open_loop(
        &mut bot,
        json!([
            {"name":"steady","duration_ms":1000},
            {"name":"recovery","duration_ms":1000,"recovery":true}
        ]),
        Duration::from_millis(1500),
    );
    let (mut client, mut peer, mut buffer) = connection().await?;
    let mut body = BytesMut::new();
    ClientboundSetHealth {
        health: 0.0,
        food: 20,
        saturation: 5.0,
    }
    .encode(&mut body)?;
    let frame = RawFrame {
        id: ClientboundSetHealth::ID,
        body: body.freeze(),
    };
    bot.frame(&mut client, frame.clone(), 2, false).await?;
    bot.frame(&mut client, frame.clone(), 2, false).await?;
    assert_eq!(bot.metrics.respawn_requests, 0);
    assert_eq!(bot.metrics.respawn_deferred, 1);
    assert_eq!(bot.metrics.windows[1].automatic_respawn_deferred, 1);
    bot.epoch
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
        .unwrap()
        .instant = Instant::now() - Duration::from_secs(3);
    bot.last_health = 20.0;
    bot.frame(&mut client, frame.clone(), 2, false).await?;
    assert_eq!(bot.metrics.respawn_deferred, 2);
    assert_eq!(bot.metrics.respawn_requests, 0);
    bot.epoch
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_mut()
        .unwrap()
        .instant = Instant::now();
    bot.last_health = 20.0;
    bot.frame(&mut client, frame, 2, false).await?;
    assert_eq!(bot.metrics.respawn_requests, 1);
    assert_eq!(bot.metrics.windows[0].automatic_respawn_requests, 1);
    tokio::time::timeout(Duration::from_secs(1), peer.read_buf(&mut buffer)).await??;
    let frame =
        try_decode_frame(&mut buffer, Compression::Disabled)?.expect("active respawn request");
    assert_eq!(frame.id, ServerboundClientCommand::ID);
    Ok(())
}

#[test]
fn wire_trace_is_selected_sequenced_and_separate_from_receipts() -> Result<()> {
    let mut args = mechanic_test_args()?;
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("wire.jsonl");
    args.wire_trace_bots = vec![0];
    args.wire_trace_sink = Some(Arc::new(Mutex::new(std::io::BufWriter::new(
        std::fs::File::create(&path)?,
    ))));
    let facts = Arc::new(Facts::load()?);
    let mut bot = Bot::new(
        0,
        &args,
        Arc::clone(&facts),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    let unselected = Bot::new(
        1,
        &args,
        facts,
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    assert!(unselected.wire_trace_sink.is_none());
    bot.trace_wire("command_written", json!({"command":"tp @s 0.5 64 0.5"}))?;
    bot.trace_wire("position_sync_received", json!({"teleport_id":42}))?;
    args.wire_trace_sink
        .as_ref()
        .unwrap()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .flush()?;
    let rows: Vec<Value> = std::fs::read_to_string(&path)?
        .lines()
        .map(serde_json::from_str)
        .collect::<std::result::Result<_, _>>()?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["sequence"], 1);
    assert_eq!(rows[1]["sequence"], 2);
    assert_eq!(rows[1]["details"]["teleport_id"], 42);
    assert_eq!(rows[0]["bot"], 0);
    assert!(rows[0]["unix_ns"].as_u64().is_some());
    assert!(rows[0]["elapsed_us"].is_null());
    assert_eq!(bot.metrics.commands_sent, 0);
    assert_eq!(bot.metrics.teleports_confirmed, 0);
    Ok(())
}

#[tokio::test]
async fn menu_opening_and_absent_close_do_not_claim_admission() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}],
            "actions":[
                {"bot":0,"kind":"close_container","window":"steady","at_ms":0},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":100},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":200},
                {"bot":0,"kind":"command","command":"tp @s 0.5 64 0.5","window":"steady","at_ms":300},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":400}
            ]
        }),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 0.5;
    bot.y = 64.0;
    bot.z = 0.5;
    let reply = bot.position_sync_reply(&SynchronizePlayerPosition {
        teleport_id: 42,
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
    assert_eq!(
        bot.mechanic_bot.last_position_sync.unwrap().1,
        [reply.x, reply.y, reply.z]
    );
    let (mut client, _peer, _buffer) = connection().await?;
    assert!(!bot.mechanic_action(&mut client, 0).await?);
    assert_eq!(
        bot.mechanic_bot.last_unavailable_reason,
        Some("no_received_open_menu")
    );
    assert!(bot.mechanic_action(&mut client, 1).await?);
    assert!(!bot.mechanic_action(&mut client, 2).await?);
    assert_eq!(
        bot.mechanic_bot.last_unavailable_reason,
        Some("menu_open_or_transfer_pending")
    );
    assert!(bot.mechanic_action(&mut client, 3).await?);
    assert_eq!(bot.mechanic_bot.last_command.unwrap().0, 3);
    assert!(bot.mechanic_bot.pose_sync_expected);
    bot.mechanic_bot.pending.clear();
    bot.mechanic_bot.opening = None;
    bot.x = 100.0;
    assert!(!bot.mechanic_action(&mut client, 4).await?);
    assert_eq!(
        bot.mechanic_bot.last_unavailable_reason,
        Some("outside_received_pose_reach")
    );
    let state = shared.lock();
    assert_eq!(state.actions[0].admitted, 0);
    assert_eq!(state.actions[1].writes_completed, 1);
    assert_eq!(state.actions[2].admitted, 0);
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

/// tp, open, one quick-move click; the bot starts far from the container.
fn delayed_pose_bot() -> Result<Bot> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}],
            "actions":[
                {"bot":0,"kind":"command","command":"tp @s 0.5 64 0.5","window":"steady","at_ms":0},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":100},
                {"bot":0,"kind":"click_slot","slot":30,"mode":1,"window":"steady","at_ms":200}
            ]
        }),
        &args,
    )?)));
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 100.0;
    bot.y = 64.0;
    bot.z = 0.5;
    Ok(bot)
}

#[tokio::test]
async fn open_waits_for_expected_pose_then_menu_then_click() -> Result<()> {
    let mut bot = delayed_pose_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    bot.mechanic_poll(&mut client, Duration::ZERO).await?;
    assert!(bot.mechanic_bot.pose_sync_expected);
    // The teleport has not been received: the open and the click wait for it.
    bot.mechanic_poll(&mut client, Duration::from_millis(100))
        .await?;
    bot.mechanic_poll(&mut client, Duration::from_millis(200))
        .await?;
    {
        let state = shared.lock();
        for action in [1, 2] {
            assert_eq!(state.actions[action].admitted, 0);
            assert_eq!(state.actions[action].dropped_unavailable, 0);
        }
    }
    assert_eq!(bot.mechanic_bot.held.len(), 2);
    // A poll without a sync does not re-evaluate or drop the held head.
    bot.mechanic_poll(&mut client, Duration::from_millis(250))
        .await?;
    assert_eq!(bot.mechanic_bot.held.len(), 2);

    sync_to_origin(&mut bot);
    assert!(!bot.mechanic_bot.pose_sync_expected);
    bot.mechanic_poll(&mut client, Duration::from_millis(300))
        .await?;
    {
        let state = shared.lock();
        assert_eq!(state.actions[1].admitted, 1);
        assert_eq!(state.actions[1].writes_completed, 1);
        assert_eq!(state.actions[2].admitted, 0);
    }
    // The click depends on the menu the open is still waiting for.
    assert_eq!(bot.mechanic_bot.held.len(), 1);

    let item = shared
        .lock()
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:iron_ore")
        .map(|(id, _)| *id)
        .unwrap();
    let mut title_nbt = Vec::new();
    mc_nbt::write_network_value(&mut title_nbt, &mc_nbt::Tag::String("furnace".into()))?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundOpenScreen {
            container_id: 42,
            menu_type: 13,
            title_nbt,
        })?,
        2,
        false,
    )
    .await?;
    let mut items = vec![ItemStack::EMPTY; 39];
    items[30] = ItemStack::new(item, 64);
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundContainerSetContent {
            container_id: 42,
            state_id: 1,
            items,
            carried_item: ItemStack::EMPTY,
        })?,
        2,
        false,
    )
    .await?;
    bot.mechanic_poll(&mut client, Duration::from_millis(350))
        .await?;
    let state = shared.lock();
    assert_eq!(state.actions[2].admitted, 1);
    assert_eq!(state.actions[2].writes_completed, 1);
    assert_eq!(state.actions[2].dropped_unavailable, 0);
    assert!(bot.mechanic_bot.held.is_empty());
    Ok(())
}

#[tokio::test]
async fn held_open_and_click_drop_when_pose_sync_is_still_out_of_reach() -> Result<()> {
    let mut bot = delayed_pose_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    for at in [0, 100, 200] {
        bot.mechanic_poll(&mut client, Duration::from_millis(at))
            .await?;
    }
    assert_eq!(bot.mechanic_bot.held.len(), 2);
    // The sync arrives but leaves the bot out of reach of the container.
    bot.position_sync_reply(&SynchronizePlayerPosition {
        teleport_id: 7,
        x: 100.0,
        y: 64.0,
        z: 0.5,
        dx: 0.0,
        dy: 0.0,
        dz: 0.0,
        yaw: 0.0,
        pitch: 0.0,
        relative_flags: 0,
    });
    bot.mechanic_poll(&mut client, Duration::from_millis(300))
        .await?;
    let state = shared.lock();
    for action in [1, 2] {
        assert_eq!(state.actions[action].admitted, 0);
        assert_eq!(state.actions[action].dropped_unavailable, 1);
    }
    assert!(bot.mechanic_bot.held.is_empty());
    Ok(())
}

#[tokio::test]
async fn held_actions_drop_at_window_end() -> Result<()> {
    let mut bot = delayed_pose_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    configure_open_loop(
        &mut bot,
        json!([
            {"name":"steady","duration_ms":1000},
            {"name":"burst","duration_ms":1000},
            {"name":"recovery","duration_ms":1000,"recovery":true}
        ]),
        Duration::ZERO,
    );
    let (mut client, _peer, _buffer) = connection().await?;
    for at in [0, 100, 200] {
        bot.mechanic_poll(&mut client, Duration::from_millis(at))
            .await?;
    }
    assert_eq!(bot.mechanic_bot.held.len(), 2);
    bot.mechanic_poll(&mut client, Duration::from_millis(1100))
        .await?;
    let state = shared.lock();
    for action in [1, 2] {
        assert_eq!(state.actions[action].dropped_unavailable, 1);
    }
    assert!(bot.mechanic_bot.held.is_empty());
    Ok(())
}

/// tp, open, three quick-move clicks (slots 30/31/32), close; the bot starts far
/// from the container so the open waits for the teleport's pose sync.
fn blocked_click_bot() -> Result<(Bot, [ItemStack; 3])> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}],
            "actions":[
                {"bot":0,"kind":"command","command":"tp @s 0.5 64 0.5","window":"steady","at_ms":0},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":100},
                {"bot":0,"kind":"click_slot","slot":30,"mode":1,"window":"steady","at_ms":200},
                {"bot":0,"kind":"click_slot","slot":31,"mode":1,"window":"steady","at_ms":300},
                {"bot":0,"kind":"click_slot","slot":32,"mode":1,"window":"steady","at_ms":400},
                {"bot":0,"kind":"close_container","window":"steady","at_ms":500}
            ]
        }),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 100.0;
    bot.y = 64.0;
    bot.z = 0.5;
    configure_open_loop(
        &mut bot,
        json!([
            {"name":"steady","duration_ms":1000},
            {"name":"burst","duration_ms":1000},
            {"name":"recovery","duration_ms":1000,"recovery":true}
        ]),
        Duration::ZERO,
    );
    let item = |name: &str| {
        shared
            .lock()
            .items
            .iter()
            .find(|(_, item)| item.as_str() == name)
            .map(|(id, _)| *id)
            .unwrap()
    };
    let stacks = [
        ItemStack::new(item("minecraft:iron_ore"), 64),
        ItemStack::new(item("minecraft:coal"), 64),
        ItemStack::new(item("minecraft:oak_planks"), 64),
    ];
    Ok((bot, stacks))
}

/// The server's open-screen and full content for the furnace menu.
async fn deliver_furnace_menu(
    bot: &mut Bot,
    client: &mut Client,
    stacks: &[ItemStack; 3],
) -> Result<()> {
    let mut title_nbt = Vec::new();
    mc_nbt::write_network_value(&mut title_nbt, &mc_nbt::Tag::String("furnace".into()))?;
    bot.frame(
        client,
        mechanic_test_frame(&ClientboundOpenScreen {
            container_id: 42,
            menu_type: 13,
            title_nbt,
        })?,
        2,
        false,
    )
    .await?;
    let mut items = vec![ItemStack::EMPTY; 39];
    for (slot, stack) in [30, 31, 32].into_iter().zip(stacks) {
        items[slot] = stack.clone();
    }
    bot.frame(
        client,
        mechanic_test_frame(&ClientboundContainerSetContent {
            container_id: 42,
            state_id: 1,
            items,
            carried_item: ItemStack::EMPTY,
        })?,
        2,
        false,
    )
    .await
}

/// The server's debit of `source` and credit of the furnace slot `source - 30`.
fn complete_transfer(
    shared: &Arc<MechanicMutex<Mechanics>>,
    version: i32,
    source: i16,
    stack: &ItemStack,
) -> Result<()> {
    let mut state = shared.lock();
    state.menu_slot(0, version, source, &ItemStack::EMPTY, Some("steady"))?;
    state.menu_slot(0, version, source - 30, stack, Some("steady"))
}

#[tokio::test]
async fn deferred_open_then_queued_clicks_and_close_are_all_admitted_in_order() -> Result<()> {
    let (mut bot, stacks) = blocked_click_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    // The teleport's pose sync is late: open, three clicks and close all wait.
    for at in [0, 100, 200, 300, 400, 500] {
        bot.mechanic_poll(&mut client, Duration::from_millis(at))
            .await?;
    }
    assert_eq!(bot.mechanic_bot.held.len(), 5);
    sync_to_origin(&mut bot);
    bot.mechanic_poll(&mut client, Duration::from_millis(600))
        .await?;
    assert_eq!(shared.lock().actions[1].admitted, 1);
    assert_eq!(shared.lock().actions[2].admitted, 0);
    // The menu arrives late too.
    deliver_furnace_menu(&mut bot, &mut client, &stacks).await?;
    // Each queued click is blocked by the previous click's pending transfer
    // until the server's debit and credit arrive.
    for (click, source) in [(2_usize, 30_i16), (3, 31), (4, 32)] {
        bot.mechanic_poll(&mut client, Duration::from_millis(600 + 50 * click as u64))
            .await?;
        {
            let state = shared.lock();
            assert_eq!(state.actions[click].admitted, 1, "click {click}");
            assert_eq!(state.actions[click].writes_completed, 1);
            assert_eq!(
                state.actions[click + 1].admitted,
                0,
                "blocked after {click}"
            );
        }
        complete_transfer(
            &shared,
            2 + click as i32,
            source,
            &stacks[usize::from(source as u16 - 30)],
        )?;
    }
    bot.mechanic_poll(&mut client, Duration::from_millis(850))
        .await?;
    let state = shared.lock();
    for action in 1..=5 {
        assert_eq!(state.actions[action].admitted, 1, "action {action}");
        assert_eq!(
            state.actions[action].dropped_unavailable, 0,
            "action {action}"
        );
    }
    assert_eq!(state.actions[5].writes_completed, 1);
    assert!(bot.mechanic_bot.held.is_empty());
    Ok(())
}

/// The pose and menu arrive before the clicks are offered; each later click and
/// the close are offered while the previous transfer is pending and must wait.
#[tokio::test]
async fn clicks_and_close_offered_during_a_pending_transfer_wait_for_it() -> Result<()> {
    let (mut bot, stacks) = blocked_click_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    for at in [0, 100] {
        bot.mechanic_poll(&mut client, Duration::from_millis(at))
            .await?;
    }
    assert_eq!(bot.mechanic_bot.held.len(), 1);
    sync_to_origin(&mut bot);
    bot.mechanic_poll(&mut client, Duration::from_millis(150))
        .await?;
    assert_eq!(shared.lock().actions[1].admitted, 1);
    deliver_furnace_menu(&mut bot, &mut client, &stacks).await?;
    // The first click is offered with the menu present and written directly.
    bot.mechanic_poll(&mut client, Duration::from_millis(200))
        .await?;
    assert_eq!(shared.lock().actions[2].admitted, 1);
    assert!(bot.mechanic_bot.held.is_empty());
    for (offer, source, next) in [(300, 30_i16, 3_usize), (400, 31, 4), (500, 32, 5)] {
        // The next action is offered while the previous click's transfer is pending.
        bot.mechanic_poll(&mut client, Duration::from_millis(offer))
            .await?;
        assert_eq!(shared.lock().actions[next].admitted, 0, "offer {next}");
        assert_eq!(shared.lock().actions[next].dropped_unavailable, 0);
        assert_eq!(bot.mechanic_bot.held.len(), 1);
        complete_transfer(
            &shared,
            2 + next as i32,
            source,
            &stacks[usize::from(source as u16 - 30)],
        )?;
        bot.mechanic_poll(&mut client, Duration::from_millis(offer + 50))
            .await?;
        assert_eq!(shared.lock().actions[next].admitted, 1, "released {next}");
        assert!(bot.mechanic_bot.held.is_empty());
    }
    let state = shared.lock();
    for action in 1..=5 {
        assert_eq!(
            state.actions[action].dropped_unavailable, 0,
            "action {action}"
        );
    }
    assert_eq!(state.actions[5].writes_completed, 1);
    Ok(())
}

fn menu_close_test_bot() -> Result<Bot> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}],
            "actions":[
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":0},
                {"bot":0,"kind":"close_container","window":"steady","at_ms":200},
                {"bot":0,"kind":"command","command":"say fixed-slot","window":"steady","at_ms":250},
                {"bot":0,"kind":"open_container","position":[0,64,0],"window":"steady","at_ms":300}
            ]
        }),
        &args,
    )?)));
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.x = 0.5;
    bot.y = 64.0;
    bot.z = 0.5;
    Ok(bot)
}

fn mechanic_test_frame<P: Packet>(packet: &P) -> Result<RawFrame> {
    let mut body = BytesMut::new();
    packet.encode(&mut body)?;
    Ok(RawFrame {
        id: P::ID,
        body: body.freeze(),
    })
}

#[tokio::test]
async fn deferred_menu_close_requires_fresh_content_without_throttling_offers() -> Result<()> {
    for screen_before_close in [false, true] {
        let mut bot = menu_close_test_bot()?;
        let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
        // A previous image cannot satisfy this new open's content dependency.
        shared
            .lock()
            .menu_image(0, 1, &vec![ItemStack::EMPTY; 39], &ItemStack::EMPTY, None)?;
        let (mut client, mut peer, mut buffer) = connection().await?;
        bot.mechanic_poll(&mut client, Duration::ZERO).await?;
        tokio::time::timeout(Duration::from_secs(1), peer.read_buf(&mut buffer)).await??;
        assert_eq!(
            try_decode_frame(&mut buffer, Compression::Disabled)?
                .unwrap()
                .id,
            ServerboundUseItemOn::ID
        );
        let mut title_nbt = Vec::new();
        mc_nbt::write_network_value(&mut title_nbt, &mc_nbt::Tag::String("furnace".into()))?;
        let screen = mechanic_test_frame(&ClientboundOpenScreen {
            container_id: 42,
            menu_type: 13,
            title_nbt,
        })?;
        if screen_before_close {
            bot.frame(&mut client, screen.clone(), 2, false).await?;
        }
        bot.mechanic_poll(&mut client, Duration::from_millis(200))
            .await?;
        assert_eq!(shared.lock().actions[1].admitted, 1);
        assert_eq!(shared.lock().actions[1].writes_completed, 0);
        assert!(shared.lock().actions[1].awaited_menu_content);
        if !screen_before_close {
            bot.frame(&mut client, screen, 2, false).await?;
        }
        assert_eq!(shared.lock().actions[1].writes_completed, 0);
        bot.mechanic_poll(&mut client, Duration::from_millis(250))
            .await?;
        tokio::time::timeout(Duration::from_secs(1), peer.read_buf(&mut buffer)).await??;
        assert_eq!(
            try_decode_frame(&mut buffer, Compression::Disabled)?
                .unwrap()
                .id,
            ServerboundChatCommand::ID
        );
        bot.mechanic_poll(&mut client, Duration::from_millis(300))
            .await?;
        assert_eq!(shared.lock().actions[2].writes_completed, 1);
        assert_eq!(shared.lock().actions[3].dropped_unavailable, 1);
        assert_eq!(bot.mechanic_bot.cursor, 4);
        let content = |id| {
            mechanic_test_frame(&ClientboundContainerSetContent {
                container_id: id,
                state_id: 2,
                items: vec![ItemStack::EMPTY; 39],
                carried_item: ItemStack::EMPTY,
            })
        };
        bot.frame(&mut client, content(43)?, 2, false).await?;
        assert_eq!(shared.lock().actions[1].writes_completed, 0);
        if !screen_before_close {
            configure_open_loop(
                &mut bot,
                json!([
                    {"name":"steady","duration_ms":1000},
                    {"name":"burst","duration_ms":1000},
                    {"name":"recovery","duration_ms":1000,"recovery":true}
                ]),
                Duration::from_millis(2500),
            );
        }
        bot.frame(&mut client, content(42)?, 2, false).await?;
        tokio::time::timeout(Duration::from_secs(1), peer.read_buf(&mut buffer)).await??;
        let close =
            try_decode_frame(&mut buffer, Compression::Disabled)?.expect("one deferred close");
        assert_eq!(close.id, ServerboundContainerClose::ID);
        assert_eq!(
            ServerboundContainerClose::decode(&mut close.body.clone())?.container_id,
            42
        );
        bot.frame(&mut client, content(42)?, 2, false).await?;
        bot.mechanic_poll(&mut client, Duration::from_millis(350))
            .await?;
        assert!(bot.mechanic_bot.deferred_close.is_none());
        assert!(bot.mechanic_bot.menu.is_none());
        {
            let state = shared.lock();
            assert_eq!(state.actions[0].completed, 1);
            assert_eq!(state.actions[1].writes_completed, 1);
            assert_eq!(state.actions[1].completed, 0); // A close write is not a server ACK.
            assert_eq!(state.actions[1].dropped_unavailable, 0);
        }
        assert!(
            tokio::time::timeout(Duration::from_millis(20), peer.read_buf(&mut buffer))
                .await
                .is_err()
        );
    }
    Ok(())
}

#[tokio::test]
async fn deferred_menu_close_expires_without_retry_or_catchup() -> Result<()> {
    let mut bot = menu_close_test_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    bot.mechanic_poll(&mut client, Duration::ZERO).await?;
    bot.mechanic_poll(&mut client, Duration::from_millis(200))
        .await?;
    bot.mechanic_bot.deferred_close.as_mut().unwrap().offered = Instant::now() - ACTION_TIMEOUT;
    bot.close_received_menu(&mut client).await?;
    bot.close_received_menu(&mut client).await?;
    assert!(bot.mechanic_bot.deferred_close.is_none());
    let state = shared.lock();
    assert_eq!(state.actions[1].admitted, 1);
    assert_eq!(state.actions[1].writes_completed, 0);
    assert_eq!(
        state.actions[1].errors,
        ["menu content missing before deferred close deadline"]
    );
    assert_eq!(
        state.json(Some(Duration::from_millis(250)), false)["actions"][1]["offered"],
        1
    );
    Ok(())
}

#[tokio::test]
async fn timely_menu_content_keeps_original_close_offer_time() -> Result<()> {
    let mut bot = menu_close_test_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    let (mut client, _peer, _buffer) = connection().await?;
    bot.mechanic_poll(&mut client, Duration::ZERO).await?;
    let mut title_nbt = Vec::new();
    mc_nbt::write_network_value(&mut title_nbt, &mc_nbt::Tag::String("furnace".into()))?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundOpenScreen {
            container_id: 42,
            menu_type: 13,
            title_nbt,
        })?,
        2,
        false,
    )
    .await?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundContainerSetContent {
            container_id: 42,
            state_id: 1,
            items: vec![ItemStack::EMPTY; 39],
            carried_item: ItemStack::EMPTY,
        })?,
        2,
        false,
    )
    .await?;
    bot.mechanic_poll(&mut client, Duration::from_millis(199))
        .await?;
    assert_eq!(shared.lock().actions[1].admitted, 0);
    bot.mechanic_poll(&mut client, Duration::from_millis(200))
        .await?;
    let state = shared.lock();
    assert_eq!(state.actions[1].admitted, 1);
    assert_eq!(state.actions[1].writes_completed, 1);
    assert!(!state.actions[1].awaited_menu_content);
    assert!(bot.mechanic_bot.menu.is_none());
    Ok(())
}

#[tokio::test]
async fn pending_menu_transfer_cannot_double_complete_one_received_credit() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}],
            "actions":[
                {"bot":0,"kind":"click_slot","slot":30,"mode":1,"window":"steady","at_ms":0},
                {"bot":0,"kind":"click_slot","slot":30,"mode":1,"window":"steady","at_ms":100}
            ]
        }),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let item = shared
        .lock()
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:iron_ore")
        .map(|(id, _)| *id)
        .unwrap();
    let mut items = vec![ItemStack::EMPTY; 39];
    items[30] = ItemStack::new(item, 64);
    shared
        .lock()
        .menu_image(0, 1, &items, &ItemStack::EMPTY, None)?;
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.mechanic_bot.menu = Some((1, 0, 1));
    let (mut client, _peer, _buffer) = connection().await?;
    assert!(bot.mechanic_action(&mut client, 0).await?);
    assert!(!bot.mechanic_action(&mut client, 1).await?);
    shared
        .lock()
        .menu_slot(0, 2, 30, &ItemStack::EMPTY, Some("steady"))?;
    shared
        .lock()
        .menu_slot(0, 2, 0, &ItemStack::new(item, 64), Some("steady"))?;
    bot.complete_mechanics(Some("steady"));
    let state = shared.lock();
    assert_eq!(state.actions[0].completed, 1);
    assert_eq!(state.actions[1].admitted, 0);
    assert_eq!(state.actions[1].completed, 0);
    Ok(())
}

#[tokio::test]
async fn contested_quick_move_never_claims_commit_or_blocks_close() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "containers":[{"name":"chest","position":[0,64,0],"slots":27}],
            "actions":[
                {"bot":0,"kind":"click_slot","slot":0,"mode":1,"allow_no_change":true,
                 "window":"steady","at_ms":0},
                {"bot":0,"kind":"close_container","window":"steady","at_ms":100}
            ]
        }),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    shared
        .lock()
        .menu_image(0, 1, &[ItemStack::EMPTY; 63], &ItemStack::EMPTY, None)?;
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    bot.mechanic_bot.menu = Some((1, 0, 1));
    let (mut client, _peer, _buffer) = connection().await?;
    assert!(bot.mechanic_action(&mut client, 0).await?);
    assert!(bot.mechanic_bot.pending.is_empty());
    assert!(bot.mechanic_action(&mut client, 1).await?);
    let state = shared.lock();
    assert_eq!(state.actions[0].writes_completed, 1);
    assert_eq!(state.actions[0].completed, 0);
    assert!(state.actions[0].errors.is_empty());
    Ok(())
}

#[test]
fn no_change_observation_belongs_only_to_contested_quick_moves() -> Result<()> {
    let args = mechanic_test_args()?;
    for action in [
        json!({"bot":0,"kind":"command","command":"time set day","allow_no_change":true}),
        json!({"bot":0,"kind":"click_slot","slot":0,"mode":0,"allow_no_change":true}),
    ] {
        assert!(mechanic_test_state(json!({"actions":[action]}), &args).is_err());
    }
    Ok(())
}

#[tokio::test]
async fn feedback_completion_keeps_actual_receive_window() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "actions":[{"bot":0,"kind":"command","command":"save-all flush",
                "expected_feedback":"Saved the game","window":"steady","at_ms":0}]
        }),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    let (mut client, _peer, _buffer) = connection().await?;
    assert!(bot.mechanic_action(&mut client, 0).await?);
    bot.mechanic_frame(&system_chat_frame("Saved the game")?, Some("steady"))?;
    bot.complete_mechanics(Some("recovery"));
    let state = shared.lock();
    assert_eq!(state.actions[0].completed, 1);
    assert_eq!(
        state.actions[0].completed_in_window.as_deref(),
        Some("steady")
    );
    assert_eq!(state.save_completions, 1);
    Ok(())
}

#[test]
fn run_baseline_rebases_received_images_and_excludes_warm_and_late_work() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({
            "blocks":[{"name":"cell","position":[0,64,0]}],
            "entities":[{"kind":"minecraft:zombie","count":1}],
            "containers":[{"name":"furnace","position":[0,64,0],"slots":3}]
        }),
        &args,
    )?;
    state.cell(0, 0, 0, true, None);
    state.update_cell(0, pack_block_pos(0, 64, 0), 1, None);
    let packet = mechanic_test_entity(&state, "minecraft:zombie", 41);
    state.spawn(0, &packet, None);
    state.movement(0, 41, [2.0, 0.0, 0.0], true, None);
    let item = *state
        .items
        .iter()
        .find(|(_, name)| name.as_str() == "minecraft:iron_ore")
        .unwrap()
        .0;
    let mut slots = vec![ItemStack::EMPTY; 39];
    slots[0] = ItemStack::new(item, 32);
    state.menu_image(0, 1, &slots, &ItemStack::EMPTY, None)?;
    state.menu_slot(0, 2, 0, &ItemStack::new(item, 16), None)?;
    state.players[0].initial_health = Some(20.0);
    state.players[0].final_health = Some(16.0);
    state.players[0].damage_events = 1;
    state.players[0].damage_amount = 4.0;
    state.record_reply(
        0,
        Some("steady"),
        &mc_nbt::Tag::String("warm feedback".into()),
    );
    let mut conflict = packet.clone();
    conflict.entity_id = 42;
    state.spawn(0, &conflict, None);
    state.begin_measurement()?;
    assert!(state.begin_measurement().is_err());
    assert_eq!(state.cells[0].initial, Some(1));
    assert_eq!(state.cells[0].update_count, 0);
    assert_eq!(
        state.entities[&packet.uuid].initial_position,
        [3.0, 64.0, 1.0]
    );
    assert_eq!(state.entities[&packet.uuid].move_count, 0);
    assert_eq!(state.menus[0].initial.as_ref().unwrap()[0].count, 16);
    assert_eq!(state.menus[0].updates, 0);
    assert_eq!(state.menus[0].content_packets, 1);
    assert_eq!(state.players[0].initial_health, Some(16.0));
    assert_eq!(state.players[0].damage_events, 0);
    assert_eq!(state.identity_errors, 1);
    assert!(state.command_replies.is_empty());
    // A pre-Run frame already in flight refines the baseline, never work.
    state.update_cell(0, pack_block_pos(0, 64, 0), 0, None);
    state.movement(0, 41, [1.0, 0.0, 0.0], true, None);
    assert_eq!(state.cells[0].initial, Some(0));
    assert_eq!(
        state.entities[&packet.uuid].initial_position,
        [4.0, 64.0, 1.0]
    );
    assert_eq!(state.entities[&packet.uuid].move_count, 0);
    state.update_cell(0, pack_block_pos(0, 64, 0), 1, Some("steady"));
    state.movement(0, 41, [1.0, 0.0, 0.0], true, Some("steady"));
    state.measurement_closed = true;
    state.update_cell(0, pack_block_pos(0, 64, 0), 0, None);
    state.movement(0, 41, [10.0, 0.0, 0.0], true, None);
    let report = state.json(Some(Duration::from_secs(3)), false);
    assert_eq!(report["baseline_captured"], true);
    assert_eq!(state.cells[0].final_state, Some(1));
    assert_eq!(state.cells[0].update_count, 1);
    assert_eq!(
        state.entities[&packet.uuid].final_position,
        [5.0, 64.0, 1.0]
    );
    assert_eq!(state.entities[&packet.uuid].move_count, 1);
    assert_eq!(report["windows"]["steady"]["block_updates"], 1);
    assert_eq!(report["windows"]["steady"]["entity_moves"], 1);
    Ok(())
}

#[test]
fn received_center_transitions_ignore_ready_duplicates_and_bound_trace() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(json!({}), &args)?;
    state.chunk_center(0, [0, 0], None);
    state.chunk_center(0, [5, 0], None);
    assert_eq!(state.players[0].chunk_center_transitions, 0);
    state.begin_measurement()?;
    state.chunk_center(0, [6, 0], Some("steady"));
    state.chunk_center(0, [6, 0], Some("steady"));
    state.chunk_center(0, [5, 0], Some("burst"));
    let report = state.json(Some(Duration::from_secs(2)), false);
    assert_eq!(report["players"][0]["initial_chunk_center"], json!([5, 0]));
    assert_eq!(report["players"][0]["final_chunk_center"], json!([5, 0]));
    assert_eq!(report["players"][0]["chunk_center_packets"], 3);
    assert_eq!(report["players"][0]["chunk_center_transitions"], 2);
    assert_eq!(report["windows"]["steady"]["chunk_center_changes"], 1);
    assert_eq!(report["windows"]["burst"]["chunk_center_changes"], 1);
    assert!(report["players"][1]["measured_chunk_center_proof"].is_null());
    assert!(report["players"][1]["initial_chunk_center"].is_null());
    for x in 0..MECHANIC_CENTER_TRACE_CAP as i32 {
        state.chunk_center(0, [x, 0], Some("burst"));
    }
    assert_eq!(
        state.players[0].chunk_center_trace.len(),
        MECHANIC_CENTER_TRACE_CAP
    );
    assert_eq!(state.players[0].chunk_center_traces_truncated, 2);
    state.measurement_closed = true;
    state.chunk_center(0, [999, 999], None);
    assert_eq!(state.players[0].final_chunk_center, Some([63, 0]));
    Ok(())
}

#[tokio::test]
async fn nutrition_credits_are_received_changes_not_inventory_debits() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({}),
        &args,
    )?)));
    let shared = Arc::clone(args.mechanics.as_ref().unwrap());
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    let health = |food, saturation| -> Result<RawFrame> {
        let mut body = BytesMut::new();
        ClientboundSetHealth {
            health: 20.0,
            food,
            saturation,
        }
        .encode(&mut body)?;
        Ok(RawFrame {
            id: ClientboundSetHealth::ID,
            body: body.freeze(),
        })
    };
    bot.mechanic_frame(&health(5, 1.0)?, None)?;
    shared.lock().begin_measurement()?;
    bot.mechanic_frame(&health(9, 3.0)?, Some("steady"))?;
    bot.mechanic_frame(&health(9, 3.0)?, Some("steady"))?;
    {
        let state = shared.lock();
        assert_eq!(state.players[0].initial_food, Some(5));
        assert_eq!(state.players[0].final_food, Some(9));
        assert_eq!(state.players[0].food_credit_events, 1);
        assert_eq!(state.players[0].food_credited, 4);
        assert_eq!(state.players[0].saturation_credited, 2.0);
    }
    // A fresh vitals baseline (as after respawn) is not a nutrition credit.
    {
        let mut state = shared.lock();
        state.players[0].final_food = None;
        state.players[0].final_saturation = None;
    }
    bot.mechanic_frame(&health(20, 5.0)?, Some("burst"))?;
    assert_eq!(shared.lock().players[0].food_credit_events, 1);
    shared.lock().measurement_closed = true;
    bot.mechanic_frame(&health(1, 0.0)?, None)?;
    assert_eq!(shared.lock().players[0].final_food, Some(20));
    Ok(())
}

#[test]
fn menu_version_retention_preserves_endpoints_and_exposes_truncation() {
    let mut menu = MenuReceipt::default();
    for version in 0..300 {
        Mechanics::record_menu_version(&mut menu, version);
    }
    assert_eq!(menu.versions.len(), 256);
    assert_eq!(menu.versions.first(), Some(&0));
    assert_eq!(menu.versions.last(), Some(&299));
    assert_eq!(menu.versions_truncated, 44);
    Mechanics::record_menu_version(&mut menu, 299);
    assert_eq!(menu.versions_truncated, 44);
}

#[test]
fn reader_controls_require_positive_rate_and_only_restore_can_use_recovery() -> Result<()> {
    let args = mechanic_test_args()?;
    for rate in [json!(null), json!(0)] {
        assert!(mechanic_test_state(json!({"actions":[
            {"bot":1,"kind":"reader_rate_limit","bytes_per_second":rate,"window":"burst","at_ms":0}
        ]}), &args).is_err());
    }
    let state = mechanic_test_state(
        json!({"actions":[
            {"bot":1,"kind":"reader_rate_limit","bytes_per_second":512,"window":"burst","at_ms":0},
            {"bot":1,"kind":"restore_reader","window":"recovery","at_ms":0}
        ]}),
        &args,
    )?;
    assert_eq!(state.actions[0].at_ms, 1000);
    assert_eq!(state.actions[1].at_ms, 2000);
    assert!(mechanic_test_state(json!({"actions":[
        {"bot":1,"kind":"reader_rate_limit","bytes_per_second":512,"window":"recovery","at_ms":0}
    ]}), &args).is_err());
    Ok(())
}

#[tokio::test]
async fn reader_restore_is_admitted_during_recovery_even_when_dead() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({
            "actions":[{"bot":0,"kind":"restore_reader","window":"recovery","at_ms":0}]
        }),
        &args,
    )?)));
    let mut bot = Bot::new(
        0,
        &args,
        Arc::new(Facts::load()?),
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(HashSet::new())),
    );
    let (control, receiver) = watch::channel(Some(1));
    bot.reader_control = Some(control);
    bot.last_health = 0.0;
    bot.respawn_pending = true;
    let (mut client, _peer, _buffer) = connection().await?;
    bot.mechanic_poll(&mut client, Duration::from_millis(2000))
        .await?;
    assert_eq!(*receiver.borrow(), None);
    let state = bot.mechanics.as_ref().unwrap().lock();
    assert_eq!(state.actions[0].admitted, 1);
    assert_eq!(state.actions[0].completed, 1);
    Ok(())
}

#[test]
fn final_reader_entities_follow_visibility_and_deadline_freezes_images() -> Result<()> {
    let args = mechanic_test_args()?;
    let mut state = mechanic_test_state(
        json!({
            "entities":[{"kind":"minecraft:cow","count":1}]
        }),
        &args,
    )?;
    let entity = mechanic_test_entity(&state, "minecraft:cow", 7);
    state.spawn(0, &entity, None);
    state.spawn(1, &entity, None);
    state.begin_measurement()?;
    state.entities.get_mut(&entity.uuid).unwrap().hide(1);
    let report = state.json(Some(Duration::from_secs(3)), false);
    assert_eq!(
        report["players"][0]["final_observed_entities"][0]["uuid"],
        entity.uuid.to_string()
    );
    assert_eq!(report["players"][1]["final_observed_entities"], json!([]));
    state.measurement_epoch = Some(Instant::now() - state.measurement_duration);
    assert!(!state.accepts_receive(None));
    assert!(!state.accepts_receive(Some("recovery")));
    state.measurement_closed = true;
    state.measurement_epoch = Some(Instant::now());
    assert!(!state.accepts_receive(Some("steady")));
    Ok(())
}

#[tokio::test]
async fn legal_health_death_received_in_recovery_requests_real_respawn() -> Result<()> {
    let mut bot = menu_close_test_bot()?;
    bot.open_loop = true;
    bot.respawn = true;
    let (control, _reader) = watch::channel(Some(1));
    bot.reader_control = Some(control);
    *bot.epoch.lock().unwrap_or_else(|error| error.into_inner()) = Some(RunEpoch {
        instant: Instant::now() - Duration::from_millis(2100),
        unix_ms: 0,
        unix_ns: 0,
    });
    bot.mechanics.as_ref().unwrap().lock().begin_measurement()?;
    let (mut client, mut peer, mut buffer) = connection().await?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundSetHealth {
            health: 0.0,
            food: 20,
            saturation: 5.0,
        })?,
        2,
        false,
    )
    .await?;
    tokio::time::timeout(Duration::from_secs(1), peer.read_buf(&mut buffer)).await??;
    let frame = try_decode_frame(&mut buffer, Compression::Disabled)?.expect("respawn request");
    assert_eq!(frame.id, ServerboundClientCommand::ID);
    let command = ServerboundClientCommand::decode(&mut frame.body.as_ref())?;
    assert!(matches!(
        command.action,
        ClientCommandAction::PerformRespawn
    ));
    assert!(bot.respawn_pending);
    assert_eq!(bot.metrics.respawn_requests, 1);
    assert_eq!(bot.mechanics.as_ref().unwrap().lock().players[0].deaths, 1);
    Ok(())
}

#[test]
fn schedule_complete_contains_full_live_mechanics_image() -> Result<()> {
    let mut args = mechanic_test_args()?;
    args.mechanics = Some(Arc::new(MechanicMutex::new(mechanic_test_state(
        json!({}),
        &args,
    )?)));
    let snapshots = vec![
        Arc::new(Mutex::new(Counters::default())),
        Arc::new(Mutex::new(Counters::default())),
    ];
    let report = aggregate(
        &snapshots,
        &args,
        2,
        Instant::now(),
        "schedule_complete",
        None,
    )?;
    assert!(report["mechanics"]["players"].is_array());
    assert!(report["mechanics"]["players"][0]["final_observed_entities"].is_array());
    assert!(report["mechanics"]["reader_scope"].is_string());
    Ok(())
}

#[tokio::test]
async fn reader_progress_excludes_non_position_frames_from_position_updates() -> Result<()> {
    let mut bot = menu_close_test_bot()?;
    *bot.epoch.lock().unwrap_or_else(|error| error.into_inner()) = Some(RunEpoch {
        instant: Instant::now(),
        unix_ms: 0,
        unix_ns: 0,
    });
    bot.mechanics.as_ref().unwrap().lock().begin_measurement()?;
    let (mut client, _peer, _buffer) = connection().await?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&ClientboundSetHealth {
            health: 20.0,
            food: 20,
            saturation: 5.0,
        })?,
        2,
        false,
    )
    .await?;
    bot.frame(
        &mut client,
        mechanic_test_frame(&MoveEntityPos {
            entity_id: 99,
            delta_x: 4096,
            delta_y: 0,
            delta_z: 0,
            on_ground: true,
        })?,
        2,
        false,
    )
    .await?;
    let state = bot.mechanics.as_ref().unwrap().lock();
    let progress = &state.players[0].reader_progress_by_window["steady"];
    assert_eq!(progress.frames, 2);
    assert!(progress.decoded_bytes > 0);
    assert_eq!(progress.position_updates, 1);
    Ok(())
}

#[test]
fn primary_explosion_rows_preserve_unique_blasts_under_observer_fanout() -> Result<()> {
    let mut bot = menu_close_test_bot()?;
    let shared = Arc::clone(bot.mechanics.as_ref().unwrap());
    shared
        .lock()
        .players
        .resize_with(100, PlayerReceipt::default);
    shared.lock().begin_measurement()?;
    for blast in 0..96 {
        let frame = mechanic_test_frame(&ClientboundExplode {
            center: EntityVec3 {
                x: f64::from(blast),
                y: 64.0,
                z: 0.0,
            },
            radius: 4.0,
            block_count: 0,
            knockback: None,
            explosion_particle_id: 29,
            sound_reference_id: 703,
            block_particles: Vec::new(),
            play_sound: true,
        })?;
        for observer in 0..100 {
            bot.index = observer;
            bot.mechanic_frame(&frame, Some("burst"))?;
        }
    }
    let state = shared.lock();
    assert_eq!(state.explosions.len(), 96);
    for (blast, row) in state.explosions.iter().enumerate() {
        assert_eq!(row["bot"], 0);
        assert_eq!(row["center"][0], blast as f64);
    }
    assert_eq!(state.receive_windows["burst"]["explosion"], 9600);
    assert!(state.json(Some(Duration::from_secs(2)), false)["explosion_scope"].is_string());
    Ok(())
}
