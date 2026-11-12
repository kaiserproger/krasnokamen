//! Bounded book-content validation and encoding, independent of inventory authority.
use mc_data::{Identifier, ItemStack, items::ItemRegistry};
use mc_nbt::{ListTag, Tag, tag_type};

pub(crate) fn replacement(
    held: &ItemStack,
    items: &ItemRegistry,
    pages: &[String],
    title: Option<&str>,
    author: &str,
) -> Option<ItemStack> {
    if pages.len() > 100
        || pages.iter().any(|page| page.encode_utf16().count() > 1024)
        || title.is_some_and(|title| title.encode_utf16().count() > 32)
        || held.count != 1
        || items
            .name_of(held.item_id)
            .is_none_or(|id| id.as_str() != "minecraft:writable_book")
    {
        return None;
    }
    let mut replacement = held.clone();
    let mut fields = vec![(
        "pages".into(),
        Tag::List(ListTag {
            element_type: tag_type::STRING,
            elements: pages.iter().cloned().map(Tag::String).collect(),
        }),
    )];
    let component = if let Some(title) = title {
        replacement.item_id = items
            .id_of(&Identifier::parse("minecraft:written_book").unwrap())
            .expect("written book item");
        fields.extend([
            ("title".into(), Tag::String(title.into())),
            ("author".into(), Tag::String(author.into())),
            ("generation".into(), Tag::Int(0)),
            // Authored pages are literal text, not commands/selectors to resolve.
            ("resolved".into(), Tag::Byte(1)),
        ]);
        "minecraft:written_book_content"
    } else {
        "minecraft:writable_book_content"
    };
    let mut value = Vec::new();
    mc_nbt::write_network_value(&mut value, &Tag::Compound(fields)).ok()?;
    replacement
        .extended_mut()
        .preserved_components
        .retain(|old| {
            !matches!(
                old.name.trim_start_matches('!'),
                "minecraft:writable_book_content" | "minecraft:written_book_content"
            )
        });
    replacement.extended_mut().preserved_components.push(
        mc_data::item_stack::PreservedItemComponent {
            name: component.into(),
            value,
            baseline: None,
        },
    );
    Some(replacement)
}
