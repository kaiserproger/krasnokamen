//! Data-driven copper weathering for all 14 full 8-variant families
//! (WeatheringCopper NEXT/PREVIOUS BiMaps). State changes retain each
//! block's inventory and all properties; paired copper chests always
//! share one weathering image.
use mc_data::Identifier;
use mc_world::{BlockRegistry, BlockStateId};

/// Every weathering family: the bare (age-0, unwaxed) block path, derived
/// from the vanilla registry (each has all 8 wax/age variants).
/// `copper_block` (WeatheringCopperFullBlock age-0 alias of exposed_copper)
/// participates in weathering at age 0 only; aged cut-family blocks use
/// their own `cut_copper`-rooted names.
const FAMILIES: [&str; 15] = [
    "chiseled_copper",
    "copper_bars",
    "copper_block",
    "copper_bulb",
    "copper_chain",
    "copper_chest",
    "copper_door",
    "copper_golem_statue",
    "copper_grate",
    "copper_lantern",
    "copper_trapdoor",
    "cut_copper",
    "cut_copper_slab",
    "cut_copper_stairs",
    "lightning_rod",
];

pub fn chest(path: &str) -> bool {
    family(path).is_some_and(|(name, _, _)| name == "copper_chest")
}

pub fn family(path: &str) -> Option<(&str, u8, bool)> {
    let waxed = path.starts_with("waxed_");
    let path = path.strip_prefix("waxed_").unwrap_or(path);
    let (path, age) = if let Some(path) = path.strip_prefix("exposed_") {
        (path, 1)
    } else if let Some(path) = path.strip_prefix("weathered_") {
        (path, 2)
    } else if let Some(path) = path.strip_prefix("oxidized_") {
        (path, 3)
    } else {
        (path, 0)
    };
    // copper_block is the age-0 full-block alias; it never takes aged forms.
    if path == "copper_block" && (waxed || age > 0) {
        return None;
    }
    FAMILIES.contains(&path).then_some((path, age, waxed))
}

pub fn name(family: &str, age: u8, waxed: bool) -> String {
    format!(
        "minecraft:{}{}{family}",
        if waxed { "waxed_" } else { "" },
        match age {
            1 => "exposed_",
            2 => "weathered_",
            3 => "oxidized_",
            _ => "",
        }
    )
}

pub fn transform(blocks: &BlockRegistry, id: u32, age: u8, waxed: bool) -> Option<u32> {
    let state = blocks.by_id(BlockStateId(id))?;
    let (family, _, _) = family(state.block.id.path())?;
    let name = Identifier::parse(name(family, age, waxed)).ok()?;
    blocks
        .by_name_and_props(&name, &state.properties)
        .map(|id| id.0)
}

pub(crate) fn compatible_chests(a: &Identifier, b: &Identifier) -> bool {
    a == b || (chest(a.path()) && chest(b.path()))
}

/// Placement unifies both halves to the least oxidized variant; mixed wax
/// status removes wax, as CopperChestBlock.getLeastOxidizedChestOfConnectedBlocks.
pub(crate) fn joined(blocks: &BlockRegistry, a: u32, b: u32) -> Option<(u32, u32)> {
    let a_part = family(blocks.by_id(BlockStateId(a))?.block.id.path())?;
    let b_part = family(blocks.by_id(BlockStateId(b))?.block.id.path())?;
    if a_part.0 != "copper_chest" || b_part.0 != a_part.0 {
        return None;
    }
    let age = a_part.1.min(b_part.1);
    let waxed = a_part.2 && b_part.2;
    Some((
        transform(blocks, a, age, waxed)?,
        transform(blocks, b, age, waxed)?,
    ))
}
