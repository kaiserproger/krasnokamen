//! Tick driver: inputs -> entity phase -> merge -> block phase ->
//! commit -> publish, at 20 TPS (ADR 0013 "Tick").
//!
//! The entity phase runs in parallel by data over scoped threads sized once
//! from the machine (no work stealing across phase boundaries, no locks on
//! the tick path: workers return owned writes). Merge applies intents one at
//! a time in fixed order with whole-intent rejection. The block phase runs
//! serially in vanilla-like update order. Commit folds blocks, applies entity
//! writes, and publishes the snapshot. Publish hands a [`TickOutput`] to broadcast
//! (world state: laggards skip to latest) and per-player
//! [`PlayerChange`](crate::boundary::PlayerChange) results to bounded
//! per-connection queues (never skipped; overflow disconnects).

use mc_physics::mth;
use std::collections::BTreeMap;

use mc_data::statistics::StatType;
use rustc_hash::{FxHashMap as HashMap, FxHashSet as HashSet};
use std::sync::Arc;

use crate::boundary::{
    BlockPos, ChangeSet, ChunkPos, ConnectionId, DigAction, EntityAppearance, EntityEvent,
    EntityId, Hand, PlayerChange, PlayerInput, TickOutput, TickTimes, WorldSnapshot,
};
use crate::entities::{
    Body, ColdFields, EntityKind, EntitySlot, EntityStore, MOTION_ALIVE, MOTION_EFFECTS,
    MOTION_OWNED, MOTION_PHASE_MASK, MOTION_PHASE_SHIFT, MOTION_PLAYER, MOTION_REST,
    MOTION_SPECIAL,
};
use crate::intent::{Footprint, Intent, IntentKind, InvClaim, sort_intents};
use crate::registry::CoreRegistry;
use crate::world::ChunkWorld;

mod auth;
pub(crate) use auth::unlock_session;
pub(crate) mod blocks;
mod book_actions;
mod connections;
pub use connections::Connections;
mod container_access;
pub(crate) mod damage;
mod death_message;
mod dimension;
pub(crate) use dimension::run_dimension_tick;
mod dying;
mod elytra;
mod entity_lifecycle;
mod entity_phase;
pub(crate) use entity_phase::coreperf_block_region;
mod hydration;
mod residency;
pub(crate) use hydration::{refuse_record, sync_chunk_block_entities};
mod input;
pub(crate) mod merchant;
pub(crate) mod mining;
mod passive_follow;
pub(crate) mod phase_laps;
pub(crate) mod spectator;
mod state;
mod stations;
mod system_graph;
use entity_phase::*;
use phase_laps::Laps;
mod players;
pub(crate) use players::*;
mod containers;
mod merge;
pub(crate) use containers::*;
mod build;
pub(crate) use build::*;

#[cfg(test)]
mod duplicate_login_tests;
#[cfg(test)]
mod furnace_tests;
#[cfg(test)]
pub(crate) mod mining_tests;

#[cfg(test)]
#[path = "auth_gate_tests.rs"]
mod auth_gate_tests;

#[cfg(test)]
#[path = "tick/mirror_tests.rs"]
mod mirror_tests;

#[cfg(test)]
#[path = "crowd_pen_tests.rs"]
mod crowd_pen_tests;

/// Fixed worker count: derived once from the machine.
#[must_use]
pub fn worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(4)
        .max(1)
}

/// Reuse capacity, not a slot snapshot: age expiry can compact the store before
/// despawn, and the two callers have different global-player admission rules.
pub(crate) fn active_entity_slots(
    state: &mut TickState,
    include_global_players: bool,
) -> Vec<EntitySlot> {
    let mut slots = std::mem::take(&mut state.serial_entity_slots);
    slots.clear();
    // Every row has current chunk membership. When all occupied chunks are
    // active, the dense range is already the unique, ordered admission list.
    let all_active = state
        .entities
        .villager_chunks()
        .keys()
        .all(|chunk| state.world.is_active(chunk.x, chunk.z))
        && state
            .entities
            .brain_sensor_chunks()
            .keys()
            .all(|chunk| state.world.is_active(chunk.x, chunk.z));
    if all_active {
        slots.extend(state.entities.slots());
        return slots;
    }
    // Exact membership permits a dense scan when most rows are active, even
    // with inactive occupied chunks. Keep sparse activity and old transition
    // entries on the indexed union below.
    if prefer_dense_entity_admission(state) {
        let mut previous: Option<(ChunkPos, bool)> = None;
        for slot in state.entities.slots() {
            let chunk = state.entities.chunk_of(slot);
            let active = match previous {
                Some((last, active)) if last == chunk => active,
                _ => {
                    let active = state.world.is_active_position(chunk.x, chunk.z);
                    previous = Some((chunk, active));
                    active
                }
            };
            if active
                || include_global_players
                    && state.players.contains_key(&state.entities.body(slot).id)
            {
                slots.push(slot);
            }
        }
        return slots;
    }
    let mut marks = std::mem::take(&mut state.serial_entity_marks);
    marks.resize(state.entities.len(), 0);
    let mut admitted = 0;
    for chunk in state
        .world
        .active_positions()
        .filter(|chunk| state.world.is_active(chunk.x, chunk.z))
    {
        state.entities.for_each_slot_in_chunk(chunk, |slot| {
            let mark = &mut marks[slot.0 as usize];
            admitted += usize::from(*mark == 0);
            *mark = 1;
        });
    }
    if include_global_players {
        for id in state.players.keys() {
            if let Some(slot) = state.entities.slot_of_any(*id) {
                let mark = &mut marks[slot.0 as usize];
                admitted += usize::from(*mark == 0);
                *mark = 1;
            }
        }
    }
    slots.reserve(admitted);
    for (index, &marked) in marks.iter().enumerate() {
        if marked != 0 {
            slots.push(EntitySlot(index as u32));
        }
    }
    marks.clear();
    state.serial_entity_marks = marks;
    slots
}

/// Read hot positions only for mostly active populations with exact membership.
/// Bucket counts reject widely dormant worlds before any per-chunk lookups;
/// population weights also reject dormant rows concentrated in a few chunks.
fn prefer_dense_entity_admission(state: &TickState) -> bool {
    let occupied =
        state.entities.villager_chunks().len() + state.entities.brain_sensor_chunks().len();
    // Each occupied chunk can occur in both indexes. Conservatively retain the
    // indexed path when active chunks cannot cover half the bucket entries.
    if occupied > state.world.active_positions().count().saturating_mul(4)
        || !state.entities.has_exact_chunk_membership()
    {
        return false;
    }
    let needed = state.entities.len().div_ceil(2);
    let mut active = 0;
    for (chunk, ids) in state
        .entities
        .villager_chunks()
        .iter()
        .chain(state.entities.brain_sensor_chunks())
    {
        if state.world.is_active_position(chunk.x, chunk.z) {
            active += ids.len();
            if active >= needed {
                return true;
            }
        }
    }
    active >= needed
}

/// The tick's authoritative plugin-login lock set, with a change counter: see
/// [`session_locks`].
mod session_locks;
pub(crate) use session_locks::SessionLocks;

/// One accepted damage intent, retaining its pre-commit hit context.
#[derive(Debug, Clone, Copy)]
pub struct DamagePiece {
    pub amount: f32,
    pub kind: crate::survival::DamageKind,
    pub source: Option<crate::intent::DamageSource>,
    pub(crate) physical: Option<crate::physical_defense::PhysicalImpact>,
    pub(crate) custom_armor: bool,
}

/// Authoritative tick state. Single owner: the tick thread.
pub struct TickState {
    pub config_sim_distance: i32,
    /// Client residency radius, independent of gameplay simulation.
    pub config_view_distance: i32,
    active_player_centers: Vec<ChunkPos>,
    active_ticket_ranges: Vec<(ChunkPos, i32, i32)>,
    active_sim_radius: i32,
    pub registry: Arc<CoreRegistry>,
    /// Where the core publishes the registry the tick runs on, so the edge
    /// sees a datapack reload even when no one was online to be told.
    pub(crate) published_registry: Option<Arc<std::sync::RwLock<Arc<CoreRegistry>>>>,
    /// Where the core publishes `send_command_feedback`, so the edge's own command
    /// replies (`/list`) obey the rule without reading tick state.
    pub(crate) published_command_feedback: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub(crate) datapacks: crate::datapacks::Runtime,
    pub(crate) command_context: crate::command_context::Runtime,
    pub(crate) debug_profiler: crate::DebugProfiler,
    /// The `/debug function` trace being recorded, if one is executing.
    pub(crate) debug_trace: Option<crate::DebugTracer>,
    pub world: ChunkWorld,
    /// This world's dimension; other worlds share its tick owner and WAL.
    pub dimension: crate::dimensions::Dimension,
    pub other_dimensions: std::collections::BTreeMap<crate::dimensions::Dimension, Box<TickState>>,
    pub(crate) dimension_requests: Vec<crate::dimensions::travel::Request>,
    pub(crate) dimension_respawns: Vec<EntityId>,
    pub(crate) dimension_routes: Vec<crate::dimensions::Dimension>,
    pub entities: EntityStore,
    pub connections: Connections,
    /// Opt-in native login fence; authorization belongs to the live session.
    pub(crate) require_plugin_login: bool,
    pub(crate) locked_sessions: SessionLocks,
    /// Consecutive unsupported non-descending movement samples permitted before
    /// anti-flight corrects and disconnects the player; zero disables it.
    pub(crate) max_airtime_ticks: u32,
    pub(crate) enable_command_block: bool,
    pub(crate) spawn_protection: u32,
    pub(crate) spawn_operators: std::collections::BTreeSet<uuid::Uuid>,
    pub(crate) brushing: crate::brushing::Brushing,
    pub(crate) command_blocks_due: Vec<BlockPos>,
    /// Per-mob switches layered under the `mob_griefing` gamerule.
    pub(crate) mob_griefing: crate::config::MobGriefingRules,
    pub(crate) test_acceleration: crate::config::test_acceleration::TestAcceleration,
    /// Player state per player entity (inventory, menus, vitals, mode).
    pub players: HashMap<EntityId, crate::player::PlayerState>,
    /// Session-only camera authority; absence means the viewer's own entity.
    pub(crate) cameras: Arc<HashMap<EntityId, EntityId>>,
    pub(crate) camera_releases: Vec<EntityId>,
    /// Recovered offline records by uuid: consumed by Join to restore full
    /// player state (inventory, vitals, xp, mode, position).
    pub offline_players: HashMap<uuid::Uuid, crate::journal::ReplayPlayer>,
    /// Game mode a player without a saved record joins in (`/defaultgamemode`).
    pub(crate) default_game_mode: crate::commands::GameMode,
    pub(crate) advancement_events: Vec<crate::advancements::Event>,
    pub(crate) advancement_reward_spawns: Vec<EntityId>,
    pub(crate) native_globals:
        std::collections::BTreeMap<String, mc_world::native::NativeDomainRecord>,
    pub(crate) plugins: crate::plugin::PluginState,
    pub(crate) plugin_runtime: Option<crate::plugin::PluginRuntime>,
    pub(crate) build_queries: Option<crate::build_precommit::PreviewQueries>,
    pub(crate) plugin_spawned: Vec<EntityId>,
    pub(crate) plugin_block_writes: Vec<(BlockPos, u32)>,
    pub(crate) plugin_players: crate::plugin_player::PlayerAuthority,
    pub(crate) plugin_damage: Vec<(EntitySlot, DamagePiece)>,
    /// Rows retired this tick; re-resolve their IDs at the commit tail.
    pub(crate) pending_removals: Vec<EntityId>,
    pub(crate) plugin_interactions: HashMap<(EntityId, u32), mc_script::ScriptEvent>,
    /// Per-connection reliable outbox (bounded; overflow disconnects).
    pub outbox: HashMap<ConnectionId, Vec<PlayerChange>>,
    pub outbox_cap: usize,
    /// Connections to drop after this tick's publish: (connection, tick the
    /// `Disconnect` was sealed into its queue).
    pub disconnects: Vec<(ConnectionId, u64)>,
    pub tick: u64,
    /// Unix seconds source for real-calendar stamps (advancement obtained
    /// dates). Production reads the system clock; deterministic runs inject a
    /// fixed one, so the wall clock is an explicit external input.
    pub(crate) wall_clock: fn() -> i64,
    /// Secret per-world salt of [`TickState::entity_uuid`]; the world's own is
    /// installed by `recover_tick_state`. The zero default is for unit-test
    /// states, which want reproducible ids.
    pub(crate) entity_salt: [u8; crate::entity_uuid::SALT_LEN],
    pub day_time: i64,
    /// Block edits staged by merge this tick: (pos, new state).
    pub merge_block_writes: Vec<(BlockPos, u32)>,
    /// Reused block-phase projection; merge writes remain visible to commit.
    block_phase_writes: Vec<(BlockPos, u32)>,
    /// Reused block cells of the item entities the twentieth-tick merge sorts.
    pub(crate) item_merge_cells: Vec<(i32, i32, i32)>,
    /// Reused ordered admission for serial simulation passes.
    pub(crate) serial_entity_slots: Vec<EntitySlot>,
    /// Byte admission scratch, rebuilt for each caller after structural changes.
    pub(crate) serial_entity_marks: Vec<u8>,
    /// Reused predicate caches; each owning pass resets them before reading.
    pub(crate) contact_hazards_scratch: crate::contact_hazards::Scratch,
    pub(crate) portal_match_cache: crate::world::SectionMatchCache,
    /// Negative portal probes, checked against identity/cells/content revision.
    pub(crate) portal_contact_stamps: Vec<Option<crate::world::ContactStamp>>,
    /// Accepted breaks retain their once-rolled, pre-write loot image.
    pub merge_broken: Vec<(EntityId, BlockPos, u32, Vec<mc_data::ItemStack>, i32)>,
    /// Command destroys use empty-tool loot regardless of sender game mode.
    pub(crate) merge_command_broken: Vec<(BlockPos, u32, Vec<mc_data::ItemStack>)>,
    pub(crate) merge_command_block_entities: Vec<(BlockPos, crate::command_block_entity::Prepared)>,
    /// Container contents staged by merge for broken containers:
    /// ((x,y,z), slots). Commit spawns each non-empty slot as a drop,
    /// closes viewers, removes storage, marks dirty, stages the tombstone.
    pub merge_container_contents: Vec<((i32, i32, i32), Vec<mc_data::ItemStack>)>,
    /// Viewers of containers broken this tick: (viewer, broken cell). A
    /// double chest's viewers close when either half breaks, so they are
    /// found at merge time, while both halves still stand. Commit closes
    /// each menu.
    pub merge_closed_viewers: Vec<(EntityId, (i32, i32, i32))>,
    /// Entity damage staged by merge: target slot and source-aware hit.
    pub merge_damage: Vec<(EntitySlot, DamagePiece)>,
    /// Item entities consumed by merge.
    pub merge_consumed_items: HashSet<EntitySlot>,
    /// Children spawned by merge.
    pub merge_spawns: Vec<MergeSpawn>,
    /// Animal harvest drops, journaled with all other committed spawns.
    pub merge_life_drops: Vec<([f64; 3], mc_data::ItemStack)>,
    /// Buckets filled/emptied out of a stack: (actor, new bucket item).
    pub merge_gives: Vec<(EntityId, u32)>,
    /// Scheduled water/lava ticks.
    pub fluid_ticks: crate::fluids::FluidTicks,
    pub(crate) block_ticks: crate::block_ticks::BlockTicks,
    pub(crate) redstone: crate::redstone::RedstoneState,
    pub(crate) redstone_containers: crate::redstone::containers::ContainerState,
    /// Random-tick section eligibility keyed by committed chunk revision.
    pub(crate) farm_sections: crate::farming::FarmSections,
    /// Pickup events staged by merge: (item slot, collector slot, count).
    pub merge_pickups: Vec<(EntitySlot, EntitySlot, i32)>,
    /// Entities whose viewer-visible appearance changed this tick (mob
    /// health, partially picked item stacks): published as metadata.
    pub appearance_changed: Vec<EntityId>,
    /// Players whose final Leave image entered `offline_players` this tick;
    /// drained by the journal so it never scans every offline record.
    pub(crate) left_players: Vec<uuid::Uuid>,
    /// Reused tick-local fence index: knot id and whether a live leash uses it.
    pub(crate) leash_knots: HashMap<[i32; 3], (EntityId, bool)>,
    /// Remote tickets: owner -> (centre chunk, radius).
    pub tickets: BTreeMap<String, (ChunkPos, i32)>,
    /// Derived pearl tickets, owner presence and the WAL ticket diff.
    pub(crate) chunk_tickets: crate::chunk_tickets::ChunkTickets,
    /// World revision when the previous entity phase read the world: a
    /// resting mob whose footprint chunks are no newer stays valid.
    pub entity_phase_revision: u64,
    /// Reused, non-authoritative entity-phase buffers; empty between ticks.
    entity_phase_scratch: EntityPhaseScratch,
    /// Reused per-tick explosion line-of-sight results; never persists or
    /// changes the ordered explosion application.
    pub(crate) explosion_exposure: crate::explosion::ExposureCache,
    /// Last swing tick per attacker (combat strength recharge).
    pub last_swing: HashMap<EntityId, u64>,
    /// Stand-specific player double-punch window; environmental hurt is unrelated.
    pub(crate) armor_stand_hits: HashMap<EntityId, u64>,
    /// World difficulty (commands).
    pub difficulty: crate::survival::Difficulty,
    pub(crate) journaled_difficulty: Option<crate::survival::Difficulty>,
    pub(crate) journaled_default_game_mode: Option<crate::commands::GameMode>,
    /// Visible weather target and independent rain-clock duration.
    pub weather: (crate::commands::Weather, i64),
    pub(crate) thunder_remaining_ticks: i64,
    pub(crate) clear_remaining_ticks: i64,
    pub(crate) thundering: bool,
    /// Reused precipitation worklist; only active resident columns are sampled.
    pub(crate) weather_chunks: Vec<ChunkPos>,
    /// Retained client fade levels, independent of the target and cycle clock.
    pub(crate) rain_level: f32,
    pub(crate) thunder_level: f32,
    pub(crate) journaled_climate: Option<crate::weather::ClimateImage>,
    pub border: crate::command_border::Border,
    pub(crate) journaled_border: Option<crate::command_border::Border>,
    /// Game rules (commands).
    pub gamerules: HashMap<String, String>,
    pub(crate) gamerules_dirty: bool,
    /// Transient category counts and chunk-local spawn overlap candidates.
    pub(crate) spawn_census: crate::natural_spawn::SpawnCensus,
    /// Column rest-height cache: (block x, block z) -> rest feet y.
    /// Next per-actor intent sequence.
    seq_counter: HashMap<EntityId, u32>,
    /// Live block-entity storage by block position (chest/furnace contents).
    /// Journaled + checkpointed as post-images.
    pub block_entities: crate::redstone::custody::BlockEntities,
    /// Block-entity positions removed this tick (container broken): emitted
    /// as `JournalEntry::RemoveBlockEntity` and cleared at commit, so the
    /// tombstone shares the tick with the block change.
    pub block_entity_removed: Vec<(i32, i32, i32)>,
    /// Menu and timed native container openers; owns visual and redstone counts.
    pub open_containers: crate::redstone::custody::OpenContainers,
    pub(crate) container_animation_recipients: Vec<ConnectionId>,
    pub(crate) expired_container_openers: Vec<EntityId>,
    pub(crate) vehicle_menus: crate::vehicles::VehicleMenus,
    /// Client-predicted block uses and respawns wait for native world admission.
    pub(crate) deferred_uses: Vec<PlayerInput>,
    pub(crate) sign_editors: HashMap<ConnectionId, crate::signs::Editor>,
    pub opaque_changes: Vec<(BlockPos, Option<Vec<u8>>)>,
    pub(crate) gateway_ages: HashMap<BlockPos, crate::end_portals::GatewayAge>,
    pub(crate) campfires: crate::campfire::Campfires,
    pub(crate) hives: crate::bees::Hives,
    pub(crate) jukeboxes: crate::jukebox::Jukeboxes,
    pub(crate) lecterns: crate::lectern::Lecterns,
    pub(crate) brewing: crate::brewing::BrewingStands,
    pub(crate) crafters: crate::crafter::Crafters,
    pub(crate) merge_shulker_placements: Vec<crate::shulker::Placement>,
    pub(crate) merge_item_component_placements: Vec<(BlockPos, u32, Vec<u8>)>,
    pub(crate) vibrations: crate::vibration::Bus,
    pub(crate) sculk: crate::sculk::SculkState,
    pub(crate) trial_spawners: crate::trial_spawner::Spawners,
    pub(crate) monster_spawners: crate::monster_spawner::Spawners,
    pub(crate) vaults: crate::vault::Vaults,
    pub(crate) shriekers: crate::shrieker::Shriekers,
    /// Players lying in a bed ([`crate::sleep`]).
    pub sleepers: HashMap<EntityId, crate::sleep::Sleeper>,
    /// Last journaled player / block-entity post-images. Images are
    /// absolute, so the journal writes one only when it differs from the
    /// last; an unchanged record is still covered by an earlier batch or
    /// the checkpoint.
    pub journaled_players: HashMap<EntityId, crate::journal::ReplayPlayer>,
    pub journaled_block_entities: HashMap<(i32, i32, i32), BlockEntityImage>,
    /// Attributed block/container history behind `/co` (dimension-local).
    pub(crate) block_log: crate::block_log::BlockLog,
    pub(crate) journaled_villagers: HashMap<EntityId, (u64, Box<crate::villager::VillagerState>)>,
    /// Last comparison tick and chunk anchor lifecycle clocks and crossings.
    pub(crate) journaled_entity_state:
        HashMap<EntityId, (u64, ChunkPos, Arc<crate::journal::EntityStateImage>)>,
    /// Touched-set journal caches (motion membership, clock rechecks, sweeps).
    pub(crate) journal_scan: crate::journal_touched::JournalScan,
    /// Creeper fuses, ranged mob charging and committed blasts.
    pub hostile: crate::hostile::HostileState,
    pub projectiles: crate::projectiles::ProjectileRuntime,
    pub end_fight: crate::end_fight::Fight,
    pub(crate) journaled_end_fight: Option<crate::end_fight::Fight>,
    pub raids: crate::raids::Raids,
    pub(crate) journaled_raids: Option<crate::raids::Raids>,
    pub(crate) trader_spawner: crate::mob_lanes::TraderSpawner,
    /// Vanilla restart-local 1200-tick admission timer; not saved data.
    pub(crate) trader_tick_delay: u32,
    pub(crate) journaled_trader_spawner: Option<crate::mob_lanes::TraderSpawner>,
    pub scoreboard: crate::scoreboard::Scoreboard,
    pub(crate) command_storage: crate::command_storage::Storage,
    pub(crate) command_storage_dirty: bool,
    pub maps: crate::maps::Maps,
    pub scoreboard_dirty: bool,
    /// Last published [`WorldSnapshot::team_names`] (`scoreboard::snapshot_team_names`).
    pub(crate) team_names: Arc<Vec<String>>,
    pub bossbars_dirty: bool,
    pub(crate) dimension_globals: crate::dimensions::globals::Globals,
    pub(crate) villager_pois: crate::settlements::allocator::PoiCache,
    pub(crate) smelting_cache: crate::villager::smelting::Cache,
    pub(crate) generated_entity_claims: std::collections::BTreeSet<String>,
    pub(crate) new_generated_entity_claims: Vec<String>,
    pub(crate) generated_entity_initial_scan: bool,
    pub(crate) settlements: crate::settlements::Registry,
    pub(crate) settlement_sequence: u64,
    pub(crate) autonomy_queue: crate::settlements::autonomy::AutonomyQueue,
    /// Per-connection settlement icon menu (transient display window, never persisted).
    pub(crate) settlement_menus: HashMap<ConnectionId, crate::settlements::ui::menus::Open>,
    /// Unified settlement Back history, discarded on disconnect.
    pub(crate) settlement_navigation:
        HashMap<ConnectionId, crate::settlements::ui::navigation::Navigation>,
    pub(crate) tick_budget: crate::tick_budget::Budget,
    /// Global `/tick` rate manager (moved between worlds with the other globals).
    pub(crate) tick_control: crate::tick_control::TickControl,
    /// Global `/bossbar` registry (moved between worlds with the other globals).
    pub bossbars: crate::bossbar::BossBars,
    /// `/locate` async searches owned by this dimension's tick (worker + drain).
    pub(crate) locate: crate::command_locate::LocateState,
}

/// Journaled block-entity contents: slots + furnace progress.
pub type BlockEntityImage = (
    Vec<mc_data::ItemStack>,
    Option<crate::journal::FurnaceImage>,
);

/// Live block-entity contents (slots + furnace progress).
#[derive(Debug, Clone, Default)]
pub struct BlockEntityState {
    pub slots: Vec<mc_data::ItemStack>,
    pub furnace: Option<crate::inventory::furnace::FurnaceState>,
}

impl BlockEntityState {
    /// Journal/checkpoint image of the furnace progress.
    pub(crate) fn furnace_image(&self) -> Option<crate::journal::FurnaceImage> {
        self.furnace
            .as_ref()
            .map(|progress| crate::journal::FurnaceImage {
                lit_remaining: progress.lit_remaining,
                lit_total: progress.lit_total,
                cook_progress: progress.cook_progress,
                cook_total: progress.cook_total,
                recipes_used: Arc::clone(&progress.recipes_used),
            })
    }
}

/// One child spawn staged by merge.
#[derive(Debug, Clone)]
pub struct MergeSpawn {
    pub pos: [f64; 3],
    pub type_id: u32,
    pub health: f32,
    pub wool: Option<u8>,
    /// Climate variant inherited from one parent (cow, pig, chicken).
    pub variant: Option<(u8, crate::boundary::EntityDataValue)>,
    pub taming: Option<crate::taming::TamingState>,
    pub advancement_breeding: Option<(EntityId, serde_json::Value)>,
}

/// Per-cell records decoded from one chunk (hydration decode phase output).
pub(crate) type CellRecords<T> = Vec<((i32, i32, i32), T)>;

/// Wool byte (color id, `0x10` sheared) of a wool-growing mob; white when
/// unset.
pub(crate) fn wool_of(appearance: &EntityAppearance) -> u8 {
    appearance
        .extra
        .iter()
        .find_map(|(index, value)| match value {
            crate::boundary::EntityDataValue::Byte(byte)
                if *index == crate::blueprint::SHEEP_WOOL_DATA_INDEX =>
            {
                Some(*byte as u8)
            }
            _ => None,
        })
        .unwrap_or(0)
}

/// Writes the wool byte of a wool-growing mob.
pub(crate) fn set_wool(appearance: &mut EntityAppearance, wool: u8) {
    let value = crate::boundary::EntityDataValue::Byte(wool as i8);
    match appearance
        .extra
        .iter_mut()
        .find(|(index, _)| *index == crate::blueprint::SHEEP_WOOL_DATA_INDEX)
    {
        Some(entry) => entry.1 = value,
        None => appearance
            .extra
            .push((crate::blueprint::SHEEP_WOOL_DATA_INDEX, value)),
    }
}

/// Block sampler over the tick's read state for `mc-physics`.
pub(crate) struct TickSampler<'a> {
    pub(crate) world: &'a ChunkWorld,
    pub(crate) registry: &'a CoreRegistry,
}

impl mc_physics::BlockSampler for TickSampler<'_> {
    fn material_at(&self, x: i32, y: i32, z: i32) -> mc_physics::BlockMaterial {
        match self.world.physics_block_at(x, y, z) {
            Some(state) => self.registry.materials.classify(state),
            // Missing chunk blocks movement: never a floor, never passable.
            None => mc_physics::BlockMaterial::Solid,
        }
    }

    fn max_collision_box_y(&self) -> u8 {
        self.registry.max_collision_box_y
    }

    fn stationary_downward_clip(
        &self,
        body: mc_physics::EntityBody,
        displacement_y: f64,
        max_scan_cells: usize,
        max_candidates: usize,
    ) -> Option<f64> {
        mc_physics::stationary_support_clip(
            self,
            body,
            displacement_y,
            max_scan_cells,
            max_candidates,
            self.registry.max_collision_boxes_per_cell,
        )
    }

    fn collision_boxes_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        emit: &mut dyn FnMut(mc_physics::BlockCollisionBox),
    ) {
        emit_collision_boxes(self.registry, self.world.physics_block_at(x, y, z), emit);
    }
}

/// Worker-thread sampler for the entity phase: missing columns resolve as
/// [`BlockMaterial::Solid`] so unloaded chunks block motion and never form
/// a floor. Shares the tick's read state by reference (no locks, no copies).
struct WorkerSampler<'a> {
    block_at: &'a dyn Fn(i32, i32, i32) -> Option<u32>,
    registry: &'a CoreRegistry,
}

impl mc_physics::BlockSampler for WorkerSampler<'_> {
    fn material_at(&self, x: i32, y: i32, z: i32) -> mc_physics::BlockMaterial {
        match (self.block_at)(x, y, z) {
            Some(state) => self.registry.materials.classify(state),
            None => mc_physics::BlockMaterial::Solid,
        }
    }

    fn max_collision_box_y(&self) -> u8 {
        self.registry.max_collision_box_y
    }

    fn stationary_downward_clip(
        &self,
        body: mc_physics::EntityBody,
        displacement_y: f64,
        max_scan_cells: usize,
        max_candidates: usize,
    ) -> Option<f64> {
        mc_physics::stationary_support_clip(
            self,
            body,
            displacement_y,
            max_scan_cells,
            max_candidates,
            self.registry.max_collision_boxes_per_cell,
        )
    }

    fn collision_boxes_at(
        &self,
        x: i32,
        y: i32,
        z: i32,
        emit: &mut dyn FnMut(mc_physics::BlockCollisionBox),
    ) {
        emit_collision_boxes(self.registry, (self.block_at)(x, y, z), emit);
    }
}

fn emit_collision_boxes(
    registry: &CoreRegistry,
    state: Option<u32>,
    emit: &mut dyn FnMut(mc_physics::BlockCollisionBox),
) {
    if let Some(boxes) = state.and_then(|state| registry.collision_boxes.get(state as usize)) {
        for &collision_box in boxes {
            emit(collision_box);
        }
    } else {
        // Unknown states and missing chunks remain conservative obstacles.
        emit(mc_physics::BlockCollisionBox::FULL_BLOCK);
    }
}

#[cfg(test)]
#[path = "tick_collision_tests.rs"]
mod collision_sampler_tests;

/// Build a [`PlayerState`] from an offline record (rejoin) or defaults.
/// Pure data hydration: inventory slots, vitals, xp, game mode.
fn hydrate_player(
    connection: ConnectionId,
    offline: Option<&crate::journal::ReplayPlayer>,
    default_game_mode: crate::commands::GameMode,
) -> crate::player::PlayerState {
    let mut player = crate::player::PlayerState::new(connection, default_game_mode);
    let Some(record) = offline else {
        return player;
    };
    player
        .opaque_player_fields
        .clone_from(&record.opaque_player_fields);
    player.statistics.clone_from(&record.statistics);
    player
        .advancement_progress
        .clone_from(&record.advancement_progress);
    player.unlocked_recipes.clone_from(&record.unlocked_recipes);
    if record.inventory.len() == player.inventory.slots.len() {
        player.inventory.slots = record.inventory.clone();
    }
    player.inventory.selected = record.selected.min(8);
    player.vitals.health = record.health;
    player.vitals.food = record.food;
    player.vitals.saturation = record.saturation;
    player.vitals.exhaustion = record.exhaustion;
    player.vitals.air = record.air;
    player.vitals.experience.level = record.experience_level;
    player.vitals.experience.progress = record.experience_progress;
    player.vitals.experience.total = record.experience_total;
    player.respawn_point = record.respawn;
    player.respawn_dimension = record.respawn_dimension;
    player.respawn_forced = record.respawn_forced;
    player.respawn_yaw = record.respawn_yaw;
    player.respawn_pitch = record.respawn_pitch;
    player.active_effects = record.active_effects.clone();
    player.current_impulse_impact_pos = record.current_impulse_impact_pos;
    player.fall_flying = record.fall_flying;
    player.warden_warning = record.warden_warning;
    player.enchantment_seed = record.enchantment_seed;
    player.enchantment_offer_seed = record.enchantment_offer_seed;
    player.game_mode = match record.game_mode {
        1 => crate::commands::GameMode::Creative,
        2 => crate::commands::GameMode::Adventure,
        3 => crate::commands::GameMode::Spectator,
        _ => crate::commands::GameMode::Survival,
    };
    player
}

/// Run one full tick: inputs -> entity phase -> merge -> block phase ->
/// commit -> publish. Returns the [`TickOutput`] plus per-connection
/// reliable outbox drains and the phase times (`total_us` is filled by the
/// tick loop).
pub fn run_tick(
    state: &mut TickState,
    inputs: &[PlayerInput],
) -> (
    TickOutput,
    Vec<(ConnectionId, Vec<PlayerChange>)>,
    TickTimes,
) {
    let wrapper_start = std::time::Instant::now();
    let phase_start = std::env::var_os("MC_CORE_PHASE_LOG")
        .is_some()
        .then(std::time::Instant::now);
    crate::plugin::pump(state);
    crate::tick_control::begin_tick(state);
    let pump_done = phase_start.map(|start| start.elapsed()).unwrap_or_default();
    let dimensions_start = std::time::Instant::now();
    let mut result = crate::dimensions::run_tick(state, inputs);
    let dimensions_time = dimensions_start.elapsed();
    let wrapper_tail_start = std::time::Instant::now();
    let dimensions_done = phase_start.map(|start| start.elapsed()).unwrap_or_default();
    crate::build_precommit::answer_previews(state);
    let previews_done = phase_start.map(|start| start.elapsed()).unwrap_or_default();
    crate::plugin::observe_players(state);
    result.2.plugins_us = dimensions_start.duration_since(wrapper_start).as_micros() as u64
        + wrapper_tail_start.elapsed().as_micros() as u64;
    // This envelope includes routing/travel/globals and destruction of local
    // phase scratch after the dimension function returns. It is not disk I/O
    // or edge network fanout.
    let local_us = result.2.input_us
        + result.2.chunks_us
        + result.2.entity_us
        + result.2.merge_us
        + result.2.block_us
        + result.2.commit_us
        + result.2.snapshot_us
        + result.2.views_us
        + result.2.publish_us;
    result.2.coordination_us = (dimensions_time.as_micros() as u64).saturating_sub(local_us);
    if let Some(start) = phase_start {
        let total = start.elapsed();
        eprintln!(
            "tick wrapper ms: tick={} pump={:.2} dimensions={:.2} previews={:.2} observe={:.2} total={:.2}",
            state.tick,
            pump_done.as_secs_f64() * 1000.0,
            dimensions_done.saturating_sub(pump_done).as_secs_f64() * 1000.0,
            previews_done.saturating_sub(dimensions_done).as_secs_f64() * 1000.0,
            total.saturating_sub(previews_done).as_secs_f64() * 1000.0,
            total.as_secs_f64() * 1000.0,
        );
    }
    result
}

/// Publish current player attributes from the inventory/effect owners.
pub(crate) fn refresh_player_attributes(state: &mut TickState, dirty_only: bool) {
    let attribute_changes: Vec<_> = state
        .players
        .iter_mut()
        .filter_map(|(&entity, player)| {
            if dirty_only && player.attribute_equipment.is_some() {
                return None;
            }
            let connection = state.connections.entity_connection(entity)?;
            let change = player.equipment_attributes(
                &state.registry.item_rules(),
                &state
                    .entities
                    .row(state.entities.slot_of(entity)?)
                    .loot
                    .attributes,
            )?;
            let max_health = player.effective_attribute("max_health").unwrap_or(20.0) as f32;
            let health = if player.vitals.health > max_health {
                player.vitals.health = max_health;
                Some((max_health, player.health_change()))
            } else {
                None
            };
            Some((entity, connection, change, health))
        })
        .collect();
    for (entity, connection, change, health) in attribute_changes {
        state.push_player(connection, change);
        if let Some((health, health_change)) = health {
            if let Some(slot) = state.entities.slot_of(entity) {
                let row = state.entities.row_mut(slot);
                row.health = health;
                Arc::make_mut(&mut row.appearance).health = health;
                state.appearance_changed.push(entity);
            }
            state.push_player(connection, health_change);
        }
    }
}

/// Apply one [`PlayerEffects`](crate::player::PlayerEffects): reliable
/// changes to the connection, drops as item-spawn intents (merge path).
pub(crate) fn apply_player_effects(
    state: &mut TickState,
    connection: ConnectionId,
    entity: EntityId,
    effects: crate::player::PlayerEffects,
    changes: &mut ChangeSet,
) {
    for change in effects.changes {
        spectator::player_change(state, entity, &change);
        state.push_player(connection, change);
    }
    for (recipe, ingredients) in effects.crafted_recipes {
        let ingredients: Vec<_> = ingredients
            .iter()
            .filter(|item| !item.is_empty())
            .map(|item| crate::advancements::item_snapshot(state, item))
            .collect();
        crate::advancements::trigger(
            state,
            entity,
            "minecraft:recipe_crafted",
            serde_json::json!({"recipe_id":recipe,"ingredients":ingredients}),
        );
    }
    if let Some(item) = &effects.consumed_item {
        let item = crate::advancements::item_snapshot(state, item);
        crate::advancements::trigger(
            state,
            entity,
            "minecraft:consume_item",
            serde_json::json!({"item":item}),
        );
    }
    for potion in effects.brewed_potions {
        crate::advancements::trigger(
            state,
            entity,
            "minecraft:brewed_potion",
            serde_json::json!({"potion":potion}),
        );
    }
    if let Some((item, levels)) = effects.enchanted_item {
        let item = crate::advancements::item_snapshot(state, &item);
        crate::advancements::trigger(
            state,
            entity,
            "minecraft:enchanted_item",
            serde_json::json!({"item":item,"levels":levels}),
        );
    }
    crate::effects::sync_appearance(state, entity);
    if effects.experience_orbs > 0 {
        let pos = state
            .open_containers
            .menu_at(&entity)
            .map(|&(x, y, z)| [f64::from(x) + 0.5, f64::from(y) + 0.5, f64::from(z) + 0.5])
            .unwrap_or_else(|| state.world.world_spawn_position());
        crate::experience_orbs::award(
            state,
            pos,
            effects.experience_orbs,
            &mut changes.entities_spawned,
        );
    }
    if effects.anvil_used {
        stations::damage_anvil(state, entity, changes);
    }
    for (item, count, kind) in effects.crafted {
        crate::plugin::craft_event(state, entity, item, count, kind);
    }
    if effects.dropped.is_empty() {
        return;
    }
    let pos = state
        .entities
        .slot_of(entity)
        .map(|slot| state.entities.pos(slot))
        .unwrap_or_else(|| state.world.world_spawn_position());
    for stack in effects.dropped {
        let id = spawn_item_entity(state, pos, stack);
        changes.entities_spawned.push(id);
    }
}

/// Sneaking with an occupied hand bypasses block-menu activation.
pub(crate) fn secondary_use(state: &TickState, entity: EntityId) -> bool {
    state
        .entities
        .slot_of(entity)
        .is_some_and(|slot| state.entities.row(slot).appearance.shared_flags & SNEAKING_FLAG != 0)
        && state.players.get(&entity).is_some_and(|player| {
            !player.inventory.slots[held_slot_index(player, crate::boundary::Hand::Main)].is_empty()
                || !player.inventory.slots[held_slot_index(player, crate::boundary::Hand::Off)]
                    .is_empty()
        })
}

/// Chunk slot shape -> core item stack.
pub(crate) fn furnace_slot_to_stack(slot: &mc_world::FurnaceSlot) -> mc_data::ItemStack {
    mc_data::ItemStack {
        count: slot.count,
        item_id: slot.item_id,
        damage: slot.damage,
        enchantments: slot.enchantments.clone(),
        custom_name: slot
            .custom_name
            .as_deref()
            .map(str::to_owned)
            .map(String::into_boxed_str),
        item_model: slot.item_model.clone(),
        stew_effects: slot.stew_effects.clone(),
        components: slot.components.clone(),
    }
}

/// Spawn one item entity immediately (input-step effect, not merge: the
/// player already paid the items in `PlayerState`). Returns its entity id.
/// Journaled as an Entity post-image + entities_spawned entry.
pub(crate) fn spawn_item_entity(
    state: &mut TickState,
    pos: [f64; 3],
    stack: mc_data::ItemStack,
) -> EntityId {
    if !state.registry.tags.contains_raw_id(
        mc_data::tags::ITEM_REGISTRY,
        "minecraft:dampens_vibrations",
        stack.item_id as i32,
    ) {
        crate::vibration::emit(state, pos, crate::vibration::GameEvent::EntityPlace, None);
    }
    let id = state.entities.alloc_id();
    let uuid = state.entity_uuid(id);
    let type_id = state.registry.item_type_id;
    let appearance = std::sync::Arc::new(EntityAppearance {
        item: Some(stack.clone()),
        health: 1.0,
        ..Default::default()
    });
    state.entities.spawn(
        id,
        uuid,
        EntityKind::Item { type_id },
        ColdFields {
            pos,
            vel: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            ground: false,
            health: 1.0,
            breed_cooldown: 0,
            pickup_delay: 10,
            item: Some(crate::entities::StackRow {
                item_id: stack.item_id,
                count: stack.count,
                stack: Some(stack),
            }),
            appearance,
            movement: crate::blueprint::MovementClass::Item,
            half_width: 0.125,
            height: 0.25,
            speed: 0.0,
            attack_damage: 0.0,
            sight_range: 0.0,
            attack_reach: 0.0,
        },
    );
    id
}

/// Shared flags bit of a sneaking entity.
pub(crate) const SNEAKING_FLAG: u8 = 0x02;
/// Vanilla `Pose.CROUCHING` ordinal.
const CROUCHING_POSE: u8 = 5;

#[cfg(test)]
mod cold_menu_residency_tests {
    //! Menu-coordinate writes: an anvil menu's stored cell outlives its chunk, so
    //! `damage_anvil` must ask residency before it reads or writes that cell.

    use super::stations::damage_anvil;
    use crate::boundary::{BlockPos, ChangeSet, ChunkPos, ConnectionId, PlayerInput};
    use crate::tick::{TickState, run_tick};
    use crate::world::ChunkWorld;
    use mc_data::Identifier;
    use std::sync::Arc;

    struct Empty(u32);
    impl mc_world::ChunkGenerator for Empty {
        fn generate(&self, pos: mc_world::ChunkPos) -> mc_world::Chunk {
            mc_world::Chunk::empty(
                pos,
                mc_world::BlockStateId(self.0),
                Identifier::parse("minecraft:plains").unwrap(),
            )
        }
    }

    fn block_state(name: &str) -> u32 {
        let registry = crate::registry::CoreRegistry::load(None).unwrap();
        registry
            .blocks
            .states()
            .find(|entry| entry.block.id.as_str() == name)
            .unwrap()
            .id
            .0
    }

    fn fixture() -> (TickState, crate::boundary::EntityId) {
        let registry = Arc::new(crate::registry::CoreRegistry::load(None).unwrap());
        let world = ChunkWorld::new(
            42,
            Arc::new(Empty(registry.states.air)),
            &registry,
            mc_world::BlockStateId(registry.states.air),
            [8, 65, 8],
            mc_world::OVERWORLD_GEOMETRY,
        );
        let mut state = TickState::new(registry, world, 2, 512);
        run_tick(
            &mut state,
            &[PlayerInput::Join {
                connection: ConnectionId(1),
                uuid: uuid::Uuid::new_v4(),
                name: "anvil".to_owned(),
                permission_level: 0,
            }],
        );
        state.world.ensure_blocking(&state.desired_chunks());
        let actor = state.connections.entity_of(ConnectionId(1)).unwrap();
        (state, actor)
    }

    #[test]
    fn anvil_damage_waits_for_a_resident_menu_coordinate() {
        let (mut state, actor) = fixture();
        let anvil = block_state("minecraft:anvil");
        let chipped = block_state("minecraft:chipped_anvil");
        let cell = BlockPos { x: 8, y: 65, z: 8 };
        state.world.set_block(cell.x, cell.y, cell.z, anvil);
        state.world.fold_overlay();
        state
            .open_containers
            .insert(actor, (cell.x, cell.y, cell.z));
        let chunk = ChunkPos { x: 0, z: 0 };
        state.world.finish_inhabited_tick(state.tick + 1);
        state
            .world
            .set_active_chunks(Default::default(), state.tick + 1);
        state.world.capture_dirty(&Default::default());
        state
            .world
            .mark_flushed(&[(chunk, state.world.storage_revision(chunk))]);
        assert!(
            state
                .world
                .evict_outside(&Default::default())
                .contains(&chunk)
        );
        assert!(!state.world.has_chunk(0, 0));
        // A later write leaves an anvil-family image in the cold overlay; the old
        // code would damage it (0.12 per tick) and write the next stage into it.
        state.world.set_block(cell.x, cell.y, cell.z, chipped);
        assert_eq!(state.world.block_at(cell.x, cell.y, cell.z), Some(chipped));
        for _ in 0..200 {
            state.tick += 1;
            let mut changes = ChangeSet::default();
            damage_anvil(&mut state, actor, &mut changes);
            assert!(
                changes.blocks.is_empty(),
                "damage wrote a cold menu coordinate into the overlay"
            );
        }
        assert_eq!(state.world.block_at(cell.x, cell.y, cell.z), Some(chipped));
    }
}
