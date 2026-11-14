//! ConsumeEffect stream codec for DeathProtection (26.3).
use super::{invalid, registry_id, registry_name};
use crate::codec::{ReadMc, WriteMc};
use crate::error::CodecError;
use bytes::{Buf, BufMut};
use mc_data::Identifier;
use mc_data::item_stack::death_protection::{self, DeathEffect, EffectTargets, Sound};
use mc_nbt::Tag;

fn identifier(value: String) -> Result<Identifier, CodecError> {
    Identifier::parse(value).map_err(|_| invalid())
}

pub(super) fn encode<B: BufMut>(buf: &mut B, value: &Tag) -> Result<(), CodecError> {
    let effects = death_protection::parse(value).map_err(CodecError::NotSupported)?;
    super::super::write_bounded_count(buf, effects.len(), 256)?;
    for effect in effects {
        match effect {
            DeathEffect::Apply {
                effects,
                probability,
            } => {
                buf.write_varint(0);
                super::super::write_bounded_count(buf, effects.len(), 256)?;
                for effect in effects {
                    buf.write_varint(registry_id("mob_effect", effect.id.as_str())?);
                    super::super::station_components::encode_effect_details(buf, &effect, 0)?;
                }
                buf.write_f32(probability);
            }
            DeathEffect::Remove(targets) => {
                buf.write_varint(1);
                match targets {
                    EffectTargets::NamedTag(tag) => {
                        buf.write_varint(0);
                        buf.write_identifier(&tag)?;
                    }
                    EffectTargets::Direct(ids) => {
                        buf.write_varint(i32::try_from(ids.len()).map_err(|_| invalid())? + 1);
                        for id in ids {
                            buf.write_varint(registry_id("mob_effect", id.as_str())?);
                        }
                    }
                }
            }
            DeathEffect::Clear => buf.write_varint(2),
            DeathEffect::Teleport { diameter } => {
                buf.write_varint(3);
                buf.write_f32(diameter);
            }
            DeathEffect::PlaySound(sound) => {
                buf.write_varint(4);
                if sound.registered {
                    buf.write_varint(registry_id("sound_event", sound.id.as_str())? + 1);
                } else {
                    buf.write_varint(0);
                    buf.write_identifier(&sound.id)?;
                    buf.write_bool(sound.fixed_range.is_some());
                    if let Some(range) = sound.fixed_range {
                        buf.write_f32(range);
                    }
                }
            }
        }
    }
    Ok(())
}

pub(super) fn decode<B: Buf>(buf: &mut B) -> Result<Tag, CodecError> {
    let effects = super::super::read_bounded_vec(buf, 256, 1, |buf| {
        Ok(match buf.read_varint()? {
            0 => {
                let effects = super::super::read_bounded_vec(buf, 256, 1, |buf| {
                    let id = identifier(registry_name("mob_effect", buf.read_varint()?)?)?;
                    super::super::station_components::decode_effect_details(buf, id, 0)
                })?;
                DeathEffect::Apply {
                    effects,
                    probability: buf.read_f32()?,
                }
            }
            1 => {
                let count = buf.read_varint()?;
                let targets = if count == 0 {
                    EffectTargets::NamedTag(buf.read_identifier()?)
                } else {
                    if count < 1 {
                        return Err(invalid());
                    }
                    let count = usize::try_from(count - 1).map_err(|_| invalid())?;
                    if count > 256 {
                        return Err(invalid());
                    }
                    let mut ids = Vec::with_capacity(count);
                    for _ in 0..count {
                        ids.push(identifier(registry_name(
                            "mob_effect",
                            buf.read_varint()?,
                        )?)?);
                    }
                    EffectTargets::Direct(ids)
                };
                DeathEffect::Remove(targets)
            }
            2 => DeathEffect::Clear,
            3 => DeathEffect::Teleport {
                diameter: buf.read_f32()?,
            },
            4 => {
                let holder = buf.read_varint()?;
                let sound = if holder == 0 {
                    let id = buf.read_identifier()?;
                    let fixed_range = if buf.read_bool()? {
                        Some(buf.read_f32()?)
                    } else {
                        None
                    };
                    Sound {
                        id,
                        fixed_range,
                        registered: false,
                    }
                } else {
                    if holder < 1 {
                        return Err(invalid());
                    }
                    Sound {
                        id: identifier(registry_name("sound_event", holder - 1)?)?,
                        fixed_range: None,
                        registered: true,
                    }
                };
                DeathEffect::PlaySound(sound)
            }
            _ => return Err(invalid()),
        })
    })?;
    let value = death_protection::to_nbt(&effects).map_err(CodecError::NotSupported)?;
    death_protection::parse(&value).map_err(CodecError::NotSupported)?;
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_negative_holders_are_errors_without_overflow() {
        for kind in [1, 4] {
            for value in [-1, i32::MIN] {
                let mut bytes = Vec::new();
                bytes.write_varint(1);
                bytes.write_varint(kind);
                bytes.write_varint(value);
                assert!(
                    decode(&mut bytes.as_slice()).is_err(),
                    "kind={kind} value={value}"
                );
            }
        }
    }

    #[test]
    fn hidden_effect_depth_limits_and_truncated_wire_fail_closed() {
        for depth in [8, 9, 31, 32] {
            let mut bytes = Vec::new();
            bytes.write_varint(1); // actions
            bytes.write_varint(0); // apply
            bytes.write_varint(1); // effects
            bytes.write_varint(registry_id("mob_effect", "minecraft:speed").unwrap());
            for level in 0..=depth {
                bytes.extend([0, 20, 0, 1, 1, u8::from(level < depth)]);
            }
            bytes.write_f32(0.5);
            let result = decode(&mut bytes.as_slice());
            if depth == 32 {
                assert!(result.is_err());
            } else {
                result.unwrap();
                for end in 0..bytes.len() {
                    assert!(
                        decode(&mut &bytes[..end]).is_err(),
                        "depth {depth}, end {end}"
                    );
                }
            }
        }
        for (kind, value) in [
            (None, i32::MIN),
            (None, i32::MAX),
            (Some(0), i32::MIN),
            (Some(0), i32::MAX),
            (Some(1), i32::MIN),
            (Some(1), i32::MAX),
            (Some(4), i32::MIN),
            (Some(4), i32::MAX),
        ] {
            let mut bytes = Vec::new();
            if let Some(kind) = kind {
                bytes.write_varint(1);
                bytes.write_varint(kind);
            }
            bytes.write_varint(value);
            assert!(decode(&mut bytes.as_slice()).is_err(), "{kind:?}: {value}");
        }
    }

    #[test]
    fn direct_and_registered_sound_and_effect_set_holders_roundtrip() {
        let name = |value: &str| Identifier::parse(value).unwrap();
        let values = vec![
            DeathEffect::Apply {
                effects: vec![mc_data::item_stack::PotionEffect {
                    id: name("minecraft:regeneration"),
                    amplifier: 1,
                    duration: 120,
                    ambient: false,
                    show_particles: true,
                    show_icon: true,
                    hidden: None,
                }],
                probability: 0.75,
            },
            DeathEffect::Clear,
            DeathEffect::Teleport { diameter: 32.0 },
            DeathEffect::Remove(EffectTargets::NamedTag(name("minecraft:test"))),
            DeathEffect::Remove(EffectTargets::Direct(vec![])),
            DeathEffect::Remove(EffectTargets::Direct(vec![name("minecraft:speed")])),
            DeathEffect::PlaySound(Sound {
                id: name("minecraft:block.bell.use"),
                fixed_range: None,
                registered: true,
            }),
            DeathEffect::PlaySound(Sound {
                id: name("example:custom"),
                fixed_range: Some(42.0),
                registered: false,
            }),
        ];
        let nbt = death_protection::to_nbt(&values).unwrap();
        let mut bytes = Vec::new();
        encode(&mut bytes, &nbt).unwrap();
        let mut input = bytes.as_slice();
        let decoded = decode(&mut input).unwrap();
        assert!(input.is_empty());
        assert_eq!(death_protection::parse(&decoded).unwrap(), values);
    }
}
