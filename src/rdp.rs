//! The Remote Desktop client's bitmap cache: pieces of the remote screen
//! the client kept to avoid fetching them again, in
//! `%LOCALAPPDATA%\Microsoft\Terminal Server Client\Cache\`. What a user
//! saw in a remote session, one tile of up to 64 by 64 pixels at a time.
//!
//! Two containers, neither with an index: tiles back to back.
//!
//! - `Cache????.bin` (Windows 7 and later): `RDP8bmp\0` and a 32-bit
//!   version, then tiles, each a 12-byte header (the 64-bit key as two
//!   halves, width, height) and its pixels, 32-bit BGRX, top row first.
//! - `bcache*.bmc` (earlier clients; `bcache2` 8-bit, `bcache22` 16-bit,
//!   `bcache24` 32-bit): tiles in fixed slots of a 20-byte header (key,
//!   width, height, data length, flags) and room for 64 by 64 pixels.
//!   Pixels are stored bottom row first, either as they are (8-bit
//!   palette index, RGB565, BGR or BGRX) or, flag `0x08`, compressed with
//!   RDP's interleaved RLE. The rest of a slot can hold an older tile's
//!   pixels.
//!
//! Every tile comes back as 32-bit BGRA, top row first, alpha 255. 8-bit
//! tiles are drawn with Windows' default 256-colour palette, since the
//! cache doesn't keep the session's.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = std::fs::read("Cache0000.bin")?;
//! let cache = imagecache::rdp::read(&data, "Cache0000.bin");
//! for (number, tile) in imagecache::rdp::distinct(&cache.tiles).into_iter().enumerate() {
//!     std::fs::write(format!("tile_{number:04}.bmp"), tile.to_bmp())?;
//! }
//! # Ok(())
//! # }
//! ```
//!
//! The crate lists tiles; putting them back together into screens is left
//! to the caller.

mod bmp;
mod color;
mod rle;

use std::cmp::Reverse;
use std::collections::HashSet;

use crate::le::{u16_at, u32_at};

/// A `.bin` container's signature.
pub const BIN_SIGNATURE: &[u8; 8] = b"RDP8bmp\0";

/// A `.bin` file's header: the signature and a version.
const BIN_HEADER: usize = 12;
/// A `.bin` tile's header.
const BIN_TILE_HEADER: usize = 12;
/// A `.bmc` tile's header.
const BMC_TILE_HEADER: usize = 20;
/// A tile's largest width and height.
const TILE_SIDE: u16 = 64;
/// The pixels a `.bmc` slot has room for.
const SLOT_PIXELS: usize = 64 * 64;
/// The `.bmc` flag for a compressed tile.
const COMPRESSED: u32 = 0x08;
/// The `.bmc` depths to try when the file name doesn't say, most common
/// first.
const DEPTHS: [u8; 4] = [4, 2, 1, 3];

/// Which container a cache is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Container {
    /// `Cache????.bin`, Windows 7 and later.
    Bin,
    /// `bcache*.bmc`, earlier clients.
    Bmc,
}

/// A tile: a piece of the remote screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tile {
    /// Its offset in the file.
    pub offset: usize,
    /// Its key, the client's 64-bit name for its content (the header's
    /// first half in the low 32 bits).
    pub key: u64,
    /// Its width in pixels (1 to 64).
    pub width: u16,
    /// Its height in pixels (1 to 64).
    pub height: u16,
    /// The bytes per pixel it was stored with (1 to 4).
    pub bytes_per_pixel: u8,
    /// Whether it was stored compressed.
    pub compressed: bool,
    /// Its pixels, 32-bit BGRA, top row first, alpha 255.
    pub pixels: Vec<u8>,
}

impl Tile {
    /// The tile as a `.bmp` file (32-bit, uncompressed).
    #[must_use]
    pub fn to_bmp(&self) -> Vec<u8> {
        bmp::encode(self.width, self.height, &self.pixels)
    }
}

/// A cache's tiles and what couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BitmapCache {
    /// The container, if recognised.
    pub container: Option<Container>,
    /// The `.bin` header's version.
    pub version: Option<u32>,
    /// The bytes per pixel of a `.bmc` file's slots (1, 2, 3 or 4).
    pub bytes_per_pixel: Option<u8>,
    /// The tiles, in file order.
    pub tiles: Vec<Tile>,
    /// What couldn't be read.
    pub problems: Vec<String>,
}

/// Whether `head` starts like a `.bin` container.
#[must_use]
pub fn is_bin(head: &[u8]) -> bool {
    head.starts_with(BIN_SIGNATURE)
}

/// Read a bitmap cache file, `.bin` or `.bmc` by its signature, a `.bmc`
/// file's depth from its name (see [`depth_from_file_name`]) or, failing
/// that, from its content.
#[must_use]
pub fn read(data: &[u8], file_name: &str) -> BitmapCache {
    if is_bin(data) {
        read_bin(data)
    } else {
        read_bmc(data, depth_from_file_name(file_name))
    }
}

/// The bytes per pixel a `.bmc` file's name implies: `bcache2.bmc` 1,
/// `bcache22.bmc` 2, `bcache24.bmc` 4. Folders before the name are
/// ignored, case too.
#[must_use]
pub fn depth_from_file_name(path: &str) -> Option<u8> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match name.to_ascii_lowercase().as_str() {
        "bcache2.bmc" => Some(1),
        "bcache22.bmc" => Some(2),
        "bcache24.bmc" => Some(4),
        _ => None,
    }
}

/// Read a `Cache????.bin` file: every tile, its pixels as BGRA.
#[must_use]
pub fn read_bin(data: &[u8]) -> BitmapCache {
    let mut cache = BitmapCache::default();
    if !is_bin(data) {
        cache
            .problems
            .push("not a .bin cache (no RDP8bmp)".to_owned());
        return cache;
    }
    cache.container = Some(Container::Bin);
    cache.version = u32_at(data, BIN_SIGNATURE.len());
    let mut offset = BIN_HEADER;
    while offset < data.len() {
        let rest = &data[offset..];
        match bin_tile(rest, offset) {
            Ok(tile) => {
                offset += BIN_TILE_HEADER + usize::from(tile.width) * usize::from(tile.height) * 4;
                cache.tiles.push(tile);
            }
            // Zeros past the last tile are only unused space.
            Err(_) if rest.iter().all(|&byte| byte == 0) => break,
            Err(why) => {
                cache.problems.push(format!("tile at {offset:#x}: {why}"));
                break;
            }
        }
    }
    cache
}

/// The `.bin` tile at the start of `rest`.
fn bin_tile(rest: &[u8], offset: usize) -> Result<Tile, String> {
    let (key, width, height) = tile_header(rest)?;
    let size = usize::from(width) * usize::from(height) * 4;
    let stored = rest
        .get(BIN_TILE_HEADER..BIN_TILE_HEADER + size)
        .ok_or("pixels past the end")?;
    Ok(Tile {
        offset,
        key,
        width,
        height,
        bytes_per_pixel: 4,
        compressed: false,
        pixels: to_bgra(stored, usize::from(width), 4, false),
    })
}

/// Read a `bcache*.bmc` file: every tile, its pixels as BGRA.
/// `bytes_per_pixel` is the slots' depth (from the file name, see
/// [`depth_from_file_name`]); `None` tries each and keeps the one the
/// file reads best with.
#[must_use]
pub fn read_bmc(data: &[u8], bytes_per_pixel: Option<u8>) -> BitmapCache {
    let read = match bytes_per_pixel {
        Some(depth @ 1..=4) => read_slots(data, depth),
        Some(depth) => BitmapCache {
            problems: vec![format!("{depth} bytes per pixel, not 1 to 4")],
            ..BitmapCache::default()
        },
        None => DEPTHS
            .into_iter()
            .map(|depth| read_slots(data, depth))
            .min_by_key(|read| (read.problems.len(), Reverse(read.tiles.len())))
            .unwrap_or_default(),
    };
    BitmapCache {
        container: Some(Container::Bmc),
        bytes_per_pixel: read.bytes_per_pixel.filter(|_| !read.tiles.is_empty()),
        ..read
    }
}

/// A `.bmc` file read with slots for `depth` bytes per pixel.
fn read_slots(data: &[u8], depth: u8) -> BitmapCache {
    let mut cache = BitmapCache {
        bytes_per_pixel: Some(depth),
        ..BitmapCache::default()
    };
    let stride = BMC_TILE_HEADER + SLOT_PIXELS * usize::from(depth);
    for (number, slot) in data.chunks(stride).enumerate() {
        let offset = number * stride;
        // An empty slot, or zeros past the last.
        if slot.iter().take(BMC_TILE_HEADER).all(|&byte| byte == 0) {
            continue;
        }
        match bmc_tile(slot, offset, depth) {
            Ok(tile) => cache.tiles.push(tile),
            Err(why) => cache.problems.push(format!("tile at {offset:#x}: {why}")),
        }
    }
    cache
}

/// The `.bmc` tile in `slot`.
fn bmc_tile(slot: &[u8], offset: usize, depth: u8) -> Result<Tile, String> {
    let (key, width, height) = tile_header(slot)?;
    let (Some(length), Some(flags)) = (u32_at(slot, 12), u32_at(slot, 16)) else {
        return Err("a header cut short".to_owned());
    };
    let length = length as usize;
    let stored = slot
        .get(BMC_TILE_HEADER..BMC_TILE_HEADER + length)
        .filter(|_| length <= SLOT_PIXELS * usize::from(depth))
        .ok_or_else(|| format!("{length} bytes of pixels, more than the slot holds"))?;
    let (columns, rows) = (usize::from(width), usize::from(height));
    let compressed = flags & COMPRESSED != 0;
    let raw = if compressed {
        rle::decompress(stored, columns, rows, usize::from(depth))?
    } else if length == columns * rows * usize::from(depth) {
        stored.to_vec()
    } else {
        return Err(format!(
            "{length} bytes for {width} by {height} pixels of {depth} bytes"
        ));
    };
    Ok(Tile {
        offset,
        key,
        width,
        height,
        bytes_per_pixel: depth,
        compressed,
        pixels: to_bgra(&raw, columns, usize::from(depth), true),
    })
}

/// A tile header's key, width and height (both 1 to 64).
fn tile_header(header: &[u8]) -> Result<(u64, u16, u16), String> {
    let (Some(low), Some(high), Some(width), Some(height)) = (
        u32_at(header, 0),
        u32_at(header, 4),
        u16_at(header, 8),
        u16_at(header, 10),
    ) else {
        return Err("a header cut short".to_owned());
    };
    let sides = 1..=TILE_SIDE;
    if sides.contains(&width) && sides.contains(&height) {
        Ok((u64::from(high) << 32 | u64::from(low), width, height))
    } else {
        Err(format!(
            "a {width} by {height} tile, not 1 to 64 pixels a side"
        ))
    }
}

/// Stored pixels, rows of `width`, as top-down BGRA.
fn to_bgra(stored: &[u8], width: usize, bytes_per_pixel: usize, bottom_up: bool) -> Vec<u8> {
    let row = width * bytes_per_pixel;
    if row == 0 {
        return Vec::new();
    }
    let mut lines: Vec<&[u8]> = stored.chunks_exact(row).collect();
    if bottom_up {
        lines.reverse();
    }
    lines
        .into_iter()
        .flat_map(|line| line.chunks_exact(bytes_per_pixel))
        .flat_map(color::to_bgra)
        .collect()
}

/// The tiles without repeats: the first of each key and content, in
/// order. Pass several caches' tiles to list what any of them saw once.
pub fn distinct<'a>(tiles: impl IntoIterator<Item = &'a Tile>) -> Vec<&'a Tile> {
    let mut seen = HashSet::new();
    tiles
        .into_iter()
        .filter(|tile| seen.insert((tile.key, tile.width, tile.height, tile.pixels.as_slice())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depths_from_names() {
        assert_eq!(depth_from_file_name(r"C:\Users\x\bcache22.bmc"), Some(2));
        assert_eq!(depth_from_file_name("BCACHE24.BMC"), Some(4));
        assert_eq!(depth_from_file_name("cache/bcache2.bmc"), Some(1));
        assert_eq!(depth_from_file_name("Cache0000.bin"), None);
    }

    #[test]
    fn keys_join_their_halves() {
        let header = [1, 0, 0, 0, 2, 0, 0, 0, 64, 0, 32, 0];
        assert_eq!(tile_header(&header), Ok((0x2_0000_0001, 64, 32)));
        let too_wide = [0, 0, 0, 0, 0, 0, 0, 0, 65, 0, 1, 0];
        assert!(tile_header(&too_wide).is_err());
        assert!(tile_header(&header[..10]).is_err());
    }

    #[test]
    fn repeats_are_dropped() {
        let tile = Tile {
            offset: 0,
            key: 7,
            width: 1,
            height: 1,
            bytes_per_pixel: 4,
            compressed: false,
            pixels: vec![1, 2, 3, 255],
        };
        let again = Tile {
            offset: 16,
            ..tile.clone()
        };
        let other = Tile {
            offset: 32,
            key: 8,
            ..tile.clone()
        };
        let tiles = [tile, again, other];
        let kept: Vec<usize> = distinct(&tiles).iter().map(|t| t.offset).collect();
        assert_eq!(kept, [0, 32]);
    }
}
