//! Vanilla `/locate structure|biome|poi` (LocateCommand.java).
//!
//! Searches the generator's actual village and non-village starts; registry
//! entries without a generator still report `commands.locate.structure.not_found`.
//! Biome search samples the generator's
//! 3D noise biome on expanding square rings (vanilla `findClosestBiome3d`,
//! 6400/32/64). POI search scans loaded chunks for the vanilla
//! [`PoiTypes`](https://minecraft.wiki/w/Point_of_interest) block lists.
//!
//! Structure and biome searches run on one lazily spawned worker thread so the
//! tick never blocks; completions drain once per tick ([`drain`]). POI scans
//! only loaded chunks and runs inline on the tick. A player with two searches
//! already in flight (or a full queue of eight) gets a plain busy refusal;
//! queued searches are never reordered or dropped.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, SyncSender, TrySendError};

use crate::boundary::{ConnectionId, EntityId, PlayerChange};
use crate::command_args::CommandError;
use crate::command_feedback::{text, translated};
use crate::dimensions::Dimension;
use crate::tick::TickState;
use mc_data::Identifier;
use mc_nbt::{ListTag, Tag};
use mc_world::{BlockRegistry, BlockStateId, ChunkGenerator};
use mc_worldgen::vanilla_features::BiomeTagIndex;

const STRUCTURE_SEARCH_RADIUS_RINGS: u32 = 100;
const MAX_BIOME_SEARCH_RADIUS: i32 = 6400;
const BIOME_SAMPLE_STEP: i32 = 32;
const BIOME_SAMPLE_STEP_Y: i32 = 64;
const POI_SEARCH_RADIUS: i32 = 256;
/// In-flight searches per player, and total queued jobs.
const MAX_JOBS_PER_PLAYER: usize = 2;
const MAX_QUEUED_JOBS: usize = 8;
/// Capacity kept back from the background lane (a player `/locate` and dolphin leads) so the
/// other lane's opens always have their own worker and queue.
const BACKGROUND_QUEUE_RESERVE: usize = 2;
/// Every structure id vanilla 26.1.2 knows
/// (`data/minecraft/worldgen/structure/*.json`): anything else is `invalid`,
/// anything here but not generated is `not_found`.
const KNOWN_STRUCTURES: &[&str] = &[
    "minecraft:ancient_city",
    "minecraft:bastion_remnant",
    "minecraft:buried_treasure",
    "minecraft:desert_pyramid",
    "minecraft:end_city",
    "minecraft:fortress",
    "minecraft:igloo",
    "minecraft:jungle_pyramid",
    "minecraft:mansion",
    "minecraft:mineshaft",
    "minecraft:mineshaft_mesa",
    "minecraft:monument",
    "minecraft:nether_fossil",
    "minecraft:ocean_ruin_cold",
    "minecraft:ocean_ruin_warm",
    "minecraft:pillager_outpost",
    "minecraft:ruined_portal",
    "minecraft:ruined_portal_desert",
    "minecraft:ruined_portal_jungle",
    "minecraft:ruined_portal_mountain",
    "minecraft:ruined_portal_nether",
    "minecraft:ruined_portal_ocean",
    "minecraft:ruined_portal_swamp",
    "minecraft:shipwreck",
    "minecraft:shipwreck_beached",
    "minecraft:stronghold",
    "minecraft:swamp_hut",
    "minecraft:trail_ruins",
    "minecraft:trial_chambers",
    "minecraft:village_desert",
    "minecraft:village_plains",
    "minecraft:village_savanna",
    "minecraft:village_snowy",
    "minecraft:village_taiga",
];

/// Vanilla structure tags (`data/minecraft/tags/worldgen/structure/*.json`).
/// Entries name structures or nested `#tags`, exactly as the JSON does.
const STRUCTURE_TAGS: &[(&str, &[&str])] = &[
    (
        "minecraft:village",
        &[
            "minecraft:village_plains",
            "minecraft:village_desert",
            "minecraft:village_savanna",
            "minecraft:village_snowy",
            "minecraft:village_taiga",
        ],
    ),
    (
        "minecraft:mineshaft",
        &["minecraft:mineshaft", "minecraft:mineshaft_mesa"],
    ),
    (
        "minecraft:ocean_ruin",
        &["minecraft:ocean_ruin_cold", "minecraft:ocean_ruin_warm"],
    ),
    (
        "minecraft:shipwreck",
        &["minecraft:shipwreck", "minecraft:shipwreck_beached"],
    ),
    (
        "minecraft:ruined_portal",
        &[
            "minecraft:ruined_portal",
            "minecraft:ruined_portal_desert",
            "minecraft:ruined_portal_jungle",
            "minecraft:ruined_portal_mountain",
            "minecraft:ruined_portal_nether",
            "minecraft:ruined_portal_ocean",
            "minecraft:ruined_portal_swamp",
        ],
    ),
    ("minecraft:cats_spawn_in", &["minecraft:swamp_hut"]),
    ("minecraft:cats_spawn_as_black", &["minecraft:swamp_hut"]),
    (
        "minecraft:dolphin_located",
        &[
            "minecraft:ocean_ruin_cold",
            "minecraft:ocean_ruin_warm",
            "minecraft:shipwreck",
            "minecraft:shipwreck_beached",
        ],
    ),
    ("minecraft:eye_of_ender_located", &["minecraft:stronghold"]),
    ("minecraft:on_ocean_explorer_maps", &["minecraft:monument"]),
    (
        "minecraft:on_woodland_explorer_maps",
        &["minecraft:mansion"],
    ),
    (
        "minecraft:on_jungle_explorer_maps",
        &["minecraft:jungle_pyramid"],
    ),
    ("minecraft:on_swamp_explorer_maps", &["minecraft:swamp_hut"]),
    ("minecraft:on_treasure_maps", &["minecraft:buried_treasure"]),
    (
        "minecraft:on_trial_chambers_maps",
        &["minecraft:trial_chambers"],
    ),
    (
        "minecraft:on_plains_village_maps",
        &["minecraft:village_plains"],
    ),
    (
        "minecraft:on_desert_village_maps",
        &["minecraft:village_desert"],
    ),
    (
        "minecraft:on_savanna_village_maps",
        &["minecraft:village_savanna"],
    ),
    (
        "minecraft:on_snowy_village_maps",
        &["minecraft:village_snowy"],
    ),
    (
        "minecraft:on_taiga_village_maps",
        &["minecraft:village_taiga"],
    ),
];

/// Vanilla POI block lists (PoiTypes.java `bootstrap` + bed/lightning-rod
/// variants). Beds match head-part states only (filtered at resolve time).
const POI_BLOCKS: &[(&str, &[&str])] = &[
    ("minecraft:armorer", &["minecraft:blast_furnace"]),
    ("minecraft:butcher", &["minecraft:smoker"]),
    ("minecraft:cartographer", &["minecraft:cartography_table"]),
    ("minecraft:cleric", &["minecraft:brewing_stand"]),
    ("minecraft:farmer", &["minecraft:composter"]),
    ("minecraft:fisherman", &["minecraft:barrel"]),
    ("minecraft:fletcher", &["minecraft:fletching_table"]),
    (
        "minecraft:leatherworker",
        &[
            "minecraft:cauldron",
            "minecraft:lava_cauldron",
            "minecraft:water_cauldron",
            "minecraft:powder_snow_cauldron",
        ],
    ),
    ("minecraft:librarian", &["minecraft:lectern"]),
    ("minecraft:mason", &["minecraft:stonecutter"]),
    ("minecraft:shepherd", &["minecraft:loom"]),
    ("minecraft:toolsmith", &["minecraft:smithing_table"]),
    ("minecraft:weaponsmith", &["minecraft:grindstone"]),
    (
        "minecraft:home",
        &[
            "minecraft:red_bed",
            "minecraft:black_bed",
            "minecraft:blue_bed",
            "minecraft:brown_bed",
            "minecraft:cyan_bed",
            "minecraft:gray_bed",
            "minecraft:green_bed",
            "minecraft:light_blue_bed",
            "minecraft:light_gray_bed",
            "minecraft:lime_bed",
            "minecraft:magenta_bed",
            "minecraft:orange_bed",
            "minecraft:pink_bed",
            "minecraft:purple_bed",
            "minecraft:white_bed",
            "minecraft:yellow_bed",
        ],
    ),
    ("minecraft:meeting", &["minecraft:bell"]),
    ("minecraft:beehive", &["minecraft:beehive"]),
    ("minecraft:bee_nest", &["minecraft:bee_nest"]),
    ("minecraft:nether_portal", &["minecraft:nether_portal"]),
    ("minecraft:lodestone", &["minecraft:lodestone"]),
    (
        "minecraft:lightning_rod",
        &[
            "minecraft:lightning_rod",
            "minecraft:exposed_lightning_rod",
            "minecraft:weathered_lightning_rod",
            "minecraft:oxidized_lightning_rod",
            "minecraft:waxed_lightning_rod",
            "minecraft:waxed_exposed_lightning_rod",
            "minecraft:waxed_weathered_lightning_rod",
            "minecraft:waxed_oxidized_lightning_rod",
        ],
    ),
    (
        "minecraft:test_instance",
        &["minecraft:test_instance_block"],
    ),
];

/// Vanilla POI tags (`data/minecraft/tags/point_of_interest_type/*.json`).
const POI_TAGS: &[(&str, &[&str])] = &[
    (
        "minecraft:village",
        &[
            "#minecraft:acquirable_job_site",
            "minecraft:home",
            "minecraft:meeting",
        ],
    ),
    (
        "minecraft:acquirable_job_site",
        &[
            "minecraft:armorer",
            "minecraft:butcher",
            "minecraft:cartographer",
            "minecraft:cleric",
            "minecraft:farmer",
            "minecraft:fisherman",
            "minecraft:fletcher",
            "minecraft:leatherworker",
            "minecraft:librarian",
            "minecraft:mason",
            "minecraft:shepherd",
            "minecraft:toolsmith",
            "minecraft:weaponsmith",
        ],
    ),
    (
        "minecraft:bee_home",
        &["minecraft:beehive", "minecraft:bee_nest"],
    ),
];

/// Parsed `/locate <kind> <id|#tag>`.
#[derive(Debug, Clone, PartialEq)]
pub enum LocateCommand {
    Structure(ResourceOrTag),
    Biome(ResourceOrTag),
    Poi(ResourceOrTag),
}

/// One resource-or-tag key: `#tag` (`tag: true`) or a bare id defaulting to
/// the `minecraft` namespace, exactly like vanilla `ResourceOrTagKeyArgument`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResourceOrTag {
    pub tag: bool,
    pub id: Identifier,
}

/// Parse `structure <ref>`, `biome <ref>` or `poi <ref>`.
pub(crate) fn parse(args: &[&str]) -> Result<LocateCommand, CommandError> {
    const USAGE: &str = "locate <structure|biome|poi> <id|#tag>";
    let [kind, reference] = args else {
        return Err(CommandError::Usage(USAGE));
    };
    let target = ResourceOrTag::parse(reference).ok_or(CommandError::Usage(USAGE))?;
    match *kind {
        "structure" => Ok(LocateCommand::Structure(target)),
        "biome" => Ok(LocateCommand::Biome(target)),
        "poi" => Ok(LocateCommand::Poi(target)),
        _ => Err(CommandError::Usage(USAGE)),
    }
}

impl ResourceOrTag {
    fn parse(word: &str) -> Option<Self> {
        let (tag, raw) = match word.strip_prefix('#') {
            Some(rest) => (true, rest),
            None => (false, word),
        };
        if raw.is_empty() {
            return None;
        }
        Identifier::parse(raw).ok().map(|id| Self { tag, id })
    }

    /// Printable form for error keys: `#tag` keeps its prefix.
    fn printable(&self) -> String {
        if self.tag {
            format!("#{}", self.id.as_str())
        } else {
            self.id.as_str().to_owned()
        }
    }
}

/// A finished search: position, resolved display name and distance.
#[derive(Debug, Clone, PartialEq)]
pub struct LocateHit {
    pub pos: (i32, i32, i32),
    /// Resolved structure/biome/POI id at the hit (for `#tag (id)` names).
    pub found_id: Identifier,
    pub distance: i32,
}

/// Resolve a `#tag` to its member ids; `None` when the tag is unknown.
fn resolve_structure_tag(tag: &Identifier) -> Option<Vec<Identifier>> {
    let members = STRUCTURE_TAGS
        .iter()
        .find(|(name, _)| *name == tag.as_str())?
        .1;
    let mut out = Vec::new();
    for member in members {
        if let Some(nested) = member.strip_prefix('#') {
            let nested = Identifier::parse((*nested).to_owned()).ok()?;
            out.extend(resolve_structure_tag(&nested)?);
        } else {
            out.push(Identifier::parse((*member).to_owned()).ok()?);
        }
    }
    Some(out)
}

/// Resolve a POI `#tag` to member POI ids; `None` when unknown.
fn resolve_poi_tag(tag: &Identifier) -> Option<Vec<Identifier>> {
    let members = POI_TAGS.iter().find(|(name, _)| *name == tag.as_str())?.1;
    let mut out = Vec::new();
    for member in members {
        if let Some(nested) = member.strip_prefix('#') {
            let nested = Identifier::parse((*nested).to_owned()).ok()?;
            out.extend(resolve_poi_tag(&nested)?);
        } else {
            out.push(Identifier::parse((*member).to_owned()).ok()?);
        }
    }
    Some(out)
}

type SavedStarts = HashMap<mc_world::ChunkPos, Vec<mc_world::chunk::GeneratedStructureSite>>;

/// Saved coverage is authoritative, including covered chunks without starts.
/// Only uncovered chunks may contribute deterministic generator plans.
fn search_structure(
    generator: &dyn ChunkGenerator,
    origin: (i32, i32, i32),
    targets: &[Identifier],
    saved: &SavedStarts,
    options: &StructureSearchOptions,
    native: Option<(&mc_world::native::NativeCheckpointReader, u32)>,
) -> Result<Option<LocateHit>, String> {
    let centre = (origin.0.div_euclid(16), origin.2.div_euclid(16));
    for ring in 0..=options.radius_rings {
        let mut candidates = generator.structure_candidates_in_ring(targets, centre, ring);
        candidates.sort_unstable_by_key(|pos| (pos.x, pos.z));
        candidates.dedup();
        let mut nearest = None;
        for pos in candidates {
            let stored = if saved.contains_key(&pos) {
                None
            } else if let Some((reader, dimension)) = native {
                reader
                    .structure_starts_at(dimension, pos, options.skip_referenced)
                    .map_err(|error| format!("structure starts at {pos:?}: {error}"))?
            } else {
                None
            };
            let planned;
            let sites = if let Some(sites) = saved.get(&pos).or(stored.as_ref()) {
                sites
            } else {
                planned = generator.structure_sites_at(targets, pos);
                &planned
            };
            for site in sites {
                if !targets.contains(&site.structure)
                    || (options.skip_referenced && options.referenced.contains(&site.position))
                {
                    continue;
                }
                let p = site.position;
                let dx = i64::from(p.x) - i64::from(origin.0);
                let dz = i64::from(p.z) - i64::from(origin.2);
                let key = (dx * dx + dz * dz, (p.x, p.y, p.z), &site.structure);
                if nearest
                    .as_ref()
                    .is_none_or(|(distance, position, id)| key < (*distance, *position, id))
                {
                    nearest = Some((key.0, key.1, site.structure.clone()));
                }
            }
        }
        if let Some((distance, pos, found_id)) = nearest {
            return Ok(Some(LocateHit {
                pos,
                found_id,
                distance: distance.isqrt() as i32,
            }));
        }
    }
    Ok(None)
}

struct StructureSnapshot {
    source: Arc<crate::chunk_loader::ChunkSource>,
    resident: Result<SavedStarts, String>,
}

impl StructureSnapshot {
    fn capture(state: &TickState, options: &StructureSearchOptions) -> Self {
        // Resident coverage must be captured on tick. Its spatial extent is not
        // a chunk-radius bound: selected placements can have different spacing.
        let resident = state
            .world
            .chunks()
            .map(|chunk| {
                mc_world::structure_starts::read(&chunk.extras, chunk.pos, options.skip_referenced)
                    .map(|sites| (chunk.pos, sites))
            })
            .collect();
        Self {
            source: state.world.chunk_source(),
            resident,
        }
    }

    /// Open one pinned storage snapshot on the worker and query only candidates
    /// actually visited. Resident coverage wins over the checkpoint.
    fn search(
        self,
        generator: &dyn ChunkGenerator,
        origin: (i32, i32, i32),
        targets: &[Identifier],
        options: &StructureSearchOptions,
    ) -> Result<Option<LocateHit>, String> {
        let reader = self
            .source
            .native
            .as_ref()
            .map(|native| native.checkpoint_reader())
            .transpose()
            .map_err(|error| error.to_string())?;
        search_structure(
            generator,
            origin,
            targets,
            &self.resident?,
            options,
            reader
                .as_ref()
                .map(|reader| (reader, self.source.dimension)),
        )
    }
}

/// Square perimeter at Chebyshev distance `radius` around `centre`, in
/// ascending order (vanilla `spiralAround` visits rings in the same order).
pub fn square_ring(centre: (i32, i32), radius: i32) -> Vec<(i32, i32)> {
    if radius == 0 {
        return vec![centre];
    }
    let mut out = Vec::with_capacity((radius * 8) as usize);
    for offset in -radius..=radius {
        out.push((centre.0 + offset, centre.1 - radius));
        out.push((centre.0 + offset, centre.1 + radius));
    }
    for offset in (-radius + 1)..radius {
        out.push((centre.0 - radius, centre.1 + offset));
        out.push((centre.0 + radius, centre.1 + offset));
    }
    out
}

/// Synchronous biome search: vanilla `findClosestBiome3d` with 32-block
/// horizontal and 64-block vertical steps. `noise` is the generator's 3D
/// noise biome; `min_y`/`max_y` bound the vertical samples (block coords).
/// First hit in spiral order wins, exactly like vanilla.
pub fn search_biome(
    noise: &dyn Fn(i32, i32, i32) -> Option<Identifier>,
    origin: (i32, i32, i32),
    targets: &[Identifier],
    min_y: i32,
    max_y: i32,
) -> Option<LocateHit> {
    if targets.is_empty() {
        return None;
    }
    let sample_radius = MAX_BIOME_SEARCH_RADIUS / BIOME_SAMPLE_STEP;
    let sample_ys = sample_heights(origin.1, min_y + 1, max_y + 1);
    for ring in 0..=sample_radius {
        for (dx, dz) in square_ring((0, 0), ring) {
            let block_x = origin.0 + dx * BIOME_SAMPLE_STEP;
            let block_z = origin.2 + dz * BIOME_SAMPLE_STEP;
            for block_y in &sample_ys {
                let Some(biome) = noise(block_x, *block_y, block_z) else {
                    continue;
                };
                if targets.iter().any(|target| target == &biome) {
                    let distance = (i64::from(block_x - origin.0) * i64::from(block_x - origin.0)
                        + i64::from(block_y - origin.1) * i64::from(block_y - origin.1)
                        + i64::from(block_z - origin.2) * i64::from(block_z - origin.2))
                    .isqrt() as i32;
                    return Some(LocateHit {
                        pos: (block_x, *block_y, block_z),
                        found_id: biome,
                        distance,
                    });
                }
            }
        }
    }
    None
}

/// Vanilla `Mth.outFromOrigin`: the clamped origin first, then alternating
/// outward steps of 64 while either side stays inside the bounds.
pub fn sample_heights(origin: i32, min: i32, max: i32) -> Vec<i32> {
    if min > max {
        return Vec::new();
    }
    let mut out = vec![origin.clamp(min, max)];
    let mut low = out[0] - BIOME_SAMPLE_STEP_Y;
    let mut high = out[0] + BIOME_SAMPLE_STEP_Y;
    while low >= min || high <= max {
        if high <= max {
            out.push(high);
        }
        if low >= min {
            out.push(low);
        }
        low -= BIOME_SAMPLE_STEP_Y;
        high += BIOME_SAMPLE_STEP_Y;
    }
    out
}

/// Synchronous POI search over one candidate column set: nearest POI state
/// within 256 blocks by horizontal distance. `candidates` yields the loaded
/// columns to scan (x, z, y-range) so callers bound the work to resident
/// chunks; `block_at` reads the folded world image (overlay included) and
/// `is_poi` tests a global block state id.
pub fn search_poi(
    candidates: &dyn Fn() -> Vec<(i32, i32, i32, i32)>,
    block_at: &dyn Fn(i32, i32, i32) -> Option<u32>,
    is_poi: &dyn Fn(u32) -> bool,
    origin: (i32, i32, i32),
) -> Option<LocateHit> {
    let mut best: Option<(i64, (i32, i32, i32))> = None;
    for (x, z, min_y, max_y) in candidates() {
        if (x - origin.0).abs() > POI_SEARCH_RADIUS || (z - origin.2).abs() > POI_SEARCH_RADIUS {
            continue;
        }
        for y in min_y..=max_y {
            let Some(state) = block_at(x, y, z) else {
                continue;
            };
            if !is_poi(state) {
                continue;
            }
            let distance_sqr = i64::from(x - origin.0) * i64::from(x - origin.0)
                + i64::from(z - origin.2) * i64::from(z - origin.2);
            if best.as_ref().is_none_or(|(known, _)| distance_sqr < *known) {
                best = Some((distance_sqr, (x, y, z)));
            }
        }
    }
    // The caller resolves the hit block back to its POI type; the placeholder
    // below only feeds tag-display tests and never reaches a player.
    best.map(|(distance_sqr, pos)| LocateHit {
        pos,
        found_id: Identifier::parse("minecraft:meeting").expect("static POI id"),
        distance: distance_sqr.isqrt() as i32,
    })
}

/// Resolve POI member ids to the global block states that carry them, using
/// the vanilla [`PoiTypes`](https://minecraft.wiki/w/Point_of_interest) block
/// lists. Bed blocks match head-part states only, like vanilla `BEDS`.
pub fn poi_states(blocks: &BlockRegistry, members: &[Identifier]) -> HashSet<u32> {
    let mut wanted: HashSet<&str> = HashSet::new();
    for member in members {
        let Some((_, block_names)) = POI_BLOCKS.iter().find(|(name, _)| *name == member.as_str())
        else {
            continue;
        };
        wanted.extend(block_names.iter().copied());
    }
    let mut out = HashSet::new();
    for name in wanted {
        let Ok(id) = Identifier::parse(name) else {
            continue;
        };
        let Some(block) = blocks.block(&id) else {
            continue;
        };
        let head_only = name.ends_with("_bed");
        for state in &block.states {
            if head_only && !is_bed_head(blocks, *state) {
                continue;
            }
            out.insert(state.0);
        }
    }
    out
}

/// Whether a bed state is a head part (vanilla `BEDS` only matches heads).
/// Falls back to matching every state when the schema has no `part` property.
fn is_bed_head(blocks: &BlockRegistry, state: BlockStateId) -> bool {
    let Some(resolved) = blocks.by_id(state) else {
        return false;
    };
    resolved
        .properties
        .iter()
        .find(|(name, _)| name == "part")
        .is_none_or(|(_, value)| value == "head")
}

/// Which worker a search runs on. Interactive opens (an exploration map in a container or a
/// cartographer trade, an eye of ender) must not wait behind a player's `/locate` or a dolphin
/// lead: a 100-ring structure search can take long enough to block an open that is queued after
/// it on one FIFO worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocateLane {
    /// A container or trade opening a map, and an eye of ender.
    Interactive = 0,
    /// A player `/locate` and a fed dolphin's treasure lead.
    Background = 1,
}

impl LocateCompletion {
    fn lane(&self) -> LocateLane {
        match self {
            Self::Callback { .. } | Self::Eye(_) => LocateLane::Interactive,
            Self::Command(_) | Self::Dolphin(_) => LocateLane::Background,
        }
    }
}

/// Both lanes, in index order (`Lane` slots are `lane as usize`).
const LANES: [LocateLane; 2] = [LocateLane::Interactive, LocateLane::Background];

/// One lane's job queue, its completion receiver and the hook a test parks its worker in.
#[derive(Default)]
struct Lane {
    jobs: Option<SyncSender<LocateJob>>,
    /// This lane's completions. No `Sender` outlives the worker: a worker that panics drops it,
    /// so the tick sees `Disconnected` here and [`fail_lane`] completes that lane's jobs.
    done: Option<Receiver<LocateDone>>,
    /// Test-only: the gate this lane's worker enters before it searches.
    #[cfg(test)]
    gate: Option<LaneGate>,
}

/// A test's parking hook for one lane's worker; never set in production.
type LaneGate = std::sync::Arc<dyn Fn() + Send + Sync>;

/// Async job state owned by one dimension's [`TickState`][crate::tick::TickState].
/// The worker thread spawns lazily on the first `/locate`; both channels stay
/// `None` until then so idle worlds pay nothing.
#[derive(Default)]
pub struct LocateState {
    /// One worker per lane, each with its own job queue and completion channel: an interactive
    /// open never queues behind a `/locate` search, and a dead worker is noticed per lane.
    lanes: [Lane; 2],
    in_flight: HashMap<ConnectionId, usize>,
    /// Entity-owned searches (a fed dolphin's treasure lookup).
    entity_in_flight: HashMap<EntityId, usize>,
    /// Accepted jobs not yet drained, keyed by run id. Their completions live
    /// here once, so a terminal outcome can be produced even without a worker.
    pending: HashMap<u64, PendingLocate>,
    /// Accepted jobs waiting or running (queue occupancy).
    queued: usize,
    /// Monotonic run id; a stale completion can never satisfy a newer run.
    next_job: u64,
    /// The last drained treasure lookup of each fed dolphin.
    dolphin: HashMap<EntityId, DolphinTreasure>,
    /// Runtime-only dolphin navigation (never persisted in the entity row).
    dolphin_nav: HashMap<EntityId, DolphinNav>,
}

/// Runtime-only treasure navigation; kept outside `DolphinState` so a waypoint
/// change cannot mark the entity row for the WAL.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct DolphinNav {
    pub target: Option<[f64; 3]>,
    pub run: u64,
    pub tries: u8,
    pub last: Option<[f64; 3]>,
    pub dimension: Dimension,
}

impl LocateState {
    /// Targets valid in one dimension, copied for the parallel prepare pass.
    pub(crate) fn dolphin_targets(
        &self,
        dimension: crate::dimensions::Dimension,
    ) -> HashMap<EntityId, [f64; 3]> {
        self.dolphin_nav
            .iter()
            .filter(|(_, nav)| nav.dimension == dimension)
            .filter_map(|(id, nav)| nav.target.map(|target| (*id, target)))
            .collect()
    }

    /// In-flight search count for one owner.
    fn busy(&self, owner: LocateOwner) -> usize {
        match owner {
            LocateOwner::Connection(connection) => {
                self.in_flight.get(&connection).copied().unwrap_or(0)
            }
            LocateOwner::Entity(entity) => self.entity_in_flight.get(&entity).copied().unwrap_or(0),
        }
    }

    fn begin(&mut self, owner: LocateOwner) {
        let slot = match owner {
            LocateOwner::Connection(connection) => self.in_flight.entry(connection).or_default(),
            LocateOwner::Entity(entity) => self.entity_in_flight.entry(entity).or_default(),
        };
        *slot += 1;
    }

    fn finish(&mut self, owner: LocateOwner) {
        let remove = match owner {
            LocateOwner::Connection(connection) => {
                let Some(count) = self.in_flight.get_mut(&connection) else {
                    return;
                };
                *count = count.saturating_sub(1);
                *count == 0
            }
            LocateOwner::Entity(entity) => {
                let Some(count) = self.entity_in_flight.get_mut(&entity) else {
                    return;
                };
                *count = count.saturating_sub(1);
                *count == 0
            }
        };
        if remove {
            match owner {
                LocateOwner::Connection(connection) => {
                    self.in_flight.remove(&connection);
                }
                LocateOwner::Entity(entity) => {
                    self.entity_in_flight.remove(&entity);
                }
            }
        }
    }

    /// One lane's job queue, spawning that lane's worker on first use. A thread that cannot
    /// start is a command failure, never a tick-thread panic: the channels are dropped with this
    /// call, so the next search retries.
    fn worker(&mut self, lane: LocateLane) -> Result<SyncSender<LocateJob>, std::io::Error> {
        let slot = &mut self.lanes[lane as usize];
        if slot.jobs.is_none() {
            let capacity = match lane {
                LocateLane::Interactive => MAX_QUEUED_JOBS,
                LocateLane::Background => MAX_QUEUED_JOBS - BACKGROUND_QUEUE_RESERVE,
            };
            let (job_tx, job_rx) = std::sync::mpsc::sync_channel::<LocateJob>(capacity);
            // The completion sender is moved into the worker alone. Keeping a copy in the state
            // would keep the receiver connected after a worker panic, and the tick would never
            // learn that the lane's jobs can no longer be answered (see [`fail_lane`]).
            let (done_tx, done_rx) = std::sync::mpsc::channel::<LocateDone>();
            let name = match lane {
                LocateLane::Interactive => "locate-open",
                LocateLane::Background => "locate",
            };
            #[cfg(test)]
            let gate = slot.gate.take();
            std::thread::Builder::new()
                .name(name.into())
                .spawn(move || {
                    #[cfg(test)]
                    locate_worker_gated(job_rx, done_tx, gate);
                    #[cfg(not(test))]
                    locate_worker(job_rx, done_tx);
                })?;
            slot.jobs = Some(job_tx);
            slot.done = Some(done_rx);
        }
        Ok(slot
            .jobs
            .as_ref()
            .expect("lane worker just spawned")
            .clone())
    }
}

#[cfg(test)]
impl LocateState {
    /// Test seam: give one lane a pre-built job channel and completion receiver, so a test
    /// injects jobs and answers without spawning that lane's worker.
    fn install_test_lane(
        &mut self,
        lane: LocateLane,
        jobs: SyncSender<LocateJob>,
        done: Receiver<LocateDone>,
    ) {
        let slot = &mut self.lanes[lane as usize];
        slot.jobs = Some(jobs);
        slot.done = Some(done);
    }

    /// Test seam: whether any lane has a job channel (the worker was started or injected).
    fn has_job_lane(&self) -> bool {
        self.lanes.iter().any(|lane| lane.jobs.is_some())
    }

    /// Test seam: park the background lane's worker in `gate` before each search.
    fn arm_background_gate(&mut self, gate: LaneGate) {
        self.lanes[LocateLane::Background as usize].gate = Some(gate);
    }

    /// Test seam: park the interactive lane's worker in `gate` before each search.
    fn arm_interactive_gate(&mut self, gate: LaneGate) {
        self.lanes[LocateLane::Interactive as usize].gate = Some(gate);
    }
}

/// One accepted search whose completion is still owed to an owner.
struct PendingLocate {
    owner: LocateOwner,
    kind: LocateKind,
    origin: (i32, i32, i32),
    dimension: Dimension,
    completion: LocateCompletion,
}

/// One async search with owned inputs and a pinned metadata source.
struct LocateJob {
    id: u64,
    kind: LocateKind,
    origin: (i32, i32, i32),
    generator: Arc<dyn ChunkGenerator>,
    targets: Vec<Identifier>,
    snapshot: Option<StructureSnapshot>,
    options: StructureSearchOptions,
    min_y: i32,
    max_y: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocateKind {
    Structure,
    Biome,
}

/// A finished worker search, delivered to the tick thread.
struct LocateDone {
    id: u64,
    hit: Option<LocateHit>,
    /// A metadata/read refusal is not "no structure": commands answer unavailable instead of
    /// not_found, and a refused dolphin lead stops (the same read cannot improve by waiting).
    error: Option<String>,
    /// The lane's worker died before it answered, so a fresh worker may serve the job: a
    /// dolphin lead keeps its fish and retries instead of stopping.
    worker_lost: bool,
    elapsed_ms: u128,
}

#[cfg(not(test))]
fn locate_worker(jobs: Receiver<LocateJob>, done: Sender<LocateDone>) {
    run_locate_worker(jobs, done, None);
}

/// The two-lane worker with a test's parking hook: the gate runs before each search, so a test
/// can hold the background lane while an interactive open is submitted behind it.
#[cfg(test)]
fn locate_worker_gated(
    jobs: Receiver<LocateJob>,
    done: Sender<LocateDone>,
    gate: Option<LaneGate>,
) {
    run_locate_worker(jobs, done, gate.as_ref());
}

/// Shared worker loop. `gate` parks a test's worker before each search and is `None` in
/// production, so both lanes run the same code whether a test holds one of them or not.
fn run_locate_worker(jobs: Receiver<LocateJob>, done: Sender<LocateDone>, gate: Option<&LaneGate>) {
    for job in jobs {
        if let Some(gate) = gate {
            gate();
        }
        let start = std::time::Instant::now();
        let mut error = None;
        let hit = match job.kind {
            LocateKind::Structure => {
                match job.snapshot.expect("structure snapshot").search(
                    job.generator.as_ref(),
                    job.origin,
                    &job.targets,
                    &job.options,
                ) {
                    Ok(hit) => hit,
                    Err(failure) => {
                        tracing::error!(%failure, "Cannot read authoritative structure starts");
                        error = Some(failure);
                        None
                    }
                }
            }
            LocateKind::Biome => {
                let generator = Arc::clone(&job.generator);
                search_biome(
                    &move |x, y, z| generator.noise_biome(x, y, z),
                    job.origin,
                    &job.targets,
                    job.min_y,
                    job.max_y,
                )
            }
        };
        let elapsed_ms = start.elapsed().as_millis();
        if done
            .send(LocateDone {
                id: job.id,
                hit,
                error,
                worker_lost: false,
                elapsed_ms,
            })
            .is_err()
        {
            return;
        }
    }
}

/// Execute a parsed `/locate` on the tick thread: immediate errors answer
/// inline, real searches queue on the worker and reply through [`drain`].
pub(crate) fn execute(
    state: &mut TickState,
    connection: ConnectionId,
    executor: EntityId,
    command: &LocateCommand,
) {
    let origin_f = crate::command_console::pose(state, executor).map_or_else(
        || state.world.world_spawn_position(),
        |(position, _)| position,
    );
    let origin = (
        origin_f[0].floor() as i32,
        origin_f[1].floor() as i32,
        origin_f[2].floor() as i32,
    );
    match command {
        LocateCommand::Structure(target) => execute_structure(state, connection, origin, target),
        LocateCommand::Biome(target) => execute_biome(state, connection, origin, target),
        LocateCommand::Poi(target) => execute_poi(state, connection, origin, target),
    }
}

fn execute_structure(
    state: &mut TickState,
    connection: ConnectionId,
    origin: (i32, i32, i32),
    target: &ResourceOrTag,
) {
    let (targets, display) =
        match resolve_structure_target(target, state.registry.datapack_structures()) {
            Ok(resolved) => resolved,
            Err(key) => {
                failure(state, connection, key, vec![text(target.printable())]);
                return;
            }
        };
    if submit(
        state,
        LocateOwner::Connection(connection),
        LocateKind::Structure,
        origin,
        targets,
        LocateCompletion::Command(display),
        StructureSearchOptions::default(),
    )
    .is_some()
    {
        feedback(
            state,
            connection,
            "commands.locate.structure.searching",
            Vec::new(),
        );
    }
}

fn execute_biome(
    state: &mut TickState,
    connection: ConnectionId,
    origin: (i32, i32, i32),
    target: &ResourceOrTag,
) {
    let (targets, display) = match resolve_biome_target(state, target) {
        Ok(resolved) => resolved,
        Err(key) => {
            failure(state, connection, key, vec![text(target.printable())]);
            return;
        }
    };
    if submit(
        state,
        LocateOwner::Connection(connection),
        LocateKind::Biome,
        origin,
        targets,
        LocateCompletion::Command(display),
        StructureSearchOptions::default(),
    )
    .is_some()
    {
        feedback(
            state,
            connection,
            "commands.locate.biome.searching",
            Vec::new(),
        );
    }
}

fn execute_poi(
    state: &mut TickState,
    connection: ConnectionId,
    origin: (i32, i32, i32),
    target: &ResourceOrTag,
) {
    let (members, display) = match resolve_poi_target(target) {
        Ok(resolved) => resolved,
        Err(key) => {
            failure(state, connection, key, vec![text(target.printable())]);
            return;
        }
    };
    let states = poi_states(&state.registry.blocks, &members);
    if states.is_empty() {
        failure(
            state,
            connection,
            "commands.locate.poi.not_found",
            vec![text(&display)],
        );
        return;
    }
    let geometry = state.world.geometry();
    let start = std::time::Instant::now();
    let hit = search_poi_loaded(state, origin, &states);
    let elapsed_ms = start.elapsed().as_millis();
    match hit {
        Some((pos, found_id)) => {
            let found_name = poi_display(&display, target, &found_id);
            show_result(
                state,
                connection,
                "commands.locate.poi.success",
                &found_name,
                origin,
                pos,
                false,
            );
            tracing::info!("Locating element {found_name} took {elapsed_ms} ms");
        }
        None => failure(
            state,
            connection,
            "commands.locate.poi.not_found",
            vec![text(&display)],
        ),
    }
    let _ = geometry;
}

/// Scan resident chunks (plus the tick overlay) for POI states, cheapest
/// first: section palettes gate the exact cell scan.
fn search_poi_loaded(
    state: &TickState,
    origin: (i32, i32, i32),
    states: &HashSet<u32>,
) -> Option<((i32, i32, i32), Identifier)> {
    let is_poi = |id: u32| states.contains(&id);
    let (min_cx, max_cx) = (
        (origin.0 - POI_SEARCH_RADIUS).div_euclid(16),
        (origin.0 + POI_SEARCH_RADIUS).div_euclid(16),
    );
    let (min_cz, max_cz) = (
        (origin.2 - POI_SEARCH_RADIUS).div_euclid(16),
        (origin.2 + POI_SEARCH_RADIUS).div_euclid(16),
    );
    let mut best: Option<(i64, (i32, i32, i32), u32)> = None;
    let mut consider = |x: i32, y: i32, z: i32, id: u32| {
        let distance_sqr = i64::from(x - origin.0) * i64::from(x - origin.0)
            + i64::from(z - origin.2) * i64::from(z - origin.2);
        if best
            .as_ref()
            .is_none_or(|(known, _, _)| distance_sqr < *known)
        {
            best = Some((distance_sqr, (x, y, z), id));
        }
    };
    for cz in min_cz..=max_cz {
        for cx in min_cx..=max_cx {
            let Some(chunk) = state.world.chunk(mc_world::ChunkPos { x: cx, z: cz }) else {
                continue;
            };
            let geometry = chunk.geometry();
            for (section_idx, section) in chunk.sections.iter().enumerate() {
                let section_min_y = geometry.min_y() + section_idx as i32 * 16;
                let palette_hit = match section.palette() {
                    Some(palette) => palette.iter().any(|id| is_poi(id.0)),
                    None => is_poi(section.get(0, 0, 0).0),
                };
                if !palette_hit {
                    continue;
                }
                for ly in 0..16u8 {
                    let y = section_min_y + i32::from(ly);
                    for lz in 0..16u8 {
                        for lx in 0..16u8 {
                            let id = section.get(lx, ly, lz);
                            if !is_poi(id.0) {
                                continue;
                            }
                            consider(cx * 16 + i32::from(lx), y, cz * 16 + i32::from(lz), id.0);
                        }
                    }
                }
            }
        }
    }
    for ((x, y, z), id) in state.world.overlay_entries() {
        if (x - origin.0).abs() > POI_SEARCH_RADIUS || (z - origin.2).abs() > POI_SEARCH_RADIUS {
            continue;
        }
        if is_poi(id) {
            consider(x, y, z, id);
        }
    }
    best.map(|(_, pos, id)| {
        let found_id = state
            .registry
            .blocks
            .by_id(BlockStateId(id))
            .map(|resolved| poi_type_of(&resolved.block.id))
            .unwrap_or_else(|| Identifier::parse("minecraft:meeting").expect("static POI id"));
        (pos, found_id)
    })
}

/// Map a block back to its POI type (first table entry wins; beds map home).
fn poi_type_of(block: &Identifier) -> Identifier {
    for (poi, blocks) in POI_BLOCKS {
        if blocks.contains(&block.as_str()) {
            return Identifier::parse((*poi).to_owned()).expect("static POI id");
        }
    }
    Identifier::parse("minecraft:meeting").expect("static POI id")
}

/// Resolve a structure target against vanilla and the enabled datapack catalog;
/// tags expand to member structures. Returns member ids plus the display name.
fn resolve_structure_target(
    target: &ResourceOrTag,
    datapack_structures: &[Identifier],
) -> Result<(Vec<Identifier>, String), &'static str> {
    if target.tag {
        let members =
            resolve_structure_tag(&target.id).ok_or("commands.locate.structure.invalid")?;
        let display = format!("#{}", target.id.as_str());
        Ok((members, display))
    } else if KNOWN_STRUCTURES.contains(&target.id.as_str())
        || datapack_structures.contains(&target.id)
    {
        let display = target.id.as_str().to_owned();
        Ok((vec![target.id.clone()], display))
    } else {
        Err("commands.locate.structure.invalid")
    }
}

/// Resolve a biome target against the biome registry: ids must name a real
/// biome, tags expand through the world's biome-tag index.
fn resolve_biome_target(
    state: &TickState,
    target: &ResourceOrTag,
) -> Result<(Vec<Identifier>, String), &'static str> {
    let known: Vec<Identifier> = state.registry.biome_spawns.biomes().cloned().collect();
    if target.tag {
        let Some(worldgen_tags) = state.registry.worldgen_tags.as_ref() else {
            return Err("argument.resource_or_id.invalid");
        };
        if state
            .registry
            .tags
            .raw_ids("minecraft:worldgen/biome", target.id.as_str())
            .is_none()
        {
            return Err("argument.resource_or_id.invalid");
        }
        let members: Vec<Identifier> = known
            .into_iter()
            .filter(|biome| worldgen_tags.biome_in_tag(&target.id, biome))
            .collect();
        Ok((members, format!("#{}", target.id.as_str())))
    } else if known.iter().any(|biome| biome == &target.id) {
        let display = target.id.as_str().to_owned();
        Ok((vec![target.id.clone()], display))
    } else {
        Err("argument.resource_or_id.invalid")
    }
}

/// Resolve a POI target: ids must be known POI types, tags expand statically.
fn resolve_poi_target(target: &ResourceOrTag) -> Result<(Vec<Identifier>, String), &'static str> {
    if target.tag {
        let members = resolve_poi_tag(&target.id).ok_or("commands.locate.poi.not_found")?;
        Ok((members, format!("#{}", target.id.as_str())))
    } else if POI_BLOCKS
        .iter()
        .any(|(name, _)| *name == target.id.as_str())
    {
        let display = target.id.as_str().to_owned();
        Ok((vec![target.id.clone()], display))
    } else {
        Err("commands.locate.poi.not_found")
    }
}

/// Who waits for an async locate result. Players own `/locate` and eye
/// completions; a fed dolphin owns its `DOLPHIN_LOCATED` treasure lookup so the
/// tick only reads the drained result and never searches chunks inline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LocateOwner {
    Connection(ConnectionId),
    Entity(EntityId),
}

/// One fed dolphin's treasure lookup (`DolphinSwimToTreasureGoal.start/tick`).
/// Every entry names its run, so a late completion cannot satisfy a newer goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DolphinTreasure {
    /// Search queued on the locate worker.
    Pending { run: u64, dimension: Dimension },
    /// Nearest `#minecraft:dolphin_located` structure site, block coords.
    Found {
        run: u64,
        dimension: Dimension,
        pos: [i32; 3],
    },
    /// No structure in vanilla's 50-chunk radius: the goal stops and clears
    /// GotFish (`Dolphin#stuck`, `DolphinSwimToTreasureGoal.stop`).
    Missing { run: u64, dimension: Dimension },
    /// The worker could not read authoritative starts. This is not "no
    /// structure": GotFish is kept, and a later tick retries after `retry_at`.
    Failed {
        run: u64,
        dimension: Dimension,
        retry_at: u64,
    },
}

type StructureLookupReply = Box<dyn FnOnce(&mut TickState, Option<mc_world::BlockPos>) + Send>;
/// Terminal failure of a deferred lookup (metadata unreadable, worker loss,
/// owner changed dimension). A loot/trade plan must release itself here: leaving
/// the map/loot entry registered blocks every later open for the rest of the
/// session.
type StructureLookupFailure = Box<dyn FnOnce(&mut TickState) + Send>;

/// Deferred loot/trade lookup. Completion runs on the native tick, including
/// disconnected requesters; the callback validates its own container/token.
pub(crate) struct StructureLookupRequest {
    pub connection: ConnectionId,
    pub origin: (i32, i32, i32),
    pub destination: Identifier,
    pub search_radius_rings: u32,
    pub skip_existing_chunks: bool,
    pub referenced: Vec<mc_world::BlockPos>,
    pub reply: StructureLookupReply,
    /// Releases the caller's pending plan when the lookup cannot complete. The
    /// container then opens on its next attempt instead of waiting forever.
    pub on_failure: Option<StructureLookupFailure>,
}

pub(crate) enum LocateCompletion {
    Command(String),
    Eye(crate::projectiles::eye::EyeUse),
    Callback {
        reply: StructureLookupReply,
        on_failure: Option<StructureLookupFailure>,
    },
    Dolphin(EntityId),
}

struct StructureSearchOptions {
    radius_rings: u32,
    skip_referenced: bool,
    referenced: HashSet<mc_world::BlockPos>,
}

impl Default for StructureSearchOptions {
    fn default() -> Self {
        Self {
            radius_rings: STRUCTURE_SEARCH_RADIUS_RINGS,
            skip_referenced: false,
            referenced: HashSet::new(),
        }
    }
}

pub(crate) fn submit_structure_lookup(
    state: &mut TickState,
    request: StructureLookupRequest,
) -> bool {
    if request.search_radius_rings > 256 {
        return false;
    }
    let Some(targets) = resolve_structure_tag(&request.destination) else {
        return false;
    };
    submit(
        state,
        LocateOwner::Connection(request.connection),
        LocateKind::Structure,
        request.origin,
        targets,
        LocateCompletion::Callback {
            reply: request.reply,
            on_failure: request.on_failure,
        },
        StructureSearchOptions {
            radius_rings: request.search_radius_rings,
            skip_referenced: request.skip_existing_chunks,
            referenced: request.referenced.into_iter().collect(),
        },
    )
    .is_some()
}

pub(crate) fn submit_eye(state: &mut TickState, request: crate::projectiles::eye::EyeUse) -> bool {
    let origin = (
        request.origin[0].floor() as i32,
        request.origin[1].floor() as i32,
        request.origin[2].floor() as i32,
    );
    submit(
        state,
        LocateOwner::Connection(request.connection),
        LocateKind::Structure,
        origin,
        vec![Identifier::parse("minecraft:stronghold").expect("static structure id")],
        LocateCompletion::Eye(request),
        StructureSearchOptions::default(),
    )
    .is_some()
}

/// Queue a worker search; refuses when the owner or the shared queue is full.
/// Only a `/locate` command answers a player; entity-owned searches stay quiet.
fn submit(
    state: &mut TickState,
    owner: LocateOwner,
    kind: LocateKind,
    origin: (i32, i32, i32),
    targets: Vec<Identifier>,
    completion: LocateCompletion,
    options: StructureSearchOptions,
) -> Option<u64> {
    let quiet = !matches!(completion, LocateCompletion::Command(_));
    let limit = match owner {
        LocateOwner::Connection(_) => MAX_JOBS_PER_PLAYER,
        LocateOwner::Entity(_) => 1,
    };
    if state.locate.busy(owner) >= limit {
        report(
            state,
            owner,
            quiet,
            "A locate search is already running; try again in a moment",
        );
        return None;
    }
    // One worker per lane: an interactive open never queues behind a `/locate` search or a
    // dolphin lead, so only the total work still needs a ceiling.
    let lane = completion.lane();
    let capacity = MAX_QUEUED_JOBS * 2 - BACKGROUND_QUEUE_RESERVE;
    if state.locate.queued >= capacity {
        report(
            state,
            owner,
            quiet,
            "Too many locate searches are running; try again in a moment",
        );
        return None;
    }
    // Readiness before the expensive resident-chunk metadata snapshot.
    if let Err(error) = state.locate.worker(lane) {
        tracing::error!(%error, "locate worker thread could not start");
        if !quiet && let LocateOwner::Connection(connection) = owner {
            state.push_player(
                connection,
                PlayerChange::Chat {
                    message: "Locate is unavailable right now".into(),
                },
            );
        }
        return None;
    }
    let id = state.locate.next_job;
    state.locate.next_job = id.wrapping_add(1);
    let geometry = state.world.geometry();
    let job = LocateJob {
        id,
        kind,
        origin,
        generator: state.world.generator(),
        targets,
        snapshot: (kind == LocateKind::Structure)
            .then(|| StructureSnapshot::capture(state, &options)),
        options,
        min_y: geometry.min_y(),
        max_y: geometry.max_y() - 1,
    };
    let worker = match state.locate.worker(lane) {
        Ok(worker) => worker,
        Err(error) => {
            // A thread that cannot start is a command failure, not a tick panic.
            tracing::error!(%error, "locate worker thread could not start");
            report(state, owner, quiet, "Locate is unavailable right now");
            return None;
        }
    };
    match worker.try_send(job) {
        Ok(()) => {
            state.locate.queued += 1;
            state.locate.begin(owner);
            state.locate.pending.insert(
                id,
                PendingLocate {
                    owner,
                    kind,
                    origin,
                    dimension: state.dimension,
                    completion,
                },
            );
            Some(id)
        }
        Err(TrySendError::Full(_)) => {
            report(
                state,
                owner,
                quiet,
                "Too many locate searches are running; try again in a moment",
            );
            None
        }
        Err(TrySendError::Disconnected(_)) => {
            report(state, owner, quiet, "Locate is unavailable right now");
            None
        }
    }
}

/// Test hook: block until the worker answers one queued search and deliver it
/// through the normal [`deliver`] path, so a test that just requested a lead
/// waits for the worker's message instead of racing its thread for CPU. The
/// tick's own [`drain`] stays non-blocking; this only runs under `cfg(test)`.
#[cfg(test)]
pub(crate) fn await_queued_search(
    state: &mut TickState,
    lane: LocateLane,
    timeout: std::time::Duration,
) -> bool {
    let received = match state.locate.lanes[lane as usize].done.as_ref() {
        Some(done) => done.recv_timeout(timeout),
        None => return false,
    };
    match received {
        Ok(result) => {
            deliver(state, result);
            true
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            fail_lane(state, lane);
            true
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => false,
    }
}

/// Drain finished worker searches into player feedback and dolphin treasure
/// state. Every accepted job has a terminal outcome, including channel loss.
pub(crate) fn drain(state: &mut TickState) {
    let mut ready: Vec<LocateDone> = Vec::new();
    let mut lost: Vec<LocateLane> = Vec::new();
    for lane in LANES {
        let Some(done) = state.locate.lanes[lane as usize].done.as_ref() else {
            continue;
        };
        loop {
            match done.try_recv() {
                Ok(result) => ready.push(result),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    lost.push(lane);
                    break;
                }
            }
        }
    }
    for done in ready {
        deliver(state, done);
    }
    for lane in lost {
        fail_lane(state, lane);
    }
    // Retire empty in-flight entries so a quiet map stays small.
    state.locate.in_flight.retain(|_, count| *count > 0);
    state.locate.entity_in_flight.retain(|_, count| *count > 0);
}

fn deliver(state: &mut TickState, done: LocateDone) {
    let Some(pending) = state.locate.pending.remove(&done.id) else {
        // A duplicate/stale completion; its slot was already released.
        return;
    };
    state.locate.queued = state.locate.queued.saturating_sub(1);
    release(state, pending.owner);
    if pending.dimension != state.dimension {
        terminal_wrong_dimension(state, pending);
        return;
    }
    match pending.completion {
        LocateCompletion::Eye(request) => {
            if let Some(hit) = done.hit {
                crate::projectiles::eye::complete(state, request, hit.pos);
            }
        }
        LocateCompletion::Callback { reply, on_failure } => {
            // A metadata/read failure is not "no destination": release the
            // caller's pending loot/trade plan so its container opens again
            // (a fresh search next time) instead of waiting forever.
            if done.error.is_some() {
                tracing::warn!(
                    "locate callback refused: metadata unreadable; pending plan released"
                );
                if let Some(release) = on_failure {
                    release(state);
                }
                return;
            }
            let hit = done.hit.map(|hit| mc_world::BlockPos {
                x: hit.pos.0,
                y: hit.pos.1,
                z: hit.pos.2,
            });
            reply(state, hit);
        }
        LocateCompletion::Dolphin(dolphin) => {
            let run = done.id;
            let matches = state.locate.dolphin.get(&dolphin).is_some_and(|treasure| {
                matches!(
                    treasure,
                    DolphinTreasure::Pending { run: pending_run, dimension }
                        if *pending_run == run && *dimension == pending.dimension
                )
            });
            let live_fish = state.entities.slot_of(dolphin).is_some_and(|slot| {
                state
                    .entities
                    .row(slot)
                    .entity_life
                    .dolphin
                    .as_ref()
                    .is_some_and(|d| d.got_fish)
            });
            if !matches || !live_fish {
                return;
            }
            let treasure = if done.worker_lost {
                // The lane's worker died before answering; a fresh worker can serve the lead, so
                // keep the fish and retry once the window passes.
                DolphinTreasure::Failed {
                    run,
                    dimension: pending.dimension,
                    retry_at: state.tick.saturating_add(200),
                }
            } else if let Some(error) = done.error.as_deref() {
                // The same metadata read cannot improve by waiting: retrying re-captured every
                // resident chunk on the tick side every 200 ticks (LOCATE-SHARED-FIFO). The lead
                // stops like a `Missing` search, so the goal releases the fish instead of looping.
                tracing::warn!(
                    entity = dolphin.0,
                    %error,
                    "dolphin treasure lookup refused; the lead stops"
                );
                DolphinTreasure::Missing {
                    run,
                    dimension: pending.dimension,
                }
            } else {
                done.hit.map_or(
                    DolphinTreasure::Missing {
                        run,
                        dimension: pending.dimension,
                    },
                    |hit| DolphinTreasure::Found {
                        run,
                        dimension: pending.dimension,
                        pos: [hit.pos.0, hit.pos.1, hit.pos.2],
                    },
                )
            };
            state.locate.dolphin.insert(dolphin, treasure);
        }
        LocateCompletion::Command(display) => {
            command_result(
                state,
                pending.owner,
                pending.kind,
                pending.origin,
                display,
                done,
            );
        }
    }
}

fn command_result(
    state: &mut TickState,
    owner: LocateOwner,
    kind: LocateKind,
    origin: (i32, i32, i32),
    display: String,
    done: LocateDone,
) {
    let LocateOwner::Connection(connection) = owner else {
        return;
    };
    // Console (connection 0) has no entity but always receives output.
    let live = connection == crate::command_console::CONNECTION
        || state.connections.all_connections().any(|c| c == connection);
    if !live {
        return;
    }
    if done.error.is_some() {
        tracing::warn!(
            connection = connection.0,
            "locate search failed to read metadata"
        );
        state.push_player(
            connection,
            PlayerChange::Chat {
                message: "Locate is unavailable right now".into(),
            },
        );
        return;
    }
    match done.hit {
        Some(hit) => {
            let found_name = match kind {
                LocateKind::Structure => structure_display(&display, &hit.found_id),
                LocateKind::Biome => biome_display(&display, &hit.found_id),
            };
            let key = match kind {
                LocateKind::Structure => "commands.locate.structure.success",
                LocateKind::Biome => "commands.locate.biome.success",
            };
            show_result(
                state,
                connection,
                key,
                &found_name,
                origin,
                hit.pos,
                matches!(kind, LocateKind::Biome),
            );
            let elapsed_ms = done.elapsed_ms;
            tracing::info!("Locating element {found_name} took {elapsed_ms} ms");
        }
        None => {
            let key = match kind {
                LocateKind::Structure => "commands.locate.structure.not_found",
                LocateKind::Biome => "commands.locate.biome.not_found",
            };
            failure(state, connection, key, vec![text(&display)]);
        }
    }
}

/// The owner changed dimension while its search ran: finish the owed job but
/// never apply a stale lead to a new goal.
fn terminal_wrong_dimension(state: &mut TickState, pending: PendingLocate) {
    match pending.completion {
        LocateCompletion::Command(_) => {
            if let LocateOwner::Connection(connection) = pending.owner
                && (connection == crate::command_console::CONNECTION
                    || state.connections.all_connections().any(|c| c == connection))
            {
                state.push_player(
                    connection,
                    PlayerChange::Chat {
                        message: "Locate is unavailable right now".into(),
                    },
                );
            }
        }
        LocateCompletion::Callback { on_failure, .. } => {
            // Cross-world: the search cannot answer this dimension, so release
            // the pending plan; the container re-plans when reopened.
            if let Some(release) = on_failure {
                release(state);
            }
        }
        LocateCompletion::Dolphin(dolphin) => {
            state.locate.dolphin.remove(&dolphin);
            state.locate.dolphin_nav.remove(&dolphin);
        }
        LocateCompletion::Eye(_) => {}
    }
}

/// Test hook: take the failure continuation of the oldest queued callback
/// search, exactly as the metadata-error, wrong-dimension and worker-loss
/// paths would invoke it.
#[cfg(test)]
pub(crate) fn take_pending_callback_release(
    state: &mut TickState,
) -> Option<StructureLookupFailure> {
    let id = *state.locate.pending.keys().next()?;
    let pending = state.locate.pending.remove(&id)?;
    match pending.completion {
        LocateCompletion::Callback { on_failure, .. } => on_failure,
        LocateCompletion::Command(_) | LocateCompletion::Eye(_) | LocateCompletion::Dolphin(_) => {
            None
        }
    }
}

/// One lane's worker is gone (it panicked or its thread ended): only that lane's accepted but
/// undelivered jobs get a terminal refusal, and only that lane's channels are dropped, so the
/// next search on it spawns a fresh worker while the other lane keeps running.
fn fail_lane(state: &mut TickState, lane: LocateLane) {
    let runs: Vec<u64> = state
        .locate
        .pending
        .iter()
        .filter(|(_, pending)| pending.completion.lane() == lane)
        .map(|(run, _)| *run)
        .collect();
    // Reset the dead lane before the continuations: a callback may submit a new search, and that
    // search belongs to a fresh worker.
    state.locate.lanes[lane as usize] = Lane::default();
    for run in runs {
        deliver(
            state,
            LocateDone {
                id: run,
                hit: None,
                error: Some("locate worker lost".into()),
                worker_lost: true,
                elapsed_ms: 0,
            },
        );
    }
}

fn release(state: &mut TickState, owner: LocateOwner) {
    state.locate.finish(owner);
}

/// Queue the vanilla `DOLPHIN_LOCATED` lookup for a fed dolphin
/// (`Dolphin$DolphinSwimToTreasureGoal.start`: `findNearestMapStructure(tag,
/// blockPosition, 50, false)`; 50 is the chunk search radius, the same ring
/// unit `/locate` uses). Runs on the existing locate worker, so the tick never
/// blocks on chunk generation; [`drain`] delivers the site to the tick thread.
pub(crate) fn request_dolphin_treasure(
    state: &mut TickState,
    dolphin: EntityId,
    origin: [f64; 3],
) -> bool {
    if state.locate.dolphin.contains_key(&dolphin) {
        return false;
    }
    let Ok(tag) = Identifier::parse("minecraft:dolphin_located") else {
        return false;
    };
    let Some(targets) = resolve_structure_tag(&tag) else {
        return false;
    };
    let origin = (
        origin[0].floor() as i32,
        origin[1].floor() as i32,
        origin[2].floor() as i32,
    );
    let Some(run) = submit(
        state,
        LocateOwner::Entity(dolphin),
        LocateKind::Structure,
        origin,
        targets,
        LocateCompletion::Dolphin(dolphin),
        StructureSearchOptions {
            radius_rings: 50,
            skip_referenced: false,
            referenced: HashSet::new(),
        },
    ) else {
        return false;
    };
    state.locate.dolphin.insert(
        dolphin,
        DolphinTreasure::Pending {
            run,
            dimension: state.dimension,
        },
    );
    true
}

/// Every dolphin with a retained lookup, for stale-goal cleanup.
pub(crate) fn dolphin_treasure_entities(state: &TickState) -> Vec<EntityId> {
    state.locate.dolphin.keys().copied().collect()
}

/// The drained treasure lookup of one dolphin, if a search was requested.
pub(crate) fn dolphin_treasure(state: &TickState, dolphin: EntityId) -> Option<DolphinTreasure> {
    state.locate.dolphin.get(&dolphin).copied()
}

/// Forget one dolphin's treasure lookup (goal stopped, arrived or died).
pub(crate) fn clear_dolphin_treasure(state: &mut TickState, dolphin: EntityId) {
    state.locate.dolphin.remove(&dolphin);
    state.locate.dolphin_nav.remove(&dolphin);
}

/// Runtime-only navigation for one dolphin (never an entity row field).
pub(crate) fn dolphin_navigation(state: &TickState, dolphin: EntityId) -> Option<DolphinNav> {
    state.locate.dolphin_nav.get(&dolphin).copied()
}

pub(crate) fn set_dolphin_navigation(state: &mut TickState, dolphin: EntityId, nav: DolphinNav) {
    state.locate.dolphin_nav.insert(dolphin, nav);
}

/// Vanilla `showLocateResult` display name: `#tag (resolved)` for tags.
fn structure_display(display: &str, found: &Identifier) -> String {
    tag_display(display, found)
}

fn biome_display(display: &str, found: &Identifier) -> String {
    tag_display(display, found)
}

fn poi_display(display: &str, target: &ResourceOrTag, found: &Identifier) -> String {
    if target.tag {
        format!("{display} ({})", found.as_str())
    } else {
        display.to_owned()
    }
}

fn tag_display(display: &str, found: &Identifier) -> String {
    if display.starts_with('#') {
        format!("{display} ({})", found.as_str())
    } else {
        display.to_owned()
    }
}

/// Vanilla `showLocateResult`: translatable success with a green
/// `[x, y|~, z]` component (suggest-command `/tp @s`, hover tooltip) plus the
/// integer distance (3D for biomes, horizontal otherwise).
fn show_result(
    state: &mut TickState,
    connection: ConnectionId,
    key: &str,
    found_name: &str,
    origin: (i32, i32, i32),
    found: (i32, i32, i32),
    include_y: bool,
) {
    let displayed_y = if include_y {
        found.1.to_string()
    } else {
        "~".to_owned()
    };
    let coordinates = Tag::Compound(vec![
        ("text".into(), Tag::String(String::new())),
        (
            "extra".into(),
            Tag::List(ListTag {
                element_type: mc_nbt::tag_type::COMPOUND,
                elements: vec![
                    Tag::Compound(vec![("text".into(), Tag::String("[".into()))]),
                    Tag::Compound(vec![
                        ("translate".into(), Tag::String("chat.coordinates".into())),
                        (
                            "with".into(),
                            Tag::List(ListTag {
                                element_type: mc_nbt::tag_type::STRING,
                                elements: vec![
                                    Tag::String(found.0.to_string()),
                                    Tag::String(displayed_y.clone()),
                                    Tag::String(found.2.to_string()),
                                ],
                            }),
                        ),
                    ]),
                    Tag::Compound(vec![("text".into(), Tag::String("]".into()))]),
                ],
            }),
        ),
        ("color".into(), Tag::String("green".into())),
        (
            "click_event".into(),
            Tag::Compound(vec![
                ("action".into(), Tag::String("suggest_command".into())),
                (
                    "command".into(),
                    Tag::String(format!("/tp @s {} {displayed_y} {}", found.0, found.2)),
                ),
            ]),
        ),
        (
            "hover_event".into(),
            Tag::Compound(vec![
                ("action".into(), Tag::String("show_text".into())),
                (
                    "value".into(),
                    Tag::Compound(vec![(
                        "translate".into(),
                        Tag::String("chat.coordinates.tooltip".into()),
                    )]),
                ),
            ]),
        ),
    ]);
    let _ = origin;
    crate::command_execution::success_feedback(
        state,
        connection,
        key,
        vec![
            text(found_name),
            coordinates,
            text(found_distance(origin, found, include_y).to_string()),
        ],
    );
}

fn found_distance(origin: (i32, i32, i32), found: (i32, i32, i32), include_y: bool) -> i32 {
    let dx = f64::from(found.0 - origin.0);
    let dz = f64::from(found.2 - origin.2);
    if include_y {
        let dy = f64::from(found.1 - origin.1);
        (dx * dx + dy * dy + dz * dz).sqrt().floor() as i32
    } else {
        (dx * dx + dz * dz).sqrt().floor() as i32
    }
}

/// Tell an interactive locate owner why their search was not started; quiet
/// owners (dolphin goals, background leads) get no chat.
fn report(state: &mut TickState, owner: LocateOwner, quiet: bool, message: &str) {
    if quiet {
        return;
    }
    let LocateOwner::Connection(connection) = owner else {
        return;
    };
    state.push_player(
        connection,
        PlayerChange::Chat {
            message: message.into(),
        },
    );
}

/// Vanilla `sendFailure`: red translatable component.
fn failure(state: &mut TickState, connection: ConnectionId, key: &str, arguments: Vec<Tag>) {
    let Tag::Compound(mut fields) = translated(key, arguments) else {
        unreachable!("translated builds a compound");
    };
    fields.push(("color".into(), Tag::String("red".into())));
    state.push_player(
        connection,
        PlayerChange::StyledChat {
            content: Tag::Compound(fields),
        },
    );
}

fn feedback(state: &mut TickState, connection: ConnectionId, key: &str, arguments: Vec<Tag>) {
    crate::command_execution::success_feedback(state, connection, key, arguments);
}

#[cfg(test)]
#[path = "command_locate_tests.rs"]
mod tests;
