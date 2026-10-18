"""Steam's binary VDF ("keyvalues") format: a reader and a writer that
round-trip byte for byte. Knows nothing about shortcuts.

A node is a list of ``(type, key, value)``; ``type`` is 0 (a section, whose value
is a node), 1 (a string) or 2 (a 32-bit little-endian integer).
"""

def parse(data, pos=0):
    items = []
    while True:
        kind = data[pos]
        pos += 1
        if kind == 8:
            return items, pos
        end = data.index(b"\x00", pos)
        key = data[pos:end].decode("utf-8")
        pos = end + 1
        if kind == 0:
            value, pos = parse(data, pos)
        elif kind == 1:
            end = data.index(b"\x00", pos)
            value = data[pos:end].decode("utf-8")
            pos = end + 1
        elif kind == 2:
            value = int.from_bytes(data[pos:pos + 4], "little")
            pos += 4
        else:
            raise ValueError(f"unsupported VDF value type {kind} at {pos}")
        items.append((kind, key, value))


def serialize(items):
    out = bytearray()
    for kind, key, value in items:
        out += bytes([kind]) + key.encode("utf-8") + b"\x00"
        if kind == 0:
            out += serialize(value)
        elif kind == 1:
            out += value.encode("utf-8") + b"\x00"
        else:
            out += value.to_bytes(4, "little")
    return bytes(out) + b"\x08"
