"""The tiles a tesserae-generated .bin must hold, from its ground truth.

tesserae (MIT) writes a .bin from a known image and records where each
tile came from (*.truth.json). This script cuts each tile out of the
source PNG (decoded here with the standard library) and prints its
pixels' SHA-256 as top-down BGRA, alpha 255, and the key tesserae gives
it: the first 8 bytes of the SHA-256 of the tile's RGB pixels, as a
little-endian 64-bit number.

    python3 -I rdp_truth.py SOURCE.png TRUTH.json > truth.tsv
"""

import hashlib
import json
import struct
import sys
import zlib


def read_png(path):
    data = open(path, "rb").read()
    assert data.startswith(b"\x89PNG\r\n\x1a\n")
    at, idat = 8, b""
    while at < len(data):
        length, kind = struct.unpack_from(">I4s", data, at)
        body = data[at + 8:at + 8 + length]
        if kind == b"IHDR":
            width, height, depth, colour, _, _, interlace = struct.unpack(">IIBBBBB", body)
            assert (depth, colour, interlace) == (8, 2, 0), "8-bit RGB, not interlaced"
        elif kind == b"IDAT":
            idat += body
        at += 12 + length
    raw = zlib.decompress(idat)
    stride = width * 3
    rows, previous = [], bytearray(stride)
    for y in range(height):
        kind = raw[y * (stride + 1)]
        line = bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for x in range(stride):
            left = line[x - 3] if x >= 3 else 0
            up = previous[x]
            corner = previous[x - 3] if x >= 3 else 0
            if kind == 1:
                line[x] = (line[x] + left) & 0xFF
            elif kind == 2:
                line[x] = (line[x] + up) & 0xFF
            elif kind == 3:
                line[x] = (line[x] + (left + up) // 2) & 0xFF
            elif kind == 4:
                guess = left + up - corner
                pa, pb, pc = abs(guess - left), abs(guess - up), abs(guess - corner)
                pick = left if pa <= pb and pa <= pc else up if pb <= pc else corner
                line[x] = (line[x] + pick) & 0xFF
        rows.append(bytes(line))
        previous = line
    return rows


def main(png_path, truth_path):
    rows = read_png(png_path)
    truth = json.load(open(truth_path))
    print("index\tkey\tsha256")
    for place in truth["placements"]:
        x, y, w, h = place["x"], place["y"], place["width"], place["height"]
        rgb = b"".join(rows[y + j][3 * x:3 * (x + w)] for j in range(h))
        bgra = b"".join(bytes([rgb[i + 2], rgb[i + 1], rgb[i], 0xFF]) for i in range(0, len(rgb), 3))
        key = struct.unpack("<Q", hashlib.sha256(rgb).digest()[:8])[0]
        print(f"{place['index']}\t{key:016x}\t{hashlib.sha256(bgra).hexdigest()}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
