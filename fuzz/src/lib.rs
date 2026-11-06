//! Decoder tables shared by the fuzz targets and the seed-corpus generator,
//! so a seed's selector byte always names the packet it was encoded from.

use bytes::Bytes;
use mc_protocol::packets::play::*;
use mc_protocol::packets::{
    ClientInformation, CustomPayload, Packet, ResourcePackStatus, configuration, handshake, login,
    status,
};

/// One named body decoder. The body is exactly what the framing layer hands over.
pub type Decoder = (&'static str, fn(&[u8]));

/// Decodes from a `&[u8]` cursor and from a `Bytes` cursor: the edge uses both.
fn decode<P: Packet>(body: &[u8]) {
    let mut slice = body;
    let _ = P::decode(&mut slice);
    let mut bytes = Bytes::copy_from_slice(body);
    let _ = P::decode(&mut bytes);
}

/// Defines the serverbound Play decoder table and the packet-id table in the
/// same order, so one selector byte names the same packet in both.
macro_rules! play_packets {
    ($($ty:ty),* $(,)?) => {
        /// Every serverbound Play packet.
        pub const PLAY: &[Decoder] = &[$((stringify!($ty), decode::<$ty> as fn(&[u8]))),*];
        /// Wire ids of [`PLAY`], same order.
        pub const PLAY_IDS: &[i32] = &[$(<$ty as Packet>::ID),*];
    };
}

/// Serverbound Handshake, Status, Login and Configuration packets plus the
/// shared custom-payload / client-information / resource-pack bodies.
pub const PRE_PLAY: &[Decoder] = &[
    ("Handshake", decode::<handshake::Handshake>),
    ("StatusRequest", decode::<status::StatusRequest>),
    ("PingRequest", decode::<status::PingRequest>),
    ("LoginStart", decode::<login::LoginStart>),
    ("EncryptionResponse", decode::<login::EncryptionResponse>),
    ("LoginPluginResponse", decode::<login::LoginPluginResponse>),
    ("LoginAcknowledged", decode::<login::LoginAcknowledged>),
    (
        "ConfigKnownPacks",
        decode::<configuration::ServerboundKnownPacks>,
    ),
    (
        "ConfigClientInformation",
        decode::<configuration::ServerboundClientInformation>,
    ),
    (
        "ConfigCustomPayload",
        decode::<configuration::ServerboundCustomPayload>,
    ),
    (
        "ConfigResourcePack",
        decode::<configuration::ServerboundResourcePack>,
    ),
    (
        "AcknowledgeFinishConfiguration",
        decode::<configuration::AcknowledgeFinishConfiguration>,
    ),
    ("CustomPayloadServerboundBytes", |body| {
        let _ = CustomPayload::decode_serverbound(&mut Bytes::copy_from_slice(body));
    }),
    ("CustomPayloadServerboundSlice", |body| {
        let _ = CustomPayload::decode_serverbound(&mut &body[..]);
    }),
    ("ClientInformation", |body| {
        let _ = ClientInformation::decode(&mut &body[..]);
    }),
    ("ResourcePackStatus", |body| {
        let _ = ResourcePackStatus::decode(&mut &body[..]);
    }),
];

play_packets![
    ConfirmTeleportation,
    ServerboundSelectBundleItem,
    ServerboundSeenAdvancements,
    ServerboundChangeGameMode,
    ServerboundChatCommand,
    ServerboundChatCommandSigned,
    ServerboundChatSessionUpdate,
    ServerboundChat,
    ServerboundCommandSuggestion,
    ServerboundSignUpdate,
    ServerboundEditBook,
    ServerboundChatAck,
    ServerboundChunkBatchReceived,
    ServerboundClientTickEnd,
    ServerboundPlayerLoaded,
    ServerboundContainerSlotStateChanged,
    ServerboundCustomClickAction,
    ServerboundResourcePack,
    ServerboundClientInformation,
    ServerboundCustomPayload,
    ServerboundKeepAlive,
    ServerboundAttack,
    ServerboundInteract,
    ServerboundPickItemFromBlock,
    ServerboundPickItemFromEntity,
    ServerboundMovePlayerPos,
    ServerboundMovePlayerPosRot,
    ServerboundMovePlayerRot,
    ServerboundMovePlayerStatusOnly,
    ServerboundPlayerInput,
    ServerboundPlayerCommand,
    ServerboundPlayerAction,
    ServerboundPunch,
    ServerboundUseItemOn,
    ServerboundUseItem,
    ServerboundClientCommand,
    ServerboundContainerButtonClick,
    ServerboundContainerClick,
    ServerboundContainerClose,
    ServerboundSetCarriedItem,
    ServerboundSetCreativeModeSlot,
    ServerboundPlaceRecipe,
    ServerboundRecipeBookChangeSettings,
    ServerboundRecipeBookSeenRecipe,
    ServerboundSelectTrade,
    ServerboundRenameItem,
    ServerboundSetBeacon,
    ServerboundMoveVehicle,
];

/// Runs the decoder selected by `data[0]` over `data[1..]`.
pub fn run(table: &[Decoder], data: &[u8]) {
    let Some((&selector, body)) = data.split_first() else {
        return;
    };
    (table[usize::from(selector) % table.len()].1)(body);
}

/// Selector byte for the decoder named `name` in `table`.
pub fn selector(table: &[Decoder], name: &str) -> u8 {
    let index = table
        .iter()
        .position(|(candidate, _)| *candidate == name)
        .unwrap_or_else(|| panic!("no decoder named {name}"));
    u8::try_from(index).expect("table fits one selector byte")
}
