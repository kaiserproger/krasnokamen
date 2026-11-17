//! Authoritative tick state's own construction, serial-pass block writes and
//! reliable per-connection pushes. The `TickState` struct itself stays in
//! `tick.rs` so its private fields remain visible to every tick module.
//! Byte-exact move out of `tick.rs`; no behaviour change.

use super::*;

impl TickState {
    /// Serial-pass block write: visible to later reads this tick and staged
    /// for the canonical block phase (neighbours, comparators, WAL).
    pub(crate) fn write_block(&mut self, pos: BlockPos, id: u32) {
        self.world.set_block(pos.x, pos.y, pos.z, id);
        self.merge_block_writes.push((pos, id));
    }

    pub fn new(
        registry: Arc<CoreRegistry>,
        world: ChunkWorld,
        sim_distance: i32,
        outbox_cap: usize,
    ) -> Self {
        Self {
            config_sim_distance: sim_distance,
            config_view_distance: sim_distance,
            active_player_centers: Vec::new(),
            active_ticket_ranges: Vec::new(),
            active_sim_radius: -1,
            registry,
            published_registry: None,
            published_command_feedback: None,
            world,
            dimension: crate::dimensions::Dimension::Overworld,
            other_dimensions: Default::default(),
            dimension_requests: Vec::new(),
            dimension_respawns: Vec::new(),
            dimension_routes: Vec::new(),
            entities: EntityStore::default(),
            connections: Connections::default(),
            require_plugin_login: false,
            locked_sessions: BTreeSet::new(),
            max_airtime_ticks: 80,
            enable_command_block: false,
            spawn_protection: 16,
            spawn_operators: Default::default(),
            brushing: Default::default(),
            command_blocks_due: Vec::new(),
            mob_griefing: crate::config::MobGriefingRules::default(),
            test_acceleration: crate::config::test_acceleration::TestAcceleration::default(),
            players: HashMap::default(),
            cameras: Arc::default(),
            camera_releases: Vec::new(),
            offline_players: HashMap::default(),
            default_game_mode: crate::commands::GameMode::Survival,
            advancement_events: Vec::new(),
            advancement_reward_spawns: Vec::new(),
            native_globals: Default::default(),
            datapacks: Default::default(),
            command_context: Default::default(),
            debug_profiler: Default::default(),
            debug_trace: None,
            plugins: Default::default(),
            plugin_runtime: None,
            build_queries: None,
            plugin_spawned: Vec::new(),
            plugin_block_writes: Vec::new(),
            plugin_players: Default::default(),
            plugin_damage: Vec::new(),
            pending_removals: Vec::new(),
            plugin_interactions: HashMap::default(),
            outbox: HashMap::default(),
            outbox_cap,
            disconnects: Vec::new(),
            tick: 0,
            wall_clock: crate::advancements::system_unix_seconds,
            entity_salt: [0; crate::entity_uuid::SALT_LEN],
            day_time: 0,
            merge_block_writes: Vec::new(),
            block_phase_writes: Vec::new(),
            item_merge_cells: Vec::new(),
            serial_entity_slots: Vec::new(),
            serial_entity_marks: Vec::new(),
            contact_hazards_scratch: Default::default(),
            portal_match_cache: Default::default(),
            portal_contact_stamps: Vec::new(),
            merge_broken: Vec::new(),
            merge_command_broken: Vec::new(),
            merge_container_contents: Vec::new(),
            merge_closed_viewers: Vec::new(),
            merge_damage: Vec::new(),
            merge_consumed_items: HashSet::default(),
            merge_spawns: Vec::new(),
            merge_life_drops: Vec::new(),
            merge_gives: Vec::new(),
            fluid_ticks: crate::fluids::FluidTicks::default(),
            block_ticks: crate::block_ticks::BlockTicks::default(),
            redstone: crate::redstone::RedstoneState::default(),
            redstone_containers: crate::redstone::containers::ContainerState::default(),
            farm_sections: crate::farming::FarmSections::default(),
            merge_pickups: Vec::new(),
            appearance_changed: Vec::new(),
            left_players: Vec::new(),
            leash_knots: HashMap::default(),
            tickets: BTreeMap::new(),
            chunk_tickets: crate::chunk_tickets::ChunkTickets::default(),
            entity_phase_revision: 0,
            entity_phase_scratch: EntityPhaseScratch::default(),
            explosion_exposure: crate::explosion::ExposureCache::default(),
            last_swing: HashMap::default(),
            armor_stand_hits: HashMap::default(),
            // Vanilla dedicated-server default; the join sends it.
            difficulty: crate::survival::Difficulty::Easy,
            journaled_difficulty: None,
            journaled_default_game_mode: None,
            weather: (crate::commands::Weather::Clear, 0),
            thunder_remaining_ticks: 0,
            clear_remaining_ticks: 0,
            thundering: false,
            weather_chunks: Vec::new(),
            rain_level: 0.0,
            thunder_level: 0.0,
            journaled_climate: None,
            border: Default::default(),
            journaled_border: None,
            gamerules: HashMap::default(),
            gamerules_dirty: false,
            spawn_census: Default::default(),
            seq_counter: HashMap::default(),
            block_entities: Default::default(),
            block_entity_removed: Vec::new(),
            open_containers: Default::default(),
            container_animation_recipients: Vec::new(),
            expired_container_openers: Vec::new(),
            vehicle_menus: crate::vehicles::VehicleMenus::default(),
            deferred_uses: Vec::new(),
            sign_editors: HashMap::default(),
            opaque_changes: Vec::new(),
            gateway_ages: HashMap::default(),
            campfires: crate::campfire::Campfires::default(),
            hives: crate::bees::Hives::default(),
            jukeboxes: crate::jukebox::Jukeboxes::default(),
            lecterns: crate::lectern::Lecterns::default(),
            brewing: crate::brewing::BrewingStands::default(),
            crafters: crate::crafter::Crafters::default(),
            merge_shulker_placements: Vec::new(),
            merge_item_component_placements: Vec::new(),
            merge_command_block_entities: Vec::new(),
            vibrations: crate::vibration::Bus::default(),
            sculk: crate::sculk::SculkState::default(),
            trial_spawners: Default::default(),
            monster_spawners: Default::default(),
            vaults: Default::default(),
            shriekers: Default::default(),
            sleepers: HashMap::default(),
            journaled_players: HashMap::default(),
            journaled_block_entities: HashMap::default(),
            block_log: Default::default(),
            journaled_villagers: HashMap::default(),
            journaled_entity_state: HashMap::default(),
            journal_scan: Default::default(),
            hostile: crate::hostile::HostileState::default(),
            projectiles: crate::projectiles::ProjectileRuntime::default(),
            end_fight: Default::default(),
            journaled_end_fight: None,
            raids: Default::default(),
            journaled_raids: None,
            trader_spawner: Default::default(),
            trader_tick_delay: 1200,
            journaled_trader_spawner: None,
            scoreboard: crate::scoreboard::Scoreboard::default(),
            command_storage: Default::default(),
            command_storage_dirty: false,
            maps: Default::default(),
            scoreboard_dirty: false,
            team_names: Arc::default(),
            bossbars_dirty: false,
            bossbars: Default::default(),
            dimension_globals: Default::default(),
            villager_pois: Default::default(),
            smelting_cache: Default::default(),
            generated_entity_claims: Default::default(),
            new_generated_entity_claims: Vec::new(),
            generated_entity_initial_scan: false,
            settlements: Default::default(),
            settlement_sequence: 0,
            autonomy_queue: Default::default(),
            settlement_marks: HashMap::default(),
            settlement_menus: HashMap::default(),
            tick_budget: Default::default(),
            tick_control: Default::default(),
            locate: Default::default(),
        }
    }

    pub(crate) fn session_locked(&self, connection: ConnectionId) -> bool {
        self.require_plugin_login && self.locked_sessions.contains(&connection)
    }

    pub(crate) fn entity_session_locked(&self, entity: EntityId) -> bool {
        self.require_plugin_login
            && self
                .players
                .get(&entity)
                .is_some_and(|player| self.locked_sessions.contains(&player.connection))
    }

    pub(crate) fn next_seq(&mut self, actor: EntityId) -> u32 {
        let seq = self.seq_counter.get(&actor).copied().unwrap_or(0);
        self.seq_counter.insert(actor, seq + 1);
        seq
    }

    /// Vanilla `ServerPlayer.acceptsChatMessages`: only a client whose chat
    /// setting is `Full` is sent other players' chat; `System` and `Hidden`
    /// clients are not.
    #[must_use]
    pub(crate) fn accepts_chat(&self, connection: ConnectionId) -> bool {
        self.connections.chat_visibility_of(connection) == crate::boundary::ChatVisibility::Full
    }

    /// Reliable per-player push. On cap overflow the queue is sealed with a
    /// terminal `Disconnect` (always last) and the connection is scheduled
    /// for Leave-equivalent removal on a later tick; further pushes are
    /// dropped so nothing can land after `Disconnect`. Overflowing text lines
    /// compact into a `TextBurst` instead (see `PlayerChange::is_text`).
    pub fn push_player(&mut self, connection: ConnectionId, change: PlayerChange) {
        if change.is_text()
            && let Some(trace) = &mut self.debug_trace
            && trace.connection == connection
            && self
                .command_context
                .current
                .is_some_and(|context| context.connection == connection)
        {
            // The traced source is the trace file, not the player.
            trace.on_message(&match &change {
                PlayerChange::Chat { message } => message.clone(),
                PlayerChange::StyledChat { content } => crate::command_debug::plain(content),
                _ => String::new(),
            });
            return;
        }
        self.push_to_recipient(connection, change);
    }

    /// Text a command delivers to a player as a target (`/tellraw`, `/say`,
    /// `/msg`), not as its source's reply: a `/debug function` trace records
    /// only what its source is sent, so this never lands in a trace.
    pub(crate) fn push_to_recipient(&mut self, connection: ConnectionId, change: PlayerChange) {
        // Vanilla `ServerPlayer.acceptsSystemMessages`: a client whose chat
        // setting is hidden gets no text at all, neither chat nor command
        // feedback. Titles and the action bar are not text lines here.
        if change.is_text()
            && self.connections.chat_visibility_of(connection)
                == crate::boundary::ChatVisibility::Hidden
        {
            return;
        }
        self.push_player_unfiltered(connection, change);
    }

    /// Like [`Self::push_player`] without the chat setting's filter: the one
    /// message a hidden client is still sent, `chat.disabled.options`, uses it.
    pub(crate) fn push_player_unfiltered(
        &mut self,
        connection: ConnectionId,
        change: PlayerChange,
    ) {
        if self
            .disconnects
            .iter()
            .any(|(sealed, _)| *sealed == connection)
        {
            return;
        }
        crate::vibration::menu(self, connection, &change);
        let queue = self.outbox.entry(connection).or_default();
        if change.is_text() {
            if let Some(PlayerChange::TextBurst { lines }) = queue.last_mut() {
                lines.push(change);
                return;
            }
            // A tick's text beyond the cap (a datapack function, a command block
            // chain) compacts its trailing run of lines into one burst entry.
            if queue.len() >= self.outbox_cap {
                let run = queue
                    .iter()
                    .rev()
                    .take_while(|entry| entry.is_text())
                    .count();
                if run > 0 {
                    let mut lines = queue.split_off(queue.len() - run);
                    lines.push(change);
                    queue.push(PlayerChange::TextBurst { lines });
                    return;
                }
            }
        }
        if queue.len() >= self.outbox_cap {
            queue.push(PlayerChange::Disconnect {
                reason: "reliable queue overflow".to_owned(),
            });
            self.disconnects.push((connection, self.tick));
            return;
        }
        queue.push(change);
    }

    /// Fence movement when the authoritative sync is created, before the
    /// off-tick edge can send it or admit an older queued client pose.
    pub(crate) fn correct_position(
        &mut self,
        connection: ConnectionId,
        position: [f64; 3],
        yaw: f32,
        pitch: f32,
    ) {
        let Some(entity) = self.connections.entity_of(connection) else {
            return;
        };
        let Some(player) = self.players.get_mut(&entity) else {
            return;
        };
        let teleport_id = player.begin_teleport();
        self.push_player(
            connection,
            PlayerChange::PositionCorrected {
                teleport_id,
                position,
                yaw,
                pitch,
            },
        );
    }

    pub(crate) fn stage_command_break(&mut self, pos: BlockPos, broken: u32) -> bool {
        let drops =
            if crate::command_rules::boolean(self, crate::command_rules::BoolRule::BLOCK_DROPS) {
                crate::block_drop_context::roll(
                    self,
                    pos,
                    broken,
                    &mc_data::ItemStack::EMPTY,
                    self.tick ^ ((pos.x as u64) << 40) ^ ((pos.z as u64) << 16) ^ pos.y as u64,
                )
            } else {
                Ok(Vec::new())
            };
        let Some(drops) = crate::block_drop_context::admit(self, pos, drops) else {
            return false;
        };
        self.stage_container_removal(pos, !crate::block_drop_context::retains_contents(&drops));
        self.merge_command_broken.push((pos, broken, drops));
        true
    }

    pub(crate) fn stage_command_container_removal(&mut self, pos: BlockPos) {
        self.stage_container_removal(pos, false);
    }

    pub(crate) fn stage_container_removal(&mut self, pos: BlockPos, drop_contents: bool) {
        crate::plugin_inventories::invalidate_block(self, pos);
        let cell = (pos.x, pos.y, pos.z);
        crate::sculk::removed(self, pos);
        if self
            .merge_container_contents
            .iter()
            .any(|(other, _)| *other == cell)
        {
            return;
        }
        let record = self.block_entities.get(&cell);
        let opaque = self
            .world
            .chunk(ChunkPos {
                x: pos.x.div_euclid(16),
                z: pos.z.div_euclid(16),
            })
            .is_some_and(|chunk| chunk.block_entities.contains_key(&pos));
        if record.is_none() && !opaque {
            return;
        }
        let slots = if drop_contents {
            record
                .map(|record| record.slots.clone())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        self.merge_container_contents.push((cell, slots));
        let viewers: Vec<EntityId> = self
            .open_containers
            .viewers()
            .filter(|(_, open)| container_cells(self, **open).contains(&cell))
            .map(|(viewer, _)| *viewer)
            .collect();
        self.merge_closed_viewers
            .extend(viewers.into_iter().map(|viewer| (viewer, cell)));
    }
}
