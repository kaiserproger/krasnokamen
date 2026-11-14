//! Equippable.STREAM_CODEC: 26.3 metadata in receipts/BvArmorAssets/equippable-26.3.javap.txt.
use super::*;

const SLOTS: [&str; 8] = [
    "mainhand", "feet", "legs", "chest", "head", "offhand", "body", "saddle",
];
const FLAGS: [(&str, bool); 5] = [
    ("dispensable", true),
    ("swappable", true),
    ("damage_on_hurt", true),
    ("equip_on_interact", false),
    ("can_be_sheared", false),
];

fn write_sound<B: BufMut>(
    buf: &mut B,
    value: Option<&Tag>,
    default: &str,
) -> Result<(), CodecError> {
    match value {
        None => buf.write_varint(registry_id("sound_event", default)? + 1),
        Some(Tag::String(name)) => buf.write_varint(registry_id("sound_event", name)? + 1),
        Some(Tag::Compound(fields)) => {
            buf.write_varint(0);
            let id =
                Identifier::parse(string(required(fields, "sound_id")?)?).map_err(|_| invalid())?;
            buf.write_identifier(&id)?;
            let range = field(fields, "range");
            buf.write_bool(range.is_some());
            if let Some(range) = range {
                let n = match range {
                    Tag::Float(n) => *n,
                    Tag::Double(n) => *n as f32,
                    Tag::Int(n) => *n as f32,
                    Tag::Short(n) => *n as f32,
                    Tag::Byte(n) => *n as f32,
                    _ => return Err(invalid()),
                };
                if !n.is_finite() {
                    return Err(invalid());
                }
                buf.write_f32(n);
            }
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

fn read_sound<B: Buf>(buf: &mut B) -> Result<Tag, CodecError> {
    let id = buf.read_varint()?;
    if id < 0 {
        return Err(invalid());
    }
    if id != 0 {
        return Ok(Tag::String(registry_name("sound_event", id - 1)?));
    }
    let mut fields = vec![(
        "sound_id".into(),
        Tag::String(buf.read_identifier()?.as_str().into()),
    )];
    if buf.read_bool()? {
        let n = buf.read_f32()?;
        if !n.is_finite() {
            return Err(invalid());
        }
        fields.push(("range".into(), Tag::Float(n)));
    }
    Ok(Tag::Compound(fields))
}

pub(super) fn encode<B: BufMut>(buf: &mut B, value: &Tag) -> Result<(), CodecError> {
    let fields = compound(value)?;
    let slot = string(required(fields, "slot")?)?;
    buf.write_varint(SLOTS.iter().position(|v| *v == slot).ok_or_else(invalid)? as i32);
    write_sound(
        buf,
        field(fields, "equip_sound"),
        "minecraft:item.armor.equip_generic",
    )?;
    for name in ["asset_id", "camera_overlay"] {
        let value = field(fields, name);
        buf.write_bool(value.is_some());
        if let Some(value) = value {
            let id = Identifier::parse(string(value)?).map_err(|_| invalid())?;
            buf.write_identifier(&id)?;
        }
    }
    let allowed = field(fields, "allowed_entities");
    buf.write_bool(allowed.is_some());
    if let Some(allowed) = allowed {
        match allowed {
            Tag::String(name) if name.starts_with('#') => {
                buf.write_varint(0);
                let id = Identifier::parse(&name[1..]).map_err(|_| invalid())?;
                buf.write_identifier(&id)?;
            }
            Tag::String(name) => {
                buf.write_varint(2);
                buf.write_varint(registry_id("entity_type", name)?);
            }
            _ => {
                let values = list(allowed)?;
                if values.len() > 256 {
                    return Err(invalid());
                }
                buf.write_varint(values.len() as i32 + 1);
                for value in values {
                    buf.write_varint(registry_id("entity_type", string(value)?)?);
                }
            }
        }
    }
    for (name, default) in FLAGS {
        let value = match field(fields, name) {
            None => default,
            Some(value) => match integer(value)? {
                0 => false,
                1 => true,
                _ => return Err(invalid()),
            },
        };
        buf.write_bool(value);
    }
    write_sound(
        buf,
        field(fields, "shearing_sound"),
        "minecraft:item.shears.snip",
    )
}

pub(super) fn decode<B: Buf>(buf: &mut B) -> Result<Tag, CodecError> {
    let slot = usize::try_from(buf.read_varint()?)
        .ok()
        .and_then(|i| SLOTS.get(i))
        .ok_or_else(invalid)?;
    let mut fields = vec![
        ("slot".into(), Tag::String((*slot).into())),
        ("equip_sound".into(), read_sound(buf)?),
    ];
    for name in ["asset_id", "camera_overlay"] {
        if buf.read_bool()? {
            fields.push((
                name.into(),
                Tag::String(buf.read_identifier()?.as_str().into()),
            ));
        }
    }
    if buf.read_bool()? {
        let count = buf.read_varint()?;
        if !(0..=257).contains(&count) {
            return Err(invalid());
        }
        let value = if count == 0 {
            Tag::String(format!("#{}", buf.read_identifier()?.as_str()))
        } else {
            let mut values = Vec::with_capacity((count - 1) as usize);
            for _ in 1..count {
                values.push(Tag::String(registry_name(
                    "entity_type",
                    buf.read_varint()?,
                )?));
            }
            make_list(values)
        };
        fields.push(("allowed_entities".into(), value));
    }
    for (name, _) in FLAGS {
        fields.push((name.into(), Tag::Byte(i8::from(buf.read_bool()?))));
    }
    fields.push(("shearing_sound".into(), read_sound(buf)?));
    Ok(Tag::Compound(fields))
}
