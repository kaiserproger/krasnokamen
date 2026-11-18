//! Settlement portion certificates. Plugin structures keep their exact box fence.
use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum PortionKind {
    Liquid,
    Saplings,
    Trees,
    Cut,
    Fill,
    Build,
    Repair,
}

impl PortionKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Liquid => "liquid",
            Self::Saplings => "saplings",
            Self::Trees => "trees",
            Self::Cut => "cut",
            Self::Fill => "fill",
            Self::Build => "build",
            Self::Repair => "repair",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        match name {
            "liquid" => Some(Self::Liquid),
            "saplings" => Some(Self::Saplings),
            "trees" => Some(Self::Trees),
            "cut" => Some(Self::Cut),
            "fill" => Some(Self::Fill),
            "build" => Some(Self::Build),
            "repair" => Some(Self::Repair),
            _ => None,
        }
    }

    pub(crate) fn is_preparation(name: &str) -> bool {
        Self::parse(name).is_some_and(|kind| kind < Self::Build)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Portion {
    pub(super) expected: Vec<u32>,
    roots: Vec<[i32; 3]>,
}

pub(super) fn identity(state: &TickState, block: u32) -> Result<u32, Failure> {
    state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .map(|definition| definition.block.raw_id as u32)
        .ok_or(Failure::InvalidRequest)
}

pub(super) fn certify(state: &TickState, batches: &[Vec<Cell>]) -> Result<Vec<Portion>, Failure> {
    let mut previous = BTreeMap::new();
    let mut portions = Vec::with_capacity(batches.len());
    for cells in batches {
        let mut expected = Vec::with_capacity(cells.len());
        let mut roots = BTreeSet::new();
        for cell in cells {
            let before = match previous.get(&cell.at) {
                Some(raw) => *raw,
                None => identity(
                    state,
                    state
                        .world
                        .block_at(cell.at[0], cell.at[1], cell.at[2])
                        .ok_or(Failure::Unloaded)?,
                )?,
            };
            expected.push(before);
            previous.insert(cell.at, identity(state, cell.state)?);
            if cell.state == state.registry.states.air
                && state
                    .settlements
                    .records()
                    .any(|record| record.planted_trees.contains(&cell.at))
            {
                roots.insert(cell.at);
            }
        }
        portions.push(Portion {
            expected,
            roots: roots.into_iter().collect(),
        });
    }
    Ok(portions)
}

pub(super) fn inside(
    state: &TickState,
    id: crate::settlements::SettlementId,
    at: [i32; 3],
) -> bool {
    state
        .settlements
        .get(id)
        .is_some_and(|town| town.contains(at))
}

/// Only unprocessed cells of the requested advance are checked, including after
/// the first cell of a portion. Earlier writes are not admission-before cells.
pub(super) fn stale_reason(
    state: &TickState,
    record: &Structure,
    cells: &[Cell],
) -> Result<Option<String>, Failure> {
    if record.settlement.is_none() {
        return Ok(
            (fingerprint(state, bounds_of_structure(record)?, false)? != record.image)
                .then(|| "footprint_changed".to_owned()),
        );
    }
    let index = record.snapshot.current_stage_index as usize;
    let offset = record.snapshot.completed_work_units as usize;
    let evidence = record.portions.get(index).ok_or(Failure::InvalidRequest)?;
    let id = record.settlement.expect("settlement certificate");
    if record.snapshot.stages[index].stage == PortionKind::Saplings.name()
        && offset == 0
        && record.snapshot.consumed.is_empty()
        && crate::settlements::construction::preparation::stale_tree_certification(
            state,
            id,
            &record.snapshot.structure_id,
            &evidence.roots,
            |block| {
                !crate::settlements::forestry::tree_or_sapling(state, block)
                    || state
                        .registry
                        .blocks
                        .by_id(mc_world::BlockStateId(block))
                        .is_some_and(|definition| definition.block.id.path().ends_with("_sapling"))
            },
        )
    {
        return Ok(Some("footprint_changed".to_owned()));
    }
    for (n, cell) in cells.iter().enumerate() {
        let actual = identity(
            state,
            state
                .world
                .block_at(cell.at[0], cell.at[1], cell.at[2])
                .ok_or(Failure::Unloaded)?,
        )?;
        if evidence.expected.get(offset + n) != Some(&actual)
            && !inside(state, id, cell.at)
            && crate::settlements::forestry::vetoed(state, cell.at)
        {
            return Err(Failure::Blocked);
        }
    }
    Ok(None)
}

pub(super) fn valid(record: &Structure, raw_count: u32, air: u32) -> bool {
    if record.settlement.is_none() {
        return record.portions.is_empty();
    }
    if record.portions.len() != record.cells.len() {
        return false;
    }
    let mut last = None;
    for ((stage, cells), portion) in record
        .snapshot
        .stages
        .iter()
        .zip(&record.cells)
        .zip(&record.portions)
    {
        let Some(kind) = PortionKind::parse(&stage.stage) else {
            return false;
        };
        if last.is_some_and(|last| last >= kind)
            || portion.expected.len() != cells.len()
            || portion.expected.iter().any(|raw| *raw >= raw_count)
            || portion.roots.windows(2).any(|pair| pair[0] >= pair[1])
            || portion.roots.iter().any(|root| {
                !cells
                    .iter()
                    .any(|cell| cell.at == *root && cell.state == air)
            })
        {
            return false;
        }
        last = Some(kind);
    }
    true
}

/// D27 property is physical: before replacing a live village cell, credit
/// exactly that block's drops in the same inventory transaction as the write.
/// Outside territory this adapter does not replace the existing protections.
pub(super) fn village_drops(
    state: &TickState,
    id: crate::settlements::SettlementId,
    cell: &Cell,
    expected: u32,
) -> Result<Option<Vec<mc_data::ItemStack>>, Failure> {
    let block = state
        .world
        .block_at(cell.at[0], cell.at[1], cell.at[2])
        .ok_or(Failure::Unloaded)?;
    let actual = identity(state, block)?;
    let unchanged = actual == expected;
    let owned = inside(state, id, cell.at);
    if unchanged
        && (cell.state == state.registry.states.air
            || block == cell.state
            || block == state.registry.states.air
            || !owned)
    {
        return Ok(None);
    }
    if !owned && crate::settlements::forestry::vetoed(state, cell.at) {
        return Err(Failure::Blocked);
    }
    let natural = crate::settlements::construction::excavatable(state, block);
    let ground = natural
        || crate::settlements::construction::relayout::village_soil(state, block);
    if unchanged && ground {
        return Ok(None);
    }
    // A foundation reshapes terrain; it does not mine a second material.
    if cell.state != state.registry.states.air && ground {
        return Ok(Some(Vec::new()));
    }
    if actual == identity(state, cell.state)? {
        return Ok(Some(Vec::new()));
    }
    if cell.state == state.registry.states.air
        && crate::settlements::construction::preparation::liquid::fluid_block(state, block)
    {
        return Err(Failure::Blocked);
    }
    if cell.state == state.registry.states.air && natural {
        for (dx, dy, dz) in [
            (1, 0, 0),
            (-1, 0, 0),
            (0, 1, 0),
            (0, -1, 0),
            (0, 0, 1),
            (0, 0, -1),
        ] {
            let neighbour = state
                .world
                .block_at(cell.at[0] + dx, cell.at[1] + dy, cell.at[2] + dz)
                .ok_or(Failure::Unloaded)?;
            if state
                .registry
                .blocks
                .by_id(mc_world::BlockStateId(neighbour))
                .is_none_or(|definition| definition.has_fluid())
            {
                return Err(Failure::Blocked);
            }
        }
    }
    conserved_drops(state, cell.at, block).map(Some)
}

/// Settlements already extract with implicit tools. Demolition uses that same
/// authority, with the lowest ordinary tier accepted by the canonical rules.
fn canonical_tool(
    state: &TickState,
    block: &mc_world::Block,
) -> Result<mc_data::ItemStack, Failure> {
    static TOOLS: std::sync::LazyLock<[mc_data::Identifier; 31]> = std::sync::LazyLock::new(|| {
        [
            "minecraft:wooden_pickaxe",
            "minecraft:wooden_shovel",
            "minecraft:wooden_axe",
            "minecraft:wooden_hoe",
            "minecraft:wooden_sword",
            "minecraft:golden_pickaxe",
            "minecraft:golden_shovel",
            "minecraft:golden_axe",
            "minecraft:golden_hoe",
            "minecraft:golden_sword",
            "minecraft:stone_pickaxe",
            "minecraft:stone_shovel",
            "minecraft:stone_axe",
            "minecraft:stone_hoe",
            "minecraft:stone_sword",
            "minecraft:iron_pickaxe",
            "minecraft:iron_shovel",
            "minecraft:iron_axe",
            "minecraft:iron_hoe",
            "minecraft:iron_sword",
            "minecraft:diamond_pickaxe",
            "minecraft:diamond_shovel",
            "minecraft:diamond_axe",
            "minecraft:diamond_hoe",
            "minecraft:diamond_sword",
            "minecraft:netherite_pickaxe",
            "minecraft:netherite_shovel",
            "minecraft:netherite_axe",
            "minecraft:netherite_hoe",
            "minecraft:netherite_sword",
            "minecraft:shears",
        ]
        .map(|name| mc_data::Identifier::parse(name).expect("canonical tool identifier"))
    });
    let items = state.registry.item_rules();
    for name in &*TOOLS {
        let Some(id) = state.registry.items.id_of(name) else {
            continue;
        };
        let tool = mc_data::ItemStack::new(id, 1);
        if crate::block_drops::correct_tool(&items, block, &tool) {
            return Ok(tool);
        }
    }
    Err(Failure::Blocked)
}

pub(super) fn conserved_drops(
    state: &TickState,
    at: [i32; 3],
    block: u32,
) -> Result<Vec<mc_data::ItemStack>, Failure> {
    let definition = state
        .registry
        .blocks
        .by_id(mc_world::BlockStateId(block))
        .ok_or(Failure::InvalidRequest)?;
    if matches!(definition.block.id.path(), "air" | "cave_air" | "void_air")
        || crate::settlements::construction::preparation::liquid::fluid_block(state, block)
    {
        return Ok(Vec::new());
    }
    if state
        .registry
        .loot
        .block_loot(&definition.block.id)
        .is_none()
        && state
            .registry
            .loot
            .block_drop_stacks(&definition.block.id)
            .is_none()
    {
        return Err(Failure::Blocked);
    }
    let tool = if state
        .registry
        .mining
        .requires_correct_tool_for_drops(block)
        .unwrap_or(false)
    {
        canonical_tool(state, &definition.block)?
    } else {
        mc_data::ItemStack::EMPTY
    };
    crate::settlements::forestry::roll_with_tool(state, at, block, &tool)
}

#[cfg(test)]
#[path = "portions_tests.rs"]
mod tests;
