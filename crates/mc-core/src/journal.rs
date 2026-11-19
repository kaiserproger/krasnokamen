//! Tick journal: framed batches of absolute post-images + tombstones,
//! native-transaction checkpoints, recovery by replay (ADR 0013 "Durability").
//!
//! Layout under `<world_dir>/`:
//! - `journal/batch-<first_tick>-<last_tick>.bin`: framed batches. One batch
//!   holds whole ticks: `[magic u32][version u32][store id 16][palette 8]
//!   [tick_lo u64][tick_hi u64][len u32][crc32 u32][payload]`.
//!   Payload is JSON lines of [`JournalEntry`]. Unpublished `.tmp` files are ignored
//!   when reading and removed by the next [`JournalWriter::open`].
//! - `native/state.redb`: one atomic checkpoint of chunks, entities, players,
//!   globals and the committed tick across all dimensions.
//!
//! Write-ahead rule: batches through checkpoint tick `c` are durable before
//! the native transaction starts. Only its Immediate commit makes pruning
//! batches at or below `c` safe; recovery folds only batches above that tick.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::boundary::{BlockPos, ChunkPos, EntityId};
use crate::config::CoreError;
mod player_nbt;
pub use player_nbt::*;
mod replay;
pub use replay::*;

const BATCH_MAGIC: u32 = 0x534F_4C4A;
/// Frame version 2 binds every batch to the store that wrote it and to the
/// block-state palette its numeric ids mean (Books R4-P3, R5-G5). Format 1 is
/// refused, not converted: ADR 0013 keeps no compatibility surface of our own,
/// and a clean stop with the build that wrote it folds the tail into the
/// checkpoint and prunes it before this build starts.
const BATCH_VERSION: u32 = 2;
/// The pre-binding frame, recognized only to name the refusal.
const BATCH_VERSION_V1: u32 = 1;
/// `[magic][version][store id][palette digest][tick_lo][tick_hi][len][crc]`.
const HEADER_LEN: usize = 4 + 4 + 16 + 8 + 8 + 8 + 4 + 4;

/// What a WAL batch is bound to: the store that wrote it and the block-state
/// palette its `JournalEntry::Block` ids mean.
///
/// Both values already persist with the world — the store id in the native
/// header (`store_uuid`, required on every open), the palette as the content
/// the build registers — so binding the frame adds no stored state and no
/// migration. A batch whose identity differs from the opened world is refused
/// by name, never replayed with a warning (Books R4-P3, R5-G5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalIdentity {
    /// The native store's immutable 16-byte id.
    pub store: [u8; 16],
    /// [`mc_world::BlockRegistry::palette_digest`] of the registry that wrote
    /// (and must read) the numeric block states.
    pub palette: [u8; 8],
}

impl WalIdentity {
    /// Identity of `store` under `blocks`.
    #[must_use]
    pub fn new(store: [u8; 16], blocks: &mc_world::BlockRegistry) -> Self {
        Self {
            store,
            palette: blocks.palette_digest(),
        }
    }

    /// Identity for journals a test writes without opening a store: batches
    /// carry this fixed value and a test reader accepts it. Production always
    /// derives the identity from the opened store ([`WalIdentity::new`]).
    #[doc(hidden)]
    #[must_use]
    pub const fn fixture() -> Self {
        Self {
            store: [0; 16],
            palette: [0; 8],
        }
    }
}

/// Lowercase hex, for an identity digest in a refusal.
pub(crate) fn identity_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// One absolute post-image inside a batch. Replay is idempotent.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JournalEntry {
    /// Native plugin custody, KV tombstones and exactly-once operation receipts.
    Plugins {
        value: Box<crate::plugin::PluginDelta>,
    },
    /// Dimension-qualified local post-images in the same global tick batch.
    Dimension {
        dimension: crate::dimensions::Dimension,
        entries: Vec<JournalEntry>,
    },
    EndFight {
        value: crate::end_fight::Fight,
    },
    Raids {
        #[serde(deserialize_with = "crate::raids::deserialize")]
        value: crate::raids::Raids,
    },
    WanderingTrader {
        value: crate::mob_lanes::TraderSpawner,
    },
    /// Absolute block state after the tick.
    Block {
        pos: [i32; 3],
        state: u32,
    },
    /// Absolute native scheduled work for one chunk, including empty clears.
    BlockTicks {
        image: crate::block_ticks::BlockTickChunkImage,
        next_sequence: u64,
    },
    FluidTicks {
        image: crate::fluids::FluidTicksImage,
    },
    /// A chunk entered/left the player-ticking set; unchanged intervals need
    /// no further records because the batch clock supplies elapsed ticks.
    InhabitedTime {
        image: InhabitedTimeImage,
    },
    /// 001 slice0 active-only debt clock (001-d/e, roadmap C/G5).
    /// Absolute per-chunk catch-up debt. Scope0 consumes with zero effects;
    /// the zero image is still journaled for exact-once durability.
    SimDebt {
        image: SimDebtImage,
    },
    /// Absolute world clock, weather duration and retained fade levels.
    Climate {
        value: crate::weather::ClimateImage,
    },
    Border {
        value: crate::command_border::Border,
    },
    Difficulty {
        value: crate::survival::Difficulty,
    },
    /// Canonical gamerule overrides, including rules without an active subsystem.
    GameRules {
        values: BTreeMap<String, String>,
    },
    /// Absolute scoreboard post-image after a scoreboard mutation.
    /// Once-only structure marker claim; retained even when its entity dies.
    GeneratedEntityClaim {
        claim: String,
    },
    Scoreboard {
        value: crate::scoreboard::Scoreboard,
    },
    /// World-global command storage as a named NBT compound.
    CommandStorage {
        value: Vec<u8>,
    },
    SavedMap {
        id: i32,
        value: Arc<crate::maps::SavedMap>,
    },
    /// Absolute bossbar post-image after a bossbar mutation: named-NBT bytes
    /// of the vanilla saved-data envelope (Tag has no serde); decoded on
    /// replay, fail-closed like checkpoints.
    Bossbars {
        value: Vec<u8>,
    },
    /// Player post-image; omitted heavy fields retain the previous image.
    /// Inventory 41 + cursor + open crafting grid ride as separate stacks
    /// (real custody; never folded). Containers ride as block-entity
    /// post-images, not here.
    Player {
        uuid: String,
        #[serde(default)]
        opaque_player_fields: Vec<u8>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_advancement_progress"
        )]
        advancement_progress: Option<Arc<serde_json::Value>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unlocked_recipes: Option<Arc<BTreeSet<String>>>,
        #[serde(default)]
        statistics: Box<mc_data::statistics::PlayerStatistics>,
        #[serde(default)]
        dimension: crate::dimensions::Dimension,
        pos: [f64; 3],
        yaw: f32,
        pitch: f32,
        health: f32,
        food: i32,
        saturation: f32,
        exhaustion: f32,
        air: i32,
        experience_level: i32,
        experience_progress: f32,
        experience_total: i32,
        game_mode: u8,
        selected: u8,
        inventory: Vec<mc_data::ItemStack>,
        carried: mc_data::ItemStack,
        grid: Vec<mc_data::ItemStack>,
        /// Bed head or forced command-selected respawn cell.
        #[serde(default)]
        respawn: Option<[i32; 3]>,
        #[serde(default)]
        respawn_dimension: crate::dimensions::Dimension,
        #[serde(default)]
        respawn_forced: bool,
        #[serde(default)]
        respawn_yaw: f32,
        #[serde(default)]
        respawn_pitch: f32,
        #[serde(default)]
        active_effects: crate::effects::ActiveEffects,
        #[serde(default)]
        enchantment_seed: i32,
        enchantment_offer_seed: i32,
        #[serde(default)]
        warden_warning: crate::shrieker::Warning,
        #[serde(default, deserialize_with = "crate::mace::deserialize_impact")]
        current_impulse_impact_pos: Option<[f64; 3]>,
        #[serde(default)]
        fall_flying: bool,
        #[serde(default)]
        burn_ticks: u32,
        #[serde(default)]
        portal: crate::portals::PortalState,
        tick: u64,
    },
    /// Statistics-only absolute changes; idle timer progression needs no entry.
    Statistics {
        uuid: String,
        #[serde(deserialize_with = "deserialize_statistics_changes")]
        changes: Vec<(u32, u32, i32)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timers: Option<mc_data::statistics::StatisticsTimers>,
        tick: u64,
    },
    /// Block-entity post-image (chest/furnace slots + furnace progress).
    BlockEntity {
        pos: [i32; 3],
        slots: Vec<mc_data::ItemStack>,
        furnace: Option<FurnaceImage>,
        tick: u64,
    },
    /// Block-entity removal tombstone (container block broken): deletes the
    /// checkpoint base record on replay so the next checkpoint's injection
    /// (which clears first) cannot resurrect it.
    RemoveBlockEntity {
        pos: [i32; 3],
        tick: u64,
    },
    /// Opaque vanilla NBT (signs), or a removal tombstone.
    OpaqueBlockEntity {
        pos: [i32; 3],
        #[serde(deserialize_with = "crate::conduit::deserialize_nbt")]
        nbt: Option<Vec<u8>>,
    },
    /// Full mob/item/projectile post-image. Mob motion remains checkpoint-only
    /// (ADR 0013), except birth coordinates. Projectile motion is journaled
    /// every flight tick together with its custody/payload, so replay cannot
    /// rewind a missile across a hit or resurrect ammunition.
    /// Retained state shares its immutable allocation with the journal baseline;
    /// other large payloads are boxed. Serde keeps the existing representation.
    Entity {
        id: i32,
        uuid: String,
        type_id: u32,
        health: f32,
        alive: bool,
        age: u32,
        breed_cooldown: u32,
        pickup_delay: u32,
        item: Option<mc_data::ItemStack>,
        appearance: EntityAppearanceImage,
        #[serde(default)]
        active_effects: crate::effects::ActiveEffects,
        #[serde(default, deserialize_with = "deserialize_optional_villager_image")]
        villager: Option<Box<crate::villager::VillagerState>>,
        #[serde(default)]
        state: Arc<EntityStateImage>,
        #[serde(default, deserialize_with = "deserialize_optional_projectile_image")]
        projectile: Option<Box<crate::projectiles::ProjectileState>>,
        tick: u64,
        /// Birth coordinates for ordinary mobs; current pose for motion-tracked bodies.
        pos: [f64; 3],
        /// Core blocks/second, retained with the full-image pose.
        #[serde(default)]
        vel: [f64; 3],
        /// Spawn yaw (same rule as `pos`).
        yaw: f32,
        /// Spawn pitch (same rule as `pos`).
        pitch: f32,
    },
    /// Compact absolute non-wire state; keeps timers out of appearance post-images.
    /// Shared with the comparison baseline, with unchanged serde representation.
    EntityState {
        id: i32,
        state: Arc<EntityStateImage>,
        tick: u64,
    },
    /// Health-only update (hurt, no spawn/appearance change): the full
    /// record (appearance, item, uuid) rides the spawn entry + checkpoint
    /// base. Keeps per-tick journal volume proportional to damage, not to
    /// appearance image size.
    EntityVitals {
        id: i32,
        health: f32,
        alive: bool,
        tick: u64,
    },
    /// Dropped-item pose and lifecycle clocks; no repeated stack/appearance image.
    ItemMotion {
        id: i32,
        pos: [f64; 3],
        vel: [f64; 3],
        yaw: f32,
        pitch: f32,
        ground: bool,
        age: u32,
        pickup_delay: u32,
        tick: u64,
    },
    /// Removal tombstone.
    RemoveEntity {
        id: i32,
        tick: u64,
    },
    /// Remote ticket state after the tick.
    Ticket {
        owner: String,
        chunk: [i32; 2],
        tick: u64,
        /// Chunk radius around the centre (default 0 = centre only; keeps
        /// pre-radius journals readable).
        #[serde(default)]
        radius: i32,
    },
    /// Absolute dimension-local settlement post-image plus monotonic high water.
    Settlement {
        image: Box<crate::settlements::RecordImage>,
        high_water: u64,
        tick: u64,
    },
    /// Tombstone for a durable ticket that no longer exists (`/forceload remove`,
    /// plugin/API release). Appended last: earlier variant indices are unchanged.
    TicketRemoved {
        owner: String,
        tick: u64,
    },
    /// Dimension-local block-log records and rollback marks (`block_log`).
    BlockLog {
        value: Box<crate::block_log::Delta>,
    },
    /// Absolute schedule image; appended to preserve older variant indices.
    ScheduledFunctions {
        value: Vec<(u64, String)>,
    },
    /// Datapack enable/disable selection; appended to preserve variant order.
    DatapackSelection {
        value: crate::datapacks::Selection,
    },
    /// `/defaultgamemode` (vanilla `WorldData.GameType`); appended to preserve variant order.
    DefaultGameMode {
        value: crate::commands::GameMode,
    },
    /// Absolute battle decision history at the same boundary as native effects.
    /// The first battle record of a dimension after a checkpoint capture or
    /// recovery; later changed ticks carry `BattlePatch`.
    Battle {
        value: Arc<String>,
    },
    /// Pose of a battle body, coupled to the decision history of this tick.
    EntityMotion {
        id: i32,
        pos: [f64; 3],
        vel: [f64; 3],
        yaw: f32,
        pitch: f32,
        ground: bool,
    },
    /// Dimension-local migration transit custody post-image (ADR 0023) in the
    /// transit module's own codec; `None` is the tombstone of a completed
    /// arrival. Appended to preserve variant order; a binary without it refuses
    /// the batch (downgrade unsupported).
    Transit {
        id: uuid::Uuid,
        record: Option<Arc<String>>,
        tick: u64,
    },
    /// The battle image's parts changed since the previous `Battle` or
    /// `BattlePatch` of this dimension, in the battle module's codec. Replay
    /// refuses one without a `Battle` earlier in the WAL tail. Appended to
    /// preserve variant order; mixed-version WALs are unsupported.
    BattlePatch {
        value: serde_json::Value,
    },
    /// Compact post-image of one live body whose appearance changed: the parts
    /// of its last `Entity` image that differ, instead of the whole image.
    /// Replay refuses one without an earlier `Entity` record for the id in the
    /// same tail: the changed appearance parts, the age and the scalars the
    /// image would have set, but no state or villager image. Replay lands the
    /// clock projection the image would have baked in, then moves the anchor.
    /// Appended to preserve variant order.
    EntityAppearance {
        id: i32,
        tick: u64,
        age: u32,
        health: f32,
        alive: bool,
        #[serde(default)]
        active_effects: crate::effects::ActiveEffects,
        appearance: EntityAppearanceDelta,
    },
    /// Compact post-image of one projectile flight tick: the authoritative pose
    /// and the projectile state delta, instead of the whole entity image.
    /// `state` carries the whole state when any field but the age and the
    /// landing transition changed. Appended to preserve variant order.
    ProjectileFlight {
        id: i32,
        tick: u64,
        age: u32,
        health: f32,
        alive: bool,
        #[serde(default)]
        active_effects: crate::effects::ActiveEffects,
        appearance: EntityAppearanceDelta,
        pos: [f64; 3],
        vel: [f64; 3],
        yaw: f32,
        pitch: f32,
        /// Absolute `ProjectileState::age` when `state` is absent.
        projectile_age: u32,
        /// Landing transition, only when it changed this tick.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        stuck: Option<bool>,
        /// Ground clock: unchanged, cleared when the arrow leaves the ground, or
        /// set on landing.
        #[serde(default, skip_serializing_if = "DeltaValue::is_unchanged")]
        grounded_at_age: DeltaValue<u32>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "deserialize_optional_projectile_image"
        )]
        state: Option<Box<crate::projectiles::ProjectileState>>,
    },
}

fn deserialize_statistics_changes<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Vec<(u32, u32, i32)>, D::Error> {
    use mc_data::statistics::{CustomStat, StatType};
    let changes = Vec::<(u32, u32, i32)>::deserialize(decoder)?;
    if changes.iter().any(|&(kind, id, _)| {
        StatType::from_id(kind).is_none()
            || kind == StatType::Custom.id() && CustomStat::from_id(id).is_none()
            || id > i32::MAX as u32
    }) {
        return Err(serde::de::Error::custom("invalid statistics registry id"));
    }
    Ok(changes)
}

fn deserialize_optional_villager_image<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Box<crate::villager::VillagerState>>, D::Error> {
    crate::villager::persistence::deserialize_optional_state(decoder)
        .map(|state| state.map(Box::new))
}

fn deserialize_optional_projectile_image<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<Box<crate::projectiles::ProjectileState>>, D::Error> {
    crate::projectiles::persistence::deserialize_optional_state(decoder)
        .map(|state| state.map(Box::new))
}

/// Fsync one file path (file must exist).
pub fn fsync_file(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

/// Fsync a directory entry. On Linux opening a directory as a `File` and
/// syncing it fsyncs the directory entry; elsewhere this is a no-op.
pub fn fsync_dir(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

/// Append batches to the journal on the journal thread. Every batch file is
/// fsynced when written; the directory every `fsync_every` appends (call
/// [`JournalWriter::flush`] to force).
pub struct JournalWriter {
    dir: PathBuf,
    /// Store and palette every batch of this writer is bound to.
    identity: WalIdentity,
    pending_fsync: u32,
    fsync_every: u32,
    /// Ticks `lo..=hi` appended contiguously since open: the WAL gate
    /// checks coverage here instead of re-reading batch files.
    appended: Option<(u64, u64)>,
    /// Immutable pre-open coverage `(checkpoint_tick, recovered_tick]`, decoded
    /// either by startup recovery or the first checkpoint gate.
    pre_open_verified: Option<(u64, u64)>,
}

/// Test-only rendezvous inside the unpublished WAL write, before file fsync.
#[cfg(test)]
type WalWriteObserver = (PathBuf, Box<dyn Fn(&mut File, &[u8]) + Send>);
#[cfg(test)]
pub(crate) static WAL_WRITE_OBSERVER: parking_lot::Mutex<Option<WalWriteObserver>> =
    parking_lot::const_mutex(None);

#[cfg(test)]
thread_local! {
    /// One-shot partial write followed by ENOSPC on this writer thread only.
    pub(crate) static WAL_WRITE_FAILURE: std::cell::Cell<Option<usize>> =
        const { std::cell::Cell::new(None) };
    pub(crate) static WAL_CRASH_STAGE: std::cell::Cell<Option<&'static str>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn wal_stage(stage: &str) {
    if WAL_CRASH_STAGE.get().is_some_and(|stop| stop == stage) {
        std::process::exit(39);
    }
}

/// Why a checkpoint's WAL gate refused.
///
/// The two kinds have different consequences: a checkpoint asking past the
/// writer's known append coverage refuses this one checkpoint and keeps serving
/// (the appended batches stay durable and pruning removes nothing), while a
/// filesystem failure means the journal can no longer promise durability and
/// the caller must stop the world instead of retrying the fsync (CS341/Books
/// review F1). A hole or damage that tail verification finds on disk is `Io`.
#[derive(Debug)]
pub enum GateError {
    /// The WAL does not cover `(prev_tick, through]`; nothing was written.
    Hole(CoreError),
    /// `flush`'s directory fsync or the tail verification failed.
    Io(CoreError),
}

impl GateError {
    /// The underlying error, for callers that treat both kinds alike.
    pub fn into_core(self) -> CoreError {
        match self {
            Self::Hole(error) | Self::Io(error) => error,
        }
    }
}

impl From<GateError> for CoreError {
    fn from(error: GateError) -> Self {
        error.into_core()
    }
}

impl JournalWriter {
    /// Open the journal of the world at `world_dir` for a writer whose batches
    /// are bound to `identity`: the opened store's id and the block palette
    /// whose numeric state ids the entries carry.
    pub fn open(
        world_dir: &Path,
        fsync_every: u32,
        identity: WalIdentity,
    ) -> Result<Self, CoreError> {
        let dir = world_dir.join("journal");
        // R4-P2: the shared helper persists every directory entry it creates,
        // including a journal directory left unflushed by an interrupted first
        // start, so no batch rename can claim durability inside a directory a
        // later recovery cannot find.
        mc_world::atomic_file::create_dir_all_durable(&dir)
            .map_err(|err| CoreError::Storage(err.to_string()))?;
        // Safe here and only here: the caller holds the exclusive world lease
        // and no append exists yet, so every `*.bin.tmp` is a leftover of a
        // failed or interrupted append. Pruning must not sweep them, because a
        // live append writes its tmp between create and rename.
        let sweep = sweep_unpublished_batches(&dir);
        if sweep.removed_files > 0 {
            tracing::warn!(
                dir = %dir.display(),
                files = sweep.removed_files,
                bytes = sweep.removed_bytes,
                "removed unpublished journal batches left by a failed or interrupted append"
            );
        }
        if sweep.failed > 0 {
            tracing::warn!(
                dir = %dir.display(),
                failed = sweep.failed,
                error = sweep.first_error.as_deref().unwrap_or("unknown"),
                "unpublished journal batches could not be removed; they stay on disk"
            );
        }
        Ok(Self {
            dir,
            identity,
            pending_fsync: 0,
            fsync_every: fsync_every.max(1),
            appended: None,
            pre_open_verified: None,
        })
    }

    /// Reuse the contiguous tail already validated while holding the world lease.
    /// Published batches are immutable; the checkpoint gate still fsyncs the
    /// directory and verifies every tick appended by this writer.
    pub(crate) fn open_recovered(
        world_dir: &Path,
        fsync_every: u32,
        checkpoint_tick: u64,
        recovered_tick: u64,
        identity: WalIdentity,
    ) -> Result<Self, CoreError> {
        let mut writer = Self::open(world_dir, fsync_every, identity)?;
        writer.pre_open_verified = Some((checkpoint_tick, recovered_tick));
        Ok(writer)
    }

    /// The identity this writer stamps on (and verifies against) every batch.
    #[must_use]
    pub fn identity(&self) -> WalIdentity {
        self.identity
    }

    /// Write-ahead gate for checkpoint `through`: every tick in
    /// `(prev_tick, through]` is in a durable batch. Appends since open
    /// are contiguous (an append failure ends the journal), so coverage is
    /// tracked in process; batches from before open are verified on disk
    /// once. Batch files are fsynced at append, so only the directory
    /// needs a final fsync. A coverage [hole](GateError::Hole) refuses this
    /// checkpoint only; an [I/O failure](GateError::Io) must stop the world.
    pub fn gate(&mut self, prev_tick: u64, through: u64) -> Result<(), GateError> {
        // Recovery may already cover the whole save without a new tick/append.
        if through == prev_tick
            || self
                .pre_open_verified
                .is_some_and(|(from, hi)| from <= prev_tick && hi >= through)
        {
            return self.flush().map_err(GateError::Io);
        }
        let hole = |from: u64| {
            Err(GateError::Hole(CoreError::Storage(format!(
                "journal hole: ticks {from}..={through} have no batch"
            ))))
        };
        let first_appended = match self.appended {
            Some((lo, hi)) if hi >= through => lo,
            Some((_, hi)) => return hole(hi + 1),
            None => return hole(prev_tick + 1),
        };
        if first_appended > prev_tick + 1
            && !self
                .pre_open_verified
                .is_some_and(|(from, through)| from <= prev_tick && through >= first_appended - 1)
        {
            verify_batches_through(&self.dir, prev_tick, first_appended - 1, self.identity)
                .map_err(GateError::Io)?;
            self.pre_open_verified = Some((prev_tick, first_appended - 1));
        }
        self.flush().map_err(GateError::Io)
    }

    /// Append one batch covering ticks `tick_lo..=tick_hi`.
    pub fn append(
        &mut self,
        tick_lo: u64,
        tick_hi: u64,
        entries: &[JournalEntry],
    ) -> Result<(), CoreError> {
        if let Some((_, hi)) = self.appended
            && tick_lo != hi + 1
        {
            return Err(CoreError::Storage(format!(
                "journal append {tick_lo} does not follow {hi}"
            )));
        }
        // Test hook (unit tests only): fail EVERY append for the armed
        // world dir while armed — per-dir so parallel tests never poison
        // each other's journals (process-global hook, same pattern as
        // FAIL_CHECKPOINT_AFTER_REGIONS).
        #[cfg(test)]
        if crate::core::FAIL_JOURNAL_APPEND
            .lock()
            .ok()
            .and_then(|armed| armed.clone())
            .is_some_and(|dir| *dir == self.dir.parent().unwrap_or(&self.dir).to_path_buf())
        {
            return Err(CoreError::Storage(
                "injected journal append failure".to_owned(),
            ));
        }
        let mut payload = String::new();
        for entry in entries {
            if let JournalEntry::Entity {
                villager: Some(state),
                ..
            } = entry
            {
                crate::villager::persistence::validate_state(state)
                    .map_err(|error| CoreError::Storage(error.to_string()))?;
            }
            if let JournalEntry::Entity {
                projectile: Some(state),
                pos,
                vel,
                yaw,
                pitch,
                ..
            } = entry
            {
                crate::projectiles::persistence::validate_state(state)
                    .map_err(|error| CoreError::Storage(error.to_owned()))?;
                if pos.iter().chain(vel.iter()).any(|value| !value.is_finite())
                    || !yaw.is_finite()
                    || !pitch.is_finite()
                {
                    return Err(CoreError::Storage("nonfinite projectile motion".into()));
                }
            }
            match entry {
                JournalEntry::EntityAppearance { health, .. } => {
                    if !health.is_finite() {
                        return Err(CoreError::Storage("nonfinite entity health".into()));
                    }
                }
                JournalEntry::ProjectileFlight {
                    health,
                    pos,
                    vel,
                    yaw,
                    pitch,
                    state,
                    ..
                } => {
                    if !health.is_finite()
                        || pos.iter().chain(vel.iter()).any(|value| !value.is_finite())
                        || !yaw.is_finite()
                        || !pitch.is_finite()
                    {
                        return Err(CoreError::Storage(
                            "nonfinite projectile flight post-image".into(),
                        ));
                    }
                    if let Some(state) = state {
                        crate::projectiles::persistence::validate_state(state)
                            .map_err(|error| CoreError::Storage(error.to_owned()))?;
                    }
                }
                _ => {}
            }
            payload.push_str(
                &serde_json::to_string(entry).map_err(|err| CoreError::Storage(err.to_string()))?,
            );
            payload.push('\n');
        }
        let frame = frame_batch(tick_lo, tick_hi, payload.as_bytes(), self.identity)?;
        let name = format!("batch-{tick_lo:020}-{tick_hi:020}.bin");
        let tmp = self.dir.join(format!("{name}.tmp"));
        let final_path = self.dir.join(&name);
        {
            let mut file = File::create(&tmp).map_err(|err| CoreError::Storage(err.to_string()))?;
            #[cfg(test)]
            wal_stage("created");
            #[cfg(test)]
            if let Some((_, observe)) = WAL_WRITE_OBSERVER
                .lock()
                .as_ref()
                .filter(|(dir, _)| dir == &self.dir)
            {
                observe(&mut file, &frame);
            }
            #[cfg(test)]
            if let Some(offset) = WAL_WRITE_FAILURE.take() {
                file.write_all(&frame[..offset.min(frame.len())])
                    .map_err(|err| CoreError::Storage(err.to_string()))?;
                return Err(CoreError::Storage(
                    std::io::Error::from_raw_os_error(28).to_string(),
                ));
            }
            file.write_all(&frame)
                .map_err(|err| CoreError::Storage(err.to_string()))?;
            #[cfg(test)]
            wal_stage("written");
            file.sync_all()
                .map_err(|err| CoreError::Storage(err.to_string()))?;
            #[cfg(test)]
            wal_stage("synced");
        }
        std::fs::rename(&tmp, &final_path).map_err(|err| CoreError::Storage(err.to_string()))?;
        #[cfg(test)]
        wal_stage("published");
        self.appended = Some((self.appended.map_or(tick_lo, |(lo, _)| lo), tick_hi));
        self.pending_fsync += 1;
        if self.pending_fsync >= self.fsync_every {
            self.flush()?;
            #[cfg(test)]
            wal_stage("durable");
        }
        Ok(())
    }

    /// Fsync the journal directory so all appended batches are durable.
    pub fn flush(&mut self) -> Result<(), CoreError> {
        // Test hook (loop-level tests only): for the armed world dir, let the
        // first `skip` flushes through and then fail every flush — per-dir so
        // parallel tests never poison each other's journals (same pattern as
        // `FAIL_JOURNAL_APPEND`).
        #[cfg(test)]
        if let Ok(mut armed) = crate::core::FAIL_JOURNAL_FLUSH.lock()
            && let Some((dir, skip)) = armed.as_mut()
            && *dir == self.dir.parent().unwrap_or(&self.dir).to_path_buf()
        {
            if *skip == 0 {
                return Err(CoreError::Storage(
                    "injected journal flush failure".to_owned(),
                ));
            }
            *skip -= 1;
        }
        fsync_dir(&self.dir).map_err(|err| CoreError::Storage(err.to_string()))?;
        self.pending_fsync = 0;
        Ok(())
    }
}

impl JournalWriter {
    /// Highest tick durable in the WAL from this writer's point of view:
    /// appended since open or verified at startup. `None` before either.
    pub(crate) fn covered_through(&self) -> Option<u64> {
        self.appended
            .map(|(_, hi)| hi)
            .into_iter()
            .chain(self.pre_open_verified.map(|(_, hi)| hi))
            .max()
    }
}

/// Parse `batch-<lo>-<hi>.bin`.
pub(crate) fn parse_batch_name(name: &str) -> Option<(u64, u64)> {
    let rest = name.strip_prefix("batch-")?.strip_suffix(".bin")?;
    let (lo, hi) = rest.split_once('-')?;
    Some((lo.parse().ok()?, hi.parse().ok()?))
}

/// Read complete published batches after the manifest, failing closed on damage
/// and on a batch that does not belong to `identity`.
/// Incomplete, unpublished `.tmp` files are ignored.
pub fn read_complete_batches_after(
    journal_dir: &Path,
    manifest_tick: u64,
    identity: WalIdentity,
) -> Result<Vec<(u64, u64, Vec<JournalEntry>)>, CoreError> {
    let mut out = Vec::new();
    visit_complete_batches_after(journal_dir, manifest_tick, identity, |batch| {
        out.push(batch);
        Ok(())
    })?;
    Ok(out)
}

/// Decode and consume one published batch at a time. Recovery memory is bounded
/// by one batch and the latest recovered images, never the complete WAL history.
/// A batch whose store or palette does not match `identity` is refused by name
/// (Books R4-P3, R5-G5), as is a format-1 frame.
pub fn visit_complete_batches_after(
    journal_dir: &Path,
    manifest_tick: u64,
    identity: WalIdentity,
    mut visit: impl FnMut((u64, u64, Vec<JournalEntry>)) -> Result<(), CoreError>,
) -> Result<u64, CoreError> {
    let entries = match std::fs::read_dir(journal_dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(manifest_tick),
        Err(err) => return Err(CoreError::Storage(err.to_string())),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| CoreError::Storage(err.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some((lo, hi)) = parse_batch_name(&name) {
            names.push((lo, hi, name));
        }
    }
    names.sort();
    let mut previous_tick = manifest_tick;
    for (lo, hi, name) in &names {
        if *hi <= manifest_tick {
            continue;
        }
        let path = journal_dir.join(name);
        let expected = previous_tick.checked_add(1).ok_or_else(|| {
            CoreError::Storage(format!("journal {}: tick range overflow", path.display()))
        })?;
        if *lo > expected {
            return Err(CoreError::Storage(format!(
                "journal {}: journal hole: ticks {expected}..={} have no batch",
                path.display(),
                lo - 1,
            )));
        }
        if *lo != expected {
            return Err(CoreError::Storage(format!(
                "journal {}: noncontiguous batch starts at {lo}, expected {expected}",
                path.display(),
            )));
        }
        let bytes = std::fs::read(&path)
            .map_err(|err| CoreError::Storage(format!("journal {}: {err}", path.display())))?;
        let entries = decode_batch(&bytes, *lo, *hi, identity).map_err(|err| err.at_path(&path))?;
        drop(bytes);
        visit((*lo, *hi, entries))?;
        previous_tick = *hi;
    }
    Ok(previous_tick)
}
/// Verify the WAL covers `(prev_tick, through]` contiguously: every tick in
/// the range must sit inside some complete batch file. A missing batch (a
/// failed append the tick thread never retried) is a hole no fsync can
/// fill — the caller must refuse the checkpoint ack. Torn tails fail too.
pub fn verify_batches_through(
    journal_dir: &Path,
    prev_tick: u64,
    through: u64,
    identity: WalIdentity,
) -> Result<(), CoreError> {
    let mut intervals: Vec<(u64, u64)> = Vec::new();
    let entries =
        std::fs::read_dir(journal_dir).map_err(|err| CoreError::Storage(err.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|err| CoreError::Storage(err.to_string()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("batch-") && name.ends_with(".bin")) {
            continue;
        }
        let Some((lo, hi)) = parse_batch_name(&name) else {
            continue;
        };
        if hi <= prev_tick || lo > through {
            continue;
        }
        let bytes = std::fs::read(journal_dir.join(&name))
            .map_err(|err| CoreError::Storage(err.to_string()))?;
        decode_batch(&bytes, lo, hi, identity)
            .map_err(|err| err.at_path(&journal_dir.join(&name)))?;
        intervals.push((lo, hi));
    }
    intervals.sort();
    let mut covered = prev_tick;
    for (lo, hi) in intervals {
        if lo > covered + 1 {
            return Err(CoreError::Storage(format!(
                "journal hole: ticks {}..={} have no batch",
                covered + 1,
                lo - 1
            )));
        }
        covered = covered.max(hi);
        if covered >= through {
            break;
        }
    }
    if covered < through {
        return Err(CoreError::Storage(format!(
            "journal hole: ticks {}..={through} have no batch",
            covered + 1
        )));
    }
    Ok(())
}
#[derive(Debug)]
enum BatchDecodeError {
    Framing(String),
    Record {
        index: usize,
        offset: usize,
        kind: Option<String>,
        source: serde_json::Error,
    },
}

impl BatchDecodeError {
    fn at_path(self, path: &Path) -> CoreError {
        let detail = match self {
            Self::Framing(reason) => {
                format!("record 0 offset 0: {reason}")
            }
            Self::Record {
                index,
                offset,
                kind,
                source,
            } => format!(
                "record {index} offset {offset} kind {}: {source}",
                kind.as_deref().unwrap_or("unknown")
            ),
        };
        CoreError::Storage(format!("journal {}: {detail}", path.display()))
    }
}

/// Every frame records the store id and palette that wrote it: a batch that
/// belongs to another world, or to another block-state mapping, is refused by
/// name instead of replayed over this store's checkpoint (Books R4-P3, R5-G5).
/// Both bindings are compared unconditionally: a zero field is a value like any
/// other, not a wildcard, and the CRC covers only the payload, so accepting a
/// zero-side match would let a rewritten header through. A test that opens a
/// world passes that world's `WalIdentity` on both sides; `WalIdentity::fixture`
/// matches only itself.
fn verify_bindings(
    store: [u8; 16],
    palette: [u8; 8],
    identity: WalIdentity,
) -> Result<(), BatchDecodeError> {
    if store != identity.store {
        return Err(BatchDecodeError::Framing(format!(
            "batch belongs to store {} but the opened store is {}; a journal from another world \
             is never replayed (restore its own journal with it)",
            uuid::Uuid::from_bytes(store),
            uuid::Uuid::from_bytes(identity.store),
        )));
    }
    if palette != identity.palette {
        return Err(BatchDecodeError::Framing(format!(
            "batch was written under block palette {} but this build registers {}; block-state \
             ids would remap, so the tail is refused",
            identity_hex(&palette),
            identity_hex(&identity.palette),
        )));
    }
    Ok(())
}

fn decode_batch(
    bytes: &[u8],
    expect_lo: u64,
    expect_hi: u64,
    identity: WalIdentity,
) -> Result<Vec<JournalEntry>, BatchDecodeError> {
    if bytes.len() < 4 + 4 {
        return Err(BatchDecodeError::Framing("incomplete batch header".into()));
    }
    let magic = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
    if magic != BATCH_MAGIC {
        return Err(BatchDecodeError::Framing(format!(
            "invalid header: magic {magic:#x}, expected {BATCH_MAGIC:#x}"
        )));
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
    if version == BATCH_VERSION_V1 {
        return Err(BatchDecodeError::Framing(
            "batch format 1 carries no store identity and this build reads format 2; start the \
             world once with the build that wrote it, let it save and stop, and confirm its \
             final checkpoint covers the last published batch before starting this build \
             (docs/OPERATING.md, upgrading a world). A batch below a covering checkpoint is \
             never replayed and needs no removal"
                .into(),
        ));
    }
    if version != BATCH_VERSION {
        return Err(BatchDecodeError::Framing(format!(
            "invalid header: version {version}, expected {BATCH_VERSION}"
        )));
    }
    if bytes.len() < HEADER_LEN {
        return Err(BatchDecodeError::Framing("incomplete batch header".into()));
    }
    verify_bindings(
        bytes[8..24].try_into().unwrap(),
        bytes[24..32].try_into().unwrap(),
        identity,
    )?;
    let lo = u64::from_le_bytes(bytes[32..40].try_into().unwrap());
    let hi = u64::from_le_bytes(bytes[40..48].try_into().unwrap());
    let len = u32::from_le_bytes(bytes[48..52].try_into().unwrap()) as usize;
    let crc = u32::from_le_bytes(bytes[52..56].try_into().unwrap());
    if lo != expect_lo || hi != expect_hi {
        return Err(BatchDecodeError::Framing(format!(
            "invalid header: magic {magic:#x}, version {version}, ticks {lo}..={hi}, expected {expect_lo}..={expect_hi}"
        )));
    }
    let actual_len = bytes.len() - HEADER_LEN;
    if actual_len < len {
        return Err(BatchDecodeError::Framing(format!(
            "incomplete payload: expected {len} bytes, found {actual_len}"
        )));
    }
    if actual_len > len {
        return Err(BatchDecodeError::Framing(format!(
            "trailing bytes: expected {len} payload bytes, found {actual_len}"
        )));
    }
    let payload = &bytes[HEADER_LEN..];
    let actual_crc = crc32fast::hash(payload);
    if actual_crc != crc {
        return Err(BatchDecodeError::Framing(format!(
            "CRC mismatch: expected {crc:#x}, found {actual_crc:#x}"
        )));
    }
    let mut entries = Vec::new();
    let mut offset = HEADER_LEN;
    for (index, line) in payload.split(|byte| *byte == b'\n').enumerate() {
        if !line.is_empty() {
            let entry = serde_json::from_slice(line).map_err(|source| {
                let kind = serde_json::from_slice::<serde_json::Value>(line)
                    .ok()
                    .and_then(|value| match value {
                        serde_json::Value::Object(object) if object.len() == 1 => {
                            object.into_iter().next().map(|(kind, _)| kind)
                        }
                        serde_json::Value::String(kind) => Some(kind),
                        _ => None,
                    })
                    .or_else(|| {
                        // An incomplete object may still expose its enum tag.
                        let colon = line.iter().position(|byte| *byte == b':')?;
                        let key = std::str::from_utf8(&line[..colon]).ok()?.trim();
                        serde_json::from_str::<String>(key.strip_prefix('{')?.trim()).ok()
                    });
                BatchDecodeError::Record {
                    index,
                    offset,
                    kind,
                    source,
                }
            })?;
            entries.push(entry);
        }
        offset += line.len() + 1;
    }
    Ok(entries)
}

/// Absolute counter at an interval boundary. Active intervals advance through
/// the recovered WAL tick, not wall time or the next player's join tick.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct InhabitedTimeImage {
    pub chunk: [i32; 2],
    pub value: i64,
    pub tick: u64,
    pub active: bool,
}

impl InhabitedTimeImage {
    pub(crate) fn value_at(self, tick: u64) -> i64 {
        let elapsed = if self.active {
            tick.saturating_sub(self.tick)
        } else {
            0
        };
        self.value
            .saturating_add(i64::try_from(elapsed).unwrap_or(i64::MAX))
    }

    pub fn apply(self, chunk: &mut mc_world::Chunk, tick: u64) {
        let value = self.value_at(tick);
        chunk.extras.retain(|(key, _)| key != "InhabitedTime");
        chunk
            .extras
            .push(("InhabitedTime".into(), mc_nbt::Tag::Long(value)));
    }
}

/// 001 slice0 active-only per-chunk debt clock (001-d/e).
/// Absolute catch-up debt at a WAL/checkpoint tick. `last_sim` advances only
/// while the chunk is active+resident; `remaining` holds pre-existing debt
/// consumed on activation (scope0: zero effects, durable zero image).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct SimDebtImage {
    pub chunk: [i32; 2],
    pub last_sim: u64,
    pub remaining: u64,
    pub tick: u64,
    /// Open interval at this image's tick mirrors
    /// [`InhabitedTimeImage::active`]: the recovered batch clock supplies
    /// elapsed ticks (`last_sim_at`), never wall time or re-accrual.
    pub active: bool,
}

impl SimDebtImage {
    pub fn last_sim_at(self, tick: u64) -> u64 {
        if self.active {
            self.last_sim.max(tick)
        } else {
            self.last_sim
        }
    }
}

/// Native checkpoint image and durable player record ticks.
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct CheckpointManifest {
    pub tick: u64,
    /// Required native checkpoint subdomain schema versions.
    pub domain_versions: BTreeMap<String, u32>,
    #[serde(default)]
    pub dimensions: BTreeMap<crate::dimensions::Dimension, Box<CheckpointManifest>>,
    #[serde(default)]
    pub end_fight: Option<crate::end_fight::Fight>,
    #[serde(default)]
    pub raids: Option<crate::raids::Raids>,
    #[serde(default)]
    pub wandering_trader: Option<crate::mob_lanes::TraderSpawner>,
    #[serde(default)]
    pub scheduled_functions: Vec<(u64, String)>,
    #[serde(default)]
    pub datapack_selection: Option<crate::datapacks::Selection>,
    /// Durable ticket post-images by owner: centre, (capture tick, radius).
    /// Portal transfer tickets are transient and excluded from checkpoint captures.
    #[serde(default)]
    pub tickets: BTreeMap<String, ([i32; 2], (u64, i32))>,
    /// Player UUID -> durable native record tick, including offline players.
    #[serde(default)]
    pub players: std::collections::BTreeMap<String, u64>,
    /// Generated world spawn block coordinates; absent only in old manifests.
    #[serde(default)]
    pub world_spawn: Option<[i32; 3]>,
    #[serde(default)]
    pub world_spawn_rotation: [f32; 2],
    #[serde(default)]
    pub generated_entity_claims: std::collections::BTreeSet<String>,
    /// Captured together with the checkpoint tick; never rebuilt from it.
    #[serde(default)]
    pub climate: Option<crate::weather::ClimateImage>,
    #[serde(default)]
    pub border: crate::command_border::Border,
    #[serde(default)]
    pub difficulty: crate::survival::Difficulty,
    /// Vanilla `level.dat` `Data.GameType`: the mode a new player joins in.
    #[serde(default)]
    pub default_game_mode: crate::commands::GameMode,
    /// Absolute deadlines keep the scheduler clock stable across restart.
    #[serde(default)]
    pub block_ticks: crate::block_ticks::BlockTicksImage,
    #[serde(default)]
    pub fluid_ticks: crate::fluids::FluidTicksImage,
    /// Open inhabited-time intervals at the same tick as the chunk capture.
    #[serde(default)]
    pub inhabited_time: Vec<InhabitedTimeImage>,
    /// 001 slice0 per-chunk debt images at the checkpoint tick; only chunks
    /// with pending (dirty) or nonzero debt are listed.
    #[serde(default)]
    pub debt: Vec<SimDebtImage>,
    #[serde(default)]
    pub gamerules: BTreeMap<String, String>,
}

/// Frame one batch payload:
/// `[magic][version][tick_lo][tick_hi][len][crc32][payload]`.
///
/// [`JournalWriter::append`] is the only production writer; fixtures (the hot
/// backup tests) use this instead of hand-rolling bytes that would not decode.
pub(crate) fn frame_batch(
    tick_lo: u64,
    tick_hi: u64,
    payload: &[u8],
    identity: WalIdentity,
) -> Result<Vec<u8>, CoreError> {
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&BATCH_MAGIC.to_le_bytes());
    frame.extend_from_slice(&BATCH_VERSION.to_le_bytes());
    frame.extend_from_slice(&identity.store);
    frame.extend_from_slice(&identity.palette);
    frame.extend_from_slice(&tick_lo.to_le_bytes());
    frame.extend_from_slice(&tick_hi.to_le_bytes());
    frame.extend_from_slice(
        &u32::try_from(payload.len())
            .map_err(|_| CoreError::Storage("batch too large".to_owned()))?
            .to_le_bytes(),
    );
    frame.extend_from_slice(&crc32fast::hash(payload).to_le_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

/// Unpublished batch file name (`batch-<lo>-<hi>.bin.tmp`).
fn is_unpublished_batch(name: &str) -> bool {
    name.strip_suffix(".tmp")
        .is_some_and(|name| parse_batch_name(name).is_some())
}

/// What one [`sweep_unpublished_batches`] attempt did, including the failures it
/// must not hide: an unremovable leftover is an operator warning, not a reason to
/// refuse startup, and it must never be attempted again from the prune path.
#[derive(Debug, Default, Clone)]
pub(crate) struct SweepOutcome {
    pub removed_files: u64,
    pub removed_bytes: u64,
    /// Failures: one per directory read/iteration error and per removal that failed.
    pub failed: u64,
    /// First failure, already carrying the path and the reason.
    pub first_error: Option<String>,
}

impl SweepOutcome {
    fn fail(&mut self, path: &Path, error: impl std::fmt::Display) {
        self.failed += 1;
        if self.first_error.is_none() {
            self.first_error = Some(format!("{}: {error}", path.display()));
        }
    }
}

/// Remove `batch-*.bin.tmp` leftovers of failed or interrupted appends and report
/// what happened. Nothing else in the directory is touched: published batches are
/// immutable and unpublished readers ignore `.tmp`.
pub(crate) fn sweep_unpublished_batches(dir: &Path) -> SweepOutcome {
    let mut outcome = SweepOutcome::default();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) => {
            outcome.fail(dir, error);
            return outcome;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                outcome.fail(dir, error);
                continue;
            }
        };
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_unpublished_batch(name) {
            continue;
        }
        let path = dir.join(name);
        let len = entry.metadata().map(|meta| meta.len()).unwrap_or(0);
        match std::fs::remove_file(&path) {
            Ok(()) => {
                outcome.removed_files += 1;
                outcome.removed_bytes += len;
            }
            Err(error) => outcome.fail(&path, error),
        }
    }
    outcome
}

/// Outcome of one [`prune_batches_through`] attempt.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PruneReport {
    /// Published batches removed by this attempt.
    pub removed: u64,
    /// Failures in this attempt: an unreadable directory counts once, and so
    /// does every batch that could not be removed.
    pub failed: u64,
    /// Bytes of covered batches that stayed on disk because removal failed.
    pub retained_bytes: u64,
    /// First failure of this attempt, for the log line.
    pub error: Option<String>,
}

/// Process-wide prune counters. One process serves one world, as with the
/// component-host counters the dashboard already reads on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct JournalPruneStats {
    /// Failed removals (plus one per unreadable journal directory) since start.
    pub failures_total: u64,
    /// Bytes the latest attempt could not remove; zero when it succeeded.
    pub retained_bytes: u64,
}

static PRUNE_FAILURES: AtomicU64 = AtomicU64::new(0);
static PRUNE_RETAINED_BYTES: AtomicU64 = AtomicU64::new(0);

/// Read the process-wide prune counters; cheap, no allocation.
#[must_use]
pub fn journal_prune_stats() -> JournalPruneStats {
    JournalPruneStats {
        failures_total: PRUNE_FAILURES.load(Ordering::Relaxed),
        retained_bytes: PRUNE_RETAINED_BYTES.load(Ordering::Relaxed),
    }
}

/// Record one failed attempt: counted for the dashboard and warned once, so a
/// read-only or EIO journal directory cannot grow the WAL silently.
fn record_prune_failure(journal_dir: &Path, report: &PruneReport) {
    PRUNE_FAILURES.fetch_add(report.failed, Ordering::Relaxed);
    PRUNE_RETAINED_BYTES.store(report.retained_bytes, Ordering::Relaxed);
    tracing::warn!(
        dir = %journal_dir.display(),
        failed = report.failed,
        retained_bytes = report.retained_bytes,
        error = report.error.as_deref().unwrap_or("unknown"),
        "journal prune failed; covered batches stay on disk and the WAL grows"
    );
}

/// Prune batches fully at or below `manifest_tick`.
///
/// A failure here is warned and counted, never fatal: batches at or below the
/// committed checkpoint are ignored on replay, so only the WAL grows. A
/// directory that cannot be written at all still fails closed on the next
/// append (`journal append failed; world halting`), which is the fatal path.
pub fn prune_batches_through(journal_dir: &Path, manifest_tick: u64) -> PruneReport {
    let mut report = PruneReport::default();
    let entries = match std::fs::read_dir(journal_dir) {
        Ok(entries) => entries,
        Err(error) => {
            report.failed += 1;
            report.error = Some(error.to_string());
            record_prune_failure(journal_dir, &report);
            return report;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.failed += 1;
                report.error.get_or_insert_with(|| error.to_string());
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((_, hi)) = parse_batch_name(&name) else {
            continue;
        };
        if hi > manifest_tick {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => report.removed += 1,
            Err(error) => {
                report.failed += 1;
                report.retained_bytes += entry.metadata().map(|meta| meta.len()).unwrap_or(0);
                report
                    .error
                    .get_or_insert_with(|| format!("{}: {error}", entry.path().display()));
            }
        }
    }
    if report.failed == 0 {
        PRUNE_RETAINED_BYTES.store(0, Ordering::Relaxed);
    } else {
        record_prune_failure(journal_dir, &report);
    }
    report
}

/// Decode one published batch file and return its tick range and entry count.
///
/// Recovery already refuses a damaged batch; the hot backup calls this on every
/// batch it copies so a backup is not merely byte-identical to a **damaged**
/// source (Books R5 G3). Memory is one batch, and the WAL tail since the last
/// checkpoint bounds how many calls a backup makes.
pub fn verify_batch_file(
    path: &Path,
    identity: WalIdentity,
) -> Result<(u64, u64, usize), CoreError> {
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    let (lo, hi) = parse_batch_name(name).ok_or_else(|| {
        CoreError::Storage(format!("journal {}: not a batch file name", path.display()))
    })?;
    let bytes = std::fs::read(path)
        .map_err(|error| CoreError::Storage(format!("journal {}: {error}", path.display())))?;
    let entries = decode_batch(&bytes, lo, hi, identity).map_err(|error| error.at_path(path))?;
    Ok((lo, hi, entries.len()))
}

/// Compact player post-image shared by the journal and checkpoints.
pub type PlayerImage = ([f64; 3], f32, f32, f32, u8, u64);

/// Furnace progress image for block-entity post-images.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Default)]
pub struct FurnaceImage {
    pub lit_remaining: i32,
    pub lit_total: i32,
    pub cook_progress: i32,
    pub cook_total: i32,
    pub recipes_used: Arc<BTreeMap<String, i32>>,
}

/// Replayed full player record.
#[derive(Debug, Clone)]
pub struct ReplayPlayer {
    /// Unknown player root fields, encoded as a named NBT compound.
    pub opaque_player_fields: Vec<u8>,
    pub statistics: mc_data::statistics::PlayerStatistics,
    /// Exact vanilla advancements document, including unknown keys.
    pub advancement_progress: Arc<serde_json::Value>,
    pub unlocked_recipes: Arc<BTreeSet<String>>,
    pub dimension: crate::dimensions::Dimension,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub food: i32,
    pub saturation: f32,
    pub exhaustion: f32,
    pub air: i32,
    pub experience_level: i32,
    pub experience_progress: f32,
    pub experience_total: i32,
    pub game_mode: u8,
    pub selected: u8,
    pub inventory: Vec<mc_data::ItemStack>,
    /// Cursor stack custody (never folded away).
    pub carried: mc_data::ItemStack,
    /// Open menu crafting grid custody (never folded away).
    pub grid: Vec<mc_data::ItemStack>,
    /// Bed head or forced command-selected respawn cell.
    pub respawn: Option<[i32; 3]>,
    pub respawn_dimension: crate::dimensions::Dimension,
    pub respawn_forced: bool,
    pub respawn_yaw: f32,
    pub respawn_pitch: f32,
    pub active_effects: crate::effects::ActiveEffects,
    pub enchantment_seed: i32,
    pub enchantment_offer_seed: i32,
    pub warden_warning: crate::shrieker::Warning,
    pub current_impulse_impact_pos: Option<[f64; 3]>,
    pub fall_flying: bool,
    pub burn_ticks: u32,
    pub portal: crate::portals::PortalState,
    pub tick: u64,
}

impl PartialEq for ReplayPlayer {
    fn eq(&self, other: &Self) -> bool {
        self.eq_except_statistics(other) && self.statistics == other.statistics
    }
}

impl ReplayPlayer {
    /// Statistics have their own compact WAL path; every other durable field
    /// still selects the complete Player postimage.
    pub(crate) fn eq_except_statistics(&self, other: &Self) -> bool {
        // No `..`: adding a durable field must also update this comparison.
        let Self {
            opaque_player_fields: _,
            statistics: _,
            advancement_progress: _,
            unlocked_recipes: _,
            dimension: _,
            pos: _,
            yaw: _,
            pitch: _,
            health: _,
            food: _,
            saturation: _,
            exhaustion: _,
            air: _,
            experience_level: _,
            experience_progress: _,
            experience_total: _,
            game_mode: _,
            selected: _,
            inventory: _,
            carried: _,
            grid: _,
            respawn: _,
            respawn_dimension: _,
            respawn_forced: _,
            respawn_yaw: _,
            respawn_pitch: _,
            active_effects: _,
            enchantment_seed: _,
            enchantment_offer_seed: _,
            warden_warning: _,
            current_impulse_impact_pos: _,
            fall_flying: _,
            burn_ticks: _,
            portal: _,
            tick: _,
        } = self;
        (Arc::ptr_eq(&self.advancement_progress, &other.advancement_progress)
            || self.advancement_progress == other.advancement_progress)
            && (Arc::ptr_eq(&self.unlocked_recipes, &other.unlocked_recipes)
                || self.unlocked_recipes == other.unlocked_recipes)
            && self.opaque_player_fields == other.opaque_player_fields
            && self.dimension == other.dimension
            && self.pos == other.pos
            && self.yaw == other.yaw
            && self.pitch == other.pitch
            && self.health == other.health
            && self.food == other.food
            && self.saturation == other.saturation
            && self.exhaustion == other.exhaustion
            && self.air == other.air
            && self.experience_level == other.experience_level
            && self.experience_progress == other.experience_progress
            && self.experience_total == other.experience_total
            && self.game_mode == other.game_mode
            && self.selected == other.selected
            && self.inventory == other.inventory
            && self.carried == other.carried
            && self.grid == other.grid
            && self.respawn == other.respawn
            && self.respawn_dimension == other.respawn_dimension
            && self.respawn_forced == other.respawn_forced
            && self.respawn_yaw == other.respawn_yaw
            && self.respawn_pitch == other.respawn_pitch
            && self.active_effects == other.active_effects
            && self.enchantment_seed == other.enchantment_seed
            && self.enchantment_offer_seed == other.enchantment_offer_seed
            && self.warden_warning == other.warden_warning
            && self.current_impulse_impact_pos == other.current_impulse_impact_pos
            && self.fall_flying == other.fall_flying
            && self.burn_ticks == other.burn_ticks
            && self.portal == other.portal
            && self.tick == other.tick
    }
}

pub(crate) fn empty_advancement_progress() -> Arc<serde_json::Value> {
    Arc::new(serde_json::Value::Object(serde_json::Map::new()))
}

// A present JSON null is retained document data, not an omitted post-image.
fn deserialize_advancement_progress<'de, D>(
    deserializer: D,
) -> Result<Option<Arc<serde_json::Value>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    serde_json::Value::deserialize(deserializer).map(|value| Some(Arc::new(value)))
}
/// Replayed block-entity record.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ReplayBlockEntity {
    pub slots: Vec<mc_data::ItemStack>,
    pub furnace: Option<FurnaceImage>,
    pub tick: u64,
}

/// Journal-safe image of [`EntityAppearance`](crate::boundary::EntityAppearance):
/// all non-motion entity state (carried item, equipment, baby, pose, flags,
/// health, custom name, variant data). `ItemStack` round-trips through JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Default)]
pub struct EntityAppearanceImage {
    pub item: Option<mc_data::ItemStack>,
    #[serde(default)]
    pub experience_orb: Option<(i32, i32)>,
    #[serde(default)]
    pub spawn_data: i32,
    pub equipment: Vec<Option<mc_data::ItemStack>>,
    pub baby: bool,
    pub pose: u8,
    pub shared_flags: u8,
    pub health: f32,
    pub custom_name: Option<String>,
    #[serde(default)]
    pub custom_name_visible: bool,
    pub extra: Vec<(u8, EntityDataValueImage)>,
}

/// Journal-safe image of [`EntityDataValue`](crate::boundary::EntityDataValue).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum EntityDataValueImage {
    Byte(i8),
    VarInt(i32),
    Float(f32),
    Rotations([f32; 3]),
    VarLong(i64),
    Boolean(bool),
    String(String),
    BlockState(u32),
    WeatheringCopper(u8),
    CopperGolemState(u8),
    SnifferState(u8),
    Direction(u8),
    PaintingVariant(u32),
    Item(Option<mc_data::ItemStack>),
    OptionalBlockPos(Option<[i32; 3]>),
    EntityEffectParticle(i32),
    OptionalUnsignedInt(Option<u32>),
    OptionalLivingEntityReference(Option<uuid::Uuid>),
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

/// A field a compact appearance record leaves alone, clears or sets.
///
/// `Option<Option<T>>` cannot express this in JSON: `null` decodes to the outer
/// `None`, so a cleared field would silently keep its old value. The derived
/// enum encoding is unambiguous (`"Unchanged"`, `"Clear"`, `{"Set": value}`) and
/// is covered by tests through a real batch encode/decode.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DeltaValue<T> {
    #[default]
    Unchanged,
    Clear,
    Set(T),
}

impl<T> DeltaValue<T> {
    fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

/// The delta of one optional live field against the image the record follows.
fn delta_of<T: Clone + PartialEq>(base: &Option<T>, live: &Option<T>) -> DeltaValue<T> {
    if base == live {
        DeltaValue::Unchanged
    } else {
        match live {
            Some(value) => DeltaValue::Set(value.clone()),
            None => DeltaValue::Clear,
        }
    }
}

/// Apply one optional delta onto a replayed field.
pub(crate) fn apply_delta<T: Clone>(delta: &DeltaValue<T>, target: &mut Option<T>) {
    match delta {
        DeltaValue::Unchanged => {}
        DeltaValue::Clear => *target = None,
        DeltaValue::Set(value) => *target = Some(value.clone()),
    }
}

/// Part-delta of [`EntityAppearanceImage`] against the last image of the same
/// body emitted in the same WAL tail. An `Unchanged` field kept its image;
/// `extra` carries only the metadata entries that changed, unless the surviving
/// entry order changed (then the whole list rides in `extra`). Every field is an
/// absolute value, so folding one record is idempotent. Unknown fields refuse
/// decoding: a newer writer's field must not be dropped silently.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntityAppearanceDelta {
    #[serde(default, skip_serializing_if = "DeltaValue::is_unchanged")]
    pub item: DeltaValue<mc_data::ItemStack>,
    #[serde(default, skip_serializing_if = "DeltaValue::is_unchanged")]
    pub experience_orb: DeltaValue<(i32, i32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spawn_data: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub equipment: Option<Vec<Option<mc_data::ItemStack>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baby: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pose: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_flags: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<f32>,
    #[serde(default, skip_serializing_if = "DeltaValue::is_unchanged")]
    pub custom_name: DeltaValue<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_name_visible: Option<bool>,
    /// Whole metadata list; set only when the surviving entry order changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<Vec<(u8, EntityDataValueImage)>>,
    /// Per-index updates in change order: `Some` sets, `None` removes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra_changes: Vec<(u8, Option<EntityDataValueImage>)>,
}

impl EntityAppearanceDelta {
    /// The parts of `live` that differ from the image `base`, borrowing both
    /// native appearances: only changed payloads are cloned, and nothing is
    /// materialised for an unchanged field or an unchanged metadata entry.
    pub(crate) fn diff(
        base: &crate::boundary::EntityAppearance,
        live: &crate::boundary::EntityAppearance,
    ) -> Self {
        let mut delta = Self {
            item: delta_of(&base.item, &live.item),
            experience_orb: delta_of(&base.experience_orb, &live.experience_orb),
            spawn_data: (base.spawn_data != live.spawn_data).then_some(live.spawn_data),
            equipment: None,
            baby: (base.baby != live.baby).then_some(live.baby),
            pose: (base.pose != live.pose).then_some(live.pose),
            shared_flags: (base.shared_flags != live.shared_flags).then_some(live.shared_flags),
            health: (base.health != live.health).then_some(live.health),
            custom_name: delta_of(&base.custom_name, &live.custom_name),
            custom_name_visible: (base.custom_name_visible != live.custom_name_visible)
                .then_some(live.custom_name_visible),
            extra: None,
            extra_changes: Vec::new(),
        };
        if base.equipment != live.equipment {
            delta.equipment = Some(live.equipment.to_vec());
        }
        // Per-index updates are exact when the entries that survive keep their
        // relative order: existing indices update in place, new ones append in
        // the order they appear here. A reordered or duplicated list needs the
        // whole image instead. No temporary index vectors.
        let mut reordered = false;
        let mut previous: Option<usize> = None;
        for (index, _) in &live.extra {
            let Some(position) = base.extra.iter().position(|(other, _)| other == index) else {
                continue;
            };
            if previous.is_some_and(|previous| position <= previous) {
                reordered = true;
                break;
            }
            previous = Some(position);
        }
        let duplicated = |entries: &[(u8, crate::boundary::EntityDataValue)]| {
            entries
                .iter()
                .enumerate()
                .any(|(at, (index, _))| entries[..at].iter().any(|(other, _)| other == index))
        };
        if reordered || duplicated(&base.extra) || duplicated(&live.extra) {
            let differs = base.extra.len() != live.extra.len()
                || live
                    .extra
                    .iter()
                    .any(|(index, value)| !base_has(base, *index, value));
            if differs {
                delta.extra = Some(
                    live.extra
                        .iter()
                        .map(|(index, value)| (*index, EntityDataValueImage::from(value)))
                        .collect(),
                );
            }
            return delta;
        }
        for (index, value) in &live.extra {
            if !base_has(base, *index, value) {
                delta
                    .extra_changes
                    .push((*index, Some(EntityDataValueImage::from(value))));
            }
        }
        for (index, _) in &base.extra {
            if !live.extra.iter().any(|(other, _)| other == index) {
                delta.extra_changes.push((*index, None));
            }
        }
        delta
    }

    /// Apply onto the replayed image.
    pub(crate) fn apply(&self, image: &mut EntityAppearanceImage) {
        if !self.item.is_unchanged() {
            apply_delta(&self.item, &mut image.item);
        }
        if !self.experience_orb.is_unchanged() {
            apply_delta(&self.experience_orb, &mut image.experience_orb);
        }
        if let Some(spawn_data) = self.spawn_data {
            image.spawn_data = spawn_data;
        }
        if let Some(equipment) = &self.equipment {
            image.equipment = equipment.clone();
        }
        if let Some(baby) = self.baby {
            image.baby = baby;
        }
        if let Some(pose) = self.pose {
            image.pose = pose;
        }
        if let Some(shared_flags) = self.shared_flags {
            image.shared_flags = shared_flags;
        }
        if let Some(health) = self.health {
            image.health = health;
        }
        if !self.custom_name.is_unchanged() {
            apply_delta(&self.custom_name, &mut image.custom_name);
        }
        if let Some(custom_name_visible) = self.custom_name_visible {
            image.custom_name_visible = custom_name_visible;
        }
        if let Some(extra) = &self.extra {
            image.extra = extra.clone();
            return;
        }
        for (index, value) in &self.extra_changes {
            match value {
                Some(value) => match image.extra.iter_mut().find(|(other, _)| other == index) {
                    Some(entry) => entry.1 = value.clone(),
                    None => image.extra.push((*index, value.clone())),
                },
                None => image.extra.retain(|(other, _)| other != index),
            }
        }
    }
}

/// Whether the baseline carries this metadata entry with this value. Both sides
/// are native values, so no image is built for an unchanged entry.
fn base_has(
    base: &crate::boundary::EntityAppearance,
    index: u8,
    live: &crate::boundary::EntityDataValue,
) -> bool {
    base.extra
        .iter()
        .any(|(other, value)| *other == index && value == live)
}

impl From<&crate::boundary::EntityAppearance> for EntityAppearanceImage {
    fn from(appearance: &crate::boundary::EntityAppearance) -> Self {
        Self {
            item: appearance.item.clone(),
            experience_orb: appearance.experience_orb,
            spawn_data: appearance.spawn_data,
            equipment: appearance.equipment.to_vec(),
            baby: appearance.baby,
            pose: appearance.pose,
            shared_flags: appearance.shared_flags,
            health: appearance.health,
            custom_name: appearance.custom_name.clone(),
            custom_name_visible: appearance.custom_name_visible,
            extra: appearance
                .extra
                .iter()
                .map(|(index, value)| (*index, EntityDataValueImage::from(value)))
                .collect(),
        }
    }
}

impl From<&crate::boundary::EntityDataValue> for EntityDataValueImage {
    fn from(value: &crate::boundary::EntityDataValue) -> Self {
        match value {
            crate::boundary::EntityDataValue::Byte(v) => Self::Byte(*v),
            crate::boundary::EntityDataValue::VarInt(v) => Self::VarInt(*v),
            crate::boundary::EntityDataValue::VarLong(v) => Self::VarLong(*v),
            crate::boundary::EntityDataValue::Float(v) => Self::Float(*v),
            crate::boundary::EntityDataValue::Rotations(v) => Self::Rotations(*v),
            crate::boundary::EntityDataValue::Boolean(v) => Self::Boolean(*v),
            crate::boundary::EntityDataValue::WeatheringCopper(v) => Self::WeatheringCopper(*v),
            crate::boundary::EntityDataValue::CopperGolemState(v) => Self::CopperGolemState(*v),
            crate::boundary::EntityDataValue::SnifferState(v) => Self::SnifferState(*v),
            crate::boundary::EntityDataValue::Direction(v) => Self::Direction(*v),
            crate::boundary::EntityDataValue::PaintingVariant(v) => Self::PaintingVariant(*v),
            crate::boundary::EntityDataValue::ChickenVariant(v) => Self::ChickenVariant(*v),
            crate::boundary::EntityDataValue::CowVariant(v) => Self::CowVariant(*v),
            crate::boundary::EntityDataValue::PigVariant(v) => Self::PigVariant(*v),
            crate::boundary::EntityDataValue::ZombieNautilusVariant(v) => {
                Self::ZombieNautilusVariant(*v)
            }
            crate::boundary::EntityDataValue::String(v) => Self::String(v.clone()),
            crate::boundary::EntityDataValue::BlockState(v) => Self::BlockState(*v),
            crate::boundary::EntityDataValue::Item(v) => Self::Item(v.clone()),
            crate::boundary::EntityDataValue::OptionalBlockPos(v) => Self::OptionalBlockPos(*v),
            crate::boundary::EntityDataValue::EntityEffectParticle(v) => {
                Self::EntityEffectParticle(*v)
            }
            crate::boundary::EntityDataValue::OptionalUnsignedInt(v) => {
                Self::OptionalUnsignedInt(*v)
            }
            crate::boundary::EntityDataValue::OptionalLivingEntityReference(v) => {
                Self::OptionalLivingEntityReference(*v)
            }
            crate::boundary::EntityDataValue::VillagerData {
                kind,
                profession,
                level,
            } => Self::VillagerData {
                kind: *kind,
                profession: *profession,
                level: *level,
            },
        }
    }
}

/// Open (and truncate) a file for the journal thread's ownership.
pub fn open_journal_stream(path: &Path) -> Result<File, CoreError> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| CoreError::Storage(err.to_string()))
}

/// Drain helper for tests: read all entries of one batch file.
pub fn read_batch_file(path: &Path, identity: WalIdentity) -> Option<Vec<JournalEntry>> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let (lo, hi) = parse_batch_name(&name)?;
    let bytes = std::fs::read(path).ok()?;
    decode_batch(&bytes, lo, hi, identity).ok()
}

/// Block position helpers for journal entries.
#[must_use]
pub fn block_pos_array(pos: BlockPos) -> [i32; 3] {
    [pos.x, pos.y, pos.z]
}

/// Chunk position helpers for journal entries.
#[must_use]
pub fn chunk_pos_array(pos: ChunkPos) -> [i32; 2] {
    [pos.x, pos.z]
}

/// Entity id helpers for journal entries.
#[must_use]
pub fn entity_id_int(id: EntityId) -> i32 {
    id.0
}

/// Read a file fully (recovery helper).
#[must_use]
pub fn read_file_bytes(path: &Path) -> Option<Vec<u8>> {
    let mut file = File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(bytes)
}
