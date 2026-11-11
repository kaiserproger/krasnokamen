//! `ItemStack.canBreakBlockInAdventureMode` (26.3 javap: `AdventureModePredicate`,
//! `BlockPredicate`, `StatePropertiesPredicate`, `NbtPredicate`): the stack's
//! `can_break` component, one block predicate or a list, admits the block at a
//! cell when any predicate matches it. A predicate checks the block (`blocks`:
//! an id, a list of ids or a `#tag`), its properties (`state`: exact values or
//! `{min, max}` ranges) and its block entity image (`nbt`, a subset match).
//! Block entity component matchers (`components`, `predicates`) are not
//! modelled: a predicate naming any of them matches nothing.

use mc_data::ItemStack;
use mc_data::item_nbt::field;
use mc_nbt::Tag;

use crate::boundary::BlockPos;
use crate::tick::TickState;

pub(crate) fn can_break(state: &TickState, stack: &ItemStack, pos: BlockPos) -> bool {
    if stack.is_empty() {
        return false;
    }
    let Ok(fields) = mc_data::item_nbt::stack_to_fields(&state.registry.items, stack) else {
        return false;
    };
    let Some(Tag::Compound(components)) = field(&fields, "components") else {
        return false;
    };
    match field(components, mc_data::item_nbt::CAN_BREAK_COMPONENT) {
        Some(Tag::List(list)) => list
            .elements
            .iter()
            .any(|predicate| matches(state, predicate, pos)),
        Some(predicate) => matches(state, predicate, pos),
        None => false,
    }
}

/// `BlockPredicate.matches(BlockInWorld)`.
fn matches(state: &TickState, predicate: &Tag, pos: BlockPos) -> bool {
    let Tag::Compound(fields) = predicate else {
        return false;
    };
    let Some(block) = state
        .world
        .block_at(pos.x, pos.y, pos.z)
        .and_then(|id| state.registry.blocks.by_id(mc_world::BlockStateId(id)))
    else {
        return false;
    };
    if let Some(blocks) = field(fields, "blocks")
        && !in_holder_set(state, blocks, block)
    {
        return false;
    }
    if let Some(properties) = field(fields, "state")
        && !properties_match(block, properties)
    {
        return false;
    }
    if ["components", "predicates"].iter().any(|name| {
        field(fields, name).is_some_and(|tag| !matches!(tag, Tag::Compound(map) if map.is_empty()))
    }) {
        return false;
    }
    if let Some(nbt) = field(fields, "nbt") {
        let expected = match nbt {
            Tag::String(snbt) => match crate::command_snbt::parse(snbt) {
                Ok(tag) => tag,
                Err(_) => return false,
            },
            Tag::Compound(_) => nbt.clone(),
            _ => return false,
        };
        // `saveWithFullMetadata` of the cell's block entity; none: no match.
        let Ok(actual) = crate::command_world::block_image(state, pos) else {
            return false;
        };
        if !crate::command_world::nbt_subset(&expected, &actual) {
            return false;
        }
    }
    true
}

/// `HolderSet<Block>` codec forms: `"id"`, `["id", ..]` or `"#tag"`.
fn in_holder_set(state: &TickState, set: &Tag, block: &mc_world::BlockState) -> bool {
    let named = |name: &str| mc_data::Identifier::parse(name).is_ok_and(|id| id == block.block.id);
    match set {
        Tag::String(tag) if tag.starts_with('#') => mc_data::Identifier::parse(&tag[1..])
            .is_ok_and(|tag| {
                state.registry.tags.contains_raw_id(
                    mc_data::tags::BLOCK_REGISTRY,
                    tag.as_str(),
                    block.block.raw_id,
                )
            }),
        Tag::String(name) => named(name),
        Tag::List(list) => list
            .elements
            .iter()
            .any(|entry| matches!(entry, Tag::String(name) if named(name))),
        _ => false,
    }
}

/// `StatePropertiesPredicate`: every named property exists on the block and
/// matches; an exact value by name, a range by the property's value order.
fn properties_match(block: &mc_world::BlockState, properties: &Tag) -> bool {
    let Tag::Compound(matchers) = properties else {
        return false;
    };
    matchers.iter().all(|(name, matcher)| {
        let Some(current) = block
            .properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
        else {
            return false;
        };
        let Some(values) = block
            .block
            .properties
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, values)| values)
        else {
            return false;
        };
        match matcher {
            Tag::String(exact) => {
                rank(values, exact).is_some_and(|wanted| Some(wanted) == rank(values, current))
            }
            Tag::Compound(range) => {
                let Some(now) = rank(values, current) else {
                    return false;
                };
                let bound = |key: &str| match field(range, key) {
                    None => Some(None),
                    Some(Tag::String(value)) => rank(values, value).map(Some),
                    Some(_) => None,
                };
                let (Some(min), Some(max)) = (bound("min"), bound("max")) else {
                    return false;
                };
                min.is_none_or(|min| now >= min) && max.is_none_or(|max| now <= max)
            }
            _ => false,
        }
    })
}

/// `Property.getValue(name)` then `Comparable` order: integers numerically,
/// booleans false before true, enums in declaration order.
fn rank(values: &[String], value: &str) -> Option<i64> {
    if values
        .iter()
        .all(|candidate| candidate.parse::<i64>().is_ok())
    {
        let number = value.parse::<i64>().ok()?;
        return values
            .iter()
            .any(|candidate| candidate.parse() == Ok(number))
            .then_some(number);
    }
    if values
        .iter()
        .all(|candidate| candidate == "true" || candidate == "false")
    {
        return match value {
            "true" => Some(1),
            "false" => Some(0),
            _ => None,
        };
    }
    values
        .iter()
        .position(|candidate| candidate == value)
        .and_then(|index| i64::try_from(index).ok())
}

#[cfg(test)]
#[path = "can_break_tests.rs"]
mod tests;
