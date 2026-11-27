//! Entity metadata, equipment and item-entity appearance encoders.
//!
//! Metadata indices are an exact entity contract per type, asserted below.
use super::*;

/// Entity's fourth defineId call uses BOOLEAN (26.1.2 javap Entity static ctor).
const ENTITY_DATA_CUSTOM_NAME_VISIBLE_INDEX: u8 = 3;

/// Appearance -> SetEntityData (+ equipment + item-entity stack).
pub(super) fn push_appearance(
    out: &mut Vec<PlannedPacket>,
    conn: ConnectionId,
    entity: &EntityView,
    previous: Option<&EntityAppearance>,
) {
    push_appearance_for(
        out,
        conn,
        entity.id,
        &entity.appearance,
        entity.type_id,
        previous,
    );
}

/// Living metadata is an exact entity contract property, not a growing list of
/// projectile exceptions. Sending living index 9 to a cloud/rocket is fatal.
pub(super) fn has_living_metadata(type_id: u32) -> bool {
    mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id).is_some_and(
        |contract| contract.behavior.archetype == mc_data::entity_types::EntityArchetype::Living,
    )
}

/// 26.1.2 accessor inheritance: Animal -> AgeableMob starts at 16.
/// Hoglin inherits Animal too; Piglin instead adds its baby accessor after
/// AbstractPiglin's immunity accessor (17). Always send the adult false image
/// so growth and skipped snapshots clear a previously visible child.
fn baby_metadata_index(type_id: u32) -> Option<u8> {
    let contract = mc_data::entity_types::entity_type_contract_26_1_2_by_protocol_id(type_id)?;
    match contract.behavior.metadata_schema.as_str() {
        "Armadillo" | "Axolotl" | "Bee" | "Camel" | "CamelHusk" | "Cat" | "Chicken" | "Cow"
        | "Donkey" | "Fox" | "Frog" | "Goat" | "HappyGhast" | "Hoglin" | "Horse" | "Llama"
        | "Mule" | "MushroomCow" | "Nautilus" | "Ocelot" | "Panda" | "Parrot" | "Pig"
        | "PolarBear" | "Rabbit" | "Sheep" | "SkeletonHorse" | "Sniffer" | "Strider"
        | "TraderLlama" | "Turtle" | "Villager" | "Wolf" | "ZombieHorse" | "ZombieNautilus" => {
            Some(AGEABLE_ENTITY_DATA_BABY_INDEX)
        }
        "Zombie" | "Husk" | "Drowned" | "ZombieVillager" | "ZombifiedPiglin" | "Zoglin" => Some(16),
        "Piglin" => Some(17),
        _ => None,
    }
}

pub(super) fn player_entity_type_id() -> u32 {
    use std::sync::LazyLock;
    static ID: LazyLock<u32> = LazyLock::new(|| {
        mc_data::entity_types::krasnokamen_required_entity_types()
            .id_of(&mc_data::Identifier::parse("minecraft:player").expect("static id parses"))
            .expect("player entity type present")
    });
    *ID
}

/// Item entities carry no LivingEntity accessors (no health float at index
/// 9): sending it hard-fails the vanilla client (`undefined/mismatched
/// entity accessor minecraft:item index 9`, session dead). Their stack
/// rides `DATA_ITEM` (index 8) via the dedicated arm below instead.
pub(crate) fn push_appearance_for(
    out: &mut Vec<PlannedPacket>,
    conn: ConnectionId,
    id: EntityId,
    appearance: &EntityAppearance,
    type_id: u32,
    previous: Option<&EntityAppearance>,
) {
    push_appearance_with(out, conn, id, appearance, type_id, previous, true);
}

pub(super) fn push_appearance_with(
    out: &mut Vec<PlannedPacket>,
    conn: ConnectionId,
    id: EntityId,
    appearance: &EntityAppearance,
    type_id: u32,
    previous: Option<&EntityAppearance>,
    send_equipment: bool,
) {
    let mut values: Vec<WireEntityDataValue> = Vec::new();
    if let Some((value, _)) = appearance.experience_orb {
        values.push(WireEntityDataValue::Int { index: 8, value });
    }
    values.push(WireEntityDataValue::Byte {
        index: ENTITY_DATA_SHARED_FLAGS_INDEX,
        value: appearance.shared_flags as i8,
    });
    if has_living_metadata(type_id) {
        values.push(WireEntityDataValue::Float {
            index: LIVING_ENTITY_DATA_HEALTH_INDEX_26_1_2,
            value: appearance.health,
        });
        values.push(WireEntityDataValue::EffectParticles {
            index: LIVING_ENTITY_DATA_EFFECT_PARTICLES_INDEX,
            particles: appearance
                .effect_particles
                .iter()
                .map(|particle| {
                    use mc_core::boundary::EffectParticle as CoreParticle;
                    use mc_protocol::packets::play::EffectParticle as WireParticle;
                    match *particle {
                        CoreParticle::EntityEffect { argb } => WireParticle::EntityEffect { argb },
                        CoreParticle::TrialOmen => WireParticle::TrialOmen,
                        CoreParticle::RaidOmen => WireParticle::RaidOmen,
                    }
                })
                .collect(),
        });
        values.push(WireEntityDataValue::Boolean {
            index: LIVING_ENTITY_DATA_EFFECT_AMBIENCE_INDEX,
            value: appearance.effects_ambient,
        });
        // LivingEntity DATA_ARROW_COUNT_ID / DATA_STINGER_COUNT_ID, INT serializers.
        values.push(WireEntityDataValue::Int {
            index: 12,
            value: appearance.arrow_count,
        });
        values.push(WireEntityDataValue::Int {
            index: 13,
            value: appearance.stinger_count,
        });
    }
    if type_id == player_entity_type_id() {
        values.push(WireEntityDataValue::Float {
            index: PLAYER_DATA_ABSORPTION_INDEX,
            value: appearance.absorption,
        });
    }
    if previous.is_none_or(|old| old.custom_name != appearance.custom_name)
        && (previous.is_some() || appearance.custom_name.is_some())
    {
        values.push(WireEntityDataValue::OptionalComponent {
            index: 2,
            value: appearance.custom_name.clone(),
        });
    }
    if previous.is_none_or(|old| old.custom_name_visible != appearance.custom_name_visible) {
        values.push(WireEntityDataValue::Boolean {
            index: ENTITY_DATA_CUSTOM_NAME_VISIBLE_INDEX,
            value: appearance.custom_name_visible,
        });
    }
    values.push(WireEntityDataValue::Pose {
        index: ENTITY_DATA_POSE_INDEX,
        pose: pose_of(appearance.pose),
    });
    if let Some(index) = baby_metadata_index(type_id) {
        values.push(WireEntityDataValue::Boolean {
            index,
            value: appearance.baby,
        });
    }
    for (index, value) in appearance.extra.iter() {
        values.push(map_extra(*index, value));
    }
    if values.len() > 64 {
        values.truncate(64);
    }
    push_best_effort(
        out,
        conn,
        &ClientboundSetEntityData {
            entity_id: id.0,
            values,
        },
    );
    let equipment_changed = previous.map_or_else(
        || {
            appearance
                .equipment
                .iter()
                .chain([&appearance.body_armor, &appearance.saddle])
                .any(|stack| stack.as_ref().is_some_and(|stack| !stack.is_empty()))
        },
        |old| {
            old.equipment != appearance.equipment
                || old.body_armor != appearance.body_armor
                || old.saddle != appearance.saddle
        },
    );
    if send_equipment && has_living_metadata(type_id) && equipment_changed {
        let equipment = appearance
            .equipment
            .iter()
            .chain([&appearance.body_armor, &appearance.saddle])
            .enumerate()
            .map(|(slot, stack)| EntityEquipment {
                slot: equipment_slot_of(slot),
                item: stack.clone().unwrap_or_default(),
            })
            .collect();
        push_best_effort(
            out,
            conn,
            &ClientboundSetEntityEquipment {
                entity_id: id.0,
                equipment,
            },
        );
    }
    if let Some(stack) = appearance.item.as_ref() {
        push_best_effort(
            out,
            conn,
            &ClientboundSetEntityData {
                entity_id: id.0,
                values: vec![WireEntityDataValue::ItemStack {
                    index: ITEM_ENTITY_DATA_ITEM_INDEX,
                    stack: stack.clone(),
                }],
            },
        );
    }
    // Links are world state like the rest of the pairing: on the world FIFO
    // after the entity's AddEntity, in vanilla `ServerEntity.sendPairingData`
    // order (data, equipment, passengers, leash), within the viewer's world
    // credit. On the reliable queue a spawn burst of mounted or leashed
    // entities overflowed it and closed a client that was still reading,
    // and the link could overtake the AddEntity it refers to.
    if previous.is_none_or(|old| old.passengers != appearance.passengers)
        && (previous.is_some() || !appearance.passengers.is_empty())
    {
        push_best_effort(
            out,
            conn,
            &mc_protocol::packets::play::ClientboundSetPassengers {
                vehicle_id: id.0,
                passenger_ids: appearance.passengers.iter().map(|id| id.0).collect(),
            },
        );
    }
    if previous.is_none_or(|old| old.leash_holder != appearance.leash_holder)
        && (previous.is_some() || appearance.leash_holder.is_some())
    {
        push_best_effort(
            out,
            conn,
            &mc_protocol::packets::play::ClientboundSetEntityLeash {
                source_entity_id: id.0,
                holder_entity_id: appearance.leash_holder.map(|id| id.0),
            },
        );
    }
}

// The core writes sheep wool at the protocol's own index.
const _: () = assert!(
    mc_core::blueprint::SHEEP_WOOL_DATA_INDEX
        == mc_protocol::packets::play::SHEEP_ENTITY_DATA_WOOL_INDEX
);
const _: () = assert!(
    mc_core::blueprint::CHICKEN_VARIANT_DATA_INDEX
        == mc_protocol::packets::play::CHICKEN_VARIANT_DATA_INDEX
);
const _: () = assert!(
    mc_core::blueprint::COW_VARIANT_DATA_INDEX
        == mc_protocol::packets::play::COW_VARIANT_DATA_INDEX
);
const _: () = assert!(
    mc_core::blueprint::PIG_VARIANT_DATA_INDEX
        == mc_protocol::packets::play::PIG_VARIANT_DATA_INDEX
);
const _: () = assert!(
    mc_core::blueprint::ZOMBIE_NAUTILUS_VARIANT_DATA_INDEX
        == mc_protocol::packets::play::ZOMBIE_NAUTILUS_VARIANT_DATA_INDEX
);

// The core writes the sleeping position at the protocol's own index.
const _: () = assert!(
    mc_core::sleep::SLEEPING_POS_DATA_INDEX
        == mc_protocol::packets::play::LIVING_ENTITY_DATA_SLEEPING_POS_INDEX
);

pub(super) fn map_extra(
    index: u8,
    value: &mc_data::entity_data_value::EntityDataValue,
) -> WireEntityDataValue {
    match value.clone() {
        mc_data::entity_data_value::EntityDataValue::Direction(value) => {
            use mc_protocol::packets::play::EntityDirection;
            let value = match value {
                0 => EntityDirection::Down,
                1 => EntityDirection::Up,
                2 => EntityDirection::North,
                3 => EntityDirection::South,
                4 => EntityDirection::West,
                _ => EntityDirection::East,
            };
            WireEntityDataValue::Direction { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::PaintingVariant(value) => {
            WireEntityDataValue::PaintingVariant { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::ChickenVariant(value) => {
            WireEntityDataValue::ChickenVariant { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::CowVariant(value) => {
            WireEntityDataValue::CowVariant { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::PigVariant(value) => {
            WireEntityDataValue::PigVariant { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::ZombieNautilusVariant(value) => {
            WireEntityDataValue::ZombieNautilusVariant { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::WeatheringCopper(value) => {
            WireEntityDataValue::WeatheringCopper { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::CopperGolemState(value) => {
            WireEntityDataValue::CopperGolemState { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::SnifferState(value) => {
            WireEntityDataValue::SnifferState { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::VillagerData {
            kind,
            profession,
            level,
        } => WireEntityDataValue::VillagerData {
            index,
            villager_type: kind,
            profession,
            level,
        },
        mc_data::entity_data_value::EntityDataValue::Byte(v) => {
            WireEntityDataValue::Byte { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::VarInt(v) => {
            WireEntityDataValue::Int { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::VarLong(v) => {
            WireEntityDataValue::Long { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::Float(v) => {
            WireEntityDataValue::Float { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::Rotations([x, y, z]) => {
            WireEntityDataValue::Rotations {
                index,
                value: EntityRotations { x, y, z },
            }
        }
        mc_data::entity_data_value::EntityDataValue::Boolean(v) => {
            WireEntityDataValue::Boolean { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::String(v) => {
            WireEntityDataValue::String { index, value: v }
        }
        mc_data::entity_data_value::EntityDataValue::BlockState(v) => {
            WireEntityDataValue::BlockState {
                index,
                value: mc_protocol::packets::play::BlockStateId::new(v).unwrap_or(
                    mc_protocol::packets::play::BlockStateId::new(0).expect("state 0 is valid"),
                ),
            }
        }
        mc_data::entity_data_value::EntityDataValue::Item(v) => WireEntityDataValue::ItemStack {
            index,
            stack: v.unwrap_or(mc_data::item_stack::ItemStack::EMPTY),
        },
        mc_data::entity_data_value::EntityDataValue::EntityEffectParticle(argb) => {
            WireEntityDataValue::EntityEffectParticle { index, argb }
        }
        mc_data::entity_data_value::EntityDataValue::OptionalUnsignedInt(value) => {
            WireEntityDataValue::OptionalUnsignedInt { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::OptionalLivingEntityReference(value) => {
            WireEntityDataValue::OptionalLivingEntityReference { index, value }
        }
        mc_data::entity_data_value::EntityDataValue::OptionalBlockPos(v) => {
            WireEntityDataValue::OptionalBlockPosition {
                index,
                value: v.map(|[x, y, z]| mc_protocol::packets::play::pack_block_pos(x, y, z)),
            }
        }
    }
}

pub(super) fn pose_of(pose: u8) -> EntityPose {
    match pose {
        1 => EntityPose::FallFlying,
        2 => EntityPose::Sleeping,
        3 => EntityPose::Swimming,
        4 => EntityPose::SpinAttack,
        5 => EntityPose::Crouching,
        6 => EntityPose::LongJumping,
        7 => EntityPose::Dying,
        8 => EntityPose::Croaking,
        9 => EntityPose::UsingTongue,
        10 => EntityPose::Sitting,
        11 => EntityPose::Roaring,
        12 => EntityPose::Sniffing,
        13 => EntityPose::Emerging,
        14 => EntityPose::Digging,
        _ => EntityPose::Standing,
    }
}

fn equipment_slot_of(slot: usize) -> EquipmentSlot {
    match slot {
        0 => EquipmentSlot::MainHand,
        1 => EquipmentSlot::OffHand,
        2 => EquipmentSlot::Feet,
        3 => EquipmentSlot::Legs,
        4 => EquipmentSlot::Chest,
        5 => EquipmentSlot::Head,
        6 => EquipmentSlot::Body,
        _ => EquipmentSlot::Saddle,
    }
}
