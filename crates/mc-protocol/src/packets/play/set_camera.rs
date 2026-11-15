//! Camera entity selection. Local GameProtocols/ClientboundSetCameraPacket
//! javap evidence: 775/776 id 0x5d, 777 id 0x5f; one signed VarInt in all three.
//! The edge's adjacent chain maps the canonical packet id by registered name.
use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientboundSetCamera {
    pub entity_id: i32,
}

impl Packet for ClientboundSetCamera {
    const ID: i32 = 0x5f;

    fn encode<B: BufMut>(&self, buf: &mut B) -> Result<(), CodecError> {
        buf.write_varint(self.entity_id);
        Ok(())
    }

    fn decode<B: Buf>(buf: &mut B) -> Result<Self, CodecError> {
        Ok(Self {
            entity_id: buf.read_varint()?,
        })
    }
}

#[cfg(test)]
#[path = "set_camera_tests.rs"]
mod tests;
