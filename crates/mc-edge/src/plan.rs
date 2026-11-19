//! Per-viewer tick planning: [`TickOutput`] + [`ViewerState`] -> packets.
//!
//! Retained entity interest consumes publish-wide changes; full held-chunk
//! walks repair window/membership gaps. Canonical dirty work and due/credit
//! debt retain packet order. Exact-baseline movement/head frames are encoded
//! once per publish and shared; bosses use cached candidate ids.
//! Chunk reconciliation follows the same rule: a publish-wide chunk revision
//! delta (`chunk_delta`) lets a viewer that was fully reconciled against the
//! previous publish, has no delivery debt and did not move its window touch
//! only the changed chunks; a skipped publish, moved window, respawn, unsent
//! chunk or overflowing delta falls back to the full held-chunk walk.
//! Chunk traffic is a Chebyshev square, nearest-first (spiral order).
//! Dense admission caches squared distances before partial nearest selection;
//! a full queue builds no spawn list.
//! These are planning-only shortcuts; range, packet credit, movement cadence,
//! per-viewer baselines and the packet encoders remain authoritative.
//!
//! [`plan_player_changes`] plans reliable replay; normal pump routing defers
//! public block-entity data to the snapshot's world FIFO.
//!
//! `Died` maps to a `SystemChat` death message (reliable); the respawn
//! screen itself is driven by the client's own death handling. `Respawned`
//! (same dimension) maps to `ClientboundRespawn` (`data_to_keep = 0`, like
//! vanilla death respawn) followed by a position sync; the viewer's cached
//! chunks/entities are reset so `plan_viewer` resends the world around the
//! new position.
//!
//! Owner modules: `chunk_frames` (framed chunk cache, encoder and the
//! Chebyshev square order), `credit` (world-queue credit and best-effort
//! framing), `viewer` (cached view and tracked baselines), `viewer_plan`
//! (the publish plan), `entities` and `seats` (entity and seat plans), and
//! `entity_motion`, `entity_appearance`, `entity_events` (wire encoders).

mod player_changes;
pub use player_changes::plan_player_changes;
pub(crate) use player_changes::plan_routed_player_changes;

mod camera;
pub(crate) mod chunk_batch;
mod chunk_delta;
mod chunk_frames;
mod credit;
mod entities;
mod entity_appearance;
mod entity_candidates;
mod entity_events;
mod entity_interest;
mod entity_motion;
mod entity_type_flags;
pub(crate) mod item_probe;
mod scratch;
mod seats;
mod shared_moves;
mod tick_index;
mod viewer;
mod viewer_plan;
use chunk_delta::{ChunkChangeKind, ChunkDelta, ChunkDeltaLog, ChunkSync, chunk_stat};
pub(crate) use chunk_frames::CHUNKS_PER_TICK;
pub(crate) use chunk_frames::find_chunk;
use chunk_frames::spiral_square;
pub use chunk_frames::{ChunkPacketCache, fill_chunk_packets};
pub(crate) use credit::{WorldCredit, push_best_effort, push_reliable, try_push};
#[cfg(test)]
use entities::entity_movement_due;
use entities::{EntityUpdate, UNDELIVERED_TYPE, entity_trackable, is_undelivered, plan_entities};
#[cfg(test)]
pub(crate) use entity_appearance::push_appearance_for;
#[cfg(test)]
use entity_appearance::{has_living_metadata, map_extra, pose_of};
use entity_appearance::{player_entity_type_id, push_appearance, push_appearance_with};
use entity_events::{EXPLOSION_RANGE_SQ, explode_packet, push_entity_event};
#[cfg(test)]
use entity_motion::floor_f32_i32;
use entity_motion::{
    MovePacket, entity_movement, entity_spawn_position, floor_i32, pack_degrees, per_tick,
};
use scratch::{PlanScratch, ViewWindow};
use seats::{clear_seats, plan_seats};
pub(crate) use tick_index::TickPlanIndex;
use tick_index::{entity_chunk, event_subject};
pub(crate) use viewer::TrackedEntity;
pub use viewer::ViewerState;
use viewer::{forget_everything, relink_passengers};
use viewer_plan::own_entity;
#[cfg(test)]
pub(crate) use viewer_plan::plan_viewer;
pub(crate) use viewer_plan::plan_viewer_indexed;

use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::sync::Arc;

use bytes::BytesMut;
use mc_core::boundary::{
    ConnectionId, EntityAppearance, EntityEvent, EntityId, EntityView, Hand, SnapshotChunk,
    TickOutput, WorldSnapshot,
};
use mc_protocol::frame::{Compression, append_frame};
use mc_protocol::packets::Packet;
use mc_protocol::packets::play::{
    AGEABLE_ENTITY_DATA_BABY_INDEX, AddEntity, BlockUpdate, ClientboundExplode,
    ClientboundSetEntityData, ClientboundSetEntityEquipment, ClientboundSetTime,
    ClientboundSwingAnimation, ClientboundTakeItemEntity, ENTITY_DATA_POSE_INDEX,
    ENTITY_DATA_SHARED_FLAGS_INDEX, EntityDataValue as WireEntityDataValue, EntityEquipment,
    EntityEvent as WireEntityEvent, EntityPose, EntityPositionSync, EntityRotations, EntityVec3,
    EquipmentSlot, ExplosionBlockParticle, ForgetLevelChunk, ITEM_ENTITY_DATA_ITEM_INDEX,
    LIVING_ENTITY_DATA_EFFECT_AMBIENCE_INDEX, LIVING_ENTITY_DATA_EFFECT_PARTICLES_INDEX,
    LIVING_ENTITY_DATA_HEALTH_INDEX_26_1_2, MoveEntityPos, MoveEntityPosRot, MoveEntityRot,
    PLAYER_DATA_ABSORPTION_INDEX, RemoveEntities, RotateHead, SectionBlockChange,
    SectionBlocksUpdate, SetCenterChunk, SetChunkCacheRadius, pack_section_pos,
    pack_section_relative_pos,
};

pub struct PlannedPacket {
    pub connection: ConnectionId,
    pub id: i32,
    pub reliable: bool,
    /// Framed wire bytes (length prefix + id + body, threshold-256
    /// compression): the only encoding, routed to the outbound queue as is.
    /// Empty only for a chunk placeholder until [`fill_chunk_packets`].
    pub data: bytes::Bytes,
    /// Chunk position of a `LevelChunkWithLight`; its `data` is empty while
    /// it is a cache-miss placeholder.
    pub chunk: Option<(i32, i32)>,
    pub chunk_dimension: mc_core::dimensions::Dimension,
}

/// Last sent authoritative `(position, yaw, pitch, head_yaw)` of an entity.
/// Relative positions subtract quantized endpoints; thresholds use raw ones.
type MoveBaseline = ([f64; 3], f32, f32, f32);

/// Plan-time chunk encoding inputs (biomes/blocks/items/block-entity
/// types); chunk encode failures skip that chunk rather than failing the
/// tick.
pub struct PlanRegistries<'a> {
    pub biomes: &'a mc_data::Registry,
    pub blocks: &'a mc_world::BlockRegistry,
    pub items: &'a mc_data::items::ItemRegistry,
    pub block_entity_types: &'a mc_data::block_entity_types::BlockEntityTypeRegistry,
}

#[cfg(test)]
#[path = "plan_tests.rs"]
mod plan_tests;

#[cfg(test)]
#[path = "mounts_wire_tests.rs"]
mod mounts_wire_tests;
#[cfg(test)]
#[path = "projectile_wire_tests.rs"]
mod projectile_wire_tests;
