//! Type-specific entity data values: the value kinds an [`EntityAppearance`]
//! extra-data list carries (variant, colour, profession, sheared, saddled, ...).
//!
//! The value kind lives here because it can hold an [`ItemStack`]; the wire
//! serializer is the protocol's and the journal image is mc-core's.

use uuid::Uuid;

use crate::item_stack::ItemStack;

/// Value kinds used by type-specific entity data.
#[derive(Debug, Clone, PartialEq)]
pub enum EntityDataValue {
    Byte(i8),
    VarInt(i32),
    VarLong(i64),
    Float(f32),
    Rotations([f32; 3]),
    Boolean(bool),
    WeatheringCopper(u8),
    CopperGolemState(u8),
    SnifferState(u8),
    Direction(u8),
    PaintingVariant(u32),
    String(String),
    BlockState(u32),
    Item(Option<ItemStack>),
    /// Single particle serializer, used by AreaEffectCloud at index 10.
    EntityEffectParticle(i32),
    /// Vanilla optional unsigned integer serializer (firework attachment).
    OptionalUnsignedInt(Option<u32>),
    /// `Optional<BlockPos>` (the sleeping position).
    OptionalBlockPos(Option<[i32; 3]>),
    OptionalLivingEntityReference(Option<Uuid>),
    VillagerData {
        kind: i32,
        profession: i32,
        level: i32,
    },
    ChickenVariant(u32),
    CowVariant(u32),
    PigVariant(u32),
    ZombieNautilusVariant(u32),
}
