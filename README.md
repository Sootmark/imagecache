# imagecache

Windows image caches, for forensics: pictures a system kept of what a user looked at. Explorer's thumbnail and icon caches (`thumbcache_*.db`, `iconcache_*.db`) and their index (`thumbcache_idx.db`), Windows Vista to 11: each thumbnail with its cache entry hash, the `ThumbnailCacheId` Windows Search records for the file, so a picture can be tied to a file even after the file is gone. And the Remote Desktop client's bitmap cache (`Cache????.bin`, `bcache*.bmc`): each tile of what the remote screen showed, as BGRA pixels. One dependency, its sibling `sootmark-common` (times, UTF-16).

```toml
[dependencies]
sootmark-imagecache = "0.1"
```

```rust
let data = std::fs::read("thumbcache_256.db")?;
for entry in &imagecache::thumbcache::read(&data).entries {
    println!("{} {:?} {} bytes", entry.cache_id(), entry.format, entry.data.len());
}

let data = std::fs::read("Cache0000.bin")?;
let cache = imagecache::rdp::read(&data, "Cache0000.bin");
for (n, tile) in imagecache::rdp::distinct(&cache.tiles).into_iter().enumerate() {
    std::fs::write(format!("tile_{n:04}.bmp"), tile.to_bmp())?;
}
```

## What you get

- `thumbcache::read(data)`: a cache file's format version (Vista, 7, 8, 8.1, 10/11), which size it is (`size_name()`: `"256"`, `"sr"`, …), where its free space starts, and each entry: offset, size, cache entry hash (`cache_id()` as Windows Search shows it), extension (Vista), identifier, width and height (Windows 8 and later), the stored data and header CRC-64s and whether each matches, the image format (BMP, JPEG, PNG, other, none) and the image bytes, borrowed from the input, with their offset. Nothing is decoded.
- `thumbcache::read_index(data)`: the index's version, entries used and allocated, and each entry in use: hash, flags, last modified (Vista) and, per cache file holding a thumbnail of it, the cache type, size name and offset.
- `rdp::read(data, file_name)`: a `.bin` or `.bmc` file, by signature; a `.bmc` file's depth from its name (`bcache2` 8-bit, `bcache22` 16-bit, `bcache24` 32-bit) or, failing that, from which slot size the file reads cleanly with (`read_bmc(data, None)`). Each tile: offset, key (64-bit), width, height, bytes per pixel stored, whether compressed, and its pixels as 32-bit BGRA, top row first, alpha 255: interleaved RLE decompressed, RGB565 widened, 8-bit tiles drawn with Windows' default 256-colour palette (the cache doesn't keep the session's).
- `rdp::distinct(tiles)`: tiles without repeats (same key and pixels), across as many caches as you pass. A list, never a collage.
- `Tile::to_bmp()`: a tile as a 32-bit `.bmp`. The only encoding the crate does.
- Damage goes to `problems`, never a panic: a cache entry or `.bin` tile that doesn't fit ends the reading, a bad `.bmc` tile is skipped (slots are fixed, so the rest stays in step).

## How it's checked

- Real caches from Windows Vista, 7, 8.1, 10 and 11 (dissect.thumbcache's test data, AGPL-3.0: fetched by `tests/oracle/fetch-dissect-samples.sh`, never committed; `cargo test -- --ignored`): all 323 entries of 14 cache files read as dissect.thumbcache reads them (offset, hash, extension, identifier, data size and SHA-256), every header and data checksum valid by an independent Python CRC-64 (`tests/oracle/thumbcache_oracle.py`), and all 292 places their 5 indexes name hold an entry with the index's hash; the Vista, 7, 10 and 11 indexes as dissect.thumbcache reads them.
- Synthetic caches for formats 20, 21, 30, 31 and 32 (`tests/fixtures/synthetic/thumbcache/`, from `tests/oracle/make_thumbcache.py`): BMP, PNG, JPEG, no-data, padded, over-1024-byte, data- and header-damaged entries, an iconcache, and indexes; read as dissect.thumbcache reads them (72 entries) and every index place followed to its entry.
- tesserae's sample `Cache0001.bin` (MIT, generated from a known image, `tests/fixtures/tesserae/`): its 51 tiles are, pixel for pixel, the pieces of the source image its truth file says they are, with the keys it derived (`tests/oracle/rdp_truth.py`), and what bmc-tools reads.
- Synthetic `.bmc` files at 1, 2, 3 and 4 bytes per pixel and a `.bin` with odd-sized tiles (`tests/oracle/make_rdp.py`; compressed tiles hand-made to use every RLE order code and run length form): 21 tiles, offsets, keys, sizes and pixels as ANSSI's bmc-tools decodes them (its RLE decoder, converters and palette, `tests/oracle/rdp_oracle.py`); its own whole-file pass agrees where it can.
- Property tests: damaged fixtures, real headers with random tails and arbitrary bytes give entries, tiles, problems or nothing, never a panic.

## Unverified

- Windows 8.0 thumbnail caches (format 30): no openly licensed sample, and dissect.thumbcache doesn't read them. The layout is libwtcdb's documentation and the pattern of 8.1 and 10 (index header 104 bytes, 56-byte index entries); only the synthetic files test it.
- The `.bmc` container as a real client writes it: no openly licensed `.bmc` file exists. Everything about it (slot size, bottom-up rows, RGB565, the palette, RLE as `.bmc` tiles use it) agrees with bmc-tools on synthetic files, not with a real Vista-era client. Where bmc-tools and the RLE specification differ (a foreground/background mask crossing the end of the first row; a background run right after one that ended exactly there), the crate follows the specification, and the fixtures avoid both.
- A real `Cache????.bin` written by Windows: the only openly licensed one is synthetic (tesserae). Rows top-down agrees with bmc-tools and tesserae, not with a Windows-written file.

## Not yet

- Carving: deleted thumbnails left in a cache's free space, older pixels left in `.bmc` slots past a smaller tile.
- `Thumbs.db` (Windows XP's per-folder cache, an OLE compound file).
- Telling `thumbcache_*.db` from `iconcache_*.db` by content: the header doesn't say; the file name does.

## Licence

MIT or Apache-2.0, at your option.
