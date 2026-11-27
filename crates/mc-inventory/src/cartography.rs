//! Cartography table slot rules and the vanilla `map_id` item component they read.
use crate::ItemRules;
use mc_data::ItemStack;

pub fn allowed(rules: &ItemRules<'_>, slot: usize, stack: &ItemStack) -> bool {
    if stack.is_empty() {
        return true;
    }
    let Some(name) = crate::anvil::logical_item(rules, stack) else {
        return false;
    };
    if rules.items.name_of(stack.item_id) != Some(name) {
        return false;
    }
    match slot {
        0 => name.as_str() == "minecraft:filled_map" && map_id(stack).is_some(),
        1 => matches!(
            name.as_str(),
            "minecraft:paper" | "minecraft:map" | "minecraft:glass_pane"
        ),
        _ => false,
    }
}

pub fn map_id(stack: &ItemStack) -> Option<i32> {
    let component = stack
        .extended()
        .preserved_components
        .iter()
        .find(|c| c.name == "minecraft:map_id")?;
    let mut bytes = component.value.as_slice();
    match mc_nbt::read_network_value(&mut bytes).ok()? {
        mc_nbt::Tag::Int(id) if id >= 0 && bytes.is_empty() => Some(id),
        _ => None,
    }
}
pub fn set_map_id(stack: &mut ItemStack, id: i32) {
    let mut value = Vec::new();
    mc_nbt::write_network_value(&mut value, &mc_nbt::Tag::Int(id)).expect("map id NBT");
    let components = &mut stack.extended_mut().preserved_components;
    components.retain(|c| c.name.trim_start_matches('!') != "minecraft:map_id");
    components.push(mc_data::item_stack::PreservedItemComponent {
        name: "minecraft:map_id".into(),
        value,
        baseline: None,
    });
}
