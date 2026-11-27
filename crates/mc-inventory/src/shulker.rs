//! Shulker box identity: the block and the item that may not nest inside one.
use mc_data::block_entity_kinds::BlockEntityKind;
use mc_data::{ItemStack, items::ItemRegistry};

pub fn is_block(path: &str) -> bool {
    BlockEntityKind::for_block(path) == Some(BlockEntityKind::ShulkerBox)
}
pub fn is_item(items: &ItemRegistry, stack: &ItemStack) -> bool {
    items
        .name_of(stack.item_id)
        .is_some_and(|id| is_block(id.path()))
}
