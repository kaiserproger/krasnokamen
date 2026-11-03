// One bounded, input-fenced runtime command per bot. A matching reply remains
// a protocol observation, not proof that a requested state transition occurred.
const RUNTIME_COMMAND_CAP: usize = 4096;
const RUNTIME_COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq)]
enum RuntimeCommandStage { Queued, WaitingMarker, MarkerObserved, WaitingReply }

struct RuntimeCommand {
    id: String,
    command: String,
    expected_prefix: String,
    marker: String,
    stage: RuntimeCommandStage,
    started: Instant,
    marker_seq: Option<u64>,
    write_seq: Option<u64>,
}

impl Mechanics {
    fn runtime_enqueue(&mut self, bot: u32, id: String, command: String, expected_prefix: String,
        username: &str, nonce: u128) -> Result<Value> {
        let index = bot as usize;
        ensure!(index < self.runtime_commands.len(), "bot index outside population");
        ensure!(!id.is_empty() && id.len() <= 128 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
            "invalid command id");
        ensure!(!command.is_empty() && command.len() <= 2048 && !command.starts_with('/')
            && !command.contains(['\n', '\r']), "invalid command");
        ensure!(!expected_prefix.is_empty() && expected_prefix.len() <= 2048
            && !expected_prefix.contains(['\n', '\r']), "invalid expected prefix");
        ensure!(self.runtime_ids.len() < RUNTIME_COMMAND_CAP && !self.runtime_ids.contains(&id),
            "duplicate id or runtime command capacity reached");
        ensure!(self.runtime_commands[index].is_none(), "bot already has an outstanding runtime command");
        self.runtime_ids.insert(id.clone());
        self.runtime_commands[index] = Some(RuntimeCommand {
            id: id.clone(), command, expected_prefix,
            marker: format!("<{username}> scale_runtime_fence_{nonce}_{bot}"),
            stage: RuntimeCommandStage::Queued, started: Instant::now(), marker_seq: None, write_seq: None,
        });
        Ok(json!({"event":"runtime_command_accepted","id":id,"bot":bot}))
    }

    fn runtime_chat(&mut self, bot: u32, tag: &mc_nbt::Tag) -> Option<Value> {
        let index = bot as usize;
        self.runtime_receive_seq[index] = self.runtime_receive_seq[index].saturating_add(1);
        let sequence = self.runtime_receive_seq[index];
        let elapsed_us = self.measurement_epoch.map(|epoch| epoch.elapsed().as_micros());
        let pending = self.runtime_commands[index].as_mut()?;
        let Some(text) = received_literal_text(tag) else {
            if pending.stage == RuntimeCommandStage::WaitingReply {
                let id = pending.id.clone();
                self.runtime_commands[index] = None;
                return Some(json!({"event":"runtime_command_rejected","id":id,"bot":bot,
                    "reason":"unrenderable or oversized chat while waiting for reply","receive_seq":sequence,"elapsed_us":elapsed_us}));
            }
            return None;
        };
        if pending.stage == RuntimeCommandStage::WaitingMarker && text == pending.marker {
            pending.stage = RuntimeCommandStage::MarkerObserved;
            pending.marker_seq = Some(sequence);
            return Some(json!({"event":"runtime_command_marker","id":pending.id,"bot":bot,
                "receive_seq":sequence,"elapsed_us":elapsed_us}));
        }
        if pending.stage == RuntimeCommandStage::WaitingReply && text.starts_with(&pending.expected_prefix) {
            let event = json!({"event":"runtime_command_reply","id":pending.id,"bot":bot,
                "reply":text,"marker_receive_seq":pending.marker_seq,"write_receive_seq":pending.write_seq,
                "receive_seq":sequence,"elapsed_us":elapsed_us,
                "scope":"matching received chat; not correlated by the protocol or proof of state"});
            self.runtime_commands[index] = None;
            return Some(event);
        }
        None
    }

    fn runtime_record(&mut self, event: &Value) {
        if self.runtime_events.len() < RUNTIME_COMMAND_CAP * 5 {
            self.runtime_events.push(event.clone());
        } else {
            self.runtime_events_truncated += 1;
        }
    }
}

impl Bot {
    async fn runtime_command_poll(&mut self, client: &mut Client, elapsed: Duration) -> Result<()> {
        let Some(shared) = self.mechanics.as_ref().map(Arc::clone) else { return Ok(()); };
        let index = self.index as usize;
        let operation = {
            let mut state = shared.lock();
            let sequence = state.runtime_receive_seq[index];
            let Some(pending) = state.runtime_commands[index].as_mut() else { return Ok(()); };
            if pending.started.elapsed() >= RUNTIME_COMMAND_TIMEOUT
                || self.schedule.window_at(elapsed).is_none_or(|i| self.schedule.windows[i].recovery)
            {
                let event = json!({"event":"runtime_command_timeout","id":pending.id,"bot":self.index,
                    "reason":"deadline or measurement window ended","receive_seq":sequence,"elapsed_us":elapsed.as_micros()});
                state.runtime_commands[index] = None;
                Some((event, None, None))
            } else if pending.stage == RuntimeCommandStage::Queued {
                pending.stage = RuntimeCommandStage::WaitingMarker;
                Some((json!({"event":"runtime_command_marker_written","id":pending.id,"bot":self.index,"elapsed_us":elapsed.as_micros()}),
                    Some(pending.marker.clone()), None))
            } else if pending.stage == RuntimeCommandStage::MarkerObserved {
                pending.stage = RuntimeCommandStage::WaitingReply;
                pending.write_seq = Some(sequence);
                Some((json!({"event":"runtime_command_written","id":pending.id,"bot":self.index,
                    "marker_receive_seq":pending.marker_seq,"write_receive_seq":sequence,"elapsed_us":elapsed.as_micros()}),
                    None, Some(pending.command.clone())))
            } else { None }
        };
        let Some((event, marker, command)) = operation else { return Ok(()); };
        if let Some(marker) = marker {
            let message = marker.strip_prefix(&format!("<{}> ", self.username)).context("invalid runtime marker")?.to_owned();
            client.write_packet(&ServerboundChat { message,
                timestamp_millis: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().min(i64::MAX as u128) as i64,
                salt: i64::from(self.index), signature: None, last_seen_offset: 0,
                last_seen_acknowledged: [0; 3], last_seen_checksum: 0 }).await?;
        } else if let Some(command) = command {
            client.write_packet(&ServerboundChatCommand { command }).await?;
            self.metrics.commands_sent += 1;
        }
        shared.lock().runtime_record(&event);
        emit(&event)?;
        Ok(())
    }
}
