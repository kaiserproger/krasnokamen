//! Entity removal (swap-remove follow-up) and mob spawning.
//! Moved verbatim from `tick.rs`.

use super::*;

impl TickState {
    /// Remove an entity by slot; fixes connection/inventory/health maps.
    pub fn remove_entity(&mut self, slot: EntitySlot) {
        let body = self.entities.body(slot);
        let id = body.id;
        self.open_containers.remove(&id);
        let kind = body.kind;
        let movement = body.movement;
        if self.entities.row(slot).projectile.is_some()
            || movement == crate::blueprint::MovementClass::Projectile
        {
            let uuid = self.entities.row(slot).uuid;
            crate::physical_attacks::drop_projectile(self, uuid);
        }
        self.armor_stand_hits.remove(&id);
        if let EntityKind::Player { .. } = kind
            && let Some(connection) = self.connections.entity_connection(id)
        {
            // One path for every player departure: it also drops the name and
            // uuid rows, and advances the membership generation the
            // unlocked-session mirror is gated on.
            self.connections.remove_by_connection(connection);
            self.outbox.remove(&connection);
        }
        self.players.remove(&id);
        let last = self.entities.len() as u32 - 1;
        self.entities.remove(slot);
        self.follow_swap_removal(slot, last);
    }

    /// Swap-removal moved the row at `last` into `removed`. Slots staged for
    /// the merge/commit (damage, pickups, container custody) follow that row;
    /// entries for the removed row itself are dropped, so no staged slot can
    /// point past the store or at an unrelated entity.
    pub(super) fn follow_swap_removal(&mut self, removed: EntitySlot, last: u32) {
        let follow = |slot: EntitySlot| {
            if slot == removed {
                None
            } else if slot.0 == last {
                Some(removed)
            } else {
                Some(slot)
            }
        };
        self.merge_damage.retain_mut(|(slot, _)| {
            follow(*slot).is_some_and(|next| {
                *slot = next;
                true
            })
        });
        self.plugin_damage.retain_mut(|(slot, _)| {
            follow(*slot).is_some_and(|next| {
                *slot = next;
                true
            })
        });
        self.merge_pickups.retain_mut(|(item, collector, _)| {
            let (Some(next_item), Some(next_collector)) = (follow(*item), follow(*collector))
            else {
                return false;
            };
            *item = next_item;
            *collector = next_collector;
            true
        });
        if !self.merge_consumed_items.is_empty() {
            self.merge_consumed_items = self
                .merge_consumed_items
                .drain()
                .filter_map(follow)
                .collect();
        }
    }

    /// Spawn a mob/item entity from a blueprint type id.
    pub fn spawn_mob(&mut self, type_id: u32, pos: [f64; 3], health: f32) -> EntityId {
        let id = self.entities.alloc_id();
        let uuid = self.entity_uuid(id);
        let blueprint = self.registry.blueprint(type_id);
        let (movement, half_width, height, speed, damage, sight, reach) = blueprint
            .map(|blueprint| {
                (
                    blueprint.movement,
                    blueprint.half_width,
                    blueprint.height,
                    blueprint.params.speed,
                    blueprint.params.attack_damage,
                    blueprint.sensors.sight_range,
                    blueprint.sensors.attack_reach,
                )
            })
            .unwrap_or((
                crate::blueprint::MovementClass::Walker,
                0.3,
                1.8,
                2.0,
                0.0,
                10.0,
                2.0,
            ));
        let is_item = type_id == self.registry.item_type_id;
        let kind = if is_item {
            EntityKind::Item { type_id }
        } else {
            EntityKind::Mob { type_id }
        };
        let mut appearance = EntityAppearance {
            health,
            ..Default::default()
        };
        if self
            .registry
            .blueprint(type_id)
            .is_some_and(|blueprint| blueprint.wool)
        {
            let roll = uuid.as_u128();
            let color = crate::blueprint::natural_wool_color(roll as u64, (roll >> 64) as u64);
            appearance.extra.push((
                crate::blueprint::SHEEP_WOOL_DATA_INDEX,
                crate::boundary::EntityDataValue::Byte(color as i8),
            ));
        }
        let appearance = Arc::new(appearance);
        self.entities.spawn(
            id,
            uuid,
            kind,
            ColdFields {
                pos,
                vel: [0.0; 3],
                yaw: 0.0,
                pitch: 0.0,
                ground: false,
                health,
                // Adults start without a cooldown; birth initializes child age
                // separately through the native lifecycle module.
                breed_cooldown: 0,
                pickup_delay: if is_item {
                    self.registry
                        .blueprint(type_id)
                        .map(|blueprint| blueprint.params.pickup_delay)
                        .unwrap_or(10)
                } else {
                    0
                },
                item: None,
                appearance,
                movement,
                half_width,
                height,
                speed,
                attack_damage: damage,
                sight_range: sight,
                attack_reach: reach,
            },
        );
        if let Some(slot) = self.entities.slot_of(id) {
            crate::villager::conversion::initialize(self, slot, type_id);
            crate::entity_loot::initialize(self, slot);
            crate::tnt::initialize(self, slot, type_id);
            crate::falling::initialize(self, slot, type_id);
            crate::vehicles::initialize(self, slot);
            crate::special_mobs::initialize(self, slot, type_id);
            crate::bees::initialize(self, slot, type_id);
            crate::allay::initialize(self, slot, type_id);
            crate::copper_golem::initialize(self, slot, type_id);
            crate::mob_lanes::initialize(self, slot, type_id);
            crate::leash::initialize(self, slot);
            crate::end_fight::initialize_entity(self, slot);
            crate::dolphin::initialize(self, slot, type_id);
            crate::turtle::initialize(self, slot, type_id);
            crate::mob_equipment::initialize(self, slot, type_id);
            if self.body_overlaps_collision(pos, half_width, height) {
                let entity_type = self
                    .registry
                    .blueprint(type_id)
                    .map(|blueprint| blueprint.name.clone());
                tracing::warn!(
                    tick = self.tick,
                    slot = slot.0,
                    type_id,
                    entity_type = entity_type.as_deref().unwrap_or("unknown"),
                    position = ?pos,
                    half_width,
                    height,
                    writer = "TickState::spawn_mob",
                    "spawned body already overlaps a collision box"
                );
            }
        }
        id
    }

    /// Whether the body AABB at `pos` already overlaps a world collision box.
    ///
    /// Every spawn path (`/summon`, fixtures, plugins, natural spawning) enters
    /// through [`TickState::spawn_mob`] without asking the world for clearance,
    /// and vanilla `/summon` also places a body inside a block, so this never
    /// refuses: the spawn path only names the writer of a body that will
    /// suffocate at a fixed position.
    fn body_overlaps_collision(&self, pos: [f64; 3], half_width: f64, height: f64) -> bool {
        use mc_physics::mth::{ceil_i32, floor_i32};
        if !pos.iter().all(|value| value.is_finite())
            || !half_width.is_finite()
            || !height.is_finite()
            || half_width <= 0.0
            || height <= 0.0
        {
            return false;
        }
        let body = [
            pos[0] - half_width,
            pos[1],
            pos[2] - half_width,
            pos[0] + half_width,
            pos[1] + height,
            pos[2] + half_width,
        ];
        let cells = [
            (floor_i32(body[0]), ceil_i32(body[3]) - 1),
            (floor_i32(body[1]), ceil_i32(body[4]) - 1),
            (floor_i32(body[2]), ceil_i32(body[5]) - 1),
        ];
        // A spawn body spans a handful of cells; anything larger is not a body
        // this diagnostic can describe, so it is not scanned.
        if cells.iter().any(|&(min, max)| max < min)
            || cells
                .iter()
                .any(|&(min, max)| i64::from(max) - i64::from(min) > 8)
        {
            return false;
        }
        let mut hit = false;
        'scan: for x in cells[0].0..=cells[0].1 {
            for y in cells[1].0..=cells[1].1 {
                for z in cells[2].0..=cells[2].1 {
                    // A cell outside the resident world is not evidence either
                    // way; the motion path's conservative missing-chunk block
                    // would turn every unloaded spawn into a false report.
                    let Some(state) = self.world.block_at(x, y, z) else {
                        continue;
                    };
                    let Some(boxes) = self.registry.collision_boxes.get(state as usize) else {
                        continue;
                    };
                    for block in boxes.iter() {
                        let local = block.as_blocks();
                        let obstacle = [
                            f64::from(x) + local[0],
                            f64::from(y) + local[1],
                            f64::from(z) + local[2],
                            f64::from(x) + local[3],
                            f64::from(y) + local[4],
                            f64::from(z) + local[5],
                        ];
                        if mc_physics::aabb_intersects_deflated_obstacle(body, obstacle, 0.0) {
                            hit = true;
                            break 'scan;
                        }
                    }
                }
            }
        }
        hit
    }
}
