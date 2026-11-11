//! Operator-owned deterministic battle staging. No scenario file is read on the tick.
use crate::battle::formation::{slot_position, Order, RegimentSpec};
use crate::boundary::{ChangeSet, ChunkPos, EntityId};
use crate::commands::CommandError;
use crate::tick::TickState;
use mc_nbt::{tag_type, ListTag, Tag};

const OWNER: &str = "solaris:battle";
const TICKET_RADIUS: i32 = 12;
const MAX_SIDE: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub enum BattleCommand {
    Setup { seed: u64, count: usize },
    Start,
    Pause,
    Reset,
    Status,
    Roster,
}

pub fn parse(args: &[&str]) -> Result<BattleCommand, CommandError> {
    let usage = || CommandError::Usage("battle setup <seed> <field-100|field-1k|field-10k> [count] | battle start|pause|reset|status|roster");
    match args {
        ["setup", seed, preset, rest @ ..] if rest.len() <= 1 => {
            let seed = seed.parse::<u64>().map_err(|_| usage())?;
            let default = match *preset { "field-100" => 100, "field-1k" => 1_000, "field-10k" => 10_000, _ => return Err(usage()) };
            let count = match rest.first() { Some(n) => n.parse::<usize>().map_err(|_| usage())?, None => default };
            if !(1..=MAX_SIDE).contains(&count) { return Err(usage()); }
            Ok(BattleCommand::Setup { seed, count })
        }
        ["start"] => Ok(BattleCommand::Start),
        ["pause"] => Ok(BattleCommand::Pause),
        ["reset"] => Ok(BattleCommand::Reset),
        ["status"] => Ok(BattleCommand::Status),
        ["roster"] => Ok(BattleCommand::Roster),
        _ => Err(usage()),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind { Infantry, Spear, Archer, Cavalry }

// One table owns the vanilla equipment until the asset slice replaces it.
fn equipment(kind: Kind) -> &'static [(&'static str, &'static str)] {
    match kind {
        Kind::Infantry => &[("mainhand", "minecraft:iron_sword"), ("offhand", "minecraft:shield"), ("head", "minecraft:iron_helmet"), ("chest", "minecraft:iron_chestplate")],
        Kind::Spear => &[("mainhand", "minecraft:trident"), ("offhand", "minecraft:shield"), ("head", "minecraft:iron_helmet"), ("chest", "minecraft:chainmail_chestplate")],
        Kind::Archer => &[("mainhand", "minecraft:bow"), ("head", "minecraft:iron_helmet"), ("chest", "minecraft:leather_chestplate")],
        Kind::Cavalry => &[("mainhand", "minecraft:iron_sword"), ("offhand", "minecraft:shield"), ("head", "minecraft:iron_helmet"), ("chest", "minecraft:iron_chestplate")],
    }
}

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn kind(seed: u64, side: u16, index: usize) -> Kind {
    match mix(seed ^ ((side as u64) << 32) ^ index as u64) % 100 {
        0..=49 => Kind::Infantry,
        50..=69 => Kind::Spear,
        70..=89 => Kind::Archer,
        _ => Kind::Cavalry,
    }
}

fn tag_bytes() -> Vec<u8> {
    let mut bytes = Vec::new();
    let tag = Tag::Compound(vec![("Tags".into(), Tag::List(ListTag {
        element_type: tag_type::STRING, elements: vec![Tag::String(OWNER.into())],
    }))]);
    mc_nbt::write_named(&mut bytes, "", &tag).expect("writing battle owner tag to memory");
    bytes
}

fn owned(bytes: &[u8]) -> bool {
    let mut input = bytes;
    let Ok((_, Tag::Compound(fields))) = mc_nbt::read_named(&mut input) else { return false };
    fields.iter().any(|(name, value)| name == "Tags" && matches!(value, Tag::List(list) if list.elements.iter().any(|tag| matches!(tag, Tag::String(name) if name == OWNER))))
}

fn tagged_ids(state: &TickState) -> Vec<EntityId> {
    state.entities.slots().filter_map(|slot| {
        owned(&state.entities.row(slot).opaque_entity_fields).then_some(state.entities.body(slot).id)
    }).collect()
}

fn reset(state: &mut TickState, changes: &mut ChangeSet) -> usize {
    let ids = tagged_ids(state);
    for id in &ids {
        if let Some(slot) = state.entities.slot_of_any(*id) {
            state.remove_entity(slot);
            changes.entities_removed.push(*id);
        }
    }
    state.tickets.remove(OWNER);
    state.hostile.battle = Default::default();
    ids.len()
}

fn summon_tag(kind: Option<Kind>) -> Tag {
    let mut fields = vec![("PersistenceRequired".into(), Tag::Byte(1))];
    if let Some(kind) = kind {
        let slots = equipment(kind).iter().map(|(slot, item)| {
            ((*slot).into(), Tag::Compound(vec![("id".into(), Tag::String((*item).into()))]))
        }).collect();
        fields.push(("equipment".into(), Tag::Compound(slots)));
    }
    Tag::Compound(fields)
}

fn spawn(state: &mut TickState, changes: &mut ChangeSet, name: &str, position: [f64; 3], unit: Option<Kind>, tag: &[u8]) -> Result<EntityId, String> {
    let identifier = mc_data::Identifier::parse(name.to_owned()).map_err(|e| e.to_string())?;
    let facts = state.registry.entity_types.facts_of(&identifier).ok_or_else(|| format!("unknown battle entity {name}"))?;
    let type_id = facts.protocol_id;
    let image = crate::command_summon::prepare(state, type_id, name, position, &summon_tag(unit)).map_err(|e| e.message())?;
    let health = state.registry.blueprint(type_id).map_or(20.0, |b| b.params.max_health);
    let id = state.spawn_mob(type_id, position, health);
    crate::command_summon::apply(state, id, image);
    let slot = state.entities.slot_of(id).expect("spawned battle entity");
    let row = state.entities.row_mut(slot);
    row.opaque_entity_fields = tag.to_vec();
    if unit == Some(Kind::Archer) { row.attack_reach = 220.0; }
    changes.entities_spawned.push(id);
    Ok(id)
}

fn setup(state: &mut TickState, changes: &mut ChangeSet, origin: [f64; 3], seed: u64, count: usize) -> Result<(), String> {
    if !tagged_ids(state).is_empty() || state.tickets.contains_key(OWNER) {
        return Err("battle already staged; reset it first".into());
    }
    if !origin.iter().all(|n| n.is_finite()) || origin[0].abs() > 29_999_000.0 || origin[2].abs() > 29_999_000.0 {
        return Err("battle origin outside safe world bounds".into());
    }
    let tag = tag_bytes();
    let centre = ChunkPos { x: (origin[0].floor() as i32).div_euclid(16), z: (origin[2].floor() as i32).div_euclid(16) };
    let spawned_before = changes.entities_spawned.len();
    let removed_before = changes.entities_removed.len();
    state.tickets.insert(OWNER.into(), (centre, TICKET_RADIUS));
    let result = (|| {
        let regiment_count = count.div_ceil(1000);
        for side in 1..=2_u16 {
            let facing = if side == 1 { [0.0, 1.0] } else { [0.0, -1.0] };
            for regiment in 0..regiment_count {
                let size = (count - regiment * 1000).min(1000);
                let files = size.min(20) as u16;
                let x = origin[0] + (regiment as f64 - (regiment_count - 1) as f64 / 2.0) * 24.0;
                let z = origin[2] + if side == 1 { -36.0 } else { 36.0 };
                let spec = RegimentSpec { id: (u32::from(side) - 1) * 100 + regiment as u32, faction: side, origin: [x, origin[1], z], facing, files, order: Order::Advance };
                let mut members = Vec::with_capacity(size);
                for index in 0..size {
                    let kind = kind(seed, side, regiment * 1000 + index);
                    let pos = slot_position(&spec, index);
                    let id = spawn(state, changes, "minecraft:zombie", pos, Some(kind), &tag)?;
                    let slot = state.entities.slot_of(id).expect("battle unit");
                    state.entities.set_faction(slot, side);
                    if kind == Kind::Cavalry {
                        let horse = spawn(state, changes, "minecraft:horse", pos, None, &tag)?;
                        crate::battle::cavalry::mount(state, id, horse).map_err(|e| format!("battle mount: {e}"))?;
                    }
                    members.push(id);
                }
                state.hostile.battle.formation.add_regiment(spec, &members).map_err(str::to_owned)?;
            }
        }
        state.hostile.battle.formation.set_bounds(
            [f64::from((centre.x - TICKET_RADIUS) * 16 + 2), f64::from((centre.z - TICKET_RADIUS) * 16 + 2)],
            [f64::from((centre.x + TICKET_RADIUS + 1) * 16 - 2), f64::from((centre.z + TICKET_RADIUS + 1) * 16 - 2)],
        ).map_err(str::to_owned)?;
        Ok(())
    })();
    if result.is_err() {
        reset(state, changes);
        changes.entities_spawned.truncate(spawned_before);
        changes.entities_removed.truncate(removed_before);
    }
    result
}

pub(crate) fn execute(state: &mut TickState, changes: &mut ChangeSet, origin: [f64; 3], command: &BattleCommand) -> Result<String, String> {
    match command {
        BattleCommand::Setup { seed, count } => { setup(state, changes, origin, *seed, *count)?; Ok(format!("Battle staged: {count} per side, seed {seed}")) }
        BattleCommand::Start => {
            if state.hostile.battle.formation.regiments.is_empty() { return Err("no staged formation; setup required".into()); }
            state.hostile.battle.running = true;
            Ok("Battle started".into())
        }
        BattleCommand::Pause => { state.hostile.battle.running = false; Ok("Battle paused".into()) }
        BattleCommand::Reset => Ok(format!("Battle reset: {} entities removed", reset(state, changes))),
        BattleCommand::Roster => {
            use std::fmt::Write;
            let ids = tagged_ids(state);
            if ids.is_empty() { return Err("no staged battle roster".into()); }
            let mut result = String::with_capacity(15 + ids.len() * 37);
            result.push_str("Battle roster: ");
            for (index, id) in ids.iter().enumerate() {
                if index != 0 { result.push(','); }
                let slot = state.entities.slot_of_any(*id).expect("tagged entity");
                write!(&mut result, "{}", state.entities.row(slot).uuid).expect("write to string");
            }
            Ok(result)
        }
        BattleCommand::Status => {
            let mut sides = [0_usize; 2];
            let (mut resident, mut active, mut moving, mut horses, mut dead) = (0, 0, 0, 0, 0);
            for id in tagged_ids(state) {
                let Some(slot) = state.entities.slot_of_any(id) else { continue };
                let faction = state.entities.body(slot).faction;
                if !(1..=2).contains(&faction) {
                    horses += 1;
                    continue;
                }
                if !state.entities.body(slot).alive || state.entities.row(slot).health <= 0.0 {
                    dead += 1;
                    continue;
                }
                sides[usize::from(faction - 1)] += 1;
                let pos = state.entities.pos(slot);
                let x = (pos[0].floor() as i32).div_euclid(16);
                let z = (pos[2].floor() as i32).div_euclid(16);
                resident += usize::from(state.world.has_chunk(x, z));
                active += usize::from(state.world.is_active_position(x, z));
                moving += usize::from(state.entities.vel(slot).iter().any(|v| *v != 0.0));
            }
            let sample_tick = crate::command_context::execution_tick(state).saturating_add(1);
            Ok(format!("Battle {} tick={} side1={} side2={} resident={} active={} nonzero_velocity={} horses={} regiments={} dead={}", if state.hostile.battle.running { "running" } else { "paused" }, sample_tick, sides[0], sides[1], resident, active, moving, horses, state.hostile.battle.formation.regiments.len(), dead))
        }
    }
}

#[cfg(test)]
#[path = "staging_tests.rs"]
mod tests;
