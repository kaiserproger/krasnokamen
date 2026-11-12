//! Neutral command text-component construction, without styling or delivery.

use mc_nbt::{ListTag, Tag, tag_type};

pub(crate) fn text(value: impl ToString) -> Tag {
    Tag::Compound(vec![("text".into(), Tag::String(value.to_string()))])
}

pub(crate) fn translated(key: impl Into<String>, arguments: Vec<Tag>) -> Tag {
    let mut fields = vec![("translate".into(), Tag::String(key.into()))];
    if !arguments.is_empty() {
        fields.push((
            "with".into(),
            Tag::List(ListTag {
                element_type: tag_type::COMPOUND,
                elements: arguments,
            }),
        ));
    }
    Tag::Compound(fields)
}

/// True for a vanilla translation key such as `argument.entity.notfound.entity`; the lowercase dotted
/// shape never occurs in the English text of an ad-hoc error, which has spaces or capitals.
pub(crate) fn is_translation_key(message: &str) -> bool {
    message.contains('.')
        && message.starts_with(|first: char| first.is_ascii_lowercase())
        && !message.ends_with('.')
        && !message.contains("..")
        && message.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
        })
}

#[cfg(test)]
#[path = "command_feedback_tests.rs"]
mod tests;
