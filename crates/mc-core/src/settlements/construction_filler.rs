//! Fill material identity for a needed lot: which real storage material becomes
//! the substrate under an authored floor. Choosing here keeps the bill exact and
//! the authored crafting chain reserved.
use super::*;
use crate::settlements::inventory::Image;

/// Fill material identities, most preferred first: soil, then the cobblestone a
/// cut on the same lot returns to storage, then stone, which is a processed
/// material a blueprint may need. Falling blocks (sand, gravel) are never fill.
const FILLERS: [&str; 6] = [
    "dirt",
    "coarse_dirt",
    "rooted_dirt",
    "podzol",
    "cobblestone",
    "stone",
];

/// Items the authored build itself needs, plus its whole crafting chain: walls
/// of stone bricks consume stone, and stairs consume more bricks, so the fill
/// must not take the stone or the bricks either. The chain walks the same recipe
/// choice the canonical funding planner takes (`INDEX::craft_chain`), so there
/// is no second recipe model here.
pub(super) fn plan_reserve(
    state: &TickState,
    authored: &[(u32, u32)],
) -> std::collections::BTreeMap<u32, u64> {
    let index = &state.registry.construction_crafting;
    let mut reserve: std::collections::BTreeMap<u32, u64> = std::collections::BTreeMap::new();
    for &(item, count) in authored {
        for (entry, need) in index.craft_chain(item, u64::from(count)) {
            *reserve.entry(entry).or_insert(0) += need;
        }
    }
    reserve
}

/// Choose the fill's material identity from what settlement storage actually
/// holds, so a lot whose fill is short of dirt is not refused for a material it
/// does have. Only the cells the plan fills below the authored floor are
/// rewritten; authored blocks keep their blueprint material and the bill stays
/// exact, so conservation is unchanged. The authored bill (and the ingredients
/// its crafters will consume) is reserved first: fill takes only the remainder.
pub(super) fn apply_filler(
    state: &TickState,
    project: &Project,
    cells: &mut [executor::PlannedCell],
    images: &[Image],
) {
    if project.building.is_some() {
        return;
    }
    let Some(level) = blueprints::get(&state.registry, &project.blueprint)
        .and_then(|blueprint| blueprint.level(project.level))
    else {
        return;
    };
    let reserve = plan_reserve(state, &level.materials);
    let rotated = level.rotated(Rotation::from_quarter_turns(project.rotation));
    let authored: rustc_hash::FxHashSet<[i32; 3]> = rotated
        .blocks
        .iter()
        .map(|(offset, _)| rotated.world(project.origin, *offset))
        .collect();
    let Some(dirt) = mc_data::Identifier::parse("dirt")
        .ok()
        .and_then(|id| state.registry.items.id_of(&id))
        .and_then(|item| state.registry.items.name_of(item))
        .map(|id| id.as_str())
    else {
        return;
    };
    let mut fill: Vec<usize> = cells
        .iter()
        .enumerate()
        .filter(|(_, cell)| {
            cell.resource.as_deref() == Some(dirt)
                && cell.at[1] <= project.origin[1]
                && !authored.contains(&cell.at)
        })
        .map(|(index, _)| index)
        .collect();
    if fill.is_empty() {
        return;
    }
    let mut held: Vec<(u32, &'static str, u64)> = Vec::new();
    for name in FILLERS {
        // A bare path defaults to the `minecraft` namespace: one typed id per
        // candidate, never an inline id string.
        let Ok(id) = mc_data::Identifier::parse(name) else {
            continue;
        };
        let (Some(block), Some(item)) = (
            state.registry.blocks.block(&id),
            state.registry.items.id_of(&id),
        ) else {
            continue;
        };
        let stored: u64 = images
            .iter()
            .flat_map(|image| &image.slots)
            .filter(|stack| !stack.is_empty() && stack.item_id == item)
            .map(|stack| stack.count.max(0) as u64)
            .sum();
        // Only the remainder after the authored bill and its craft ingredients.
        let available = stored.saturating_sub(reserve.get(&item).copied().unwrap_or(0));
        if available > 0 {
            held.push((block.default.0, name, available));
        }
    }
    // Top layer first, then downwards: the visible floor takes the preferred
    // identity and only the deepest fill takes a fallback.
    fill.sort_by(|a, b| cells[*b].at[1].cmp(&cells[*a].at[1]).then(a.cmp(b)));
    let mut next = 0;
    for (block, name, available) in held {
        let resource = mc_data::Identifier::parse(name)
            .map(|id| id.as_str().to_owned())
            .unwrap_or_else(|_| name.to_owned());
        for _ in 0..available {
            let Some(&index) = fill.get(next) else {
                return;
            };
            next += 1;
            cells[index].state = block;
            cells[index].resource = Some(resource.clone());
        }
    }
}
