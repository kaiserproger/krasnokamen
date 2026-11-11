//! Adult sniffers search diggable ground, dig a seed, remember the site and cool down.
use crate::{entities::ColdRow, tick::TickState};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Sniffer {
    pub cooldown: u32,
    pub elapsed: u16,
    pub target: Option<[i32; 3]>,
    pub explored: Vec<[i32; 3]>,
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
        let mut sniffer = state
            .entities
            .row_mut(slot)
            .entity_life
            .sniffer
            .take()
            .unwrap_or_default();
        sniffer.cooldown = sniffer.cooldown.saturating_sub(1);
        if sniffer.cooldown == 0 {
            if let Some(target) = sniffer.target {
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
                        .is_some_and(|at| state.tick.saturating_sub(at) < 100)
                {
                    sniffer.target = None;
                    sniffer.elapsed = 0;
                } else if crate::bees::distance(pos, centre) < 1.0 && state.entities.ground(slot) {
                    sniffer.elapsed += 1;
                    if sniffer.elapsed == 160 {
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
                        sniffer.explored.push(target);
                        if sniffer.explored.len() > 20 {
                            sniffer.explored.remove(0);
                        }
                    }
                    if sniffer.elapsed >= 200 {
                        sniffer.cooldown = 9600;
                        sniffer.elapsed = 0;
                        sniffer.target = None;
                    }
                }
            } else if state.entities.ground(slot) {
                let [x, y, z] = pos.map(|v| v.floor() as i32);
                let mut best = None;
                for dx in -8..=8 {
                    for dz in -8..=8 {
                        let at = [x + dx, y - 1, z + dz];
                        let distance = dx * dx + dz * dz;
                        if !sniffer.explored.contains(&at)
                            && diggable(state, at)
                            && best.is_none_or(|(_, old)| distance < old)
                        {
                            best = Some((at, distance));
                        }
                    }
                }
                sniffer.target = best.map(|(at, _)| at);
            }
        }
        let animation = if sniffer.target.is_none() {
            0
        } else if sniffer.elapsed == 0 {
            4
        } else if sniffer.elapsed <= 40 {
            3
        } else if sniffer.elapsed < 160 {
            5
        } else {
            6
        };
        crate::special_mobs::metadata(
            state,
            slot,
            17,
            crate::boundary::EntityDataValue::SnifferState(animation),
        );
        state.entities.row_mut(slot).entity_life.sniffer = Some(sniffer);
    }
    slots.clear();
    state.serial_entity_slots = slots;
}

pub(crate) fn to_fields(sniffer: &Sniffer) -> Vec<(String, mc_nbt::Tag)> {
    use mc_nbt::{ListTag, Tag, tag_type};
    let mut fields = vec![
        ("Cooldown".into(), Tag::Int(sniffer.cooldown as i32)),
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
            ("Cooldown", Tag::Int(value)) if (0..=9600).contains(value) => {
                sniffer.cooldown = *value as u32
            }
            ("Elapsed", Tag::Short(value)) if (0..200).contains(value) => {
                sniffer.elapsed = *value as u16
            }
            ("Target", Tag::IntArray(value)) if value.len() == 3 => {
                sniffer.target = Some([value[0], value[1], value[2]])
            }
            ("Explored", Tag::List(list))
                if list.element_type == mc_nbt::tag_type::INT_ARRAY
                    && list.elements.len() <= 20 =>
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
    if sniffer.elapsed != 0 && sniffer.target.is_none() {
        return Err(fail());
    }
    Ok(Some(sniffer))
}
