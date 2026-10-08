//! Windows image caches, for forensics: pictures a system kept of what a
//! user looked at.
//!
//! - [`thumbcache`]: Explorer's thumbnail and icon caches
//!   (`thumbcache_*.db`, `iconcache_*.db`) and their index
//!   (`thumbcache_idx.db`), Windows Vista to 11. Each thumbnail with its
//!   cache entry hash (the `ThumbnailCacheId` Windows Search records for
//!   the file), its checksums checked, and its image bytes as stored (BMP,
//!   JPEG or PNG), borrowed from the input.
//! - [`rdp`]: the Remote Desktop client's bitmap cache (`Cache????.bin`,
//!   `bcache*.bmc`). Each tile of the remote screen with its key, size and
//!   pixels as 32-bit BGRA, compressed tiles decompressed.
//!
//! Nothing is decoded or encoded beyond that, except a plain `.bmp` of a
//! tile on request ([`rdp::Tile::to_bmp`]).
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = std::fs::read("thumbcache_256.db")?;
//! let cache = imagecache::thumbcache::read(&data);
//! for entry in &cache.entries {
//!     if let Some(extension) = entry.format.extension() {
//!         std::fs::write(format!("{}.{extension}", entry.cache_id()), entry.data)?;
//!     }
//! }
//!
//! let data = std::fs::read("Cache0000.bin")?;
//! let tiles = imagecache::rdp::read(&data, "Cache0000.bin");
//! println!("{} tiles, {} problems", tiles.tiles.len(), tiles.problems.len());
//! # Ok(())
//! # }
//! ```
//!
//! Damage goes to `problems`, never a panic.

mod le;
pub mod rdp;
pub mod thumbcache;

/// This crate's version, for records of what parsed them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
