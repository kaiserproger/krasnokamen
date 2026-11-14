//! Adult sniffer search, digging animation and seed custody.
use crate::boundary::EntityDataValue;
use crate::entities::{ColdRow, EntitySlot};
use crate::tick::TickState;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

static TYPE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    crate::piglin::type_id_of("minecraft:sniffer").expect("sniffer type")
});

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct Sniffer {
    pub cooldown: u32,
    pub explored: Vec<[i32; 3]>,
    phase: u8,
    elapsed: u16,
    target: Option<[i32; 3]>,
}
pub(crate) fn fields(sniffer: &Sniffer) -> Vec<(String, mc_nbt::Tag)> {
    use mc_nbt::Tag;
    let mut out = vec![
        (
            "krasnokamen:SnifferCooldown".into(),
            Tag::Int(sniffer.cooldown.min(i32::MAX as u32) as i32),
        ),
        (
            "krasnokamen:SnifferPhase".into(),
            Tag::Byte(sniffer.phase as i8),
        ),
        (
            "krasnokamen:SnifferElapsed".into(),
            Tag::Int(i32::from(sniffer.elapsed)),
        ),
        (
            "krasnokamen:SnifferExplored".into(),
            Tag::IntArray(sniffer.explored.iter().flatten().copied().collect()),
        ),
    ];
    if let Some(target) = sniffer.target {
        out.push((
            "krasnokamen:SnifferTarget".into(),
            Tag::IntArray(target.to_vec()),
        ));
    }
    out
}

pub(crate) fn validate_fields(fields: &[(String, mc_nbt::Tag)]) -> Result<(), crate::CoreError> {
    use mc_nbt::Tag;
    let keys = [
        "krasnokamen:SnifferCooldown",
        "krasnokamen:SnifferPhase",
        "krasnokamen:SnifferElapsed",
        "krasnokamen:SnifferTarget",
        "krasnokamen:SnifferExplored",
    ];
    let mut phase = 0;
    let mut target = false;
    let mut required = 0u8;
    for key in keys {
        let mut values = fields
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value);
        let Some(value) = values.next() else {
            continue;
        };
        if key != "krasnokamen:SnifferTarget" {
            required += 1;
        }
        let valid = match (key, value) {
            ("krasnokamen:SnifferCooldown", Tag::Int(value)) => (0..=9600).contains(value),
            ("krasnokamen:SnifferPhase", Tag::Byte(value)) => {
                phase = *value;
                matches!(*value, 0 | 3 | 4 | 5 | 6)
            }
            ("krasnokamen:SnifferElapsed", Tag::Int(value)) => {
                (0..=i32::from(u16::MAX)).contains(value)
            }
            ("krasnokamen:SnifferTarget", Tag::IntArray(value)) => {
                target = true;
                value.len() == 3
            }
            ("krasnokamen:SnifferExplored", Tag::IntArray(value)) => {
                value.len() <= 60 && value.len().is_multiple_of(3)
            }
            _ => false,
        };
        if !valid || values.next().is_some() {
            return Err(crate::CoreError::Storage(format!(
                "invalid sniffer field {key}"
            )));
        }
    }
    if required != 0 && required != 4 || target && required != 4 {
        return Err(crate::CoreError::Storage("incomplete sniffer state".into()));
    }
    if phase != 0 && !target {
        return Err(crate::CoreError::Storage(
            "invalid sniffer phase without target".into(),
        ));
    }
    Ok(())
}

pub(crate) fn from_fields(fields: &[(String, mc_nbt::Tag)]) -> Option<Box<Sniffer>> {
    use mc_nbt::Tag;
    let mut sniffer = Sniffer::default();
    let mut found = false;
    for (key, value) in fields {
        match (key.as_str(), value) {
            ("krasnokamen:SnifferCooldown", Tag::Int(value)) if *value >= 0 => {
                sniffer.cooldown = *value as u32;
                found = true;
            }
            ("krasnokamen:SnifferPhase", Tag::Byte(value))
                if matches!(*value, 0 | 3 | 4 | 5 | 6) =>
            {
                sniffer.phase = *value as u8
            }
            ("krasnokamen:SnifferElapsed", Tag::Int(value))
                if (0..=i32::from(u16::MAX)).contains(value) =>
            {
                sniffer.elapsed = *value as u16
            }
            ("krasnokamen:SnifferTarget", Tag::IntArray(value)) if value.len() == 3 => {
                sniffer.target = Some([value[0], value[1], value[2]])
            }
            ("krasnokamen:SnifferExplored", Tag::IntArray(value))
                if value.len() <= 60 && value.len().is_multiple_of(3) =>
            {
                sniffer.explored = value.chunks_exact(3).map(|v| [v[0], v[1], v[2]]).collect();
            }
            _ => {}
        }
    }
    if sniffer.phase != 0 && sniffer.target.is_none() {
        sniffer.phase = 0;
        sniffer.elapsed = 0;
    }
    found.then_some(Box::new(sniffer))
}
pub(crate) fn restore_appearance(row: &mut ColdRow) {
    let sniffer = row.entity_life.sniffer.as_deref();
    let phase = sniffer.map_or(0, |sniffer| sniffer.phase);
    let remaining = sniffer
        .filter(|sniffer| sniffer.phase == 5 && sniffer.elapsed < 120)
        .map_or(0, |sniffer| u32::from(120 - sniffer.elapsed));
    let seed_at = if remaining == 0 {
        0
    } else {
        row.age.saturating_add(remaining).min(i32::MAX as u32) as i32
    };
    let appearance = Arc::make_mut(&mut row.appearance);
    appearance
        .extra
        .retain(|(index, _)| *index != 18 && *index != 19);
    appearance
        .extra
        .push((18, EntityDataValue::SnifferState(phase)));
    appearance
        .extra
        .push((19, EntityDataValue::VarInt(seed_at)));
}

pub(crate) fn is_sniffer(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    let sniffer = row.entity_life.sniffer.as_deref()?;
    match sniffer.phase {
        3 | 5 | 6 => sniffer.target.map(|target| {
            (
                [
                    f64::from(target[0]) + 0.5,
                    f64::from(target[1]) + 1.0,
                    f64::from(target[2]) + 0.5,
                ],
                0.0,
            )
        }),
        4 => sniffer.target.map(|target| {
            (
                [
                    f64::from(target[0]) + 0.5,
                    f64::from(target[1]) + 1.0,
                    f64::from(target[2]) + 0.5,
                ],
                f64::from(row.speed) * 1.25,
            )
        }),
        _ => None,
    }
}

fn diggable(state: &TickState, pos: [i32; 3], explored: &[[i32; 3]]) -> bool {
    if explored.contains(&pos) {
        return false;
    }
    state
        .world
        .block_at(pos[0], pos[1], pos[2])
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
        .is_some_and(|block| {
            state.registry.tags.contains_raw_id(
                "minecraft:block",
                "minecraft:sniffer_diggable_block",
                block.block.raw_id,
            )
        })
        && state.world.block_at(pos[0], pos[1] + 1, pos[2]) == Some(state.registry.states.air)
}

fn find_site(state: &TickState, slot: EntitySlot, explored: &[[i32; 3]]) -> Option<[i32; 3]> {
    let at = state.entities.pos(slot);
    let origin = at.map(|v| v.floor() as i32);
    let mut best = None;
    for x in origin[0] - 8..=origin[0] + 8 {
        for z in origin[2] - 8..=origin[2] + 8 {
            for y in origin[1] - 3..=origin[1] + 2 {
                let pos = [x, y, z];
                if !diggable(state, pos, explored) {
                    continue;
                }
                let distance = (f64::from(x) + 0.5 - at[0]).powi(2)
                    + (f64::from(y + 1) - at[1]).powi(2)
                    + (f64::from(z) + 0.5 - at[2]).powi(2);
                if best.is_none_or(|(_, old)| distance < old)
                    && crate::hostile::sight_clear(
                        state,
                        [at[0], at[1] + 0.5, at[2]],
                        [f64::from(x) + 0.5, f64::from(y) + 1.5, f64::from(z) + 0.5],
                    )
                {
                    best = Some((pos, distance));
                }
            }
        }
    }
    best.map(|(pos, _)| pos)
}

fn metadata(state: &mut TickState, slot: EntitySlot, phase: u8, seed_at: i32) {
    let row = state.entities.row_mut(slot);
    let appearance = Arc::make_mut(&mut row.appearance);
    let mut changed = false;
    for (index, value) in [
        (18, EntityDataValue::SnifferState(phase)),
        (19, EntityDataValue::VarInt(seed_at)),
    ] {
        if let Some((_, old)) = appearance.extra.iter_mut().find(|(i, _)| *i == index) {
            if *old != value {
                *old = value;
                changed = true;
            }
        } else {
            appearance.extra.push((index, value));
            changed = true;
        }
    }
    if changed {
        state.appearance_changed.push(state.entities.body(slot).id);
    }
}

fn drop_seed(state: &mut TickState, slot: EntitySlot) {
    let id =
        mc_data::Identifier::parse("minecraft:gameplay/sniffer_digging").expect("sniffer loot id");
    let seed = state.tick ^ state.entities.row(slot).uuid.as_u128() as u64;
    let Some(table) = state.registry.chest_loot.get(&id) else {
        return;
    };
    let Ok(stacks) = table.roll(
        &mut mc_data::loot::chest_26_1_2::ChestRng::new(seed),
        &state.registry.items,
        0.0,
    ) else {
        return;
    };
    let at = state.entities.pos(slot);
    let yaw = f64::from(state.entities.rot(slot)[0]).to_radians();
    let head = [
        at[0] - f64::from(mc_physics::mth::sin(yaw)) * 1.0,
        at[1],
        at[2] + f64::from(mc_physics::mth::cos(yaw)) * 1.0,
    ];
    state
        .merge_life_drops
        .extend(stacks.into_iter().map(|stack| (head, stack)));
}

pub(crate) fn run(state: &mut TickState) {
    let slots: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in slots {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let eligible = body.alive
            && !row.controls.no_ai
            && crate::entity_life::adult(row)
            && row.entity_life.mount.vehicle.is_none()
            && row.entity_life.leash.is_none()
            && row.love == 0
            && !row
                .last_player_hit
                .is_some_and(|(_, tick)| state.tick.saturating_sub(tick) < 100)
            && !crate::conduit::water(state, pos);
        let id = body.id;
        let mut sniff = state
            .entities
            .row_mut(slot)
            .entity_life
            .sniffer
            .take()
            .unwrap_or_default();
        if sniff.cooldown > 0 {
            sniff.cooldown -= 1;
        }
        if !eligible {
            if sniff.phase != 0 {
                sniff.phase = 0;
                sniff.target = None;
                metadata(state, slot, 0, 0);
            }
            state.entities.row_mut(slot).entity_life.sniffer = Some(sniff);
            continue;
        }
        if sniff.phase == 5
            && (!state.entities.ground(slot)
                || state.entities.row(slot).love > 0
                || crate::conduit::water(state, pos)
                || sniff
                    .target
                    .is_none_or(|target| !diggable(state, target, &sniff.explored)))
        {
            sniff.phase = 0;
            sniff.elapsed = 0;
            sniff.target = None;
            metadata(state, slot, 0, 0);
            state.entities.row_mut(slot).entity_life.sniffer = Some(sniff);
            continue;
        }
        if sniff.phase != 0 {
            sniff.elapsed = sniff.elapsed.saturating_add(1);
        }
        match sniff.phase {
            0 if sniff.cooldown == 0
                && state.entities.ground(slot)
                && state.tick.wrapping_add(id.0 as u64).is_multiple_of(200) =>
            {
                if let Some(target) = find_site(state, slot, &sniff.explored) {
                    sniff.target = Some(target);
                    sniff.phase = 3;
                    sniff.elapsed = 0;
                    metadata(state, slot, 3, 0);
                }
            }
            3 if sniff.elapsed >= 60 => {
                sniff.phase = 4;
                sniff.elapsed = 0;
                metadata(state, slot, 4, 0);
            }
            4 => {
                let Some(target) = sniff.target else {
                    sniff.phase = 0;
                    metadata(state, slot, 0, 0);
                    state.entities.row_mut(slot).entity_life.sniffer = Some(sniff);
                    continue;
                };
                let distance = (pos[0] - f64::from(target[0]) - 0.5).powi(2)
                    + (pos[2] - f64::from(target[2]) - 0.5).powi(2);
                if !diggable(state, target, &sniff.explored) || sniff.elapsed > 600 {
                    sniff.phase = 0;
                    sniff.target = None;
                    metadata(state, slot, 0, 0);
                } else if distance < 2.25 && state.entities.ground(slot) {
                    sniff.phase = 5;
                    sniff.elapsed = 0;
                    let tick = state
                        .entities
                        .row(slot)
                        .age
                        .saturating_add(120)
                        .min(i32::MAX as u32) as i32;
                    metadata(state, slot, 5, tick);
                    crate::special_mobs::event(state, id, 63);
                }
            }
            5 if sniff.elapsed == 120 => drop_seed(state, slot),
            5 if sniff.elapsed >= 170 => {
                if let Some(target) = sniff.target {
                    if sniff.explored.len() == 20 {
                        sniff.explored.remove(0);
                    }
                    sniff.explored.push(target);
                }
                sniff.cooldown = 9600;
                sniff.phase = 6;
                sniff.elapsed = 0;
                metadata(state, slot, 6, 0);
            }
            6 if sniff.elapsed >= 40 => {
                sniff.phase = 0;
                sniff.elapsed = 0;
                sniff.target = None;
                metadata(state, slot, 0, 0);
            }
            _ => {}
        }
        state.entities.row_mut(slot).entity_life.sniffer = Some(sniff);
    }
}

#[cfg(test)]
#[path = "sniffer_tests.rs"]
mod tests;
