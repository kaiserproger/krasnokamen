//! Adult sniffers search diggable ground, dig a seed, remember the site and cool down.
use crate::{entities::ColdRow, tick::TickState};
use serde::{Deserialize, Serialize};

/// `Sniffer.DATA_STATE`: Entity 0-7, LivingEntity 8-14, Mob 15, AgeableMob 16-17,
/// then the sniffer's own accessors (local javap).
const STATE_INDEX: u8 = 18;
const EXPLORED_LIMIT: usize = 20;
const COOLDOWN_TICKS: u64 = 9600;
/// Ticks between ground scans; each sniffer scans on its own phase.
const SCAN_PERIOD: u64 = 20;
/// A fresh sniffer waits this long plus up to `FIRST_SEARCH_SPREAD` before searching.
const FIRST_SEARCH_DELAY: u64 = 100;
const FIRST_SEARCH_SPREAD: u64 = 100;

/// Persisted dig state. The cooldown is an absolute world-tick deadline so a
/// cooling sniffer is never rewritten while it waits.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SnifferImage")]
pub struct Sniffer {
    pub cooldown_until: u64,
    pub elapsed: u16,
    pub target: Option<[i32; 3]>,
    pub explored: Vec<[i32; 3]>,
}

/// Unvalidated WAL image of [`Sniffer`]; unknown fields are refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SnifferImage {
    cooldown_until: u64,
    elapsed: u16,
    target: Option<[i32; 3]>,
    explored: Vec<[i32; 3]>,
}

impl TryFrom<SnifferImage> for Sniffer {
    type Error = &'static str;

    fn try_from(image: SnifferImage) -> Result<Self, Self::Error> {
        let sniffer = Self {
            cooldown_until: image.cooldown_until,
            elapsed: image.elapsed,
            target: image.target,
            explored: image.explored,
        };
        sniffer.validate()?;
        Ok(sniffer)
    }
}

impl Sniffer {
    /// The one validator for NBT and WAL images.
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.elapsed >= 200 {
            return Err("sniffer elapsed out of range");
        }
        if self.explored.len() > EXPLORED_LIMIT {
            return Err("sniffer explored list too long");
        }
        if self.elapsed != 0 && self.target.is_none() {
            return Err("sniffer is digging without a target");
        }
        Ok(())
    }
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    let sniffer = row.entity_life.sniffer.as_ref()?;
    let [x, y, z] = sniffer.target?;
    Some((
        [f64::from(x) + 0.5, f64::from(y) + 1.0, f64::from(z) + 0.5],
        if sniffer.elapsed > 0 {
            0.0
        } else {
            f64::from(row.speed)
        },
    ))
}

fn diggable(state: &TickState, pos: [i32; 3]) -> bool {
    static TAG: std::sync::LazyLock<(mc_data::Identifier, mc_data::Identifier)> =
        std::sync::LazyLock::new(|| {
            (
                mc_data::Identifier::parse("minecraft:block").unwrap(),
                mc_data::Identifier::parse("minecraft:sniffer_diggable_block").unwrap(),
            )
        });
    let [x, y, z] = pos;
    state
        .world
        .block_at(x, y, z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .is_some_and(|block| {
            state
                .registry
                .blocks
                .block(&block.block.id)
                .is_some_and(|kind| {
                    state
                        .registry
                        .tags
                        .registries
                        .get(&TAG.0)
                        .and_then(|tags| tags.get(&TAG.1))
                        .is_some_and(|ids| ids.contains(&kind.raw_id))
                })
        })
        && state.world.block_at(x, y + 1, z) == Some(state.registry.states.air)
}

/// The row's sniffer state, created on first write only.
fn sniffer_mut(state: &mut TickState, slot: crate::entities::EntitySlot) -> &mut Sniffer {
    state
        .entities
        .row_mut(slot)
        .entity_life
        .sniffer
        .get_or_insert_default()
}

fn animation(sniffer: Option<&Sniffer>) -> u8 {
    match sniffer.and_then(|sniffer| sniffer.target.map(|_| sniffer.elapsed)) {
        None => 0,
        Some(0) => 4,
        Some(1..=40) => 3,
        Some(41..=159) => 5,
        Some(_) => 6,
    }
}

/// Publishes the state accessor only when it changed; the unchanged case reads
/// the row without marking it dirty.
fn publish(state: &mut TickState, slot: crate::entities::EntitySlot, value: u8) {
    let current = state
        .entities
        .row(slot)
        .appearance
        .extra
        .iter()
        .find_map(|(index, data)| match data {
            crate::boundary::EntityDataValue::SnifferState(state) if *index == STATE_INDEX => {
                Some(*state)
            }
            _ => None,
        })
        .unwrap_or(0);
    if current != value {
        crate::special_mobs::metadata(
            state,
            slot,
            STATE_INDEX,
            crate::boundary::EntityDataValue::SnifferState(value),
        );
    }
}

fn nearest_diggable(state: &TickState, pos: [f64; 3], explored: &[[i32; 3]]) -> Option<[i32; 3]> {
    let [x, y, z] = pos.map(|v| v.floor() as i32);
    let mut best = None;
    for dx in -8..=8 {
        for dz in -8..=8 {
            let at = [x + dx, y - 1, z + dz];
            let distance = dx * dx + dz * dz;
            if !explored.contains(&at)
                && diggable(state, at)
                && best.is_none_or(|(_, old)| distance < old)
            {
                best = Some((at, distance));
            }
        }
    }
    best.map(|(at, _)| at)
}

pub(crate) fn run(state: &mut TickState) {
    let Some(kind) = crate::piglin::type_id_of("minecraft:sniffer") else {
        return;
    };
    let mut slots = std::mem::take(&mut state.serial_entity_slots);
    slots.extend(state.entities.mob_slots(kind));
    for &slot in &slots {
        let pos = state.entities.pos(slot);
        let row = state.entities.row(slot);
        if !state.entities.body(slot).alive
            || row.controls.no_ai
            || !crate::entity_life::adult(row)
            || !state.world.is_active(
                (pos[0].floor() as i32).div_euclid(16),
                (pos[2].floor() as i32).div_euclid(16),
            )
        {
            continue;
        }
        let tick = state.tick;
        let phase = row.uuid.as_u128() as u64;
        let stored = row
            .entity_life
            .sniffer
            .as_deref()
            .map(|sniffer| (sniffer.cooldown_until, sniffer.target));
        let (until, target) = stored.unwrap_or_else(|| {
            // A fresh sniffer wanders for a while before its first search
            // instead of digging where it was born.
            let until = tick + FIRST_SEARCH_DELAY + phase % FIRST_SEARCH_SPREAD;
            sniffer_mut(state, slot).cooldown_until = until;
            (until, None)
        });
        if until > tick + COOLDOWN_TICKS {
            // No legitimate writer stores a deadline beyond one cooldown.
            sniffer_mut(state, slot).cooldown_until = tick + COOLDOWN_TICKS;
        }
        if until <= tick {
            if let Some(target) = target {
                let centre = [
                    f64::from(target[0]) + 0.5,
                    f64::from(target[1]) + 1.0,
                    f64::from(target[2]) + 0.5,
                ];
                if !diggable(state, target)
                    || state
                        .entities
                        .body(slot)
                        .hurt_at
                        .is_some_and(|at| tick.saturating_sub(at) < 100)
                {
                    let sniffer = sniffer_mut(state, slot);
                    sniffer.target = None;
                    sniffer.elapsed = 0;
                } else if crate::bees::distance(pos, centre) < 1.0 && state.entities.ground(slot) {
                    let elapsed = {
                        let sniffer = sniffer_mut(state, slot);
                        sniffer.elapsed += 1;
                        sniffer.elapsed
                    };
                    if elapsed == 160 {
                        let item = if crate::taming::roll(state.entities.row_mut(slot), 2) == 0 {
                            "minecraft:torchflower_seeds"
                        } else {
                            "minecraft:pitcher_pod"
                        };
                        let item = state
                            .registry
                            .items
                            .id_of(&mc_data::Identifier::parse(item).unwrap())
                            .expect("sniffer seed item");
                        state
                            .merge_life_drops
                            .push((pos, mc_data::ItemStack::new(item, 1)));
                        let sniffer = sniffer_mut(state, slot);
                        sniffer.explored.push(target);
                        if sniffer.explored.len() > EXPLORED_LIMIT {
                            sniffer.explored.remove(0);
                        }
                    }
                    if elapsed >= 200 {
                        let sniffer = sniffer_mut(state, slot);
                        sniffer.cooldown_until = tick + COOLDOWN_TICKS;
                        sniffer.elapsed = 0;
                        sniffer.target = None;
                    }
                }
            } else if state.entities.ground(slot) && tick.wrapping_add(phase) % SCAN_PERIOD == 0 {
                let explored = state
                    .entities
                    .row(slot)
                    .entity_life
                    .sniffer
                    .as_deref()
                    .map_or(&[][..], |sniffer| &sniffer.explored[..]);
                if let Some(found) = nearest_diggable(state, pos, explored) {
                    sniffer_mut(state, slot).target = Some(found);
                }
            }
        }
        let value = if until > tick {
            0
        } else {
            animation(state.entities.row(slot).entity_life.sniffer.as_deref())
        };
        publish(state, slot, value);
    }
    slots.clear();
    state.serial_entity_slots = slots;
}

pub(crate) fn to_fields(sniffer: &Sniffer) -> Vec<(String, mc_nbt::Tag)> {
    use mc_nbt::{ListTag, Tag, tag_type};
    let mut fields = vec![
        (
            "CooldownUntil".into(),
            Tag::Long(i64::try_from(sniffer.cooldown_until).unwrap_or(i64::MAX)),
        ),
        ("Elapsed".into(), Tag::Short(sniffer.elapsed as i16)),
        (
            "Explored".into(),
            Tag::List(ListTag {
                element_type: tag_type::INT_ARRAY,
                elements: sniffer
                    .explored
                    .iter()
                    .map(|pos| Tag::IntArray(pos.to_vec()))
                    .collect(),
            }),
        ),
    ];
    if let Some(target) = sniffer.target {
        fields.push(("Target".into(), Tag::IntArray(target.to_vec())));
    }
    vec![("solaris:Sniffer".into(), Tag::Compound(fields))]
}

pub(crate) fn from_record(
    record: &mc_data::entity_nbt::VanillaEntity,
) -> Result<Option<Box<Sniffer>>, crate::CoreError> {
    use mc_nbt::Tag;
    let fail = || crate::CoreError::Storage("invalid sniffer state: solaris:Sniffer".into());
    let mut records = record
        .extra_fields
        .iter()
        .filter(|(key, _)| key == "solaris:Sniffer");
    let Some((_, tag)) = records.next() else {
        return Ok(None);
    };
    if records.next().is_some() {
        return Err(fail());
    }
    let Tag::Compound(fields) = tag else {
        return Err(fail());
    };
    let mut sniffer = Box::<Sniffer>::default();
    let mut seen = std::collections::HashSet::new();
    for (key, value) in fields {
        if !seen.insert(key) {
            return Err(fail());
        }
        match (key.as_str(), value) {
            ("CooldownUntil", Tag::Long(value)) if *value >= 0 => {
                sniffer.cooldown_until = *value as u64
            }
            ("Elapsed", Tag::Short(value)) if *value >= 0 => sniffer.elapsed = *value as u16,
            ("Target", Tag::IntArray(value)) if value.len() == 3 => {
                sniffer.target = Some([value[0], value[1], value[2]])
            }
            // An empty list carries no element type worth checking.
            ("Explored", Tag::List(list))
                if list.elements.len() <= EXPLORED_LIMIT
                    && (list.elements.is_empty()
                        || list.element_type == mc_nbt::tag_type::INT_ARRAY) =>
            {
                for value in &list.elements {
                    let Tag::IntArray(pos) = value else {
                        return Err(fail());
                    };
                    if pos.len() != 3 {
                        return Err(fail());
                    }
                    sniffer.explored.push([pos[0], pos[1], pos[2]]);
                }
            }
            _ => return Err(fail()),
        }
    }
    sniffer.validate().map_err(|_| fail())?;
    Ok(Some(sniffer))
}
