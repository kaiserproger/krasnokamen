// Observer movement uses only that observer's received actor image and terrain.
// Following is not an actor-arrival receipt and cannot advance a haul phase.
struct ObserverFollow {
    haul_index: usize,
    last_actor: Option<[f64; 3]>,
    last_actor_version: u64,
    planned_target: Option<[f64; 3]>,
    failed_target: Option<[f64; 3]>,
    route: Vec<[i32; 3]>,
    waypoint: usize,
    attempts: u32,
}

impl Bot {
    fn observer_follow_poll(&mut self) -> Result<()> {
        let Some(shared) = self.mechanics.as_ref().map(Arc::clone) else { return Ok(()); };
        let (active, actor_image) = {
            let state = shared.lock();
            let mut active = state.spec.hauls.iter().enumerate().filter(|(index, spec)|
                spec.observer_bot == self.index && state.haul_reports[*index]["status"] == "running");
            let Some((index, spec)) = active.next() else {
                self.mechanic_bot.observer_follow = None;
                return Ok(());
            };
            ensure!(active.next().is_none(), "observer has multiple active haul assignments");
            ensure!(!state.spec.actions.iter().any(|action| action.bot == self.index)
                && !state.spec.hauls.iter().enumerate().any(|(index, spec)| spec.bot == self.index
                    && matches!(state.haul_reports[index]["status"].as_str(), Some("running" | "requested")))
                && state.runtime_commands[self.index as usize].is_none(),
                "haul observer has a conflicting work assignment");
            let image = state.entities.get(&spec.player_uuid)
                .filter(|entity| entity.kind == "minecraft:player" && entity.visible_to(self.index))
                .and_then(|entity| Some((entity.reader_positions.get(self.index as usize).copied().flatten()?,
                    *entity.reader_versions.get(self.index as usize)?)));
            (index, image)
        };
        let mut follow = match self.mechanic_bot.observer_follow.take() {
            Some(follow) if follow.haul_index == active => follow,
            _ => ObserverFollow { haul_index:active, last_actor:None, last_actor_version:0,
                planned_target:None, failed_target:None, route:Vec::new(), waypoint:0, attempts:0 },
        };
        if let Some((position, version)) = actor_image {
            follow.last_actor = Some(position);
            follow.last_actor_version = version;
        }
        if let Some(target) = follow.last_actor {
            let near = (self.x - target[0]).abs() <= 4.0 && (self.y - target[1]).abs() <= 4.0
                && (self.z - target[2]).abs() <= 4.0;
            let centered = follow.route.get(follow.waypoint).is_some_and(|point|
                (self.x - f64::from(point[0]) - 0.5).abs() < 0.001
                && (self.y - f64::from(point[1])).abs() < 0.1
                && (self.z - f64::from(point[2]) - 0.5).abs() < 0.001);
            let finished = follow.route.is_empty() || (centered && follow.waypoint + 1 == follow.route.len());
            let moved = follow.planned_target.is_some_and(|old| (0..3).any(|axis| (target[axis] - old[axis]).abs() > 4.0));
            let blocked = centered && !finished
                && self.farmer_retained_route(&follow.route, follow.waypoint, target).is_err();
            if !near && (finished || (centered && (moved || blocked))) && follow.failed_target != Some(target) {
                ensure!(follow.attempts < FARMER_REPLAN_CAP, "haul observer follow replan cap reached");
                follow.attempts += 1;
                match self.farmer_path(target) {
                    Ok(path) => {
                        follow.route = path.route;
                        follow.waypoint = 0;
                        follow.planned_target = Some(target);
                        follow.failed_target = None;
                        emit(&json!({"event":"haul_observer_follow_plan","observer_bot":self.index,
                            "haul_index":active,"actor_last_received_position":target,
                            "actor_reader_version":follow.last_actor_version,"attempt":follow.attempts,
                            "waypoints":follow.route.len(),"search":path.stats,
                            "scope":"observer-received actor image and observer terrain; not actor arrival"}))?;
                    }
                    Err(error) => {
                        follow.route.clear();
                        follow.failed_target = Some(target);
                        emit(&json!({"event":"haul_observer_follow_search_failed","observer_bot":self.index,
                            "haul_index":active,"actor_last_received_position":target,
                            "actor_reader_version":follow.last_actor_version,"attempt":follow.attempts,
                            "reason":format!("{error:#}"),"scope":"no movement or arrival inferred"}))?;
                    }
                }
            }
        }
        self.mechanic_bot.observer_follow = Some(follow);
        Ok(())
    }

    fn observer_follow_step(&mut self, speed: f64) -> Option<(f64, f64, f32)> {
        let follow = self.mechanic_bot.observer_follow.as_mut()?;
        let Some(target) = follow.last_actor else { return Some((0.0, 0.0, self.yaw)); };
        if (self.x - target[0]).abs() <= 4.0 && (self.y - target[1]).abs() <= 4.0
            && (self.z - target[2]).abs() <= 4.0 {
            return Some((0.0, 0.0, self.yaw));
        }
        let Some(mut point) = follow.route.get(follow.waypoint).copied() else {
            return Some((0.0, 0.0, self.yaw));
        };
        if (self.x - f64::from(point[0]) - 0.5).abs() < 0.001
            && (self.y - f64::from(point[1])).abs() < 0.1
            && (self.z - f64::from(point[2]) - 0.5).abs() < 0.001
            && follow.waypoint + 1 < follow.route.len() {
            follow.waypoint += 1;
            point = follow.route[follow.waypoint];
        }
        let mut dx = f64::from(point[0]) + 0.5 - self.x;
        let mut dz = f64::from(point[2]) + 0.5 - self.z;
        if dx.abs() > 0.001 && dz.abs() > 0.001 { dz = 0.0; }
        if dx.abs() <= 0.001 { dx = 0.0; }
        if dz.abs() <= 0.001 { dz = 0.0; }
        let distance = dx.abs() + dz.abs();
        if distance == 0.0 { return Some((0.0, 0.0, self.yaw)); }
        let factor = speed.min(distance) / distance;
        let yaw = if dx > 0.0 { -90.0 } else if dx < 0.0 { 90.0 } else if dz > 0.0 { 0.0 } else { 180.0 };
        Some((dx * factor, dz * factor, yaw))
    }
}
