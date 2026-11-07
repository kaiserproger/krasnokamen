"""Received 26.1/26.3 palettes, matching mc-edge/src/legacy/chunk.rs.

Keep compact palettes rather than allocating one Python object per block. The
caller must supply the actual dimension minimum Y; an unknown dimension never
turns into guessed air. Block-entity/light tails are outside this observer.
"""
import struct


def decode_chunk(body):
    from .wire import read_varint_bytes
    x, z = struct.unpack_from('>ii', body)
    offset = 8
    count, offset = read_varint_bytes(body, offset)
    if not 0 <= count <= 16:
        raise RuntimeError('invalid heightmap count')
    for _ in range(count):
        _, offset = read_varint_bytes(body, offset)
        length, offset = read_varint_bytes(body, offset)
        if not 0 <= length <= 4096 or offset + length * 8 > len(body):
            raise RuntimeError('invalid heightmap array')
        offset += length * 8
    size, offset = read_varint_bytes(body, offset)
    if not 0 <= size <= 2**21 or offset + size > len(body):
        raise RuntimeError('invalid section payload length')
    payload = memoryview(body)[offset:offset + size]
    cursor = 0

    def container(volume, indirect):
        nonlocal cursor
        if cursor >= len(payload):
            raise RuntimeError('truncated palette')
        bits = payload[cursor]
        cursor += 1
        palette = None
        if bits == 0:
            value, cursor = read_varint_bytes(payload, cursor)
            return (0, (value,), b'')
        if not 1 <= bits <= 32:
            raise RuntimeError('invalid palette bit width')
        if bits <= indirect:
            length, cursor = read_varint_bytes(payload, cursor)
            if not 1 <= length <= min(volume, 1 << bits):
                raise RuntimeError('invalid palette length')
            palette = []
            for _ in range(length):
                value, cursor = read_varint_bytes(payload, cursor)
                palette.append(value)
        words = (volume + 64 // bits - 1) // (64 // bits)
        end = cursor + words * 8
        if end > len(payload):
            raise RuntimeError('truncated packed palette')
        packed = bytes(payload[cursor:end])
        cursor = end
        return bits, palette, packed

    sections = []
    while cursor < len(payload):
        if len(sections) >= 256 or cursor + 4 > len(payload):
            raise RuntimeError('invalid section header')
        cursor += 4  # non-air and fluid i16 counts
        sections.append(container(4096, 8))
        container(64, 3)
    return (x, z), sections


def block_state(sections, position, min_y):
    x, y, z = position
    section = (y - min_y) // 16
    if not 0 <= section < len(sections):
        raise RuntimeError('block outside received dimension sections')
    bits, palette, packed = sections[section]
    if bits == 0:
        return palette[0]
    index = ((y % 16) << 8) | ((z % 16) << 4) | (x % 16)
    per_word = 64 // bits
    word = struct.unpack_from('>Q', packed, (index // per_word) * 8)[0]
    value = (word >> ((index % per_word) * bits)) & ((1 << bits) - 1)
    if palette is None:
        return value
    if value >= len(palette):
        raise RuntimeError('palette index outside received palette')
    return palette[value]
