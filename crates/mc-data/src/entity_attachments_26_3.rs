//! Vanilla 26.3 entity attachment points used for seating a rider: the first
//! PASSENGER point of the vehicle and the VEHICLE point of the rider, in blocks
//! relative to the entity's feet before its yaw rotation, as `EntityType`
//! registration sets them (local `javap -c -p -constants` of
//! `net.minecraft.world.entity.EntityTypes` in server-26.3.jar, extracted by
//! receipts/BvContact/attachments.py: `passengerAttachments(float...)` is
//! `(0, y, 0)`, `ridingOffset(f)` is a VEHICLE point `(0, -f, 0)`, players and
//! mannequins carry `Avatar.DEFAULT_VEHICLE_ATTACHMENT` `(0, 0.6, 0)`).
//! Types not listed keep `EntityAttachment.createFallbackPoints`: the
//! PASSENGER point at the top of the box and the VEHICLE point at the feet;
//! a `None` PASSENGER point below is that fallback too. Code overrides of
//! `getPassengerAttachmentPoint` (boats, camels, a rearing horse) are not here.

/// Attachment points of one entity type (`f32` as vanilla stores them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attachments {
    /// First PASSENGER point; `None` is the fallback `(0, height, 0)`.
    pub passenger: Option<[f32; 3]>,
    /// VEHICLE point.
    pub vehicle: [f32; 3],
}

const DEFAULT: Attachments = Attachments {
    passenger: None,
    vehicle: [0.0; 3],
};

/// Types whose attachments differ from the fallback, sorted by name.
const TABLE: &[(&str, Option<[f32; 3]>, [f32; 3])] = &[
    ("minecraft:allay", None, [0.0, -0.04, 0.0]),
    ("minecraft:bogged", None, [0.0, 0.7, 0.0]),
    ("minecraft:cat", Some([0.0, 0.5125, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:chest_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:chicken", Some([0.0, 0.7, -0.1]), [0.0, 0.0, 0.0]),
    (
        "minecraft:command_block_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:cow", Some([0.0, 1.36875, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:donkey",
        Some([0.0, 1.1125, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:drowned",
        Some([0.0, 2.0125, 0.0]),
        [0.0, 0.7, 0.0],
    ),
    (
        "minecraft:elder_guardian",
        Some([0.0, 2.350625, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:ender_dragon",
        Some([0.0, 3.0, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:enderman",
        Some([0.0, 2.80625, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:endermite",
        Some([0.0, 0.2375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:evoker", Some([0.0, 2.0, 0.0]), [0.0, 0.6, 0.0]),
    ("minecraft:fox", Some([0.0, 0.6375, -0.25]), [0.0, 0.0, 0.0]),
    ("minecraft:frog", Some([0.0, 0.375, -0.25]), [0.0, 0.0, 0.0]),
    (
        "minecraft:furnace_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:ghast",
        Some([0.0, 4.0625, 0.0]),
        [0.0, -0.5, 0.0],
    ),
    ("minecraft:giant", None, [0.0, 3.75, 0.0]),
    ("minecraft:goat", Some([0.0, 1.1125, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:guardian",
        Some([0.0, 0.975, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:happy_ghast",
        Some([0.0, 4.0, 1.7]),
        [0.0, -0.5, 0.0],
    ),
    (
        "minecraft:hoglin",
        Some([0.0, 1.49375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:hopper_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:horse",
        Some([0.0, 1.44375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:husk", Some([0.0, 2.075, 0.0]), [0.0, 0.7, 0.0]),
    (
        "minecraft:illusioner",
        Some([0.0, 2.0, 0.0]),
        [0.0, 0.6, 0.0],
    ),
    ("minecraft:llama", Some([0.0, 1.37, -0.3]), [0.0, 0.0, 0.0]),
    ("minecraft:mannequin", None, [0.0, 0.6, 0.0]),
    (
        "minecraft:minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:mooshroom",
        Some([0.0, 1.36875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:mule", Some([0.0, 1.2125, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:nautilus",
        Some([0.0, 1.1375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:ocelot",
        Some([0.0, 0.6375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:parched", None, [0.0, 0.7, 0.0]),
    (
        "minecraft:parrot",
        Some([0.0, 0.4625, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:phantom",
        Some([0.0, 0.3375, 0.0]),
        [0.0, 0.125, 0.0],
    ),
    ("minecraft:pig", Some([0.0, 0.86875, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:piglin",
        Some([0.0, 2.0125, 0.0]),
        [0.0, 0.7, 0.0],
    ),
    (
        "minecraft:piglin_brute",
        Some([0.0, 2.0125, 0.0]),
        [0.0, 0.7, 0.0],
    ),
    ("minecraft:pillager", Some([0.0, 2.0, 0.0]), [0.0, 0.6, 0.0]),
    ("minecraft:player", None, [0.0, 0.6, 0.0]),
    (
        "minecraft:ravager",
        Some([0.0, 2.2625, -0.0625]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:sheep", Some([0.0, 1.2375, 0.0]), [0.0, 0.0, 0.0]),
    (
        "minecraft:silverfish",
        Some([0.0, 0.2375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:skeleton", None, [0.0, 0.7, 0.0]),
    (
        "minecraft:skeleton_horse",
        Some([0.0, 1.31875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:sniffer",
        Some([0.0, 2.09375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:spawner_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:spider", Some([0.0, 0.765, 0.0]), [0.0, 0.0, 0.0]),
    ("minecraft:stray", None, [0.0, 0.7, 0.0]),
    (
        "minecraft:tnt_minecart",
        Some([0.0, 0.1875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:trader_llama",
        Some([0.0, 1.37, -0.3]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:turtle",
        Some([0.0, 0.55625, -0.25]),
        [0.0, 0.0, 0.0],
    ),
    ("minecraft:vex", Some([0.0, 0.7375, 0.0]), [0.0, -0.04, 0.0]),
    (
        "minecraft:vindicator",
        Some([0.0, 2.0, 0.0]),
        [0.0, 0.6, 0.0],
    ),
    ("minecraft:warden", Some([0.0, 3.15, 0.0]), [0.0, 0.0, 0.0]),
    ("minecraft:witch", Some([0.0, 2.2625, 0.0]), [0.0, 0.0, 0.0]),
    ("minecraft:wither_skeleton", None, [0.0, 0.875, 0.0]),
    (
        "minecraft:wolf",
        Some([0.0, 0.81875, -0.0625]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:zoglin",
        Some([0.0, 1.49375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:zombie",
        Some([0.0, 2.0125, 0.0]),
        [0.0, 0.7, 0.0],
    ),
    (
        "minecraft:zombie_horse",
        Some([0.0, 1.31875, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:zombie_nautilus",
        Some([0.0, 1.1375, 0.0]),
        [0.0, 0.0, 0.0],
    ),
    (
        "minecraft:zombie_villager",
        Some([0.0, 2.125, 0.0]),
        [0.0, 0.7, 0.0],
    ),
    (
        "minecraft:zombified_piglin",
        Some([0.0, 2.0, 0.0]),
        [0.0, 0.7, 0.0],
    ),
];

/// Attachment points of the entity type `name` (`minecraft:horse`).
#[must_use]
pub fn attachments(name: &str) -> Attachments {
    TABLE
        .binary_search_by(|row| row.0.cmp(name))
        .map_or(DEFAULT, |index| Attachments {
            passenger: TABLE[index].1,
            vehicle: TABLE[index].2,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_carries_the_javap_points() {
        assert!(TABLE.windows(2).all(|pair| pair[0].0 < pair[1].0));
        assert_eq!(
            attachments("minecraft:horse").passenger,
            Some([0.0, 1.443_75, 0.0])
        );
        assert_eq!(attachments("minecraft:husk").vehicle, [0.0, 0.7, 0.0]);
        assert_eq!(attachments("minecraft:player").vehicle, [0.0, 0.6, 0.0]);
        assert_eq!(attachments("minecraft:creeper"), DEFAULT);
    }
}
