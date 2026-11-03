use super::*;
use crate::block::BlockStateId;

fn read_varint(bytes: &[u8], cursor: &mut usize) -> u32 {
    let mut value = 0;
    for shift in (0..35).step_by(7) {
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= u32::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
    }
    panic!("invalid VarInt");
}

#[test]
fn compact_block_palettes_emit_vanilla_words_and_projected_states() {
    // Both ends of each compact-width range and the indirect/direct boundary.
    for palette_len in [1, 2, 3, 4, 5, 8, 9, 16, 17, 256, 257] {
        let mut section = ChunkSection::filled(BlockStateId(0), BlockStateId(0));
        for cell in 0..SECTION_VOLUME {
            section.set(
                (cell % 16) as u8,
                (cell / 256) as u8,
                ((cell / 16) % 16) as u8,
                BlockStateId((cell % palette_len) as u32),
            );
        }
        let mut bytes = Vec::new();
        encode_block_palette(&mut bytes, &section, 15, &|state| {
            BlockStateId(state.0 + 1000)
        });
        let bits = bytes[0] as usize;
        let mut cursor = 1;
        if palette_len == 1 {
            assert_eq!(bits, 0);
            assert_eq!(read_varint(&bytes, &mut cursor), 1000);
        } else {
            let expected_bits = if palette_len > 256 {
                15
            } else {
                ((palette_len - 1).ilog2() as usize + 1).max(4)
            };
            assert_eq!(bits, expected_bits);
            let palette = if palette_len <= 256 {
                assert_eq!(read_varint(&bytes, &mut cursor) as usize, palette_len);
                (0..palette_len)
                    .map(|_| read_varint(&bytes, &mut cursor))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let entries_per_word = 64 / bits;
            let word_count = SECTION_VOLUME.div_ceil(entries_per_word);
            assert_eq!(bytes.len() - cursor, word_count * 8);
            for word_idx in 0..word_count {
                let word = u64::from_be_bytes(bytes[cursor..cursor + 8].try_into().unwrap());
                cursor += 8;
                let entries = entries_per_word.min(SECTION_VOLUME - word_idx * entries_per_word);
                for offset in 0..entries {
                    let cell = word_idx * entries_per_word + offset;
                    let value = ((word >> (offset * bits)) & ((1 << bits) - 1)) as u32;
                    let state = if palette_len <= 256 {
                        palette[value as usize]
                    } else {
                        value
                    };
                    assert_eq!(state, (cell % palette_len) as u32 + 1000);
                }
                if entries * bits < 64 {
                    assert_eq!(word >> (entries * bits), 0);
                }
            }
        }
        assert_eq!(cursor, bytes.len());
    }
}

#[test]
fn direct_palette_preserves_high_26_3_states_and_projection() {
    // 26.3 reports/blocks.json has dense IDs 0..=35722; the last is
    // minecraft:firefly_bush. 26.1.2 has 29873 states, all below 32768.
    let registry_size = 35_723;
    let bits = bits_for_distinct_values(registry_size);
    assert_eq!(bits, 16);
    let mut section = ChunkSection::filled(BlockStateId(0), BlockStateId(0));
    let state_at = |cell: usize| {
        if cell % 257 == 256 {
            35_722
        } else {
            (cell % 257) as u32
        }
    };
    for cell in 0..SECTION_VOLUME {
        section.set(
            (cell % 16) as u8,
            (cell / 256) as u8,
            ((cell / 16) % 16) as u8,
            BlockStateId(state_at(cell)),
        );
    }
    // Exercise projection in both directions without inventing registry IDs.
    let project = |state: BlockStateId| match state.0 {
        1 => BlockStateId(35_722),
        35_722 => BlockStateId(1),
        _ => state,
    };
    let mut bytes = Vec::new();
    encode_block_palette(&mut bytes, &section, bits, &project);
    assert_eq!(bytes[0], 16);
    let per_word = 64 / usize::from(bytes[0]);
    assert_eq!(bytes.len(), 1 + SECTION_VOLUME.div_ceil(per_word) * 8);
    for cell in 0..SECTION_VOLUME {
        let start = 1 + (cell / per_word) * 8;
        let word = u64::from_be_bytes(bytes[start..start + 8].try_into().unwrap());
        let decoded = ((word >> ((cell % per_word) * usize::from(bits))) & 0xffff) as u32;
        assert_eq!(
            decoded,
            project(BlockStateId(state_at(cell))).0,
            "cell {cell}"
        );
    }
}
