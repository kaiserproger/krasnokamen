//! Turtle egg custody, homeward movement and beach nesting.
use crate::boundary::{BlockPos, EntityDataValue, EntityId};
use crate::entities::{ColdRow, EntitySlot};
use crate::tick::TickState;
use serde::{Deserialize, Serialize};

static TYPE: std::sync::LazyLock<u32> = std::sync::LazyLock::new(|| {
    crate::piglin::type_id_of("minecraft:turtle").expect("turtle type")
});

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurtleState {
    pub home: [i32; 3],
    pub has_egg: bool,
    #[serde(default)]
    pub lay_ticks: u16,
    #[serde(default)]
    pub nest: Option<[i32; 3]>,
}
impl TurtleState {
    pub(crate) fn new(pos: [f64; 3]) -> Self {
        Self {
            home: pos.map(|v| v.floor() as i32),
            has_egg: false,
            lay_ticks: 0,
            nest: None,
        }
    }
}

pub(crate) fn is_turtle(type_id: u32) -> bool {
    type_id == *TYPE
}

pub(crate) fn initialize(state: &mut TickState, slot: EntitySlot, type_id: u32) {
    if is_turtle(type_id) {
        let pos = state.entities.pos(slot);
        state.entities.row_mut(slot).entity_life.turtle = Some(TurtleState::new(pos));
    }
}

pub(crate) fn restore(state: &mut TickState, slot: EntitySlot) {
    let Some(turtle) = state.entities.row(slot).entity_life.turtle.as_ref() else {
        return;
    };
    let (egg, laying) = (turtle.has_egg, turtle.lay_ticks > 0);
    crate::special_mobs::metadata(state, slot, 18, EntityDataValue::Boolean(egg));
    crate::special_mobs::metadata(state, slot, 19, EntityDataValue::Boolean(laying));
}

pub(crate) fn set_egg(state: &mut TickState, id: EntityId) {
    let Some(slot) = state.entities.slot_of(id) else {
        return;
    };
    let pos = state.entities.pos(slot);
    let turtle = state
        .entities
        .row_mut(slot)
        .entity_life
        .turtle
        .get_or_insert_with(|| TurtleState::new(pos));
    turtle.has_egg = true;
    turtle.nest = None;
    turtle.lay_ticks = 0;
    crate::special_mobs::metadata(state, slot, 18, EntityDataValue::Boolean(true));
    state.appearance_changed.push(id);
}

pub(crate) fn drive(row: &ColdRow) -> Option<([f64; 3], f64)> {
    let turtle = row.entity_life.turtle.as_ref()?;
    if !turtle.has_egg {
        return None;
    }
    let target = turtle
        .nest
        .unwrap_or(turtle.home)
        .map(|v| f64::from(v) + 0.5);
    Some((
        target,
        if turtle.lay_ticks > 0 {
            0.0
        } else {
            f64::from(row.speed)
        },
    ))
}

fn valid_nest(state: &TickState, pos: [i32; 3]) -> bool {
    let [x, y, z] = pos;
    state.world.block_at(x, y, z) == Some(state.registry.states.air)
        && state
            .world
            .block_at(x, y - 1, z)
            .and_then(|block| state.registry.blocks.by_id(mc_world::BlockStateId(block)))
            .is_some_and(|block| {
                matches!(
                    block.block.id.path(),
                    "sand" | "red_sand" | "suspicious_sand"
                )
            })
}

fn find_nest(state: &TickState, home: [i32; 3], pos: [f64; 3]) -> Option<[i32; 3]> {
    let mut best = None;
    for x in home[0] - 8..=home[0] + 8 {
        for z in home[2] - 8..=home[2] + 8 {
            for y in home[1] - 4..=home[1] + 4 {
                let candidate = [x, y, z];
                if !valid_nest(state, candidate) {
                    continue;
                }
                let d = candidate
                    .into_iter()
                    .zip(pos)
                    .map(|(a, b)| (f64::from(a) + 0.5 - b).powi(2))
                    .sum::<f64>();
                if best.is_none_or(|(_, old)| d < old) {
                    best = Some((candidate, d));
                }
            }
        }
    }
    best.map(|(candidate, _)| candidate)
}

pub(crate) fn run(state: &mut TickState) {
    let slots: Vec<_> = state.entities.mob_slots(*TYPE).collect();
    for slot in slots {
        let body = state.entities.body(slot);
        let row = state.entities.row(slot);
        let Some(turtle) = row.entity_life.turtle.as_ref() else {
            continue;
        };
        if !body.alive || row.controls.no_ai || !turtle.has_egg {
            continue;
        }
        let id = body.id;
        let pos = state.entities.pos(slot);
        if !state.world.is_active(
            (pos[0].floor() as i32).div_euclid(16),
            (pos[2].floor() as i32).div_euclid(16),
        ) {
            continue;
        }
        let home = turtle.home;
        let mut nest = turtle.nest.filter(|nest| valid_nest(state, *nest));
        let home_distance = home
            .into_iter()
            .zip(pos)
            .map(|(a, b)| (f64::from(a) + 0.5 - b).powi(2))
            .sum::<f64>();
        if nest.is_none()
            && home_distance < 81.0
            && state.tick.wrapping_add(id.0 as u64).is_multiple_of(20)
        {
            nest = find_nest(state, home, pos);
        }
        let arrived = nest.is_some_and(|nest| {
            nest.into_iter()
                .zip(pos)
                .map(|(a, b)| (f64::from(a) + 0.5 - b).powi(2))
                .sum::<f64>()
                < 2.25
        });
        let old_laying = turtle.lay_ticks > 0;
        let ticks = if arrived {
            turtle.lay_ticks.saturating_add(1)
        } else {
            0
        };
        let turtle = state
            .entities
            .row_mut(slot)
            .entity_life
            .turtle
            .as_mut()
            .expect("turtle");
        turtle.nest = nest;
        turtle.lay_ticks = ticks;
        if old_laying != (ticks > 0) {
            crate::special_mobs::metadata(state, slot, 19, EntityDataValue::Boolean(ticks > 0));
            state.appearance_changed.push(id);
        }
        if ticks <= 200 {
            continue;
        }
        let Some([x, y, z]) = nest else { continue };
        let name = mc_data::Identifier::parse("minecraft:turtle_egg").expect("egg id");
        let mut rng =
            crate::natural_spawn::Rng(state.world.seed() as u64 ^ state.tick ^ id.0 as u64);
        let props = vec![
            ("eggs".to_owned(), (1 + rng.below(4)).to_string()),
            ("hatch".to_owned(), "0".to_owned()),
        ];
        if let Some(egg) = state.registry.blocks.by_name_and_props(&name, &props) {
            state.write_block(BlockPos { x, y, z }, egg.0);
            let turtle = state
                .entities
                .row_mut(slot)
                .entity_life
                .turtle
                .as_mut()
                .expect("turtle");
            turtle.has_egg = false;
            turtle.lay_ticks = 0;
            turtle.nest = None;
            crate::special_mobs::metadata(state, slot, 18, EntityDataValue::Boolean(false));
            crate::special_mobs::metadata(state, slot, 19, EntityDataValue::Boolean(false));
            state.appearance_changed.push(id);
        }
    }
}
