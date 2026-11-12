//! Read-only native POI presentation. Labels never enter the entity store/WAL.
use super::SettlementRecord;
use super::allocator::PoiKind;
use crate::boundary::BlockPos;
use crate::tick::TickState;
use crate::villager::VillagerProfession as Profession;
use rustc_hash::FxHashMap;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct SettlementLabel {
    /// The POI/entrance that owns this label; unique within one dimension.
    pub anchor: [i32; 3],
    pub position: [f64; 3],
    pub text: String,
}

#[derive(Default)]
pub(crate) struct Cache {
    // `PoiCache` is taken/restored by each native phase: Default must not allocate.
    current: Option<(u64, Arc<Vec<SettlementLabel>>)>,
    entrances: Entrances,
}

/// Entrance of each POI, valid while the revisions of the chunks around it
/// are unchanged; POIs that left the loaded set drop out at the next rebuild.
#[derive(Default)]
pub(crate) struct Entrances {
    known: FxHashMap<BlockPos, (u64, [i32; 3])>,
}

/// Fixed algorithm/list, independent of process hash seeds and entity wire IDs.
pub(crate) fn villager_name(uuid: uuid::Uuid) -> &'static str {
    const NAMES: [&str; 24] = [
        "Иван",
        "Анна",
        "Пётр",
        "Мария",
        "Алексей",
        "Ольга",
        "Михаил",
        "Елена",
        "Николай",
        "Наталья",
        "Сергей",
        "Татьяна",
        "Дмитрий",
        "Ирина",
        "Андрей",
        "Дарья",
        "Василий",
        "Софья",
        "Павел",
        "Вера",
        "Фёдор",
        "Надежда",
        "Борис",
        "Любовь",
    ];
    let hash = uuid.as_bytes().iter().fold(0_u64, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u64::from(*byte))
    });
    NAMES[(hash % NAMES.len() as u64) as usize]
}

pub(crate) fn profession_name(profession: Profession) -> &'static str {
    match profession {
        Profession::None => "безработный",
        Profession::Armorer => "бронник",
        Profession::Butcher => "мясник",
        Profession::Cartographer => "картограф",
        Profession::Cleric => "священник",
        Profession::Farmer => "фермер",
        Profession::Fisherman => "рыбак",
        Profession::Fletcher => "лучник",
        Profession::Leatherworker => "кожевник",
        Profession::Librarian => "библиотекарь",
        Profession::Mason => "каменщик",
        Profession::Nitwit => "простак",
        Profession::Shepherd => "пастух",
        Profession::Toolsmith => "инструментальщик",
        Profession::Weaponsmith => "оружейник",
        Profession::Miner => "шахтёр",
        Profession::Lumberjack => "лесоруб",
        Profession::Builder => "строитель",
        Profession::Soldier => "солдат",
    }
}

fn employed(villager: &crate::villager::VillagerState, dimension: &str) -> bool {
    villager.job_site.is_some()
        && villager.job_site_dimension == dimension
        && !matches!(
            villager.data.profession,
            Profession::None | Profession::Nitwit
        )
}

fn owner_display<'a>(state: &'a TickState, record: &'a SettlementRecord) -> &'a str {
    let Some(owner) = record.owner.as_deref() else {
        return "нет (автономное)";
    };
    let Ok(uuid) = uuid::Uuid::parse_str(owner) else {
        return owner;
    };
    for world in std::iter::once(state).chain(state.other_dimensions.values().map(|world| &**world))
    {
        for connection in world.connections.all_connections() {
            if world.connections.uuid_of(connection) == Some(uuid)
                && let Some(entity) = world.connections.entity_of(connection)
                && let Some(name) = world.connections.name_of(entity)
            {
                return name;
            }
        }
    }
    state
        .dimension_globals
        .roster
        .iter()
        .find(|player| player.uuid == uuid)
        .map_or(record.owner_name.as_deref().unwrap_or(owner), |player| {
            player.name.as_str()
        })
}

/// Same words and counts in chat and above the settlement's bell.
pub(crate) fn summary(state: &TickState, record: &SettlementRecord) -> String {
    let mut residents = 0;
    let mut workers = 0;
    for slots in state.entities.villager_chunks().values() {
        for &id in slots {
            let Some(slot) = state.entities.slot_of(id) else {
                continue;
            };
            if !state.entities.body(slot).alive {
                continue;
            }
            let Some(villager) = state.entities.row(slot).villager.as_ref() else {
                continue;
            };
            if villager.settlement == Some(record.id) {
                residents += 1;
                workers += usize::from(employed(villager, state.dimension.name()));
            }
        }
    }
    format!(
        "«{}» · глава: {} · жителей {} · работают {} · безработных {}",
        record.name,
        owner_display(state, record),
        residents,
        workers,
        residents - workers,
    )
}

fn coordinates(pos: BlockPos) -> [i32; 3] {
    [pos.x, pos.y, pos.z]
}

/// Hash only inputs which can change a label, never the simulation clock or
/// motion. Unchanged snapshots share both strings and label allocation.
fn key(state: &TickState) -> u64 {
    let mut key = std::collections::hash_map::DefaultHasher::new();
    for (pos, chunk) in &state.villager_pois.chunks {
        if chunk.pois.is_empty() {
            continue;
        }
        pos.hash(&mut key);
        chunk.revision.hash(&mut key);
        // Include neighboring chunk changes: a doorway may straddle a border.
        for x in pos.x - 1..=pos.x + 1 {
            for z in pos.z - 1..=pos.z + 1 {
                state.world.chunk_revision(x, z).hash(&mut key);
            }
        }
        for poi in &chunk.pois {
            coordinates(poi.pos).hash(&mut key);
            match poi.kind {
                PoiKind::Home => 0_i32,
                PoiKind::Bell => 1,
                PoiKind::Job(profession) => 2 + profession as i32,
            }
            .hash(&mut key);
        }
    }
    for record in state.settlements.records() {
        record.id.hash(&mut key);
        record.name.hash(&mut key);
        record.owner.hash(&mut key);
        owner_display(state, record).hash(&mut key);
        record.anchor.hash(&mut key);
        record.radius.hash(&mut key);
    }
    let mut residents = 0_u64;
    for slots in state.entities.villager_chunks().values() {
        for &id in slots {
            let Some(slot) = state.entities.slot_of(id) else {
                continue;
            };
            let body = state.entities.body(slot);
            let row = state.entities.row(slot);
            let Some(villager) = row.villager.as_ref() else {
                continue;
            };
            let mut resident = std::collections::hash_map::DefaultHasher::new();
            row.uuid.hash(&mut resident);
            body.alive.hash(&mut resident);
            villager.settlement.hash(&mut resident);
            villager.home.map(coordinates).hash(&mut resident);
            villager.job_site.map(coordinates).hash(&mut resident);
            villager.home_dimension.hash(&mut resident);
            villager.job_site_dimension.hash(&mut resident);
            villager.data.profession.hash(&mut resident);
            residents ^= resident.finish();
        }
    }
    residents.hash(&mut key);
    key.finish()
}

/// POIs do not carry building bounds. The nearest lower door in the bounded
/// loaded neighborhood is the entrance; outdoor worksites stay at their POI.
fn entrance(
    state: &TickState,
    doors: &mut crate::world::SectionMatchCache,
    poi: BlockPos,
) -> [i32; 3] {
    // A 17x5x17 box scan costs 1445 block reads per POI; section palettes
    // prove the common door-less neighbourhood (pens, farms) without them.
    // Overlay and missing columns fail open to the exact scan below.
    let box_min = (
        poi.x.saturating_sub(8),
        poi.y.saturating_sub(2),
        poi.z.saturating_sub(8),
    );
    let box_max = (
        poi.x.saturating_add(8),
        poi.y.saturating_add(2),
        poi.z.saturating_add(8),
    );
    if !state
        .world
        .may_hold_matching_state_cached(box_min, box_max, doors, |id| {
            state
                .registry
                .blocks
                .by_id(id)
                .is_some_and(|block| block.block.id.path().ends_with("_door"))
        })
    {
        return coordinates(poi);
    }
    let mut nearest = None;
    for x in poi.x.saturating_sub(8)..=poi.x.saturating_add(8) {
        for z in poi.z.saturating_sub(8)..=poi.z.saturating_add(8) {
            for y in poi.y.saturating_sub(2)..=poi.y.saturating_add(2) {
                let Some(id) = state.world.block_at(x, y, z) else {
                    continue;
                };
                let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
                    continue;
                };
                if !block.block.id.path().ends_with("_door")
                    || !block
                        .properties
                        .iter()
                        .any(|(key, value)| key == "half" && value == "lower")
                {
                    continue;
                }
                let distance = (i64::from(x) - i64::from(poi.x)).pow(2)
                    + (i64::from(y) - i64::from(poi.y)).pow(2)
                    + (i64::from(z) - i64::from(poi.z)).pow(2);
                let candidate = (distance, [x, y, z]);
                if nearest.is_none_or(|old| candidate < old) {
                    nearest = Some(candidate);
                }
            }
        }
    }
    nearest.map_or(coordinates(poi), |(_, at)| at)
}

fn label_position(state: &TickState, anchor: [i32; 3], source: [i32; 3]) -> [f64; 3] {
    let mut position = [
        f64::from(anchor[0]) + 0.5,
        f64::from(anchor[1]) + 2.6,
        f64::from(anchor[2]) + 0.5,
    ];
    let Some(id) = state.world.block_at(anchor[0], anchor[1], anchor[2]) else {
        return position;
    };
    let Some(block) = state.registry.blocks.by_id(mc_world::BlockStateId(id)) else {
        return position;
    };
    if !block.block.id.path().ends_with("_door") {
        return position;
    }
    let axis = match block
        .properties
        .iter()
        .find(|(key, _)| key == "facing")
        .map(|(_, value)| value.as_str())
    {
        Some("east" | "west") => 0,
        Some("north" | "south") => 2,
        _ => return position,
    };
    // Outside the house, beyond a one-block eave rather than inside its lintel.
    position[axis] += if source[axis] >= anchor[axis] {
        -1.75
    } else {
        1.75
    };
    position
}

#[derive(Default)]
struct Building {
    // A home's interior is a stronger outside-direction hint than a nearby job.
    source: Option<(bool, [i32; 3])>,
    homes: usize,
    occupants: usize,
    jobs: Vec<(Profession, Vec<&'static str>)>,
    bell: Option<String>,
}

/// Revisions of the 3x3 chunks around `chunk`: everything `entrance` can read
/// (its box reaches 8 blocks, so never beyond the neighbouring chunk).
fn neighbourhood_revision(state: &TickState, chunk: crate::boundary::ChunkPos) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    for x in chunk.x - 1..=chunk.x + 1 {
        for z in chunk.z - 1..=chunk.z + 1 {
            state.world.chunk_revision(x, z).hash(&mut hash);
        }
    }
    hash.finish()
}

fn labels(state: &TickState, entrances: &mut Entrances) -> Vec<SettlementLabel> {
    let occupancy = occupancy(state);
    let mut doors = crate::world::SectionMatchCache::default();
    doors.reset(&state.world);
    let mut known = std::mem::take(&mut entrances.known);
    let mut buildings: BTreeMap<[i32; 3], Building> = BTreeMap::new();
    for (&chunk_pos, chunk) in &state.villager_pois.chunks {
        let revision = neighbourhood_revision(state, chunk_pos);
        for poi in &chunk.pois {
            let anchor = if poi.kind == PoiKind::Bell {
                coordinates(poi.pos)
            } else {
                let anchor = match known.remove(&poi.pos) {
                    Some((seen, anchor)) if seen == revision => anchor,
                    _ => entrance(state, &mut doors, poi.pos),
                };
                entrances.known.insert(poi.pos, (revision, anchor));
                anchor
            };
            let building = buildings.entry(anchor).or_default();
            let source = (poi.kind != PoiKind::Home, coordinates(poi.pos));
            if building.source.is_none_or(|old| source < old) {
                building.source = Some(source);
            }
            match poi.kind {
                PoiKind::Home => {
                    building.homes += 1;
                    building.occupants += occupancy.homes.get(&poi.pos).copied().unwrap_or(0);
                }
                PoiKind::Job(profession) => {
                    let mut names: Vec<_> = occupancy
                        .jobs
                        .get(&poi.pos)
                        .into_iter()
                        .flatten()
                        .copied()
                        .map(villager_name)
                        .collect();
                    names.sort_unstable();
                    building.jobs.push((profession, names));
                }
                PoiKind::Bell => {
                    let record = state
                        .settlements
                        .records()
                        .filter(|record| {
                            anchor
                                .iter()
                                .zip(record.anchor)
                                .map(|(at, centre)| (i64::from(*at) - i64::from(centre)).pow(2))
                                .sum::<i64>()
                                <= i64::from(record.radius).pow(2)
                        })
                        .min_by_key(|record| {
                            (
                                anchor
                                    .iter()
                                    .zip(record.anchor)
                                    .map(|(at, centre)| (i64::from(*at) - i64::from(centre)).pow(2))
                                    .sum::<i64>(),
                                record.id,
                            )
                        });
                    building.bell = Some(record.map_or_else(
                        || "Деревня (без хозяина) · /settlement create <name>".into(),
                        |record| summary(state, record),
                    ));
                }
            }
        }
    }
    buildings
        .into_iter()
        .map(|(anchor, mut building)| {
            let mut lines = Vec::new();
            if building.homes != 0 {
                lines.push(format!(
                    "Дом · жильцов {}/{}",
                    building.occupants, building.homes
                ));
            }
            building
                .jobs
                .sort_by(|(left, left_names), (right, right_names)| {
                    (*left as u8)
                        .cmp(&(*right as u8))
                        .then_with(|| left_names.cmp(right_names))
                });
            for (profession, names) in building.jobs {
                let title = if profession == Profession::Farmer {
                    "Ферма"
                } else {
                    "Мастерская"
                };
                lines.push(format!(
                    "{title} · {} {}",
                    profession_name(profession),
                    if names.is_empty() {
                        "нет".into()
                    } else {
                        names.join(", ")
                    }
                ));
            }
            if let Some(bell) = building.bell {
                lines.push(bell);
            }
            SettlementLabel {
                anchor,
                position: label_position(
                    state,
                    anchor,
                    building.source.map_or(anchor, |(_, source)| source),
                ),
                text: lines.join("\n"),
            }
        })
        .collect()
}

/// Residents by claimed home / job site, built in ONE pass over the villagers
/// per rebuild. A scan of every villager per POI made a rebuild
/// `loaded POIs x all villagers` (2.8 s ticks at 15k villagers).
#[derive(Default)]
struct Occupancy {
    homes: FxHashMap<BlockPos, usize>,
    jobs: FxHashMap<BlockPos, Vec<uuid::Uuid>>,
}

fn occupancy(state: &TickState) -> Occupancy {
    let mut occupancy = Occupancy::default();
    let dimension = state.dimension.name();
    for &id in state.entities.villager_chunks().values().flatten() {
        let Some(slot) = state.entities.slot_of(id) else {
            continue;
        };
        let Some(villager) = state.entities.row(slot).villager.as_ref() else {
            continue;
        };
        if !state.entities.body(slot).alive {
            continue;
        }
        if let Some(home) = villager.home
            && villager.home_dimension == dimension
        {
            *occupancy.homes.entry(home).or_default() += 1;
        }
        if let Some(job) = villager.job_site
            && villager.job_site_dimension == dimension
        {
            occupancy
                .jobs
                .entry(job)
                .or_default()
                .push(state.entities.row(slot).uuid);
        }
    }
    occupancy
}

pub(crate) fn snapshot(state: &mut TickState) -> Arc<Vec<SettlementLabel>> {
    let key = key(state);
    if state
        .villager_pois
        .presentation
        .current
        .as_ref()
        .is_none_or(|(old, _)| *old != key)
    {
        let mut entrances = std::mem::take(&mut state.villager_pois.presentation.entrances);
        let labels = Arc::new(labels(state, &mut entrances));
        state.villager_pois.presentation.entrances = entrances;
        state.villager_pois.presentation.current = Some((key, labels));
    }
    Arc::clone(
        &state
            .villager_pois
            .presentation
            .current
            .as_ref()
            .expect("initialized label snapshot")
            .1,
    )
}

#[cfg(test)]
#[path = "presentation_tests.rs"]
mod tests;
