const SETUP_COMMAND_BATCH: usize = 32;

impl Bot {
    async fn setup_fence(
        &mut self,
        client: &mut Client,
        scope: &str,
        preparing: bool,
    ) -> Result<()> {
        // Exact own echo is an input-ordered fence; no-op commands need not emit
        // feedback. This chat belongs to preparation, never measured chat offers.
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?;
        let message = format!(
            "scale_setup_fence_{}_{}_{}",
            now.as_nanos(),
            self.index,
            scope
        );
        let expected = format!("<{}> {message}", self.username);
        client
            .write_packet(&ServerboundChat {
                message,
                timestamp_millis: now.as_millis().min(i64::MAX as u128) as i64,
                salt: i64::from(self.index),
                signature: None,
                last_seen_offset: 0,
                last_seen_acknowledged: [0; 3],
                last_seen_checksum: 0,
            })
            .await?;
        self.metrics.fixture_input_fences_sent += 1;
        let echoed = async {
            loop {
                let frame = client.read_frame().await?;
                let acknowledged = if frame.id == ClientboundSystemChat::ID {
                    let packet = ClientboundSystemChat::decode(&mut frame.body.as_ref())?;
                    let tag = mc_nbt::read_network(&mut packet.content_nbt.as_slice())?;
                    literal_component_matches(&tag, &expected)
                } else {
                    false
                };
                self.frame(client, frame, self.view_distance, preparing)
                    .await?;
                if acknowledged {
                    self.metrics.fixture_input_fences_completed += 1;
                    return Ok::<(), anyhow::Error>(());
                }
            }
        };
        if preparing {
            // Bot::prepare is already bounded by --prepare-seconds for the whole
            // readiness operation. A slow checkpoint must not get a shorter,
            // unrelated per-batch deadline while its connection is still live.
            echoed.await?;
        } else {
            tokio::time::timeout(Duration::from_secs(60), echoed)
                .await
                .context("public preparation input echo fence deadline")??;
        }
        Ok(())
    }
    async fn setup_world_fence(&mut self, client: &mut Client) -> Result<()> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as u64;
        let channel = mc_data::Identifier::parse(mc_protocol::SETUP_WORLD_FENCE_CHANNEL)?;
        client
            .write_packet(&ServerboundCustomPayload {
                payload: mc_protocol::packets::CustomPayload::Unknown {
                    channel: channel.clone(),
                    payload: nonce.to_be_bytes().to_vec(),
                },
            })
            .await?;
        loop {
            let frame = client.read_frame().await?;
            let complete = if frame.id == ClientboundCustomPayload::ID {
                let packet = ClientboundCustomPayload::decode(&mut frame.body.as_ref())?;
                match packet.payload {
                    mc_protocol::packets::CustomPayload::Unknown {
                        channel: received,
                        payload,
                    } if received == channel => {
                        ensure!(
                            payload == nonce.to_be_bytes(),
                            "incorrect setup world fence nonce"
                        );
                        true
                    }
                    _ => false,
                }
            } else {
                false
            };
            self.frame(client, frame, self.view_distance, true).await?;
            if complete {
                emit(
                    &json!({"event":"fixture_world_fence_complete","bot":self.index,
                    "scope":"viewer geometry planned and preceding frames received"}),
                )?;
                return Ok(());
            }
        }
    }
    async fn setup_commands(&mut self, client: &mut Client, preparing: bool) -> Result<()> {
        // Preparation alone may response-gate. Measured open-loop demand never
        // calls this path and retains its fixed wall-time denominator.
        let commands = Arc::clone(&self.commands);
        ensure!(
            commands.frame_positions.is_empty() == commands.save_after_frames.is_none(),
            "frame fixture requires positions and a final save together"
        );
        ensure!(
            commands.frame_positions.is_empty() || self.mechanics.is_some(),
            "frame fixture needs entity observation"
        );
        let batches = commands.setup.len().div_ceil(SETUP_COMMAND_BATCH);
        for (index, batch) in commands.setup.chunks(SETUP_COMMAND_BATCH).enumerate() {
            for command in batch {
                client
                    .write_packet(&ServerboundChatCommand {
                        command: command.clone(),
                    })
                    .await?;
                self.metrics.commands_sent += 1;
            }
            if index + 1 == batches {
                // Write receipt must precede receipt of the final save ACK.
                emit(
                    &json!({"event":"fixture_commands_sent","bot":self.index,"commands_sent":commands.setup.len(),
                        "batch_capacity":SETUP_COMMAND_BATCH}),
                )?;
            }
            self.setup_fence(client, &format!("batch_{index}"), preparing)
                .await?;
        }
        if !commands.frame_positions.is_empty() {
            self.setup_map_frames(client, &commands.frame_positions, preparing)
                .await?;
            let save = commands
                .save_after_frames
                .as_ref()
                .context("frame fixture lacks final save")?;
            client
                .write_packet(&ServerboundChatCommand {
                    command: save.clone(),
                })
                .await?;
            self.metrics.commands_sent += 1;
            self.setup_fence(client, "frame_save", preparing).await?;
        }
        if !commands.setup.is_empty() {
            emit(&json!({"event":"fixture_setup_complete","bot":self.index,
                "commands_sent":commands.setup.len(),"fence":"literal own input-ordered chat echo"}))?;
        }
        Ok(())
    }

    /// Populate pre-summoned empty frames using ordinary player interaction and
    /// received entity metadata; command writes alone are never completion.
    async fn setup_map_frames(
        &mut self,
        client: &mut Client,
        positions: &[[i32; 3]],
        preparing: bool,
    ) -> Result<()> {
        let shared = Arc::clone(
            self.mechanics
                .as_ref()
                .context("frame fixture needs entity observation")?,
        );
        client
            .write_packet(&ServerboundSetCarriedItem { slot: 0 })
            .await?;
        let mut used = HashSet::new();
        for (index, &[x, y, z]) in positions.iter().enumerate() {
            if index % 256 == 0 {
                // Each survival use allocates a new real map. A map owner may
                // persist at most 256 distinct frame markers.
                for command in [
                    "gamemode survival @s",
                    "item replace entity @s hotbar.0 with minecraft:map",
                ] {
                    client
                        .write_packet(&ServerboundChatCommand {
                            command: command.into(),
                        })
                        .await?;
                    self.metrics.commands_sent += 1;
                }
                self.setup_fence(client, &format!("frame_empty_map_{index}"), preparing)
                    .await?;
                ensure!(
                    shared.lock().players[self.index as usize]
                        .final_inventory
                        .as_ref()
                        .is_some_and(|items| items.iter().any(|item| item.slot == 36
                            && item.name.as_deref() == Some("minecraft:map")
                            && item.count == 1)),
                    "frame fixture did not receive an empty map"
                );
                self.sequence = self
                    .sequence
                    .checked_add(1)
                    .context("map use sequence overflow")?;
                client
                    .write_packet(&ServerboundUseItem {
                        hand: InteractionHand::MainHand,
                        sequence: self.sequence,
                        y_rot: self.yaw,
                        x_rot: self.pitch,
                    })
                    .await?;
                self.setup_fence(client, &format!("frame_map_{index}"), preparing)
                    .await?;
                loop {
                    let filled = shared.lock().players[self.index as usize]
                        .final_inventory
                        .as_ref()
                        .is_some_and(|items| {
                            items.iter().any(|item| {
                                item.slot == 36
                                    && item.name.as_deref() == Some("minecraft:filled_map")
                                    && item.count > 0
                            })
                        });
                    if filled {
                        break;
                    }
                    let frame = client.read_frame().await?;
                    self.frame(client, frame, self.view_distance, preparing)
                        .await?;
                }
                client
                    .write_packet(&ServerboundChatCommand {
                        command: "gamemode creative @s".into(),
                    })
                    .await?;
                self.metrics.commands_sent += 1;
                self.setup_fence(client, &format!("frame_creative_{index}"), preparing)
                    .await?;
            }
            self.teleport(
                client,
                f64::from(x) + 0.5,
                f64::from(y - 1),
                f64::from(z) + 0.5,
                self.view_distance,
            )
            .await?;
            self.setup_fence(client, &format!("frame_teleport_{index}"), preparing)
                .await?;
            // Frames outside the current view are not sent to the bot. Discover
            // each target after arriving rather than waiting for the whole fixture.
            let (uuid, entity_id) = loop {
                let target = {
                    let state = shared.lock();
                    state
                        .entities
                        .values()
                        .find(|entity| {
                            entity.kind == "minecraft:item_frame"
                                && entity.active
                                && entity.visible_to(self.index)
                                && entity
                                    .final_position
                                    .map(|coordinate| coordinate.floor() as i32)
                                    == [x, y, z]
                        })
                        .map(|entity| (entity.uuid, entity.entity_id))
                };
                if let Some(target) = target {
                    break target;
                }
                let frame = client.read_frame().await?;
                self.frame(client, frame, self.view_distance, preparing)
                    .await?;
            };
            ensure!(used.insert(uuid), "frame cell used twice");
            client
                .write_packet(&ServerboundInteract {
                    entity_id,
                    hand: InteractionHand::MainHand,
                    location: EntityVec3::ZERO,
                    using_secondary_action: false,
                })
                .await?;
            self.setup_fence(client, &format!("frame_interact_{index}"), preparing)
                .await?;
            loop {
                let observed = shared
                    .lock()
                    .entities
                    .get(&uuid)
                    .and_then(|entity| entity.item.as_ref())
                    .is_some_and(|item| {
                        item.name.as_deref() == Some("minecraft:filled_map") && item.count == 1
                    });
                if observed {
                    break;
                }
                let frame = client.read_frame().await?;
                self.frame(client, frame, self.view_distance, preparing)
                    .await?;
            }
        }
        emit(
            &json!({"event":"fixture_map_frames_observed","bot":self.index,"count":used.len(),
                    "scope":"ordinary interact and received displayed-item metadata"}),
        )?;
        Ok(())
    }
}
