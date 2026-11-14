//! Vanilla death-protection consume actions carried by item component NBT.
use super::{Identifier, PotionEffect, station_nbt};
use crate::item_nbt::field;
use mc_nbt::{ListTag, Tag};

#[derive(Debug, Clone, PartialEq)]
pub enum EffectTargets {
    NamedTag(Identifier),
    Direct(Vec<Identifier>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Sound {
    pub id: Identifier,
    pub fixed_range: Option<f32>,
    pub registered: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DeathEffect {
    Apply {
        effects: Vec<PotionEffect>,
        probability: f32,
    },
    Remove(EffectTargets),
    Clear,
    Teleport {
        diameter: f32,
    },
    PlaySound(Sound),
}

fn compound(tag: &Tag) -> Result<&[(String, Tag)], &'static str> {
    match tag {
        Tag::Compound(fields) => Ok(fields),
        _ => Err("death protection effect must be a compound"),
    }
}

fn name(tag: &Tag) -> Result<Identifier, &'static str> {
    match tag {
        Tag::String(value) => Identifier::parse(value.clone()).map_err(|_| "invalid identifier"),
        _ => Err("expected identifier string"),
    }
}

fn effect_name(tag: &Tag) -> Result<Identifier, &'static str> {
    let id = name(tag)?;
    if crate::mob_effects_26_1_2::MobEffect::from_name(id.as_str()).is_none() {
        return Err("unknown mob effect");
    }
    Ok(id)
}

fn float(tag: &Tag) -> Result<f32, &'static str> {
    let value = match tag {
        Tag::Byte(v) => f32::from(*v),
        Tag::Short(v) => f32::from(*v),
        Tag::Int(v) => *v as f32,
        Tag::Long(v) => *v as f32,
        Tag::Float(v) => *v,
        Tag::Double(v) => *v as f32,
        _ => return Err("expected numeric value"),
    };
    if value.is_finite() {
        Ok(value)
    } else {
        Err("non-finite value")
    }
}

pub fn parse(value: &Tag) -> Result<Vec<DeathEffect>, &'static str> {
    let root = compound(value)?;
    let Some(value) = field(root, "death_effects") else {
        return Ok(Vec::new());
    };
    let Tag::List(list) = value else {
        return Err("death_protection death_effects must be a list");
    };
    if list.elements.len() > 256 {
        return Err("too many death protection effects");
    }
    list.elements
        .iter()
        .map(|value| {
            let fields = compound(value)?;
            let kind = name(field(fields, "type").ok_or("missing consume effect type")?)?;
            match kind.as_str() {
                "minecraft:apply_effects" => {
                    let Some(Tag::List(effects)) = field(fields, "effects") else {
                        return Err("apply_effects requires effects list");
                    };
                    if effects.elements.len() > super::MAX_POTION_CUSTOM_EFFECTS {
                        return Err("too many status effects");
                    }
                    let effects = effects
                        .elements
                        .iter()
                        .map(|tag| station_nbt::potion_effect_from_nbt(tag, None, 0))
                        .collect::<Result<Vec<_>, _>>()?;
                    let probability = field(fields, "probability")
                        .map(float)
                        .transpose()?
                        .unwrap_or(1.0);
                    if !(0.0..=1.0).contains(&probability) {
                        return Err("probability must be between zero and one");
                    }
                    Ok(DeathEffect::Apply {
                        effects,
                        probability,
                    })
                }
                "minecraft:remove_effects" => {
                    let targets =
                        field(fields, "effects").ok_or("remove_effects requires effects")?;
                    let targets = match targets {
                        Tag::String(value) if value.starts_with('#') => EffectTargets::NamedTag(
                            Identifier::parse(value[1..].to_owned())
                                .map_err(|_| "invalid effect tag")?,
                        ),
                        Tag::String(_) => EffectTargets::Direct(vec![effect_name(targets)?]),
                        Tag::List(values) if values.elements.len() <= 256 => EffectTargets::Direct(
                            values
                                .elements
                                .iter()
                                .map(effect_name)
                                .collect::<Result<_, _>>()?,
                        ),
                        _ => return Err("invalid remove_effects targets"),
                    };
                    Ok(DeathEffect::Remove(targets))
                }
                "minecraft:clear_all_effects" => Ok(DeathEffect::Clear),
                "minecraft:teleport_randomly" => {
                    let diameter = field(fields, "diameter")
                        .map(float)
                        .transpose()?
                        .unwrap_or(16.0);
                    if diameter <= 0.0 {
                        return Err("teleport diameter must be positive");
                    }
                    Ok(DeathEffect::Teleport { diameter })
                }
                "minecraft:play_sound" => {
                    let sound = field(fields, "sound").ok_or("play_sound requires sound")?;
                    let sound = match sound {
                        Tag::String(_) => {
                            let id = name(sound)?;
                            if crate::builtin_sounds_26_3::id(id.as_str()).is_none() {
                                return Err("unknown registered sound");
                            }
                            Sound {
                                id,
                                fixed_range: None,
                                registered: true,
                            }
                        }
                        Tag::Compound(fields) => {
                            let id =
                                name(field(fields, "sound_id").ok_or("sound requires sound_id")?)?;
                            let fixed_range = field(fields, "range").map(float).transpose()?;
                            Sound {
                                id,
                                fixed_range,
                                registered: false,
                            }
                        }
                        _ => return Err("invalid sound holder"),
                    };
                    Ok(DeathEffect::PlaySound(sound))
                }
                _ => Err("unknown death protection consume effect"),
            }
        })
        .collect()
}

pub fn to_nbt(effects: &[DeathEffect]) -> Result<Tag, &'static str> {
    let mut result = Vec::with_capacity(effects.len());
    for effect in effects {
        let mut fields = Vec::new();
        let kind = match effect {
            DeathEffect::Apply {
                effects,
                probability,
            } => {
                let effects = effects
                    .iter()
                    .map(|effect| station_nbt::potion_effect_to_nbt(effect, None, 0))
                    .collect::<Result<Vec<_>, _>>()?;
                fields.push((
                    "effects".into(),
                    Tag::List(ListTag {
                        element_type: mc_nbt::tag_type::COMPOUND,
                        elements: effects,
                    }),
                ));
                fields.push(("probability".into(), Tag::Float(*probability)));
                "apply_effects"
            }
            DeathEffect::Remove(targets) => {
                let value = match targets {
                    EffectTargets::NamedTag(tag) => Tag::String(format!("#{tag}")),
                    EffectTargets::Direct(ids) => Tag::List(ListTag {
                        element_type: mc_nbt::tag_type::STRING,
                        elements: ids
                            .iter()
                            .map(|id| Tag::String(id.as_str().into()))
                            .collect(),
                    }),
                };
                fields.push(("effects".into(), value));
                "remove_effects"
            }
            DeathEffect::Clear => "clear_all_effects",
            DeathEffect::Teleport { diameter } => {
                fields.push(("diameter".into(), Tag::Float(*diameter)));
                "teleport_randomly"
            }
            DeathEffect::PlaySound(sound) => {
                let value = if sound.registered {
                    Tag::String(sound.id.as_str().into())
                } else {
                    let mut value =
                        vec![("sound_id".into(), Tag::String(sound.id.as_str().into()))];
                    if let Some(range) = sound.fixed_range {
                        value.push(("range".into(), Tag::Float(range)));
                    }
                    Tag::Compound(value)
                };
                fields.push(("sound".into(), value));
                "play_sound"
            }
        };
        fields.push(("type".into(), Tag::String(format!("minecraft:{kind}"))));
        result.push(Tag::Compound(fields));
    }
    Ok(Tag::Compound(vec![(
        "death_effects".into(),
        Tag::List(ListTag {
            element_type: mc_nbt::tag_type::COMPOUND,
            elements: result,
        }),
    )]))
}
