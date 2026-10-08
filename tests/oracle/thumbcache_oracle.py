"""Thumbnail cache oracle: what dissect.thumbcache reads, plus checksums
computed here, independently of the crate.

Structure (entries, offsets, hashes, identifiers, data) comes from
dissect.thumbcache (Fox-IT, AGPL-3.0; run, never vendored). The CRC-64
checks are this script's own, from the published algorithm: reflected,
polynomial 0x92C64265D32139A4, header from all ones over the header
before its checksum, data from zero, past 1024 bytes the first 1024 in
full XOR a CRC of the first 4 bytes of every 400 after.

    python3 -I thumbcache_oracle.py caches DIR...  > caches.tsv
    python3 -I thumbcache_oracle.py index FILE...  > index.tsv

Paths are written as given (run from the fixtures folder for short ones).
"""

import hashlib
import struct
import sys

from dissect.thumbcache.index import ThumbnailIndex
from dissect.thumbcache.thumbcache_file import ThumbcacheEntry, ThumbcacheFile

POLY = 0x92C64265D32139A4


def _table():
    table = []
    for byte in range(256):
        crc = byte
        for _ in range(8):
            crc = (crc >> 1) ^ POLY if crc & 1 else crc >> 1
        table.append(crc)
    return table


TABLE = _table()


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


def image_format(data):
    if not data:
        return "empty"
    if data.startswith(b"BM"):
        return "bmp"
    if data.startswith(b"\xff\xd8\xff"):
        return "jpeg"
    if data.startswith(b"\x89PNG\r\n\x1a\n"):
        return "png"
    return "other"


def caches(paths):
    print("file\toffset\thash\textension\tidentifier\tdata_size\tdata_sha256\tformat\theader_ok\tdata_ok")
    for path in paths:
        raw = open(path, "rb").read()
        version = struct.unpack_from("<I", raw, 4)[0]
        header_len = 48 if version == 21 else 56
        with open(path, "rb") as fh:
            cache = ThumbcacheFile(fh)
            fh.seek(24)
            while fh.tell() < len(raw):
                offset = fh.tell()
                try:
                    entry = ThumbcacheEntry(fh, cache.version)
                except EOFError:
                    break
                except Exception as error:  # dissect raises on a bad signature
                    print(f"# {path} at {offset:#x}: {error}", file=sys.stderr)
                    break
                hash_value = int.from_bytes(bytes.fromhex(entry.hash), "little")
                # dissect returns the rest of the entry; Windows pads some
                # entries past their data, so keep the header's data size.
                data = entry.data[:entry._header.DataSize]
                if hash_value == 0 and not entry.identifier:
                    break  # the free space: one empty entry to the end
                header = raw[offset:offset + header_len]
                header_ok = crc64(header[:-8], (1 << 64) - 1) == struct.unpack("<Q", header[-8:])[0]
                data_ok = data_crc(data) == struct.unpack("<Q", header[-16:-8])[0]
                print("\t".join([
                    path, str(offset), f"{hash_value:016x}", entry.extension, entry.identifier,
                    str(len(data)), hashlib.sha256(data).hexdigest(), image_format(data),
                    str(header_ok).lower(), str(data_ok).lower(),
                ]))


def index(paths):
    print("file\thash\tflags\tlocations")
    for path in paths:
        with open(path, "rb") as fh:
            for entry in ThumbnailIndex(fh).entries():
                hash_value = int.from_bytes(entry.identifier, "little")
                locations = ",".join(
                    f"{cache}:{offset}" for cache, offset in enumerate(entry.cache_offsets)
                    if offset not in (0, 0xFFFFFFFF)
                )
                print(f"{path}\t{hash_value:016x}\t{entry.flags}\t{locations}")


if __name__ == "__main__":
    {"caches": caches, "index": index}[sys.argv[1]](sys.argv[2:])
