"""RDP bitmap cache oracle: tiles as ANSSI's bmc-tools decodes them.

bmc-tools (CeCILL 2.1) is run, never vendored: this script imports its
BMCContainer and uses its interleaved RLE decoder (b_uncompress), its
pixel converters (b_parse_rgb565, b_parse_rgb24b, b_parse_rgb32b) and its
8-bit palette for every tile. Only walking the container (headers, slot
stride) and putting rows top-down are done here. Each tile's pixels are
then hashed as top-down BGRA.

It also runs bmc-tools' own whole-file pass (b_process) and checks that
it finds the same tiles with the same pixels, where bmc-tools' pass is
known to be right: it converts every compressed tile as 16-bit, flips
.bin rows as if every tile were 64 pixels wide and drops .bin tiles of
fewer than 64 pixels.

    python3 -I rdp_oracle.py BMC_TOOLS_PY DEPTH:FILE... > tiles.tsv

DEPTH is a .bmc file's bytes per pixel (or "bin"). The summary goes to
stderr.
"""

import contextlib
import hashlib
import importlib.util
import struct
import sys


def load_bmc_tools(path):
    spec = importlib.util.spec_from_file_location("bmc_tools", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module.BMCContainer


def to_bgra(container, raw, depth):
    container.btype = container.BMC_CONTAINER  # convert only, never flip
    if depth == 1:
        palette = container.PALETTE
        return b"".join(palette[4 * i:4 * i + 3] + b"\xff" for i in raw)
    if depth == 2:
        return container.b_parse_rgb565(raw)
    if depth == 3:
        return container.b_parse_rgb24b(raw)
    return container.b_parse_rgb32b(raw)


def rows(data, width, flip):
    lines = [data[i:i + width * 4] for i in range(0, len(data), width * 4)]
    return b"".join(reversed(lines) if flip else lines)


def tiles(container, data, depth):
    """(offset, key, width, height, compressed, raw, bytes per pixel) per tile."""
    if depth == "bin":
        at = 12
        while at + 12 <= len(data):
            key1, key2, width, height = struct.unpack_from("<IIHH", data, at)
            if not (1 <= width <= 64 and 1 <= height <= 64):
                break
            yield at, key2 << 32 | key1, width, height, False, data[at + 12:at + 12 + width * height * 4], 4
            at += 12 + width * height * 4
        return
    stride = 20 + 64 * 64 * depth
    for at in range(0, len(data), stride):
        key1, key2, width, height, length, flags = struct.unpack_from("<IIHHII", data, at)
        body = data[at + 20:at + 20 + length]
        compressed = bool(flags & 0x08)
        raw = container.b_uncompress(body, depth) if compressed else body
        yield at, key2 << 32 | key1, width, height, compressed, raw, depth


def main(bmc_tools, specs):
    Container = load_bmc_tools(bmc_tools)
    print("file\toffset\tkey\twidth\theight\tcompressed\tsha256")
    for spec in specs:
        depth, path = spec.split(":", 1)
        depth = depth if depth == "bin" else int(depth)
        data = open(path, "rb").read()
        container = Container()
        mine = []
        for offset, key, width, height, compressed, raw, bpp in tiles(container, data, depth):
            assert len(raw) == width * height * bpp, (path, offset)
            pixels = rows(to_bgra(container, raw, bpp), width, depth != "bin")
            digest = hashlib.sha256(pixels).hexdigest()
            mine.append((width, height, compressed, pixels))
            print(f"{path}\t{offset}\t{key:016x}\t{width}\t{height}\t{str(compressed).lower()}\t{digest}")
        whole_file(Container, path, depth, mine)


def whole_file(Container, path, depth, mine):
    """bmc-tools' own pass over the file agrees, where it can."""
    container = Container()
    with contextlib.redirect_stdout(sys.stderr):  # bmc-tools logs to stdout
        container.b_import(path)
        container.b_process()
    if depth == "bin":
        # bmc-tools gathers .bin rows 64 pixels at a time and drops a tile
        # with fewer than 64 pixels in all.
        mine = [tile for tile in mine if tile[0] * tile[1] % 64 == 0]
    assert len(container.bmps) == len(mine), (path, len(container.bmps), len(mine))
    compared = 0
    for theirs, (width, height, compressed, pixels) in zip(container.bmps, mine):
        if compressed and depth != 2 or depth == "bin" and width != 64:
            continue
        if depth == 1:
            theirs = to_bgra(container, theirs[len(container.PALETTE):], 1)
        # bmc-tools' tiles are bottom-up, as a .bmp stores them.
        assert rows(theirs, width, True) == pixels, (path, compressed, width, height)
        compared += 1
    print(f"# {path}: bmc-tools finds {len(container.bmps)} tiles; "
          f"{compared} compared pixel for pixel, all equal", file=sys.stderr)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2:])
