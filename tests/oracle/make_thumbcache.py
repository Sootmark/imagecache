"""Write the synthetic thumbnail caches in tests/fixtures/synthetic/thumbcache/.

No openly licensed cache written by Windows exists, so these are built
from the format as libwtcdb documents it and as real caches show it
(format versions 20, 21, 30, 31 and 32; a cache file per size and the
index). Python standard library only; deterministic.

    python3 -I tests/oracle/make_thumbcache.py tests/fixtures/synthetic/thumbcache

Each version's folder gets thumbcache_32.db and thumbcache_256.db (and,
for version 32, iconcache_16.db) with: a BMP, a PNG, a JPEG
(assets/tiny.jpg, an 8x8 picture made for this), an entry without data
naming a shell folder, a BMP over 1024 bytes (sampled data checksum),
an entry with padding, one whose data was damaged after its checksum
and one whose header was; then the free space as one empty entry to the
end of an 8 KiB file, with stale bytes in it. thumbcache_idx.db points
at every entry with a hash.
"""

import os
import struct
import sys
import zlib

POLY = 0x92C64265D32139A4
TABLE = []
for _byte in range(256):
    _crc = _byte
    for _ in range(8):
        _crc = (_crc >> 1) ^ POLY if _crc & 1 else _crc >> 1
    TABLE.append(_crc)

SIZES = {
    20: ["32", "96", "256", "1024", "sr"],
    21: ["32", "96", "256", "1024", "sr"],
    30: ["16", "32", "48", "96", "256", "1024", "sr", "wide", "exif"],
    31: ["16", "32", "48", "96", "256", "1024", "1600", "sr", "wide", "exif", "wide_alternate"],
    32: ["16", "32", "48", "96", "256", "768", "1280", "1920", "2560", "sr", "wide", "exif",
         "wide_alternate", "custom_stream"],
}
FOLDERS = {20: "vista", 21: "windows7", 30: "windows8", 31: "windows81", 32: "windows10"}
FILE_SIZE = 8 * 1024
HERE = os.path.dirname(os.path.abspath(__file__))


def crc64(data, crc):
    for byte in data:
        crc = TABLE[(crc ^ byte) & 0xFF] ^ (crc >> 8)
    return crc


def data_crc(data):
    if len(data) <= 1024:
        return crc64(data, 0)
    tail = 0
    for start in range(1024, len(data), 400):
        tail = crc64(data[start:start + 4], tail)
    return crc64(data[:1024], 0) ^ tail


def bmp(width, height, seed):
    pixels = b""
    for y in range(height):
        row = b"".join(bytes([(x * 37 + seed) % 256, (y * 53 + seed) % 256, (x * y + seed) % 256])
                       for x in range(width))
        pixels += row + b"\0" * ((4 - len(row) % 4) % 4)
    return (struct.pack("<2sIHHI", b"BM", 54 + len(pixels), 0, 0, 54)
            + struct.pack("<IiiHHIIiiII", 40, width, height, 1, 24, 0, len(pixels), 2835, 2835, 0, 0)
            + pixels)


def png(width, height, seed):
    def chunk(kind, body):
        return struct.pack(">I", len(body)) + kind + body + struct.pack(">I", zlib.crc32(kind + body))
    raw = b"".join(b"\0" + bytes([(x * 11 + y * 7 + seed) % 256 for x in range(width * 3)])
                   for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))


def utf16(text):
    return text.encode("utf-16-le")


def entry(version, hash_value, identifier, data, *, extension="", padding=0, size=(0, 0),
          damage_data=False, damage_header=False, total=None):
    ident = utf16(identifier)
    data_checksum = data_crc(data)
    if version == 20:
        fixed = struct.pack("<8s", utf16(extension).ljust(8, b"\0"))
        sizes = struct.pack("<IIII", len(ident), padding, len(data), 0)
    elif version == 21:
        fixed = b""
        sizes = struct.pack("<IIII", len(ident), padding, len(data), 0)
    else:
        fixed = b""
        sizes = struct.pack("<IIIIII", len(ident), padding, len(data), size[0], size[1], 0)
    header_len = (48 if version == 21 else 56)
    if total is None:
        total = header_len + len(ident) + padding + len(data)
    head = b"CMMM" + struct.pack("<IQ", total, hash_value) + fixed + sizes + struct.pack("<Q", data_checksum)
    assert len(head) == header_len - 8
    header_checksum = crc64(head, (1 << 64) - 1)
    if damage_header:
        header_checksum ^= 1
    if damage_data:
        data = data[:-1] + bytes([data[-1] ^ 0xFF])
    return head + struct.pack("<Q", header_checksum) + ident + b"\0" * padding + data


def cache_file(version, cache_type, entries):
    first = 24
    body = b"".join(entries)
    free = first + len(body)
    if version in (20, 21):
        header = b"CMMM" + struct.pack("<IIIII", version, cache_type, first, free, len(entries))
    else:
        header = b"CMMM" + struct.pack("<IIIII", version, cache_type, 0, first, free)
    # The free space: one entry with nothing in it, to the end, stale bytes after its header.
    free_entry = entry(version, 0, "", b"", total=FILE_SIZE - free)
    stale = bytes((i * 7) % 251 for i in range(256))
    data = header + body + free_entry + stale
    return data + b"\0" * (FILE_SIZE - len(data))


def entries_for(version, salt):
    """The entries of one cache file and their hashes, in order."""
    jpeg = open(os.path.join(HERE, "assets", "tiny.jpg"), "rb").read()
    vista = version == 20
    specs = [
        (0x1111_0000_0000_0001 ^ salt, None, bmp(4, 4, 1), dict(extension="bmp", size=(4, 4))),
        (0x2222_0000_0000_0002 ^ salt, None, png(5, 3, 2), dict(extension="png", size=(5, 3))),
        (0x3333_0000_0000_0003 ^ salt, None, jpeg, dict(extension="jpg", size=(8, 8))),
        (0x0924_bc51_f9b8_4ee8 ^ salt, "::{645FF040-5081-101B-9F08-00AA002F954E}", b"", {}),
        (0x5555_0000_0000_0005 ^ salt, None, bmp(24, 20, 5), dict(extension="bmp", size=(24, 20))),
        (0x6666_0000_0000_0006 ^ salt, None, bmp(2, 2, 6), dict(padding=2, size=(2, 2))),
        (0x7777_0000_0000_0007 ^ salt, None, png(2, 2, 7), dict(damage_data=True, size=(2, 2))),
        (0x8888_0000_0000_0008 ^ salt, None, bmp(3, 1, 8), dict(damage_header=True, size=(3, 1))),
    ]
    out = []
    for hash_value, identifier, data, options in specs:
        if not vista:
            options.pop("extension", None)
        if identifier is None:
            identifier = f"{hash_value:x}"
        out.append((hash_value, entry(version, hash_value, identifier, data, **options)))
    return out


def index_file(version, locations):
    """locations: {hash: {cache_type: offset}}"""
    count = len(SIZES[version])
    total = len(locations) + 3  # a few unused entries
    entries = b""
    for hash_value, where in locations.items():
        offsets = [0xFFFFFFFF] * count
        for cache_type, offset in where.items():
            offsets[cache_type] = offset
        packed = struct.pack(f"<{count}I", *offsets)
        if version == 20:
            entries += struct.pack("<QQI", hash_value, 0x01D9_0000_0000_0000, 0x08008002) + packed
        elif version == 21:
            entries += struct.pack("<QI", hash_value, 0x08008002) + packed
        else:
            raw = struct.pack("<QII", hash_value, 0x08008002, 0) + packed
            entries += raw + b"\0" * ((8 - len(raw) % 8) % 8)
    entry_size = len(entries) // len(locations)
    entries += b"\0" * (entry_size * (total - len(locations)))
    if version in (20, 21):
        header = b"IMMM" + struct.pack("<IIIII", version, 0, len(locations), total, 0)
    else:
        header = struct.pack("<I", 0x2030000C) + b"IMMM" + struct.pack("<IIIII", version, 0, 0, len(locations), total)
        # Then each cache file's size and entry count, padded to 8 bytes.
        header += struct.pack(f"<{count}I", *[FILE_SIZE] * count)
        header += struct.pack(f"<{count}I", *[len(locations)] * count)
        header += b"\0" * ((8 - len(header) % 8) % 8)
    return header + entries


def main(out_dir):
    for version, folder in FOLDERS.items():
        path = os.path.join(out_dir, folder)
        os.makedirs(path, exist_ok=True)
        sizes = SIZES[version]
        locations = {}
        files = [("thumbcache", "32", 0), ("thumbcache", "256", 0)]
        if version == 32:
            files.append(("iconcache", "16", 0x0202))
        for prefix, size, salt in files:
            cache_type = sizes.index(size)
            entries = entries_for(version, salt)
            data = cache_file(version, cache_type, [raw for _, raw in entries])
            with open(os.path.join(path, f"{prefix}_{size}.db"), "wb") as out:
                out.write(data)
            if prefix == "iconcache":
                continue
            offset = 24
            for hash_value, raw in entries:
                locations.setdefault(hash_value, {})[cache_type] = offset
                offset += len(raw)
        with open(os.path.join(path, "thumbcache_idx.db"), "wb") as out:
            out.write(index_file(version, locations))


if __name__ == "__main__":
    main(sys.argv[1])
