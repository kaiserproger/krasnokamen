//! Loaded-only adoption of the generator's own village sites.
//!
//! Identity comes from the dimension and structure start chunk, never from an
//! arbitrary bell. The meeting piece supplies the bell search bounds and every
//! resident is proved against a generated entity marker belonging to a piece.
//! Queues are ephemeral; native rows, entity columns and generated claims are
//! the durable authorities. Restart rebuilds queues from resident chunks.
use std::collections::{BTreeSet, VecDeque};

use mc_script::ScriptWorldBounds;
use mc_world::{GeneratedEntityMarker, GeneratedVillagePiece, GeneratedVillageSite};

use super::{SettlementId, buildings::BuildingKind};
use crate::boundary::{BlockPos, ChangeSet, ChunkPos};
use crate::tick::TickState;
use crate::villager::generated_uuid;

const INITIAL_REGION_QUERIES_PER_TICK: usize = 8;
const PENDING_REGION_QUERIES_PER_TICK: usize = 2;
const VILLAGES_PER_TICK: usize = 4;
const BINDS_PER_TICK: usize = 32;
/// Unreadable classification retries per generated piece before it is dropped.
const PIECE_ATTEMPTS: u8 = 8;
const PIECES_PER_PASS: usize = 8;
const ZONE_PROBES_PER_PASS: usize = 4;
const NATURAL_ZONE_CYCLES: usize = 2;
// VillagePlanSource documents a maximum reach of seven chunks plus a margin.
const START_SEARCH_RADIUS: i32 = 8;
/// Columns a natural lumber camp's zone may grow beyond the proved trunk: the
/// grove a camp is for, not the single column the tree search probes (CIV-6).
const GROVE_RADIUS: i32 = 6;

#[derive(Debug, Default)]
pub(crate) struct AutonomyQueue {
    initialized: bool,
    chunks: VecDeque<ChunkPos>,
    villages: VecDeque<PendingVillage>,
}

#[derive(Debug)]
struct PendingVillage {
    site: GeneratedVillageSite,
    source: String,
    residents: VecDeque<uuid::Uuid>,
    known_residents: BTreeSet<uuid::Uuid>,
    meeting_bell: Option<Option<[i32; 3]>>,
    uncertified: VecDeque<usize>,
    /// `None` = not classified yet (an unreadable column retries); `Some(vec![])`
    /// = classified as no building.
    piece_kinds: Vec<Option<Vec<BuildingKind>>>,
    /// Unreadable classifications per piece; `PIECE_ATTEMPTS` bounds the retries
    /// so a permanently unreadable column cannot hold the village's queue alive.
    piece_attempts: Vec<u8>,
    zone_cursor: usize,
    natural_candidates: VecDeque<NaturalCandidate>,
}

#[derive(Debug)]
struct NaturalCandidate {
    kind: BuildingKind,
    bounds: ScriptWorldBounds,
    entrance: [i32; 3],
    queued_at: u64,
    stock_proved: bool,
    after_resident: Option<uuid::Uuid>,
}

fn adopted_marker(source: &str) -> String {
    format!("settlement-village:{source}")
}

pub(crate) fn tick(state: &mut TickState, changes: &mut ChangeSet) {
    let mut queue = std::mem::take(&mut state.autonomy_queue);
    for &pos in &changes.chunks_loaded {
        if !queue.chunks.contains(&pos) {
            queue.chunks.push_back(pos);
        }
    }
    if !queue.initialized {
        queue.initialized = true;
        let mut resident = state.world.resident_positions();
        resident.sort_unstable_by_key(|pos| (pos.x, pos.z));
        for pos in resident {
            if !queue.chunks.contains(&pos) {
                queue.chunks.push_back(pos);
            }
        }
    }
    let region_queries = if queue.villages.is_empty() {
        INITIAL_REGION_QUERIES_PER_TICK
    } else {
        PENDING_REGION_QUERIES_PER_TICK
    };
    for _ in 0..region_queries {
        let Some(pos) = queue.chunks.pop_front() else {
            break;
        };
        let Some(chunk) = state.world.chunk(pos) else {
            continue;
        };
        let markers = chunk.generated_entities();
        let sites = state.world.village_sites_in_region(
            (pos.x - START_SEARCH_RADIUS, pos.z - START_SEARCH_RADIUS),
            (pos.x + START_SEARCH_RADIUS, pos.z + START_SEARCH_RADIUS),
        );
        for site in sites {
            if site.min.x.div_euclid(16) > pos.x
                || site.max.x.div_euclid(16) < pos.x
                || site.min.z.div_euclid(16) > pos.z
                || site.max.z.div_euclid(16) < pos.z
            {
                continue;
            }
            let index = queue
                .villages
                .iter()
                .position(|pending| pending.site.start_chunk == site.start_chunk);
            let pending = if let Some(index) = index {
                &mut queue.villages[index]
            } else {
                let source = format!(
                    "village:{}:{}:{}",
                    state.dimension.name(),
                    site.start_chunk.0,
                    site.start_chunk.1
                );
                // A removed village remains retired even after its chunks reload.
                if state
                    .generated_entity_claims
                    .contains(&adopted_marker(&source))
                    && state.settlements.find_village(&source).is_none()
                {
                    continue;
                }
                let piece_kinds = vec![None; site.pieces.len()];
                queue.villages.push_back(PendingVillage {
                    uncertified: (0..site.pieces.len()).collect(),
                    piece_kinds,
                    piece_attempts: vec![0; site.pieces.len()],
                    site,
                    source,
                    residents: VecDeque::new(),
                    known_residents: BTreeSet::new(),
                    meeting_bell: None,
                    zone_cursor: 0,
                    natural_candidates: VecDeque::new(),
                });
                queue.villages.back_mut().expect("queued village")
            };
            for marker in &markers {
                if marker.entity_type != "minecraft:villager"
                    || !state.generated_entity_claims.contains(&marker.claim)
                    || !belongs_to_site(&pending.site, marker)
                {
                    continue;
                }
                let uuid = generated_uuid(&marker.claim);
                if pending.known_residents.insert(uuid) {
                    pending.residents.push_back(uuid);
                }
            }
        }
    }
    let passes = queue.villages.len().min(VILLAGES_PER_TICK);
    let mut binds = 0;
    for _ in 0..passes {
        let mut pending = queue.villages.pop_front().expect("counted village");
        if advance(state, changes, &mut pending, &mut binds) {
            queue.villages.push_back(pending);
        }
    }
    state.autonomy_queue = queue;
}

fn belongs_to_site(site: &GeneratedVillageSite, marker: &GeneratedEntityMarker) -> bool {
    let Some((owner, placement)) = marker.claim.split_once('@') else {
        return false;
    };
    let Some((origin, _entity)) = placement.split_once('#') else {
        return false;
    };
    let mut coordinates = origin.split(':');
    let mut at = [0; 3];
    for coordinate in &mut at {
        let Some(value) = coordinates.next().and_then(|part| part.parse::<i32>().ok()) else {
            return false;
        };
        *coordinate = value;
    }
    coordinates.next().is_none()
        && site.pieces.iter().any(|piece| {
            piece.entity_owner.as_deref() == Some(owner)
                && [piece.position.x, piece.position.y, piece.position.z] == at
        })
}

/// Return true while this site has work to resume. Unloaded footprints retry;
/// completed scans and missing residents do not keep the queue alive.
fn advance(
    state: &mut TickState,
    changes: &mut ChangeSet,
    pending: &mut PendingVillage,
    binds: &mut usize,
) -> bool {
    let existing = state.settlements.find_village(&pending.source);
    if existing.is_none()
        && state
            .generated_entity_claims
            .contains(&adopted_marker(&pending.source))
    {
        return false;
    }
    let (id, anchor) = if let Some(record) = existing {
        (record.id, record.anchor)
    } else {
        let bell = if let Some(bell) = pending.meeting_bell {
            bell
        } else {
            let Some(bell) = meeting_bell(state, &pending.site) else {
                return true;
            };
            pending.meeting_bell = Some(bell);
            bell
        };
        let Some(anchor) = bell else {
            return false;
        };
        let Ok(id) = state.settlements.adopt_village(
            &mut state.settlement_sequence,
            &pending.source,
            anchor,
        ) else {
            return false;
        };
        let marker = adopted_marker(&pending.source);
        state.generated_entity_claims.insert(marker.clone());
        state.new_generated_entity_claims.push(marker);
        (id, anchor)
    };
    if !state
        .world
        .has_chunk(anchor[0].div_euclid(16), anchor[2].div_euclid(16))
    {
        return true;
    }
    grow_radius(state, id, anchor, &pending.site);
    ensure_storage_chest(state, changes, id, anchor, None);
    let attempts = pending
        .residents
        .len()
        .min(BINDS_PER_TICK.saturating_sub(*binds));
    for _ in 0..attempts {
        let uuid = pending.residents.pop_front().expect("counted resident");
        let Some(slot) = state.entities.slot_of_uuid(uuid) else {
            continue;
        };
        let row = state.entities.row(slot);
        let Some(v) = row.villager.as_ref() else {
            continue;
        };
        if v.settlement.is_some()
            || !state.entities.body(slot).alive
            || v.wandering
            || v.conversion_time.is_some()
        {
            continue;
        }
        let entity = state.entities.body(slot).id;
        if super::bind(state, id, entity).is_ok() {
            *binds += 1;
        } else {
            pending.residents.push_back(uuid);
        }
    }
    // Unloaded pieces retain their queue position without starving loaded
    // workplaces. All advertised piece footprints remain protected from zones.
    for _ in 0..pending.uncertified.len().min(PIECES_PER_PASS) {
        let index = pending.uncertified.pop_front().expect("counted piece");
        let piece = &pending.site.pieces[index];
        if let Some((lo, hi)) = piece.bounds {
            let Ok(bounds) = ScriptWorldBounds::new([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]) else {
                continue;
            };
            if !bounds_loaded(state, &bounds) {
                pending.uncertified.push_back(index);
                continue;
            }
            if !overlaps_existing(state, id, &bounds) {
                let kinds = match pending.piece_kinds[index].clone() {
                    Some(kinds) => kinds.clone(),
                    None => match piece_kinds(piece, state, &bounds) {
                        Some(kinds) => {
                            pending.piece_kinds[index] = Some(kinds.clone());
                            kinds
                        }
                        // An unreadable column is not a verdict: keep the queue
                        // position and retry, never a "no building" for the
                        // world's life.
                        None => {
                            pending.piece_attempts[index] =
                                pending.piece_attempts[index].saturating_add(1);
                            if pending.piece_attempts[index] >= PIECE_ATTEMPTS {
                                pending.piece_kinds[index] = Some(Vec::new());
                            } else {
                                pending.uncertified.push_back(index);
                            }
                            continue;
                        }
                    },
                };
                let retry_tick = state.tick.is_multiple_of(RETRY_SPACING_TICKS);
                let mut refused_any = false;
                for kind in kinds {
                    // A refused adopted piece used to vanish here: the gate could
                    // only report "no certified House", never why (ReviewD123Civ13).
                    if let Err(error) = super::buildings::certify_generated(state, id, kind, bounds)
                    {
                        let missing: Vec<String> = error
                            .missing
                            .iter()
                            .map(|requirement| {
                                format!(
                                    "{}({}/{})",
                                    requirement.description,
                                    requirement.found,
                                    requirement.required
                                )
                            })
                            .collect();
                        // Read the same accessor `assess` reads, so a refusal can be
                        // told apart from a mismatch: air answers mean the blocks are
                        // not there, ids mean the predicates disagree.
                        let centre = [(lo.x + hi.x) / 2, (lo.y + hi.y) / 2, (lo.z + hi.z) / 2];
                        let corner_min = block_name(state, [lo.x, lo.y, lo.z]);
                        let centre_block = block_name(state, centre);
                        let corner_max = block_name(state, [hi.x, hi.y, hi.z]);
                        let mut non_air = 0;
                        for dx in -1..=1 {
                            for dy in -1..=1 {
                                for dz in -1..=1 {
                                    if state
                                        .world
                                        .block_at(centre[0] + dx, centre[1] + dy, centre[2] + dz)
                                        .is_some_and(|id| id != state.registry.states.air)
                                    {
                                        non_air += 1;
                                    }
                                }
                            }
                        }
                        let mut chunks = Vec::new();
                        for cx in lo.x.div_euclid(16)..=hi.x.div_euclid(16) {
                            for cz in lo.z.div_euclid(16)..=hi.z.div_euclid(16) {
                                chunks.push((
                                    cx,
                                    cz,
                                    state.world.has_chunk(cx, cz),
                                    state.world.is_active(cx, cz),
                                ));
                            }
                        }
                        tracing::debug!(
                            target: "settlements::construction",
                            settlement = id.0,
                            ?kind,
                            template = ?piece.template,
                            min = ?lo,
                            max = ?hi,
                            corner_min,
                            centre_block,
                            corner_max,
                            centre_non_air = non_air,
                            chunks = ?chunks,
                            missing = ?missing,
                            "adopted piece refused certification"
                        );
                        refused_any = true;
                    }
                }
                // A village's blocks can arrive after the claim, so a refusal is
                // requeued and re-examined a few game minutes later instead of
                // dropping the piece for the world's life. Spacing keeps the bound
                // cheap: at most one scan per RETRY_SPACING_TICKS per piece.
                if refused_any && should_retry_refusal(retry_tick, pending.piece_attempts[index]) {
                    pending.piece_attempts[index] = pending.piece_attempts[index].saturating_add(1);
                    pending.uncertified.push_back(index);
                } else if refused_any && retry_tick {
                    tracing::debug!(
                        target: "settlements::construction",
                        settlement = id.0,
                        template = ?piece.template,
                        attempts = pending.piece_attempts[index],
                        "adopted piece refused certification at the retry bound"
                    );
                }
            }
        }
    }
    // Chunk adoption only queues terrain candidates. Ordinary route proof is
    // deferred to later planner passes, one resident/candidate per pass.
    if state.tick.is_multiple_of(20) {
        prove_natural(
            state,
            id,
            Some(&pending.site),
            &pending.known_residents,
            &mut pending.natural_candidates,
        );
    }
    let zone_limit = state
        .settlements
        .get(id)
        .map_or(0, |record| zone_probe_limit(record.radius));
    if pending.natural_candidates.is_empty()
        && pending.zone_cursor < zone_limit
        && (pending.zone_cursor == 0 || state.tick.is_multiple_of(20))
    {
        for _ in 0..ZONE_PROBES_PER_PASS {
            if pending.zone_cursor == zone_limit {
                break;
            }
            natural_zones(
                state,
                id,
                anchor,
                &pending.site,
                pending.zone_cursor,
                &mut pending.natural_candidates,
            );
            pending.zone_cursor += 1;
            if !pending.natural_candidates.is_empty()
                || (has_kind(state, id, BuildingKind::Mine)
                    && has_kind(state, id, BuildingKind::LumberCamp))
            {
                break;
            }
        }
    }
    !pending.residents.is_empty()
        || !pending.uncertified.is_empty()
        || !pending.natural_candidates.is_empty()
        || state
            .settlements
            .get(id)
            .is_some_and(|record| record.bootstrap_storage.is_none())
        || ((!has_kind(state, id, BuildingKind::Mine)
            || !has_kind(state, id, BuildingKind::LumberCamp))
            && pending.zone_cursor < zone_limit)
}

fn meeting_bell(state: &TickState, site: &GeneratedVillageSite) -> Option<Option<[i32; 3]>> {
    let Some(piece) = site.pieces.iter().find(|piece| {
        piece
            .template
            .as_deref()
            .is_some_and(|name| name.contains("/town_centers/"))
    }) else {
        return Some(None);
    };
    let Some((lo, hi)) = piece.bounds else {
        return Some(None);
    };
    let Ok(bounds) = ScriptWorldBounds::new([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]) else {
        return Some(None);
    };
    if !bounds_loaded(state, &bounds) {
        return None;
    }
    let mut bell = None;
    for x in lo.x..=hi.x {
        for y in lo.y..=hi.y {
            for z in lo.z..=hi.z {
                if state
                    .world
                    .block_at(x, y, z)
                    .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
                    .is_some_and(|block| block.block.id.path() == "bell")
                {
                    let at = [x, y, z];
                    bell = Some(bell.map_or(at, |known: [i32; 3]| known.min(at)));
                }
            }
        }
    }
    Some(bell)
}

fn grow_radius(
    state: &mut TickState,
    id: SettlementId,
    anchor: [i32; 3],
    site: &GeneratedVillageSite,
) {
    let distance: f64 = (0..3)
        .map(|axis| {
            let min = [site.min.x, site.min.y, site.min.z][axis];
            let max = [site.max.x, site.max.y, site.max.z][axis];
            (f64::from(min) - f64::from(anchor[axis]))
                .abs()
                .max((f64::from(max) - f64::from(anchor[axis])).abs())
                .powi(2)
        })
        .sum();
    let radius = (distance.sqrt().ceil() as u32)
        .saturating_add(4)
        .min(super::registry::MAX_RADIUS);
    state.settlements.grow_radius(id, radius);
}

/// A checked, empty location for a founding chest. Finding no site never
/// prevents founding, and does not increase the player's granted-chest count.
pub(crate) struct StorageChestSite {
    at: [i32; 3],
    block: u32,
}

pub(crate) fn storage_chest_site(
    state: &TickState,
    executor: crate::boundary::EntityId,
    anchor: [i32; 3],
) -> Option<StorageChestSite> {
    let chest = mc_data::Identifier::parse("minecraft:chest")
        .ok()
        .and_then(|name| state.registry.blocks.block(&name))?
        .default
        .0;
    let at = [
        (1, 0),
        (-1, 0),
        (0, 1),
        (0, -1),
        (1, 1),
        (-1, -1),
        (1, -1),
        (-1, 1),
    ]
    .into_iter()
    .flat_map(|(dx, dz)| {
        [0, 1, -1, 2, -2, 3, -3]
            .into_iter()
            .map(move |dy| [anchor[0] + dx, anchor[1] + dy, anchor[2] + dz])
    })
    .find(|at| {
        crate::plugin_player::allows_block(state, executor, BlockPos {
            x: at[0], y: at[1], z: at[2],
        })
            && state.world.block_at(at[0], at[1], at[2]) == Some(state.registry.states.air)
            && state.world.block_at(at[0], at[1] + 1, at[2]) == Some(state.registry.states.air)
            && state.world.block_at(at[0], at[1] - 1, at[2])
                .is_some_and(|id| !state.registry.collision_boxes[id as usize].is_empty())
            && !is_chest_block(state, [at[0], at[1] - 1, at[2]])
            && !state.block_entities.contains_key(&(at[0], at[1] - 1, at[2]))
            // Never merge with an existing (possibly foreign or stocked) chest.
            && [(1, 0), (-1, 0), (0, 1), (0, -1)].into_iter()
                .all(|(dx, dz)| !is_chest_block(state, [at[0] + dx, at[1], at[2] + dz]))
            && !state.block_entities.contains_key(&(at[0], at[1], at[2]))
    })?;
    Some(StorageChestSite { at, block: chest })
}

/// Generated villages may adopt existing storage. Player founding supplies a
/// prechecked new site instead, guaranteeing a distinct, empty starter chest.
pub(crate) fn ensure_storage_chest(
    state: &mut TickState,
    changes: &mut ChangeSet,
    id: SettlementId,
    anchor: [i32; 3],
    site: Option<StorageChestSite>,
) {
    if state
        .settlements
        .get(id)
        .is_some_and(|record| record.bootstrap_storage.is_some())
    {
        return;
    }
    let mut existing = None;
    if site.is_none() {
        for x in anchor[0] - 8..=anchor[0] + 8 {
            for y in anchor[1] - 4..=anchor[1] + 4 {
                for z in anchor[2] - 8..=anchor[2] + 8 {
                    let at = [x, y, z];
                    let distance =
                        (x - anchor[0]).pow(2) + (y - anchor[1]).pow(2) + (z - anchor[2]).pow(2);
                    if distance <= 64 && is_chest_block(state, at) {
                        let candidate = (distance, at);
                        if existing.is_none_or(|known| candidate < known) {
                            existing = Some(candidate);
                        }
                    }
                }
            }
        }
    }
    let position = if let Some((_, at)) = existing {
        at
    } else {
        let Some(StorageChestSite { at, block }) =
            site.or_else(|| storage_chest_site(state, crate::command_console::EXECUTOR, anchor))
        else {
            return;
        };
        state.world.set_block(at[0], at[1], at[2], block);
        changes.blocks.push(crate::boundary::BlockChange {
            position: BlockPos {
                x: at[0],
                y: at[1],
                z: at[2],
            },
            state: block,
        });
        state.block_entities.insert(
            (at[0], at[1], at[2]),
            crate::tick::BlockEntityState {
                slots: vec![mc_data::ItemStack::EMPTY; 27],
                furnace: None,
            },
        );
        at
    };
    let record = state.settlements.get_mut(id).expect("created settlement");
    record.bootstrap_storage = Some(position);
    record.revision = record.revision.saturating_add(1);
}

fn is_chest_block(state: &TickState, at: [i32; 3]) -> bool {
    state
        .world
        .block_at(at[0], at[1], at[2])
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .is_some_and(|block| matches!(block.block.id.path(), "chest" | "trapped_chest" | "barrel"))
}

fn has_kind(state: &TickState, id: SettlementId, kind: BuildingKind) -> bool {
    state.settlements.get(id).is_some_and(|record| {
        record
            .buildings
            .values()
            .any(|building| building.kind == kind)
    })
}

fn zone_probe_count(radius: u32) -> usize {
    let reach = (radius.min(super::registry::MAX_RADIUS) + 31).div_ceil(16) as usize;
    (reach * 2 + 1).pow(2)
}

fn zone_probe_limit(radius: u32) -> usize {
    zone_probe_count(radius).saturating_mul(NATURAL_ZONE_CYCLES)
}

fn zone_probe(anchor: [i32; 3], radius: u32, cursor: usize) -> [i32; 3] {
    // A square spiral visits nearby terrain first, then every 16-block cell
    // across the radius. Include the westward grove window at the east edge.
    let index = cursor % zone_probe_count(radius);
    if index == 0 {
        return anchor;
    }
    let ring = index.isqrt().div_ceil(2);
    let side = ring * 2;
    let offset = index - (side - 1).pow(2);
    let along = (offset % side) as i32;
    let r = ring as i32;
    let (x, z) = match offset / side {
        0 => (r, -r + 1 + along),
        1 => (r - 1 - along, r),
        2 => (-r, r - 1 - along),
        _ => (-r + 1 + along, -r),
    };
    [anchor[0] + x * 16, anchor[1], anchor[2] + z * 16]
}

fn natural_zones(
    state: &mut TickState,
    id: SettlementId,
    anchor: [i32; 3],
    site: &GeneratedVillageSite,
    cursor: usize,
    candidates: &mut VecDeque<NaturalCandidate>,
) {
    let Some(record) = state.settlements.get(id) else {
        return;
    };
    let probe = zone_probe(anchor, record.radius, cursor);
    if !has_kind(state, id, BuildingKind::Mine) {
        let min = [probe[0] + 9, anchor[1] - 17, probe[2] - 2];
        let max = [probe[0] + 23, anchor[1] + 18, probe[2] + 12];
        if let Ok(search) = ScriptWorldBounds::new(min, max)
            && let Some(entrance) = open_entrance(state, &search, [min[0], anchor[1], min[2] + 1])
            && let Ok(bounds) = ScriptWorldBounds::new(
                [min[0], entrance[1] - 14, min[2]],
                [max[0], entrance[1] + 1, max[2]],
            )
            && crate::villager::mining::initial_cut_safe(
                state,
                bounds,
                crate::boundary::BlockPos {
                    x: entrance[0],
                    y: entrance[1],
                    z: entrance[2],
                },
            )
        {
            queue_natural(
                site,
                BuildingKind::Mine,
                bounds,
                entrance,
                state.tick,
                candidates,
            );
        }
    }
    if !has_kind(state, id, BuildingKind::LumberCamp) {
        let mut queued = 0;
        'columns: for x in probe[0] - 31..probe[0] {
            for z in probe[2] - 15..=probe[2] + 15 {
                let mut lo = i32::MAX;
                let mut hi = i32::MIN;
                let mut logs = 0;
                for y in probe[1] - 3..=probe[1] + 12 {
                    if state
                        .world
                        .block_at(x, y, z)
                        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
                        .is_some_and(|block| block.block.id.path().ends_with("_log"))
                    {
                        logs += 1;
                        lo = lo.min(y);
                        hi = hi.max(y);
                    }
                }
                if logs < 4 || hi - lo + 1 != logs || !natural_tree(state, x, lo, hi, z) {
                    continue;
                }
                for [dx, dz] in [[-1, 0], [1, 0], [0, -1], [0, 1]] {
                    let entrance_x = x + dx;
                    let entrance_z = z + dz;
                    let min = [x.min(entrance_x), lo - 1, z.min(entrance_z)];
                    let max = [x.max(entrance_x), hi + 1, z.max(entrance_z)];
                    if let Ok(bounds) = ScriptWorldBounds::new(min, max)
                        && let Some(entrance) =
                            open_entrance(state, &bounds, [entrance_x, anchor[1], entrance_z])
                    {
                        let before = candidates.len();
                        queue_natural(
                            site,
                            BuildingKind::LumberCamp,
                            bounds,
                            entrance,
                            state.tick,
                            candidates,
                        );
                        queued += candidates.len() - before;
                        if queued == 16 {
                            break 'columns;
                        }
                    }
                }
            }
        }
    }
}

// Match the lumberjack's harvestable trunk and wild-crown requirements.
fn natural_tree(state: &TickState, x: i32, base: i32, top: i32, z: i32) -> bool {
    let block = |x, y, z| {
        state
            .world
            .block_at(x, y, z)
            .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
    };
    if !block(x, base - 1, z).is_some_and(|b| {
        matches!(
            b.block.id.path(),
            "grass_block"
                | "dirt"
                | "coarse_dirt"
                | "podzol"
                | "rooted_dirt"
                | "mud"
                | "farmland"
                | "moss_block"
        )
    }) || !block(x, base, z).is_some_and(|b| !b.block.id.path().starts_with("stripped_"))
    {
        return false;
    }
    (-2..=2).any(|dx| {
        (-2..=2).any(|dz| {
            (-2..=2).any(|dy| {
                block(x + dx, top + dy, z + dz).is_some_and(|b| {
                    b.block.id.path().ends_with("_leaves")
                        && b.properties
                            .iter()
                            .any(|(k, v)| k == "persistent" && v == "false")
                })
            })
        })
    })
}

fn queue_natural(
    site: &GeneratedVillageSite,
    kind: BuildingKind,
    bounds: ScriptWorldBounds,
    entrance: [i32; 3],
    queued_at: u64,
    candidates: &mut VecDeque<NaturalCandidate>,
) {
    if site
        .pieces
        .iter()
        .filter_map(|piece| piece.bounds)
        .any(|(min, max)| {
            let min = [min.x, min.y, min.z];
            let max = [max.x, max.y, max.z];
            [0, 2]
                .into_iter()
                .all(|axis| min[axis] <= bounds.max[axis] && max[axis] >= bounds.min[axis])
        })
    {
        return;
    }
    candidates.push_back(NaturalCandidate {
        kind,
        bounds,
        entrance,
        queued_at,
        stock_proved: false,
        after_resident: None,
    });
}

fn prove_natural(
    state: &mut TickState,
    settlement: SettlementId,
    site: Option<&GeneratedVillageSite>,
    residents: &BTreeSet<uuid::Uuid>,
    candidates: &mut VecDeque<NaturalCandidate>,
) {
    use super::allocator::{RouteCtx, at, find_path_ctx};
    use crate::tick_budget::Consumer;
    use std::ops::Bound::{Excluded, Unbounded};

    let Some(candidate) = candidates.front() else {
        return;
    };
    if candidate.queued_at >= state.tick {
        return;
    }
    if has_kind(state, settlement, candidate.kind) {
        candidates.pop_front();
        return;
    }
    let [x, y, z] = candidate.entrance;
    // A loaded route can lead beyond entity simulation. Such a job strands
    // its worker before they can take the profession or bring stock home.
    if !state.world.is_active(x.div_euclid(16), z.div_euclid(16)) {
        candidates.pop_front();
        return;
    }
    let after = candidate.after_resident.map_or(Unbounded, Excluded);
    let resident = residents.range((after, Unbounded)).find_map(|uuid| {
        let slot = state.entities.slot_of_uuid(*uuid)?;
        let villager = state.entities.row(slot).villager.as_ref()?;
        let body = state.entities.body(slot);
        (body.alive
            && villager.settlement == Some(settlement)
            && villager.age >= 0
            && ((villager.job_site.is_none()
                && villager.data.profession == crate::villager::VillagerProfession::None)
                || (matches!(
                    villager.data.profession,
                    crate::villager::VillagerProfession::Miner
                        | crate::villager::VillagerProfession::Lumberjack
                ) && !super::allocator::traded(villager))))
        .then(|| {
            (
                *uuid,
                state.entities.pos(slot),
                body.half_width,
                body.height,
            )
        })
    });
    let Some((uuid, start, width, height)) = resident else {
        candidates.pop_front();
        return;
    };
    let stock_proved = candidate.stock_proved;
    let (start, target) = if stock_proved {
        (start, BlockPos { x, y, z })
    } else {
        let Some([sx, sy, sz]) = super::buildings::storage_points(state, settlement)
            .first()
            .copied()
        else {
            return;
        };
        (
            [f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5],
            BlockPos {
                x: sx,
                y: sy,
                z: sz,
            },
        )
    };
    let from = at(start);
    let horizontal = from
        .x
        .abs_diff(target.x)
        .max(from.z.abs_diff(target.z))
        .saturating_add(8)
        .clamp(48, 512);
    let vertical = from.y.abs_diff(target.y).saturating_add(4).clamp(16, 512);
    let mut cache = std::mem::take(&mut state.villager_pois);
    let mut path = std::mem::take(&mut cache.candidate_path);
    let mut ctx = RouteCtx {
        world: &state.world,
        registry: &state.registry,
        dimension: state.dimension,
        consumer: Consumer::Allocator,
        budget: &mut state.tick_budget,
    };
    let admitted = ctx.admit();
    let reached = admitted
        && find_path_ctx(
            &mut ctx, &mut cache, start, target, width, height, &mut path, false, horizontal,
            vertical, true,
        );
    cache.candidate_path = path;
    state.villager_pois = cache;
    if !admitted {
        return;
    }
    if !stock_proved {
        if reached {
            candidates
                .front_mut()
                .expect("proved candidate")
                .stock_proved = true;
        } else {
            candidates.pop_front();
        }
        return;
    }
    if reached {
        let candidate = candidates.pop_front().expect("proved candidate");
        // A natural lumber camp is the grove it stands in, not the one tree the
        // probe found: the zone grows sideways at certification time (CIV-6), and
        // the plain column stays the fallback when the wider rectangle is refused.
        let grown = match candidate.kind {
            BuildingKind::LumberCamp => grove_bounds(state, settlement, site, candidate.bounds),
            _ => candidate.bounds,
        };
        if super::buildings::certify_zone(
            state,
            settlement,
            candidate.kind,
            grown,
            candidate.entrance,
        )
        .is_err()
        {
            let _ = super::buildings::certify_zone(
                state,
                settlement,
                candidate.kind,
                candidate.bounds,
                candidate.entrance,
            );
        }
    } else {
        // A failed bounded route never installs a job. Try another resident on
        // a later pass, then let the existing terrain sweep seek another zone.
        candidates
            .front_mut()
            .expect("unproved candidate")
            .after_resident = Some(uuid);
    }
}

/// The zone a natural lumber camp is certified from: the proved tree's column
/// widened sideways, so the camp's zone is the grove the design describes
/// instead of one trunk (CIV-6; the phase log of 20261124T020538 found certified
/// bounds of `1x8x1`, and a lumberjack's `plant_zone` walk could then only ever
/// cover one tree).
///
/// The grove is a bounded square around the trunk (at most `GROVE_RADIUS`
/// columns per side), not a log census: the tree search already proved this
/// stand is reachable, and a census would walk the world volume on every
/// certification. Each side grows only while the rectangle's corners stay
/// inside the settlement sphere (`buildings::certify_zone`'s own `within`
/// rule), every column of the rectangle is simulated (`bounds_active`, not
/// mere residency), and no certificate or generated village piece already
/// stands there.
///
/// Cost per call: at most `4 * GROVE_RADIUS` = 24 probes, each an O(1) corner
/// test, an O(1) chunk-activity lookup and one clearance scan over `B`
/// certificates and `P` village pieces (`O(B + P)`); no world volume walk and
/// no new per-tick work.
///
/// Certification still owns the remaining rules — overlap with another
/// certificate, at least four zone logs, an open entrance — and `prove_natural`
/// keeps the plain column when the wider rectangle is refused; a grove is an
/// improvement, never a new failure. A natural zone keeps only its own columns
/// against other worksites (`villager::lumber::base_protected` shields the
/// grove from every camp but its own), and the lot planner reserves a natural
/// zone's own footprint only (`construction_site_refusals`).
fn grove_bounds(
    state: &TickState,
    settlement: SettlementId,
    site: Option<&GeneratedVillageSite>,
    bounds: ScriptWorldBounds,
) -> ScriptWorldBounds {
    let Some(record) = state.settlements.get(settlement) else {
        return bounds;
    };
    let inside = |min: [i32; 3], max: [i32; 3]| {
        [min[0], max[0]].into_iter().all(|x| {
            [min[1], max[1]].into_iter().all(|y| {
                [min[2], max[2]].into_iter().all(|z| {
                    [x, y, z]
                        .into_iter()
                        .zip(record.anchor)
                        .map(|(v, a)| (i128::from(v) - i128::from(a)).pow(2))
                        .sum::<i128>()
                        <= i128::from(record.radius).pow(2)
                })
            })
        })
    };
    let loaded = |min: [i32; 3], max: [i32; 3]| {
        ScriptWorldBounds::new(min, max).is_ok_and(|rect| bounds_active(state, &rect))
    };
    // A certificate or a generated village piece already standing there ends
    // the grove on that side (`queue_natural` refuses a tree inside a piece for
    // the same reason: the zone never covers a house); the narrower rectangle
    // still certifies, the plain column still falls back.
    let clear = |min: [i32; 3], max: [i32; 3]| {
        !record.buildings.values().any(|building| {
            (0..3).all(|axis| {
                building.bounds.min[axis] <= max[axis] && building.bounds.max[axis] >= min[axis]
            })
        }) && !site.is_some_and(|site| {
            site.pieces
                .iter()
                .filter_map(|piece| piece.bounds)
                .any(|(lo, hi)| {
                    let lo = [lo.x, lo.y, lo.z];
                    let hi = [hi.x, hi.y, hi.z];
                    [0, 2]
                        .into_iter()
                        .all(|axis| lo[axis] <= max[axis] && hi[axis] >= min[axis])
                })
        })
    };
    let mut min = bounds.min;
    let mut max = bounds.max;
    for axis in [0_usize, 2] {
        for (sign, limit) in [(-1_i32, GROVE_RADIUS), (1, GROVE_RADIUS)] {
            for _ in 0..limit {
                let mut probe = [min, max];
                let index = usize::from(sign > 0);
                probe[index][axis] += sign;
                let (next_min, next_max) = (probe[0], probe[1]);
                if !inside(next_min, next_max)
                    || !loaded(next_min, next_max)
                    || !clear(next_min, next_max)
                {
                    break;
                }
                min = next_min;
                max = next_max;
            }
        }
    }
    ScriptWorldBounds::new(min, max).unwrap_or(bounds)
}

fn open_entrance(
    state: &TickState,
    bounds: &ScriptWorldBounds,
    want: [i32; 3],
) -> Option<[i32; 3]> {
    for offset in 0..=16 {
        for y in [want[1] + offset, want[1] - offset] {
            if y <= bounds.min[1] || y >= bounds.max[1] {
                continue;
            }
            let loaded_open = |y| {
                state
                    .world
                    .block_at(want[0], y, want[2])
                    .is_some_and(|id| state.registry.collision_boxes[id as usize].is_empty())
            };
            if loaded_open(y)
                && loaded_open(y + 1)
                && state
                    .world
                    .block_at(want[0], y - 1, want[2])
                    .is_some_and(|id| !state.registry.collision_boxes[id as usize].is_empty())
            {
                return Some([want[0], y, want[2]]);
            }
        }
    }
    None
}

/// Retry pacing for a refused adopted piece: re-examined once per spacing window,
/// at most PIECE_ATTEMPTS times. A non-retry tick defers at no cost.
const RETRY_SPACING_TICKS: u64 = 1200;

fn should_retry_refusal(retry_tick: bool, attempts: u8) -> bool {
    retry_tick && attempts < PIECE_ATTEMPTS
}

fn block_name(state: &TickState, at: [i32; 3]) -> String {
    state
        .world
        .block_at(at[0], at[1], at[2])
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .map_or_else(
            || "unloaded".to_owned(),
            |block| block.block.id.path().to_owned(),
        )
}

fn bounds_loaded(state: &TickState, bounds: &ScriptWorldBounds) -> bool {
    (bounds.min[0].div_euclid(16)..=bounds.max[0].div_euclid(16)).all(|x| {
        (bounds.min[2].div_euclid(16)..=bounds.max[2].div_euclid(16))
            .all(|z| state.world.has_chunk(x, z))
    })
}

/// Whether every chunk column of `bounds` is *simulated*, not merely resident:
/// a zone column in a resident-but-inactive chunk cannot tick its worker or the
/// blocks it edits, so a grove never grows into one.
fn bounds_active(state: &TickState, bounds: &ScriptWorldBounds) -> bool {
    (bounds.min[0].div_euclid(16)..=bounds.max[0].div_euclid(16)).all(|x| {
        (bounds.min[2].div_euclid(16)..=bounds.max[2].div_euclid(16))
            .all(|z| state.world.is_active(x, z))
    })
}

fn overlaps_existing(state: &TickState, id: SettlementId, bounds: &ScriptWorldBounds) -> bool {
    state.settlements.get(id).is_none_or(|record| {
        record.buildings.values().any(|building| {
            (0..3).all(|axis| {
                building.bounds.min[axis] <= bounds.max[axis]
                    && building.bounds.max[axis] >= bounds.min[axis]
            })
        })
    })
}

/// What one generated piece certifies, or `None` when a column could not be read
/// (the caller retries on a later tick rather than dropping the piece forever).
///
/// A piece that holds both beds and its own workstation certifies as both: a
/// plains library's lectern stands beside its beds, so the House keeps the beds
/// and the Workshop keeps the profession slot.
pub(crate) fn piece_kinds(
    piece: &GeneratedVillagePiece,
    state: &TickState,
    bounds: &ScriptWorldBounds,
) -> Option<Vec<BuildingKind>> {
    let template = piece.template.as_deref()?;
    // A street is never a building. Its bounds frequently graze the beds of the
    // houses it runs past, and the bed probe below then certified the street as a
    // house site (seed-24 listing: streets/straight_02/03/06 and turn_01 all came
    // back as House pieces).
    if template.contains("/streets/") || template.contains("/roads/") {
        return Some(Vec::new());
    }
    if template.contains("farm") || template.contains("field") || template.contains("crop") {
        return Some(vec![BuildingKind::Workshop(
            crate::villager::VillagerProfession::Farmer,
        )]);
    }
    use super::allocator::{PoiKind, poi_kind};
    let mut bed = false;
    let mut job = None;
    for x in bounds.min[0]..=bounds.max[0] {
        for z in bounds.min[2]..=bounds.max[2] {
            for y in bounds.min[1]..=bounds.max[1] {
                let id = state.world.block_at(x, y, z)?;
                let block = state.registry.blocks.by_id(mc_world::BlockStateId(id))?;
                if job.is_none()
                    && let Some(PoiKind::Job(profession)) = poi_kind(&state.registry, id)
                {
                    job = Some(profession);
                }
                bed |= block.block.id.path().ends_with("_bed")
                    && block
                        .properties
                        .iter()
                        .any(|(name, value)| name == "part" && value == "head");
                if bed && job.is_some() {
                    break;
                }
            }
        }
    }
    let mut kinds = Vec::new();
    if bed {
        kinds.push(BuildingKind::House);
    }
    if let Some(profession) = job {
        kinds.push(BuildingKind::Workshop(profession));
    }
    Some(kinds)
}

#[cfg(test)]
#[path = "autonomy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "starter_chest_tests.rs"]
mod starter_tests;
