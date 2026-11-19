//! Horizon VERSION 1 wire format; reference `lod_net.rs:76-247` at bd431b38.
use bytes::BufMut;
use mc_protocol::codec::{ReadMc, WriteMc};

#[derive(Clone, Copy, Debug)]
pub(super) struct Request {
    pub id: u32,
    pub x: i32,
    pub z: i32,
    pub stride: i32,
}

pub(super) fn offer(distance: u32) -> Vec<u8> {
    let mut out = Vec::new();
    for v in [1, distance as i32, 8, 64] {
        out.write_varint(v);
    }
    out
}

pub(super) fn request(mut bytes: &[u8]) -> anyhow::Result<Request> {
    let id = bytes.read_varint()? as u32;
    let x = bytes.read_i32()?;
    let z = bytes.read_i32()?;
    let stride = bytes.read_varint()?;
    let _approximate = bytes.read_u8()?;
    anyhow::ensure!(bytes.is_empty(), "trailing Horizon request bytes");
    Ok(Request { id, x, z, stride })
}

pub(super) fn refusal(id: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.write_varint(id as i32);
    out.put_u8(1);
    out
}

/// Only known stored surface blocks are emitted. A missing sample refuses the
/// whole tile instead of replacing the client's existing terrain with emptiness.
pub(super) fn reply(id: u32, samples: &[mc_core::lod::LodSample]) -> Vec<u8> {
    use mc_core::lod::LodOrigin;
    if samples.len() != 256
        || samples
            .iter()
            .any(|s| s.origin != LodOrigin::Stored || (s.block.is_some() && s.biome.is_none()))
    {
        return refusal(id);
    }
    let mut names = Vec::new();
    for sample in samples {
        if let (Some(_), Some(name)) = (sample.block, &sample.biome)
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    let mut out = Vec::new();
    out.write_varint(id as i32);
    out.put_u8(0);
    out.write_varint(names.len() as i32);
    for name in &names {
        let name = name.to_string();
        if name.len() > 256 {
            return refusal(id);
        }
        out.write_varint(name.len() as i32);
        out.extend_from_slice(name.as_bytes());
    }
    out.write_varint(256);
    for sample in samples {
        if let (Some(state), Some(name)) = (sample.block, &sample.biome) {
            let Some(height) = sample.height.checked_add(1) else {
                return refusal(id);
            };
            if !(-63..=4096).contains(&height) {
                return refusal(id);
            }
            let biome = names.iter().position(|n| *n == name).unwrap() as i32;
            out.put_u8(1);
            out.put_i32(height);
            out.write_varint(state.0 as i32);
            out.put_u8(0);
            out.write_varint(biome);
            out.write_varint(1);
            out.put_i16(-64);
            out.put_i16(height as i16);
            out.write_varint(state.0 as i32);
            out.write_varint(biome);
            out.put_u16(240);
        } else {
            out.put_u8(0);
            out.write_varint(0);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use mc_core::lod::{LodOrigin, LodSample};
    use mc_data::Identifier;
    use mc_world::BlockStateId;

    #[test]
    fn horizon_offer_and_request_reference_bytes() {
        // Horizon lod_net.rs:94-100,130-137 (pin bd431b38).
        assert_eq!(offer(256), [1, 0x80, 2, 8, 64]);
        let request = request(&[7, 0xff, 0xff, 0xff, 0xf0, 0, 0, 0, 32, 2, 1]).unwrap();
        assert_eq!(
            (request.id, request.x, request.z, request.stride),
            (7, -16, 32, 2)
        );
    }

    #[test]
    fn horizon_reply_reference_decoder() {
        // Wire reader follows Horizon's decode_reply, lod_net.rs:263-354.
        let sample = LodSample {
            origin: LodOrigin::Stored,
            height: 63,
            block: Some(BlockStateId(1)),
            biome: Some(Identifier::parse("minecraft:plains").unwrap()),
        };
        let bytes = reply(7, &vec![sample.clone(); 256]);
        let mut data = bytes.as_slice();
        assert_eq!(data.read_varint().unwrap(), 7);
        assert_eq!(data.read_u8().unwrap(), 0);
        assert_eq!(data.read_varint().unwrap(), 1);
        assert_eq!(data.read_string(256).unwrap(), "minecraft:plains");
        assert_eq!(data.read_varint().unwrap(), 256);
        for _ in 0..256 {
            assert_ne!(data.read_u8().unwrap(), 0);
            assert_eq!(data.read_i32().unwrap(), 64);
            assert_eq!(data.read_varint().unwrap(), 1);
            assert_eq!(data.read_u8().unwrap(), 0);
            assert_eq!(data.read_varint().unwrap(), 0);
            assert_eq!(data.read_varint().unwrap(), 1);
            assert_eq!(
                (data.read_i16().unwrap(), data.read_i16().unwrap()),
                (-64, 64)
            );
            assert_eq!(data.read_varint().unwrap(), 1);
            assert_eq!(data.read_varint().unwrap(), 0);
            assert_eq!(data.read_u16().unwrap(), 240);
        }
        assert!(data.is_empty());
        let mut samples = vec![sample; 256];
        samples[5].origin = LodOrigin::Missing;
        assert_eq!(reply(7, &samples), [7, 1]);
        samples[5].origin = LodOrigin::Stored;
        samples[5].biome = None;
        assert_eq!(reply(7, &samples), [7, 1]);
    }
}
