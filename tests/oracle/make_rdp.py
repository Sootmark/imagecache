"""Write the synthetic RDP bitmap caches in tests/fixtures/synthetic/rdp/.

No openly licensed .bmc file written by a Remote Desktop client exists,
so these are built from the format: tiles in fixed slots of a 20-byte
header and room for 64x64 pixels, stored bottom row first, uncompressed
or compressed with interleaved RLE ([MS-RDPBCGR] 2.2.9.1.1.3.1.2.4). The
compressed tiles are hand-made order streams that use every order code
and run length form. Python standard library only; deterministic.

    python3 -I tests/oracle/make_rdp.py tests/fixtures/synthetic/rdp

- bcache2.bmc, bcache22.bmc, bcache24.bmc (1, 2 and 4 bytes per pixel):
  per file, an uncompressed 64x64 tile, an uncompressed 64x20 tile (the
  slot's rest keeps older pixels), two compressed 64x64
  tiles and a compressed 64x64 tile repeating the first's key and content.
- bcache-depth3.bmc: 3 bytes per pixel, uncompressed only, a name that
  doesn't give the depth.
- Cache0002.bin: a .bin with tiles of 64x64, 64x16, 16x64 and 5x3, then
  zeros.
"""

import os
import struct
import sys


class Random:
    """A small deterministic generator (an LCG), so output never changes."""

    def __init__(self, seed):
        self.state = seed

    def byte(self):
        self.state = (self.state * 1103515245 + 12345) & 0x7FFFFFFF
        return (self.state >> 16) & 0xFF

    def bytes(self, n):
        return bytes(self.byte() for _ in range(n))


class Stream:
    """An RLE order stream for a 64-pixel-wide tile, counting pixels."""

    def __init__(self, depth, random):
        self.depth = depth
        self.random = random
        self.out = b""
        self.pixels = 0
        self.last_was_background = False

    def pixel(self):
        return self.random.bytes(self.depth)

    def order(self, header, extra, count, background=False):
        # Keep to what bmc-tools and the specification agree on: no
        # foreground/background image across the end of the first row, no
        # background run straight after one that ended exactly there.
        if not background and self.pixels < 64 < self.pixels + count:
            assert header & 0xF0 not in (0x40, 0x50, 0xD0) and header not in (0xF2, 0xF7, 0xF9, 0xFA)
        if background and self.last_was_background:
            assert self.pixels != 64
        self.out += bytes([header]) + extra
        self.pixels += count
        self.last_was_background = background

    def regular(self, code, count, payload=b"", background=False):
        assert 1 <= count <= 255 + 32
        if count < 32:
            self.order((code << 5) | count, payload, count, background)
        else:
            self.order(code << 5, bytes([count - 32]) + payload, count, background)

    def lite(self, code, count, payload, pixels=None):
        assert 1 <= count <= 255 + 16
        drawn = count if pixels is None else pixels
        if count < 16:
            self.order((code << 4) | count, payload, drawn)
        else:
            self.order(code << 4, bytes([count - 16]) + payload, drawn)

    def mega(self, header, count, payload=b"", pixels=None, background=False):
        self.order(header, struct.pack("<H", count) + payload, count if pixels is None else pixels, background)

    def masks(self, count):
        return self.random.bytes((count + 7) // 8)

    def regular_fgbg(self, count):
        if count % 8 == 0 and count // 8 < 32:
            self.order(0x40 | (count // 8), self.masks(count), count)
        else:
            self.order(0x40, bytes([count - 1]) + self.masks(count), count)

    def lite_set_fgbg(self, count):
        colour = self.pixel()
        if count % 8 == 0 and count // 8 < 16:
            self.order(0xD0 | (count // 8), colour + self.masks(count), count)
        else:
            self.order(0xD0, bytes([count - 1]) + colour + self.masks(count), count)

    def finish(self):
        """Fill the rest of the 64x64 tile with a colour image."""
        left = 64 * 64 - self.pixels
        assert left >= 0
        while left > 0:
            count = min(left, 0xFFFF)
            image = b"".join(self.pixel() for _ in range(count))
            if count < 32 + 256 and count >= 1:
                self.regular(4, count, image)
            else:
                self.mega(0xF4, count, image)
            left -= count
        return self.out


def first_stream(depth, random):
    s = Stream(depth, random)
    # First row (64 pixels): background runs (an implied foreground pixel
    # between the two), foreground run, white, black, colour run.
    s.regular(0, 10, background=True)
    s.regular(0, 6, background=True)
    s.regular(1, 8)
    s.order(0xFD, b"", 1)
    s.order(0xFE, b"", 1)
    s.regular(3, 38, s.pixel())
    # Row 2 onwards, drawn against the row above.
    s.regular(0, 40, background=True)       # long form: next byte + 32
    s.regular(0, 3, background=True)        # implied foreground, XOR above
    s.regular(1, 50)                        # foreground run, long form
    s.lite(0xC, 5, s.pixel())               # set foreground, foreground run
    s.lite(0xC, 20, s.pixel())              # long form: next byte + 16
    s.regular_fgbg(16)                      # mask, 16 pixels
    s.regular_fgbg(13)                      # mask, long form
    s.lite_set_fgbg(24)                     # set foreground, mask
    s.lite_set_fgbg(11)                     # long form
    s.order(0xF9, b"", 8)                   # fixed mask 0x03
    s.order(0xFA, b"", 8)                   # fixed mask 0x05
    s.lite(0xE, 7, s.pixel() + s.pixel(), pixels=14)   # dithered
    s.lite(0xE, 17, s.pixel() + s.pixel(), pixels=34)  # long form
    s.regular(4, 12, b"".join(s.pixel() for _ in range(12)))
    return s.finish()


def second_stream(depth, random):
    s = Stream(depth, random)
    s.mega(0xF3, 64, s.pixel())                     # mega colour run, the first row
    s.mega(0xF0, 300, background=True)              # mega background run
    s.mega(0xF0, 70, background=True)               # implied foreground
    s.mega(0xF1, 90)                                # mega foreground run
    s.mega(0xF6, 33, s.pixel())                     # mega set foreground
    count = 77
    s.mega(0xF2, count, s.masks(count))             # mega mask
    count = 45
    s.mega(0xF7, count, s.pixel() + s.masks(count))  # mega set foreground, mask
    s.mega(0xF8, 40, s.pixel() + s.pixel(), pixels=80)  # mega dithered
    s.regular(0, 1, background=True)
    s.order(0xFD, b"", 1)                           # breaks the background pair
    s.regular(0, 2, background=True)
    s.mega(0xF4, 100, b"".join(s.pixel() for _ in range(100)))
    return s.finish()


def bmc_slot(depth, key, width, height, data, compressed):
    header = struct.pack("<IIHHII", key & 0xFFFFFFFF, key >> 32, width, height, len(data),
                         0x08 if compressed else 0)
    room = 64 * 64 * depth
    assert len(data) <= room
    stale = bytes((i * 13 + 7) % 256 for i in range(room - len(data)))
    return header + data + stale


def bmc_file(depth, compressed, random):
    slots = [
        bmc_slot(depth, 0x1000_0001_0000_0001 + depth, 64, 64, random.bytes(64 * 64 * depth), False),
        bmc_slot(depth, 0x1000_0002_0000_0002 + depth, 64, 20, random.bytes(64 * 20 * depth), False),
    ]
    if compressed:
        first = first_stream(depth, random)
        slots += [
            bmc_slot(depth, 0x2000_0003_0000_0003 + depth, 64, 64, first, True),
            bmc_slot(depth, 0x2000_0004_0000_0004 + depth, 64, 64, second_stream(depth, random), True),
            bmc_slot(depth, 0x2000_0003_0000_0003 + depth, 64, 64, first, True),
        ]
    return b"".join(slots)


def bin_file(random):
    out = b"RDP8bmp\0" + struct.pack("<I", 6)
    for key, width, height in [(0xA1, 64, 64), (0xA2, 64, 16), (0xA3, 16, 64), (0xA4, 5, 3)]:
        out += struct.pack("<IIHH", key, 0xB0, width, height) + random.bytes(width * height * 4)
    return out + b"\0" * 48  # zeros, a whole number of tile headers for bmc-tools


def main(out_dir):
    os.makedirs(out_dir, exist_ok=True)
    random = Random(2026)
    files = {
        "bcache2.bmc": bmc_file(1, True, random),
        "bcache22.bmc": bmc_file(2, True, random),
        "bcache24.bmc": bmc_file(4, True, random),
        "bcache-depth3.bmc": bmc_file(3, False, random),
        "Cache0002.bin": bin_file(random),
    }
    for name, data in files.items():
        with open(os.path.join(out_dir, name), "wb") as out:
            out.write(data)


if __name__ == "__main__":
    main(sys.argv[1])
