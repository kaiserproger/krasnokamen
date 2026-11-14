//! Immutable vanilla identities for operator-built battle kits and bodies.
//! Visual component variants remain authored by staging; no loadout cache.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(usize)]
pub(super) enum KitItem {
    Spear,
    Shield,
    Helmet,
    Plate,
    Chain,
    Bow,
    LeatherChest,
    Sword,
    Legs,
    HorseArmor,
    Trident,
}

impl KitItem {
    const ALL: [Self; 11] = [
        Self::Spear,
        Self::Shield,
        Self::Helmet,
        Self::Plate,
        Self::Chain,
        Self::Bow,
        Self::LeatherChest,
        Self::Sword,
        Self::Legs,
        Self::HorseArmor,
        Self::Trident,
    ];

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Spear => "minecraft:iron_spear",
            Self::Shield => mc_data::items::SHIELD,
            Self::Helmet => mc_data::items::IRON_HELMET,
            Self::Plate => mc_data::items::IRON_CHESTPLATE,
            Self::Chain => mc_data::items::CHAINMAIL_CHESTPLATE,
            Self::Bow => mc_data::items::BOW,
            Self::LeatherChest => mc_data::items::LEATHER_CHESTPLATE,
            Self::Sword => mc_data::items::IRON_SWORD,
            Self::Legs => "minecraft:leather",
            Self::HorseArmor => mc_data::items::IRON_HORSE_ARMOR,
            Self::Trident => mc_data::items::TRIDENT,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) struct EntityType {
    pub(super) name: &'static str,
    pub(super) id: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct Ids {
    items: [u32; KitItem::ALL.len()],
    pub(super) husk: EntityType,
    pub(super) horse: EntityType,
}

impl Ids {
    pub(crate) fn load(
        items: &mc_data::items::ItemRegistry,
        entities: &mc_data::entity_types::EntityTypeRegistry,
    ) -> Result<Self, String> {
        let mut resolved = [0; KitItem::ALL.len()];
        for item in KitItem::ALL {
            let name = mc_data::Identifier::parse(item.name()).expect("battle item identifier");
            resolved[item as usize] = items
                .id_of(&name)
                .ok_or_else(|| format!("missing battle item {name}"))?;
        }
        let entity = |name: &'static str| {
            let identifier = mc_data::Identifier::parse(name).expect("battle entity identifier");
            entities
                .id_of(&identifier)
                .map(|id| EntityType { name, id })
                .ok_or_else(|| format!("missing battle entity {name}"))
        };
        Ok(Self {
            items: resolved,
            husk: entity(mc_data::entity_types::HUSK)?,
            horse: entity(mc_data::entity_types::HORSE)?,
        })
    }

    pub(super) fn item(&self, item: KitItem) -> u32 {
        self.items[item as usize]
    }
}
