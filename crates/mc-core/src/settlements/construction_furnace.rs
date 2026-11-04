//! Demand-driven, one-cell workstation projects through the canonical builder.
use super::*;

pub(crate) const NAME: &str = "furnace";

/// No free workstation: the builder's existing recursive crafting/fetch path
/// consumes either a real furnace item or its eight-cobblestone grid recipe.
pub(super) fn plan(state: &mut TickState, id: SettlementId) -> bool {
    let Some(record) = state.settlements.get(id) else {
        return false;
    };
    if record.project.is_some()
        || crate::villager::smelting::owned_furnaces(state, id)
            .next()
            .is_some()
    {
        return false;
    }
    let demand = iron_demand(state, id, None);
    if demand == 0 {
        return false;
    }
    let images = executor::source_images(state, id);
    if !availability::needs_furnace(state, id, demand, &images) {
        return false;
    }
    let name = mc_data::Identifier::parse("minecraft:furnace").expect("identifier");
    let Some(item) = state.registry.items.id_of(&name) else {
        return false;
    };
    if !super::super::construction_crafting::can_fund(state, &[(item, 1)], &images) {
        return false;
    }
    let Some(block) = state
        .registry
        .blocks
        .block(&name)
        .map(|block| block.default.0)
    else {
        return false;
    };
    // A bounded local search beside real storage. Existing buildings and their
    // canonical footprints remain protected; this is a registered outdoor
    // workstation, not an unaccounted edit inside a completed building.
    for chest in sources(state, id) {
        for distance in 2i32..=5 {
            for (dx, dz) in [(distance, 0), (-distance, 0), (0, distance), (0, -distance)] {
                let (x, z) = (chest[0] + dx, chest[2] + dz);
                let Some(y) = surface_y(state, chest[1], x, z) else {
                    continue;
                };
                let at = [x, y + 1, z];
                if at
                    .into_iter()
                    .zip(record.anchor)
                    .map(|(a, b)| (i64::from(a) - i64::from(b)).pow(2))
                    .sum::<i64>()
                    > i64::from(record.radius).pow(2)
                    || state.world.physics_block_at(x, y + 1, z) != Some(state.registry.states.air)
                    || state.world.physics_block_at(x, y + 2, z) != Some(state.registry.states.air)
                    || !state.world.is_active(x.div_euclid(16), z.div_euclid(16))
                    || record.buildings.values().any(|building| {
                        !building.natural_zone
                            && (0..3).all(|axis| {
                                at[axis] >= building.bounds.min[axis]
                                    && at[axis] <= building.bounds.max[axis]
                            })
                    })
                {
                    continue;
                }
                let bounds = ScriptWorldBounds::new(at, at).expect("one cell");
                if executor::area_reserved(state, bounds) {
                    continue;
                }
                let project = Project {
                    settlement: id,
                    blueprint: NAME.into(),
                    level: 1,
                    origin: at,
                    rotation: 0,
                    building: None,
                    structure: String::new(),
                    ordered: false,
                };
                if stand(state, &project, at).is_none() {
                    continue;
                }
                return begin_cells(
                    state,
                    project,
                    bounds,
                    vec![executor::PlannedCell {
                        at,
                        state: block,
                        resource: Some(name.to_string()),
                        quantity: 1,
                    }],
                );
            }
        }
    }
    false
}

pub(crate) fn stand(state: &TickState, project: &Project, next: [i32; 3]) -> Option<[i32; 3]> {
    let record = state.settlements.get(project.settlement)?;
    [(1, 0), (-1, 0), (0, 1), (0, -1)]
        .into_iter()
        .find_map(|(dx, dz)| {
            let (x, z) = (next[0] + dx, next[2] + dz);
            let y = surface_y(state, record.anchor[1], x, z)?;
            (state.world.physics_block_at(x, y + 1, z) == Some(state.registry.states.air)
                && state.world.physics_block_at(x, y + 2, z) == Some(state.registry.states.air))
            .then_some([x, y + 1, z])
        })
}
