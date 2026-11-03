// Finite player hauling driven by received terrain, menu images and an independent
// player's server-broadcast pose. No position or inventory is synthesized.
// Conservative horizontal margin around the actor, not a proof of future pickup.
const FARMER_OFFER_COMPETITOR_RADIUS: f64 = 4.0;
#[derive(Clone, Copy)]
enum FarmerOfferDecision { Replan, Wait, Drop([f64; 3]) }
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HaulSpec {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    defer_start: bool,
    bot: u32,
    observer_bot: u32,
    player_uuid: Uuid,
    source: String,
    destination: String,
    item: String,
    count: i32,
    source_route: Vec<[i32; 3]>,
    destination_route: Vec<[i32; 3]>,
    #[serde(default)]
    farmer_delivery: Option<FarmerDeliverySpec>,
    timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FarmerDeliverySpec {
    farmer_uuid: Uuid,
}

fn haul_route_valid(route: &[[i32; 3]]) -> bool {
    (2..=4096).contains(&route.len()) && route.iter().all(|p| {
        (MIN_Y..MIN_Y + HEIGHT as i32).contains(&p[1])
            && p[0].unsigned_abs() < 30_000_000 && p[2].unsigned_abs() < 30_000_000
    }) && route.windows(2).all(|edge| {
        let a = edge[0];
        let b = edge[1];
        (i64::from(a[0]) - i64::from(b[0])).abs()
            + (i64::from(a[2]) - i64::from(b[2])).abs() == 1
            && (i64::from(a[1]) - i64::from(b[1])).abs() <= 1
    })
}

fn haul_balanced(before: (i64, i64), after: (i64, i64), count: i32, pickup: bool) -> bool {
    let count = i64::from(count);
    if pickup {
        before.0 - after.0 == count && after.1 - before.1 == count
    } else {
        after.0 - before.0 == count && before.1 - after.1 == count
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum HaulPhase { AwaitTrigger, AwaitStart, SourceApproach, SourceRoute, SourceOpen, SourceImage, SourceTransfer, DestinationRoute, DestinationOpen, DestinationImage, DestinationTransfer, WithdrawOpen, WithdrawImage, WithdrawTransfer, FarmerPursuitPlan, FarmerPursuitWait, FarmerObserverWait, FarmerPursuitWalk, FarmerDrop, FarmerDebit, Done, Failed }

struct HaulState {
    spec_index: usize,
    phase: HaulPhase,
    waypoint: usize,
    leg_version: u64,
    started: Instant,
    menu_index: usize,
    image_before: u64,
    counts_before: (i64, i64),
    hotbar_slot: usize,
    drop_item_id: u32,
    drop_position: [f64; 3],
    prior_items: HashSet<Uuid>,
    approach_route: Vec<[i32; 3]>,
    pursuit_route: Vec<[i32; 3]>,
    pursuit_target: [f64; 3],
    pursuit_replans: u32,
    pursuit_wait_version: u64,
    observer_wait_resume_walk: bool,
    pursuit_reused_fence: bool,
    pursuit_retained: bool,
    offer_observation_seq: Option<u64>,
}
impl HaulState {
    fn new(spec_index: usize) -> Self {
        Self { spec_index, phase: HaulPhase::AwaitStart, waypoint: 1, leg_version: 0,
            started: Instant::now(), menu_index: 0, image_before: 0, counts_before: (0, 0),
            hotbar_slot: 0, drop_item_id: 0, drop_position: [0.0; 3], prior_items: HashSet::new(),
            approach_route: Vec::new(), pursuit_route: Vec::new(), pursuit_target: [0.0; 3], pursuit_replans: 0,
            pursuit_wait_version: 0, observer_wait_resume_walk: false,
            pursuit_reused_fence: false, pursuit_retained: false, offer_observation_seq: None }
    }
}

fn haul_counts(menu: &MenuReceipt, name: &str) -> Option<(i64, i64)> {
    let sum = |slots: &[SlotImage]| -> Option<i64> {
        let mut total = 0i64;
        for slot in slots.iter().filter(|slot| slot.name.as_deref() == Some(name) && slot.count > 0) {
            if !slot.plain { return None; }
            total += i64::from(slot.count);
        }
        Some(total)
    };
    Some((sum(menu.final_slots.as_deref()?)?, sum(menu.final_player_inventory.as_deref()?)?))
}

fn haul_hotbar_slot(menu: &MenuReceipt, name: &str, count: i32) -> Option<(usize, u32)> {
    let first = menu.final_slots.as_ref()?.len() + 27;
    let slot = menu.final_player_inventory.as_ref()?.iter().find(|slot|
        (first..first + 9).contains(&slot.slot) && slot.plain && slot.count == count
            && slot.name.as_deref() == Some(name))?;
    Some((slot.slot - first, slot.item_id?))
}

impl Mechanics {
    fn farmer_offer_images(&self, bot: u32, farmer: Uuid, actor: [f64; 3]) -> Result<(Vec<Value>, bool)> {
        ensure!(self.identity_errors == 0 && self.position_errors == 0 && self.capacity_errors == 0,
            "villager offer observations incomplete: identity, position or capacity error");
        let mut villagers: Vec<_> = self.entities.values().filter(|entity|
            entity.kind == "minecraft:villager" && entity.visible_to(bot)).collect();
        villagers.sort_unstable_by_key(|entity| entity.uuid);
        ensure!(villagers.iter().any(|entity| entity.uuid == farmer), "offer farmer not visible");
        let mut blocked = false;
        let mut images = Vec::with_capacity(villagers.len());
        for entity in villagers {
            let position = entity.reader_positions.get(bot as usize).copied().flatten()
                .context("visible villager has no reader position")?;
            let version = entity.reader_versions.get(bot as usize).copied().filter(|v| *v > 0)
                .context("visible villager has no reader version")?;
            ensure!(position.iter().all(|coordinate| coordinate.is_finite()), "invalid villager reader position");
            let competitor = entity.uuid != farmer
                && (position[0] - actor[0]).abs() <= FARMER_OFFER_COMPETITOR_RADIUS
                && (position[2] - actor[2]).abs() <= FARMER_OFFER_COMPETITOR_RADIUS;
            blocked |= competitor;
            images.push(json!({"uuid":entity.uuid,"entity_id":entity.entity_id,"position":position,
                "reader_version":version,"within_competitor_margin":competitor}));
        }
        Ok((images, blocked))
    }
    fn request_haul(&mut self, bot: u32, id: &str) -> Result<usize> {
        let index = self.spec.hauls.iter().position(|haul|
            haul.bot == bot && haul.id.as_deref() == Some(id)).context("unknown haul id for bot")?;
        ensure!(self.haul_requests.get(bot as usize).is_some_and(Option::is_none),
            "haul start already pending or bot unavailable");
        if let Some(previous) = self.spec.hauls[..index].iter().rposition(|haul| haul.bot == bot) {
            ensure!(self.haul_reports[previous]["status"] == "finished", "previous haul has not finished");
        } else {
            ensure!(self.spec.hauls[index].defer_start, "first haul starts automatically");
        }
        ensure!(self.haul_reports[index]["status"] == "pending", "haul already started");
        self.haul_requests[bot as usize] = Some(index);
        self.haul_reports[index]["status"] = json!("requested");
        Ok(index)
    }
    fn fail_hauls_for_dead_bot(&mut self, dead_bot: u32, predicted_position: [f64; 3]) -> Vec<Value> {
        let mut events = Vec::new();
        for (index, haul) in self.spec.hauls.iter().enumerate() {
            if haul.bot != dead_bot && haul.observer_bot != dead_bot { continue; }
            if !matches!(self.haul_reports[index]["status"].as_str(), Some("pending" | "requested" | "running")) {
                continue;
            }
            let reason = if haul.bot == dead_bot { "actor died" } else { "observer died" };
            self.haul_reports[index]["status"] = json!("failed");
            self.haul_reports[index]["error"] = json!(reason);
            self.haul_requests[haul.bot as usize] = None;
            events.push(json!({"event":"haul_finished","bot":haul.bot,"id":haul.id,"outcome":"failed",
                "dead_bot":dead_bot,"dead_bot_client_predicted_position":predicted_position,
                "elapsed_us":self.measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
                "receipt":self.haul_reports[index]}));
        }
        events
    }
}

impl Bot {
    fn trace_farmer_refusal(&self, nx: f64, nz: f64, reason: &str) -> Result<()> {
        let Some(haul) = self.mechanic_bot.haul.as_ref().filter(|haul|
            haul.phase == HaulPhase::FarmerPursuitWalk) else {
            return Ok(());
        };
        let Some(shared) = &self.mechanics else { return Ok(()); };
        let mut state = shared.lock();
        if !state.haul_reports[haul.spec_index]["farmer_delivery"]["first_route_refusal"].is_null() {
            return Ok(());
        }
        let mut columns = Vec::new();
        for x in (nx - 0.3).floor() as i32..=(nx + 0.3).floor() as i32 {
            for z in (nz - 0.3).floor() as i32..=(nz + 0.3).floor() as i32 {
                columns.push(match self.column(x, z) {
                    Some(column) => json!({"x":x,"z":z,"known":true,"walkable":column.walkable,
                        "feet_y":column.feet_y,"top_state":column.top,
                        "top_name":usize::try_from(column.top).ok().and_then(|id| state.registry.get(id))
                            .and_then(Option::as_ref).map(|image| &image.name),
                        "support_height":self.facts.get(column.top).support_height}),
                    None => json!({"x":x,"z":z,"known":false}),
                });
            }
        }
        let event = json!({"event":"farmer_route_refusal","bot":self.index,
            "id":state.spec.hauls[haul.spec_index].id,"waypoint":haul.waypoint,
            "elapsed_us":state.measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
            "predicted_position":[self.x,self.y,self.z],"next_xz":[nx,nz],
            "velocity_y":self.velocity_y,"reason":reason,"columns":columns,
            "scope":"first local route refusal; position is client-predicted"});
        state.haul_reports[haul.spec_index]["farmer_delivery"]["first_route_refusal"] = event.clone();
        emit(&event)?;
        Ok(())
    }

    fn haul_step(&mut self, speed: f64) -> Option<(f64, f64, f32)> {
        let haul = self.mechanic_bot.haul.as_mut()?;
        let shared = self.mechanics.as_ref()?;
        let state = shared.lock();
        let spec = &state.spec.hauls[haul.spec_index];
        let route = match haul.phase {
            HaulPhase::SourceApproach => &haul.approach_route,
            HaulPhase::SourceRoute => &spec.source_route,
            HaulPhase::DestinationRoute => &spec.destination_route,
            HaulPhase::FarmerPursuitWalk => &haul.pursuit_route,
            _ => return Some((0.0, 0.0, self.yaw)),
        };
        let mut target = route[haul.waypoint];
        if (self.x - (f64::from(target[0]) + 0.5)).abs() < 0.001
            && (self.z - (f64::from(target[2]) + 0.5)).abs() < 0.001
            && (self.y - f64::from(target[1])).abs() < 0.1
            && haul.waypoint + 1 < route.len()
        {
            haul.waypoint += 1;
            target = route[haul.waypoint];
        }
        let dx = f64::from(target[0]) + 0.5 - self.x;
        let mut dz = f64::from(target[2]) + 0.5 - self.z;
        if dx.abs() > 0.001 && dz.abs() > 0.001 {
            if haul.phase == HaulPhase::SourceApproach && haul.waypoint == 0 { dz = 0.0; }
            else { return Some((0.0, 0.0, self.yaw)); }
        }
        let distance = dx.abs() + dz.abs();
        if distance < 0.001 { return Some((0.0, 0.0, self.yaw)); }
        let factor = speed.min(distance) / distance;
        let yaw = if dx > 0.001 { -90.0 } else if dx < -0.001 { 90.0 } else if dz > 0.0 { 0.0 } else { 180.0 };
        Some((dx * factor, dz * factor, yaw))
    }

    async fn haul_poll(&mut self, client: &mut Client, elapsed: Duration) -> Result<()> {
        let result = self.haul_poll_inner(client, elapsed).await;
        if let Err(error) = &result && let Some(shared) = &self.mechanics {
            let mut state = shared.lock();
            if let Some(index) = state.spec.hauls.iter().enumerate().position(|(index, haul)|
                haul.bot == self.index && state.haul_reports[index]["status"] == "running")
            {
                state.haul_reports[index]["status"] = json!("failed");
                state.haul_reports[index]["error"] = json!(format!("{error:#}"));
                emit(&json!({"event":"haul_finished","bot":self.index,"id":state.spec.hauls[index].id,
                    "outcome":"failed","receipt":state.haul_reports[index]}))?;
            }
        }
        result
    }

    async fn haul_poll_inner(&mut self, client: &mut Client, elapsed: Duration) -> Result<()> {
        let Some(shared) = self.mechanics.as_ref().map(Arc::clone) else { return Ok(()); };
        if self.mechanic_bot.haul.is_none() {
            let (index, deferred, failed) = {
                let mut state = shared.lock();
                let Some(index) = state.spec.hauls.iter().position(|spec| spec.bot == self.index) else { return Ok(()); };
                let deferred = state.spec.hauls[index].defer_start;
                let failed = state.haul_reports[index]["status"] == "failed";
                if !deferred && !failed { state.haul_reports[index]["status"] = json!("running"); }
                (index, deferred, failed)
            };
            let mut haul = HaulState::new(index);
            if failed { haul.phase = HaulPhase::Failed; }
            else if deferred { haul.phase = HaulPhase::AwaitTrigger; }
            self.mechanic_bot.haul = Some(haul);
        }
        if let Some(haul) = self.mechanic_bot.haul.as_mut()
            && shared.lock().haul_reports[haul.spec_index]["status"] == "failed"
        {
            haul.phase = HaulPhase::Failed;
        }
        if self.schedule.window_at(elapsed).is_none_or(|index| self.schedule.windows[index].recovery) {
            let haul = self.mechanic_bot.haul.as_mut().expect("haul initialized");
            if !matches!(haul.phase, HaulPhase::Done | HaulPhase::Failed) {
                let mut state = shared.lock();
                state.haul_reports[haul.spec_index]["error"] = json!("haul measurement window ended");
                state.haul_reports[haul.spec_index]["status"] = json!("failed");
                emit(&json!({"event":"haul_finished","bot":self.index,
                    "id":state.spec.hauls[haul.spec_index].id,"outcome":"failed",
                    "receipt":state.haul_reports[haul.spec_index]}))?;
                haul.phase = HaulPhase::Failed;
            }
            return Ok(());
        }
        if self.mechanic_bot.haul.as_ref().is_some_and(|haul|
            matches!(haul.phase, HaulPhase::Done | HaulPhase::AwaitTrigger)) {
            let next = {
                let mut state = shared.lock();
                let next = state.haul_requests[self.index as usize].take();
                if let Some(index) = next {
                    state.haul_reports[index]["status"] = json!("running");
                }
                next
            };
            if let Some(index) = next {
                self.mechanic_bot.haul = Some(HaulState::new(index));
            }
        }
        let mut haul = self.mechanic_bot.haul.take().expect("haul initialized");
        let spec = shared.lock().spec.hauls[haul.spec_index].clone();
        let previous_phase = haul.phase;
        if haul.started.elapsed() >= Duration::from_millis(spec.timeout_ms)
            && !matches!(haul.phase, HaulPhase::Done | HaulPhase::Failed | HaulPhase::AwaitTrigger) {
            shared.lock().haul_reports[haul.spec_index]["error"] = json!("haul deadline exceeded");
            haul.phase = HaulPhase::Failed;
        }
        match haul.phase {
            HaulPhase::AwaitTrigger => {}
            HaulPhase::AwaitStart => {
                let observed = {
                    let state = shared.lock();
                    state.entities.get(&spec.player_uuid)
                        .filter(|e| e.entity_id == self.entity_id && e.visible_to(spec.observer_bot))
                        .and_then(|e| Some((e.reader_positions.get(spec.observer_bot as usize).copied().flatten()?,
                            *e.reader_versions.get(spec.observer_bot as usize)?)))
                };
                if let Some((position, version)) = observed
                    && (position[0] - self.x).abs() <= 0.35
                    && (position[1] - self.y).abs() <= 0.35
                    && (position[2] - self.z).abs() <= 0.35
                {
                    let start = spec.source_route[0];
                    if haul_near(position, start)
                        && (self.x - f64::from(start[0]) - 0.5).abs() < 0.001
                        && (self.y - f64::from(start[1])).abs() < 0.1
                        && (self.z - f64::from(start[2]) - 0.5).abs() < 0.001
                    {
                        haul.leg_version = version;
                        haul.phase = HaulPhase::SourceRoute;
                    } else {
                        let path = self.source_approach_path(start).context("source approach path failed")?;
                        let event = json!({"event":"haul_source_approach_plan","id":spec.id,"bot":self.index,
                            "observer_bot":spec.observer_bot,"actor_observed_position":position,
                            "leg_start_version":version,"route":path.route,"search":path.stats,
                            "scope":"received terrain plan; not source arrival"});
                        emit(&event)?;
                        shared.lock().haul_reports[haul.spec_index]["source_approach_plan"] = event;
                        haul.approach_route = path.route;
                        haul.leg_version = version;
                        haul.waypoint = 0;
                        haul.phase = HaulPhase::SourceApproach;
                    }
                }
            }
            HaulPhase::SourceApproach => {
                let start = spec.source_route[0];
                let state = shared.lock();
                let observed = state.entities.get(&spec.player_uuid)
                    .filter(|e| e.entity_id == self.entity_id && e.visible_to(spec.observer_bot))
                    .and_then(|e| Some((e.reader_positions.get(spec.observer_bot as usize).copied().flatten()?,
                        *e.reader_versions.get(spec.observer_bot as usize)?)));
                if let Some((position, version)) = observed
                    && version > haul.leg_version && haul_near(position, start)
                    && haul.waypoint + 1 == haul.approach_route.len()
                    && (self.x - f64::from(start[0]) - 0.5).abs() < 0.001
                    && (self.y - f64::from(start[1])).abs() < 0.1
                    && (self.z - f64::from(start[2]) - 0.5).abs() < 0.001
                {
                    let receipt = json!({"observer_bot":spec.observer_bot,"player_uuid":spec.player_uuid,
                        "position":position,"reader_version":version,"leg_start_version":haul.leg_version,
                        "scope":"observer packet after approach start; not source container arrival"});
                    drop(state);
                    shared.lock().haul_reports[haul.spec_index]["source_approach_arrival"] = receipt.clone();
                    emit(&json!({"event":"haul_source_approach_arrival","id":spec.id,"bot":self.index,"receipt":receipt}))?;
                    haul.leg_version = version;
                    haul.waypoint = 1;
                    haul.phase = HaulPhase::SourceRoute;
                }
            }
            HaulPhase::SourceRoute | HaulPhase::DestinationRoute => {
                let source = haul.phase == HaulPhase::SourceRoute;
                let route = if source { &spec.source_route } else { &spec.destination_route };
                let endpoint = *route.last().expect("validated route");
                let mut state = shared.lock();
                let observed = state.entities.get(&spec.player_uuid)
                    .filter(|e| e.entity_id == self.entity_id && e.visible_to(spec.observer_bot))
                    .and_then(|e| Some((e.reader_positions.get(spec.observer_bot as usize).copied().flatten()?, *e.reader_versions.get(spec.observer_bot as usize)?)));
                if let Some((pose, version)) = observed && version > haul.leg_version
                    && haul.waypoint + 1 == route.len() && haul_near(pose, endpoint)
                    && (self.x - f64::from(endpoint[0]) - 0.5).abs() < 0.001
                    && (self.y - f64::from(endpoint[1])).abs() < 0.1
                    && (self.z - f64::from(endpoint[2]) - 0.5).abs() < 0.001
                {
                    let arrival = json!({"observer_bot":spec.observer_bot,"player_uuid":spec.player_uuid,"position":pose,"reader_version":version,"leg_start_version":haul.leg_version,"elapsed_us":haul.started.elapsed().as_micros()});
                    let key = if source { "source_arrival" } else { "destination_arrival" };
                    state.haul_reports[haul.spec_index][key] = arrival;
                    haul.phase = if source { HaulPhase::SourceOpen } else { HaulPhase::DestinationOpen };
                }
            }
            HaulPhase::SourceOpen | HaulPhase::DestinationOpen | HaulPhase::WithdrawOpen => {
                let source = haul.phase == HaulPhase::SourceOpen;
                let name = if source { &spec.source } else { &spec.destination };
                let (index, position, before) = {
                    let state = shared.lock();
                    let index = state.spec.containers.iter().position(|p| p.bot == self.index && &p.name == name).context("haul container probe missing")?;
                    (index, state.spec.containers[index].position, state.menus[index].content_packets)
                };
                self.mechanic_bot.opening = Some(index);
                self.mechanic_use_block(client, position).await?;
                haul.menu_index = index;
                haul.image_before = before;
                haul.phase = if source { HaulPhase::SourceImage } else if haul.phase == HaulPhase::WithdrawOpen {
                    HaulPhase::WithdrawImage
                } else { HaulPhase::DestinationImage };
            }
            HaulPhase::SourceImage | HaulPhase::DestinationImage | HaulPhase::WithdrawImage => {
                let source = matches!(haul.phase, HaulPhase::SourceImage | HaulPhase::WithdrawImage);
                let withdraw = haul.phase == HaulPhase::WithdrawImage;
                let choice = {
                    let state = shared.lock();
                    let menu = &state.menus[haul.menu_index];
                    if self.mechanic_bot.menu.is_none_or(|(_, index, _)| index != haul.menu_index) || menu.content_packets <= haul.image_before { None } else {
                        ensure!(menu.carried.as_ref().is_some_and(|slot| slot.count == 0),
                            "haul cursor must be empty");
                        let counts = haul_counts(menu, &spec.item).context("haul item image unavailable or non-plain")?;
                        let slots = if source { menu.final_slots.as_ref() } else { menu.final_player_inventory.as_ref() }.context("haul inventory image unavailable")?;
                        let slot = slots.iter().find(|slot| slot.name.as_deref() == Some(&spec.item) && slot.plain && slot.count == spec.count).map(|slot| slot.slot);
                        if (source && counts.1 != 0) || (!source && counts.1 != i64::from(spec.count)) { bail!("haul custody baseline mismatch"); }
                        let slot = slot.context("haul exact stack unavailable")?;
                        Some((counts, slot, state.predicted_carried(haul.menu_index, slot, 0, 1).context("haul click prediction unavailable")?))
                    }
                };
                if let Some((counts, slot, carried_item)) = choice {
                    let (id, _, version) = self.mechanic_bot.menu.context("haul menu closed")?;
                    client.write_packet(&ServerboundContainerClick { container_id: id, state_id: version, slot_num: i16::try_from(slot)?, button_num: 0, container_input: ContainerInput::QuickMove, changed_slots: Vec::new(), carried_item }).await?;
                    haul.counts_before = counts;
                    haul.phase = if withdraw { HaulPhase::WithdrawTransfer } else if source {
                        HaulPhase::SourceTransfer
                    } else { HaulPhase::DestinationTransfer };
                }
            }
            HaulPhase::SourceTransfer | HaulPhase::DestinationTransfer | HaulPhase::WithdrawTransfer => {
                let withdraw = haul.phase == HaulPhase::WithdrawTransfer;
                let source = matches!(haul.phase, HaulPhase::SourceTransfer | HaulPhase::WithdrawTransfer);
                let completed = {
                    let mut state = shared.lock();
                    let counts = haul_counts(&state.menus[haul.menu_index], &spec.item).context("haul post-transfer image unavailable")?;
                    if haul_balanced(haul.counts_before, counts, spec.count, source) {
                        let transfer = json!({"item":spec.item,"count":spec.count,"before_container_player":haul.counts_before,"after_container_player":counts,"elapsed_us":haul.started.elapsed().as_micros()});
                        if withdraw {
                            let (slot, item_id) = haul_hotbar_slot(&state.menus[haul.menu_index], &spec.item, spec.count)
                                .context("withdrawn wheat is not in one received hotbar slot")?;
                            haul.hotbar_slot = slot;
                            haul.drop_item_id = item_id;
                            state.haul_reports[haul.spec_index]["farmer_delivery"]["withdrawal"] = transfer;
                        } else {
                            let key = if source { "source_transfer" } else { "destination_transfer" };
                            state.haul_reports[haul.spec_index][key] = transfer;
                        }
                        if source && !withdraw {
                            let entity = state.entities.get(&spec.player_uuid).filter(|e| e.entity_id == self.entity_id && e.visible_to(spec.observer_bot)).context("haul observer lost player")?;
                            haul.leg_version = *entity.reader_versions.get(spec.observer_bot as usize).context("haul observer pose unavailable")?;
                        }
                        true
                    } else {
                        false
                    }
                };
                if completed {
                    let (id, _, _) = self.mechanic_bot.menu.take().context("haul menu closed")?;
                    client.write_packet(&ServerboundContainerClose { container_id: id }).await?;
                    if withdraw {
                        haul.phase = HaulPhase::FarmerPursuitPlan;
                    } else if source {
                        haul.waypoint = 1;
                        haul.phase = HaulPhase::DestinationRoute;
                    } else {
                        shared.lock().haul_reports[haul.spec_index]["completed"] = json!(true);
                        haul.phase = if spec.farmer_delivery.is_some() { HaulPhase::WithdrawOpen } else { HaulPhase::Done };
                    }
                }
            }
            HaulPhase::FarmerPursuitPlan => {
                ensure!(haul.pursuit_replans < FARMER_REPLAN_CAP, "farmer pursuit replan cap reached");
                let farmer_uuid = spec.farmer_delivery.as_ref().expect("farmer delivery").farmer_uuid;
                let (target, target_version, actor_version) = {
                    let state = shared.lock();
                    let farmer = state.entities.get(&farmer_uuid)
                        .filter(|entity| entity.kind == "minecraft:villager" && entity.visible_to(self.index))
                        .context("farmer not visible to actor during pursuit")?;
                    let target = farmer.reader_positions.get(self.index as usize).copied().flatten()
                        .context("farmer received position unavailable during pursuit")?;
                    let version = *farmer.reader_versions.get(self.index as usize)
                        .context("farmer reader version unavailable")?;
                    let actor_version = state.entities.get(&spec.player_uuid)
                        .filter(|entity| entity.entity_id == self.entity_id && entity.visible_to(spec.observer_bot))
                        .and_then(|entity| entity.reader_positions.get(spec.observer_bot as usize).copied().flatten()
                            .and_then(|_| entity.reader_versions.get(spec.observer_bot as usize).copied()));
                    (target, version, actor_version)
                };
                let Some(actor_version) = actor_version else {
                    haul.observer_wait_resume_walk = false;
                    haul.phase = HaulPhase::FarmerObserverWait;
                    emit(&json!({"event":"farmer_pursuit_observer_wait","id":spec.id,"bot":self.index,
                        "observer_bot":spec.observer_bot,"scope":"waiting for a real observer actor image"}))?;
                    self.mechanic_bot.haul = Some(haul);
                    return Ok(());
                };
                haul.pursuit_replans += 1;
                let FarmerPath { route, stats } = match self.farmer_path(target) {
                    Ok(path) => path,
                    Err(error) => {
                        let search = error.downcast_ref::<FarmerSearchFailure>().map(|failure| &failure.stats);
                        let retained = self.farmer_retained_route(&haul.pursuit_route, haul.waypoint, target);
                        let continued = search.is_some() && retained.is_ok();
                        let waiting = search.is_some() && !continued;
                        let event = json!({"event":"farmer_pursuit_search_failed","id":spec.id,"bot":self.index,
                            "farmer_uuid":farmer_uuid,"farmer_last_received_position":target,
                            "farmer_reader_version":target_version,"actor_client_predicted_position":[self.x,self.y,self.z],
                            "reason":format!("{error:#}"),"search":search,"replan":haul.pursuit_replans,
                            "continued_previous_route":continued,"waiting_for_farmer_update":waiting,
                            "retention_refusal":retained.err(),
                            "retained_waypoint":haul.waypoint,"retained_route_length":haul.pursuit_route.len(),
                            "elapsed_us":shared.lock().measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
                            "scope":"search over received topmost walkable columns only; retained movement is not arrival or pickup"});
                        shared.lock().haul_reports[haul.spec_index]["farmer_delivery"]["pursuit_search_failure"] = event.clone();
                        emit(&event)?;
                        if continued {
                            // A new target cell can trigger another replan; otherwise
                            // advance the retained route before searching again.
                            haul.pursuit_target = target;
                            haul.pursuit_retained = true;
                            haul.phase = HaulPhase::FarmerPursuitWalk;
                            self.mechanic_bot.haul = Some(haul);
                            return Ok(());
                        }
                        if waiting {
                            haul.pursuit_target = target;
                            haul.pursuit_wait_version = target_version;
                            haul.phase = HaulPhase::FarmerPursuitWait;
                            self.mechanic_bot.haul = Some(haul);
                            return Ok(());
                        }
                        return Err(error);
                    }
                };
                haul.pursuit_target = target;
                haul.pursuit_retained = false;
                haul.pursuit_reused_fence = route.len() == 1;
                if !haul.pursuit_reused_fence { haul.leg_version = actor_version; }
                haul.waypoint = usize::from(route.len() > 1);
                let event = json!({"event":"farmer_pursuit_plan","id":spec.id,"bot":self.index,
                    "farmer_uuid":farmer_uuid,"farmer_last_received_position":target,
                    "farmer_reader_version":target_version,"actor_client_predicted_position":[self.x,self.y,self.z],
                    "planned_waypoints":route.len(),"route":route,"replan":haul.pursuit_replans,
                    "search":stats,
                    "observer_bot":spec.observer_bot,"leg_start_version":haul.leg_version,
                    "reused_previous_actor_fence":haul.pursuit_reused_fence,
                    "elapsed_us":shared.lock().measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
                    "scope":"farmer pose is last received, not a fresh server-tick fence; search uses topmost received columns"});
                {
                    let mut state = shared.lock();
                    state.haul_reports[haul.spec_index]["farmer_delivery"]["pursuit_plans"]
                        .as_array_mut().expect("pursuit plan array").push(event.clone());
                }
                emit(&event)?;
                haul.pursuit_route = route;
                haul.phase = HaulPhase::FarmerPursuitWalk;
            }
            HaulPhase::FarmerPursuitWait => {
                let farmer_uuid = spec.farmer_delivery.as_ref().expect("farmer delivery").farmer_uuid;
                let state = shared.lock();
                let farmer = state.entities.get(&farmer_uuid)
                    .filter(|entity| entity.kind == "minecraft:villager" && entity.visible_to(self.index))
                    .context("farmer disappeared while awaiting pursuit update")?;
                let target = farmer.reader_positions.get(self.index as usize).copied().flatten()
                    .context("farmer pose unavailable while awaiting pursuit update")?;
                let version = *farmer.reader_versions.get(self.index as usize)
                    .context("farmer reader version unavailable while awaiting pursuit update")?;
                if version > haul.pursuit_wait_version && target != haul.pursuit_target {
                    emit(&json!({"event":"farmer_pursuit_retry_ready","id":spec.id,"bot":self.index,
                        "farmer_uuid":farmer_uuid,"previous_position":haul.pursuit_target,
                        "previous_version":haul.pursuit_wait_version,"farmer_last_received_position":target,
                        "farmer_reader_version":version,
                        "scope":"changed actor-reader pose only; not arrival or pickup"}))?;
                    haul.phase = HaulPhase::FarmerPursuitPlan;
                }
            }
            HaulPhase::FarmerObserverWait => {
                let state = shared.lock();
                let observed = state.entities.get(&spec.player_uuid)
                    .filter(|entity| entity.entity_id == self.entity_id && entity.visible_to(spec.observer_bot))
                    .and_then(|entity| Some((entity.reader_positions.get(spec.observer_bot as usize).copied().flatten()?,
                        *entity.reader_versions.get(spec.observer_bot as usize)?)));
                if let Some((position, version)) = observed {
                    emit(&json!({"event":"farmer_pursuit_observer_reacquired","id":spec.id,"bot":self.index,
                        "observer_bot":spec.observer_bot,"actor_received_position":position,"reader_version":version,
                        "scope":"real observer actor image, not arrival"}))?;
                    haul.phase = if haul.observer_wait_resume_walk { HaulPhase::FarmerPursuitWalk }
                        else { HaulPhase::FarmerPursuitPlan };
                }
            }
            HaulPhase::FarmerPursuitWalk => {
                let farmer_uuid = spec.farmer_delivery.as_ref().expect("farmer delivery").farmer_uuid;
                let (target, target_version, actor) = {
                    let state = shared.lock();
                    let farmer = state.entities.get(&farmer_uuid)
                        .filter(|entity| entity.kind == "minecraft:villager" && entity.visible_to(self.index))
                        .context("farmer disappeared during pursuit")?;
                    let target = farmer.reader_positions.get(self.index as usize).copied().flatten()
                        .context("farmer received position disappeared")?;
                    let version = *farmer.reader_versions.get(self.index as usize).context("farmer version disappeared")?;
                    let actor = state.entities.get(&spec.player_uuid)
                        .filter(|entity| entity.entity_id == self.entity_id && entity.visible_to(spec.observer_bot))
                        .and_then(|entity| Some((entity.reader_positions.get(spec.observer_bot as usize).copied().flatten()?,
                            *entity.reader_versions.get(spec.observer_bot as usize)?)));
                    (target, version, actor)
                };
                let Some((actor_pose, actor_version)) = actor else {
                    haul.observer_wait_resume_walk = true;
                    haul.phase = HaulPhase::FarmerObserverWait;
                    emit(&json!({"event":"farmer_pursuit_observer_wait","id":spec.id,"bot":self.index,
                        "observer_bot":spec.observer_bot,"scope":"waiting for a real observer actor image"}))?;
                    self.mechanic_bot.haul = Some(haul);
                    return Ok(());
                };
                let waypoint = haul.pursuit_route[haul.waypoint];
                let centered = (self.x - f64::from(waypoint[0]) - 0.5).abs() < 0.001
                    && (self.y - f64::from(waypoint[1])).abs() < 0.1
                    && (self.z - f64::from(waypoint[2]) - 0.5).abs() < 0.001;
                let replan_reason = if centered && haul.waypoint + 1 < haul.pursuit_route.len() {
                    if haul.pursuit_retained {
                        if (0..3).any(|axis| (target[axis] - haul.pursuit_target[axis]).abs() > FARMER_GOAL_HYSTERESIS) {
                            Some("farmer materially displaced since failed replan")
                        } else {
                            self.farmer_retained_route(&haul.pursuit_route, haul.waypoint, target).err()
                        }
                    } else {
                        self.farmer_route_hysteresis(&haul.pursuit_route, haul.waypoint, target).err()
                    }
                } else { None };
                if let Some(reason) = replan_reason {
                    let event = json!({"event":"farmer_pursuit_replan_trigger","id":spec.id,"bot":self.index,
                        "reason":reason,"waypoint":haul.waypoint,"route_length":haul.pursuit_route.len(),
                        "retained_route":haul.pursuit_retained,"farmer_last_received_position":target,
                        "previous_plan_target":haul.pursuit_target});
                    shared.lock().haul_reports[haul.spec_index]["farmer_delivery"]["pursuit_replan_trigger"] = event.clone();
                    emit(&event)?;
                    haul.phase = HaulPhase::FarmerPursuitPlan;
                } else if centered && haul.waypoint + 1 == haul.pursuit_route.len() {
                    if (self.x - target[0]).abs() > 1.0 || (self.y - target[1]).abs() > 1.0
                        || (self.z - target[2]).abs() > 1.0
                    {
                        haul.phase = HaulPhase::FarmerPursuitPlan;
                    } else if actor_version > haul.leg_version && haul_near(actor_pose, waypoint)
                        && (0..3).all(|axis| (actor_pose[axis] - target[axis]).abs() <= 1.0) {
                        let receipt = json!({"observer_bot":spec.observer_bot,"player_uuid":spec.player_uuid,
                            "position":actor_pose,"reader_version":actor_version,"leg_start_version":haul.leg_version,
                            "farmer_uuid":farmer_uuid,"farmer_last_received_position":target,
                            "farmer_reader_version":target_version,
                            "elapsed_us":shared.lock().measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
                            "reused_previous_actor_fence":haul.pursuit_reused_fence,
                            "scope":"actor packet after stated movement leg start; farmer pose is last received and may be stale"});
                        shared.lock().haul_reports[haul.spec_index]["farmer_delivery"]["pursuit_arrival"] = receipt.clone();
                        emit(&json!({"event":"farmer_pursuit_arrival","id":spec.id,"bot":self.index,
                            "receipt":receipt}))?;
                        haul.phase = HaulPhase::FarmerDrop;
                    }
                }
            }
            HaulPhase::FarmerDrop => {
                let delivery = spec.farmer_delivery.as_ref().expect("farmer delivery");
                let target = {
                    let mut state = shared.lock();
                    let farmer = state.entities.get(&delivery.farmer_uuid)
                        .filter(|entity| entity.kind == "minecraft:villager" && entity.visible_to(self.index))
                        .context("farmer UUID not visible to actor")?;
                    let pose = farmer.reader_positions.get(self.index as usize).copied().flatten()
                        .context("farmer received pose unavailable")?;
                    let version = *farmer.reader_versions.get(self.index as usize).context("farmer pose version missing")?;
                    ensure!(state.identity_errors == 0 && state.position_errors == 0 && state.capacity_errors == 0,
                        "villager offer observations incomplete: identity, position or capacity error");
                    if (pose[0] - self.x).abs() > 1.0 || (pose[2] - self.z).abs() > 1.0
                        || (pose[1] - self.y).abs() > 1.0 {
                        haul.offer_observation_seq = None;
                        FarmerOfferDecision::Replan
                    } else {
                        let seq = state.villager_observation_seq[self.index as usize];
                        if haul.offer_observation_seq == Some(seq) {
                            FarmerOfferDecision::Wait
                        } else {
                            let (villagers, blocked) = state.farmer_offer_images(
                                self.index, delivery.farmer_uuid, [self.x,self.y,self.z])?;
                            let snapshot = json!({"event":"farmer_offer_observation","id":spec.id,
                                "bot":self.index,"farmer_uuid":delivery.farmer_uuid,
                                "actor_client_predicted_position":[self.x,self.y,self.z],
                                "observation_seq":seq,"horizontal_competitor_margin":FARMER_OFFER_COMPETITOR_RADIUS,
                                "villagers":villagers,"blocked":blocked,
                                "scope":"actor-visible last-received images only; not global or same-tick isolation"});
                            state.haul_reports[haul.spec_index]["farmer_delivery"]["offer_observation"] = snapshot.clone();
                            emit(&snapshot)?;
                            haul.offer_observation_seq = Some(seq);
                            if blocked {
                                FarmerOfferDecision::Wait
                            } else {
                                let inventory = state.players[self.index as usize].final_inventory.as_ref()
                                    .context("fresh player inventory unavailable")?;
                                let slot = inventory.get(36 + haul.hotbar_slot).context("hotbar image unavailable")?;
                                ensure!(slot.item_id == Some(haul.drop_item_id) && slot.count == spec.count && slot.plain,
                                    "received hotbar custody differs from withdrawn stack");
                                haul.prior_items = state.entities.values().filter(|entity| entity.kind == "minecraft:item")
                                    .map(|entity| entity.uuid).collect();
                                haul.drop_position = [self.x, self.y, self.z];
                                state.haul_reports[haul.spec_index]["farmer_delivery"]["drop_offer"] =
                                    json!({"farmer_uuid":delivery.farmer_uuid,"farmer_pose":pose,"farmer_reader_version":version,
                                        "actor_position":haul.drop_position,"hotbar_slot":haul.hotbar_slot,
                                        "item":spec.item,"count":spec.count,"villager_observation":snapshot,
                                        "elapsed_us":haul.started.elapsed().as_micros()});
                                FarmerOfferDecision::Drop(pose)
                            }
                        }
                    }
                };
                if let FarmerOfferDecision::Drop(target) = target {
                let yaw = (-(target[0] - self.x)).atan2(target[2] - self.z).to_degrees() as f32;
                client.write_packet(&ServerboundSetCarriedItem { slot: i16::try_from(haul.hotbar_slot)? }).await?;
                client.write_packet(&ServerboundMovePlayerRot { yaw, pitch: 0.0,
                    flags: MovePlayerFlags::new(self.last_on_ground, false) }).await?;
                self.sequence = self.sequence.checked_add(1).context("farmer drop sequence overflow")?;
                client.write_packet(&ServerboundPlayerAction { action: PlayerActionKind::DropAllItems,
                    position: pack_block_pos(0, 0, 0), direction: Direction::Up, sequence: self.sequence }).await?;
                shared.lock().haul_reports[haul.spec_index]["farmer_delivery"]["drop_written"] =
                    json!({"sequence":self.sequence,"elapsed_us":haul.started.elapsed().as_micros()});
                haul.phase = HaulPhase::FarmerDebit;
                } else if matches!(target, FarmerOfferDecision::Replan) {
                    haul.phase = HaulPhase::FarmerPursuitPlan;
                }
            }
            HaulPhase::FarmerDebit => {
                let mut state = shared.lock();
                let inventory = state.players[self.index as usize].final_inventory.as_ref()
                    .context("player inventory disappeared after drop")?;
                let slot = inventory.get(36 + haul.hotbar_slot).context("hotbar image disappeared")?;
                if slot.count == 0 {
                    let candidates: Vec<_> = state.entities.values().filter(|entity|
                        entity.kind == "minecraft:item" && !haul.prior_items.contains(&entity.uuid)
                        && matches!(entity.item_observer, Some(bot) if bot == self.index || bot == spec.observer_bot)
                        && entity.item.as_ref().is_some_and(|item| item.item_id == Some(haul.drop_item_id)
                            && item.count == spec.count && item.plain)
                        && entity.initial_position.iter().zip(haul.drop_position)
                            .map(|(a, b)| (a - b).powi(2)).sum::<f64>() <= 9.0
                    ).map(|entity| entity.uuid).collect();
                    ensure!(candidates.len() <= 1, "ambiguous dropped item identity");
                    if let Some(item_uuid) = candidates.first() {
                        state.haul_reports[haul.spec_index]["farmer_delivery"]["drop"] =
                            json!({"item_uuid":item_uuid,"item":spec.item,"count":spec.count,
                                "hotbar_slot":haul.hotbar_slot,"inventory_before":spec.count,
                                "inventory_after":0,"elapsed_us":haul.started.elapsed().as_micros(),
                                "scope":"received inventory debit and one nearby new item entity; pickup requires separate oracle"});
                        state.haul_reports[haul.spec_index]["farmer_delivery"]["completed"] = json!(true);
                        haul.phase = HaulPhase::Done;
                        emit(&json!({"event":"farmer_drop_observed","bot":self.index,"id":spec.id,
                            "player_uuid":spec.player_uuid,
                            "farmer_uuid":spec.farmer_delivery.as_ref().expect("farmer delivery").farmer_uuid,
                            "item_uuid":item_uuid,"item":spec.item,"count":spec.count,
                            "inventory_before":spec.count,"inventory_after":0,
                            "item_collection_sequence_at_emit":state.item_collections_received,
                            "retention_truncated":state.item_collections_truncated,
                            "elapsed_us":state.measurement_epoch.map(|epoch| epoch.elapsed().as_micros()),
                            "scope":"received debit and unique nearby item identity; not pickup or custody proof"}))?;
                    }
                }
            }
            HaulPhase::Done | HaulPhase::Failed => {}
        }
        if !matches!(previous_phase, HaulPhase::Done | HaulPhase::Failed)
            && matches!(haul.phase, HaulPhase::Done | HaulPhase::Failed)
        {
            let mut state = shared.lock();
            let outcome = if haul.phase == HaulPhase::Done { "finished" } else { "failed" };
            state.haul_reports[haul.spec_index]["status"] = json!(outcome);
            emit(&json!({"event":"haul_finished","bot":self.index,"id":spec.id,
                "outcome":outcome,"receipt":state.haul_reports[haul.spec_index]}))?;
        }
        self.mechanic_bot.haul = Some(haul);
        Ok(())
    }
}

fn haul_near(position: [f64; 3], point: [i32; 3]) -> bool {
    (position[0] - f64::from(point[0]) - 0.5).abs() <= 0.35
        && (position[1] - f64::from(point[1])).abs() <= 0.35
        && (position[2] - f64::from(point[2]) - 0.5).abs() <= 0.35
}
