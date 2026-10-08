//! Explorer's thumbnail and icon caches: `thumbcache_*.db` and
//! `iconcache_*.db` (`CMMM`), and their index `thumbcache_idx.db`
//! (`IMMM`), from Windows Vista to Windows 11, in
//! `%LOCALAPPDATA%\Microsoft\Windows\Explorer\`.
//!
//! A cache file is a 24-byte header (signature, format version, which
//! size of cache it is, where its entries start and where its free space
//! starts) and then entries back to back, each: `CMMM`, the entry's size,
//! the cache entry hash, the sizes of the identifier, padding and data,
//! (Vista: the file's extension; Windows 8 and later: the thumbnail's
//! width and height), a CRC-64 of the data and a CRC-64 of the header,
//! then the identifier (UTF-16), padding and the image itself (BMP, JPEG
//! or PNG). The free space at the end is one more entry, zeroed, which
//! isn't returned.
//!
//! The cache entry hash is the `ThumbnailCacheId` Windows Search records
//! for a file (`System_ThumbnailCacheId`), so a thumbnail can be tied to
//! the file it shows, even after the file is gone.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let data = std::fs::read("thumbcache_256.db")?;
//! let cache = imagecache::thumbcache::read(&data);
//! for entry in &cache.entries {
//!     println!("{} {:?} {} bytes", entry.cache_id(), entry.format, entry.data.len());
//! }
//! # Ok(())
//! # }
//! ```

mod crc64;
mod index;

pub use index::{is_index, read_index, Index, IndexEntry, Location};

use crate::le::{u32_at, u64_at};

/// A cache file's signature.
pub const CACHE_SIGNATURE: &[u8; 4] = b"CMMM";
/// The index file's signature.
pub const INDEX_SIGNATURE: &[u8; 4] = b"IMMM";

/// A cache file's header.
const CACHE_HEADER: usize = 24;

/// The cache files of each format version, in cache type order: the part
/// of the name after `thumbcache_` or `iconcache_`.
const SIZES_VISTA_7: &[&str] = &["32", "96", "256", "1024", "sr"];
const SIZES_8: &[&str] = &["16", "32", "48", "96", "256", "1024", "sr", "wide", "exif"];
const SIZES_81: &[&str] = &[
    "16",
    "32",
    "48",
    "96",
    "256",
    "1024",
    "1600",
    "sr",
    "wide",
    "exif",
    "wide_alternate",
];
const SIZES_10: &[&str] = &[
    "16",
    "32",
    "48",
    "96",
    "256",
    "768",
    "1280",
    "1920",
    "2560",
    "sr",
    "wide",
    "exif",
    "wide_alternate",
    "custom_stream",
];

/// The format version a cache or index was written in, by Windows release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Version {
    /// Format version 20, Windows Vista.
    Vista,
    /// Format version 21, Windows 7.
    Windows7,
    /// Format version 30, Windows 8.
    Windows8,
    /// Format version 31, Windows 8.1.
    Windows81,
    /// Format version 32, Windows 10 and 11.
    Windows10,
}

impl Version {
    /// The version a format version number names.
    #[must_use]
    pub fn from_number(number: u32) -> Option<Self> {
        match number {
            20 => Some(Self::Vista),
            21 => Some(Self::Windows7),
            30 => Some(Self::Windows8),
            31 => Some(Self::Windows81),
            32 => Some(Self::Windows10),
            _ => None,
        }
    }

    /// Its format version number.
    #[must_use]
    pub fn number(self) -> u32 {
        match self {
            Self::Vista => 20,
            Self::Windows7 => 21,
            Self::Windows8 => 30,
            Self::Windows81 => 31,
            Self::Windows10 => 32,
        }
    }

    /// The cache files it has, in cache type order: the part of each
    /// name after `thumbcache_` or `iconcache_` (`"256"`, `"sr"`).
    #[must_use]
    pub fn sizes(self) -> &'static [&'static str] {
        match self {
            Self::Vista | Self::Windows7 => SIZES_VISTA_7,
            Self::Windows8 => SIZES_8,
            Self::Windows81 => SIZES_81,
            Self::Windows10 => SIZES_10,
        }
    }

    /// The name part of a cache type (`"256"` for `thumbcache_256.db`).
    #[must_use]
    pub fn size_name(self, cache_type: u32) -> Option<&'static str> {
        self.sizes().get(cache_type as usize).copied()
    }

    /// Where an entry's fields are.
    fn entry_layout(self) -> EntryLayout {
        match self {
            Self::Vista => EntryLayout {
                header: 56,
                sizes_at: 24,
                extension: true,
                dimensions: false,
            },
            Self::Windows7 => EntryLayout {
                header: 48,
                sizes_at: 16,
                extension: false,
                dimensions: false,
            },
            Self::Windows8 | Self::Windows81 | Self::Windows10 => EntryLayout {
                header: 56,
                sizes_at: 16,
                extension: false,
                dimensions: true,
            },
        }
    }

    /// Where the cache header keeps the offsets of the first entry and of
    /// the free space.
    fn header_offsets_at(self) -> usize {
        match self {
            Self::Vista | Self::Windows7 => 12,
            Self::Windows8 | Self::Windows81 | Self::Windows10 => 16,
        }
    }
}

/// An entry's fixed part.
struct EntryLayout {
    /// The header's length, its checksum last.
    header: usize,
    /// Where the identifier, padding and data sizes are, in that order.
    sizes_at: usize,
    /// Whether the extension follows the hash (Vista).
    extension: bool,
    /// Whether width and height follow the sizes (Windows 8 and later).
    dimensions: bool,
}

/// What an entry's data is, from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageFormat {
    /// A Windows bitmap (`BM`).
    Bmp,
    /// A JPEG (`FF D8 FF`).
    Jpeg,
    /// A PNG (`89 50 4E 47 0D 0A 1A 0A`).
    Png,
    /// Data of another kind.
    Other,
    /// No data: the entry only records that the file has no thumbnail.
    Empty,
}

impl ImageFormat {
    /// The format of `data`, from its signature.
    #[must_use]
    pub fn sniff(data: &[u8]) -> Self {
        if data.is_empty() {
            Self::Empty
        } else if data.starts_with(b"BM") {
            Self::Bmp
        } else if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Self::Jpeg
        } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
            Self::Png
        } else {
            Self::Other
        }
    }

    /// The usual file extension for it, if it is an image.
    #[must_use]
    pub fn extension(self) -> Option<&'static str> {
        match self {
            Self::Bmp => Some("bmp"),
            Self::Jpeg => Some("jpg"),
            Self::Png => Some("png"),
            Self::Other | Self::Empty => None,
        }
    }
}

/// A cache entry: one thumbnail or icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry<'a> {
    /// Its offset in the file.
    pub offset: usize,
    /// Its size, header included.
    pub size: u32,
    /// The cache entry hash: the file's `ThumbnailCacheId` in Windows
    /// Search.
    pub hash: u64,
    /// The file's extension (Vista only, and not always set).
    pub extension: Option<String>,
    /// The identifier: usually the hash in hex, or a shell item such as
    /// `::{645FF040-5081-101B-9F08-00AA002F954E}`.
    pub identifier: String,
    /// The thumbnail's width in pixels (Windows 8 and later).
    pub width: Option<u32>,
    /// The thumbnail's height in pixels (Windows 8 and later).
    pub height: Option<u32>,
    /// The data's CRC-64, as stored.
    pub data_checksum: u64,
    /// The header's CRC-64, as stored.
    pub header_checksum: u64,
    /// Whether the data matches its stored checksum.
    pub data_checksum_valid: bool,
    /// Whether the header matches its stored checksum.
    pub header_checksum_valid: bool,
    /// What the data is.
    pub format: ImageFormat,
    /// Where the data starts in the file.
    pub data_offset: usize,
    /// The image, as stored (not decoded).
    pub data: &'a [u8],
}

impl Entry<'_> {
    /// The hash as Windows Search shows `System_ThumbnailCacheId`: 16 hex
    /// digits.
    #[must_use]
    pub fn cache_id(&self) -> String {
        format!("{:016x}", self.hash)
    }
}

/// A cache file's entries and what couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Cache<'a> {
    /// The format version, if the header could be read.
    pub version: Option<Version>,
    /// Which size of cache it is (see [`Cache::size_name`]).
    pub cache_type: u32,
    /// Where its free space starts.
    pub free_offset: u32,
    /// The entries, in file order.
    pub entries: Vec<Entry<'a>>,
    /// What couldn't be read.
    pub problems: Vec<String>,
}

impl Cache<'_> {
    /// The part of the file's name after `thumbcache_` or `iconcache_`
    /// (`"256"`, `"sr"`), from its cache type.
    #[must_use]
    pub fn size_name(&self) -> Option<&'static str> {
        self.version?.size_name(self.cache_type)
    }
}

/// Whether `head` starts like a cache file.
#[must_use]
pub fn is_cache(head: &[u8]) -> bool {
    head.starts_with(CACHE_SIGNATURE)
}

/// Read a `thumbcache_*.db` or `iconcache_*.db` file: its entries, each
/// with its hash, identifier, checksums checked and image bytes.
#[must_use]
pub fn read(data: &[u8]) -> Cache<'_> {
    let mut cache = Cache::default();
    if !is_cache(data) {
        cache.problems.push("not a cache file (no CMMM)".to_owned());
        return cache;
    }
    let Some(number) = u32_at(data, 4) else {
        cache.problems.push("a header cut short".to_owned());
        return cache;
    };
    let Some(version) = Version::from_number(number) else {
        cache
            .problems
            .push(format!("format version {number}, not one known"));
        return cache;
    };
    let offsets_at = version.header_offsets_at();
    let (Some(cache_type), Some(first), Some(free)) = (
        u32_at(data, 8),
        u32_at(data, offsets_at),
        u32_at(data, offsets_at + 4),
    ) else {
        cache.problems.push("a header cut short".to_owned());
        return cache;
    };
    cache.version = Some(version);
    cache.cache_type = cache_type;
    cache.free_offset = free;
    if version.size_name(cache_type).is_none() {
        cache
            .problems
            .push(format!("cache type {cache_type}, not one known"));
    }
    let free = free as usize;
    // An empty cache has no entries and gives 0 as its first.
    let mut offset = match first as usize {
        0 => CACHE_HEADER,
        first => first,
    };
    while offset < data.len() && offset != free {
        match entry(data, offset, version) {
            Ok(Some(found)) => {
                offset += found.size as usize;
                cache.entries.push(found);
            }
            // The free space: an entry with nothing in it.
            Ok(None) => break,
            Err(why) => {
                cache.problems.push(format!("entry at {offset:#x}: {why}"));
                break;
            }
        }
    }
    cache
}

/// The entry at `offset`, `None` for the free space's empty entry.
fn entry(data: &[u8], offset: usize, version: Version) -> Result<Option<Entry<'_>>, String> {
    let layout = version.entry_layout();
    let rest = &data[offset..];
    if !rest.starts_with(CACHE_SIGNATURE) {
        return Err("no CMMM signature".to_owned());
    }
    let short = || "a header cut short".to_owned();
    let size = u32_at(rest, 4).ok_or_else(short)?;
    let header = rest.get(..layout.header).ok_or_else(short)?;
    let field = |at: usize| u32_at(header, at).ok_or_else(short);
    let hash = u64_at(header, 8).ok_or_else(short)?;
    let identifier_size = field(layout.sizes_at)? as usize;
    let padding_size = field(layout.sizes_at + 4)? as usize;
    let data_size = field(layout.sizes_at + 8)? as usize;
    if hash == 0 && identifier_size == 0 && data_size == 0 {
        return Ok(None);
    }
    let end = size as usize;
    if end < layout.header || end > rest.len() {
        return Err(format!("a size of {size} bytes that doesn't fit"));
    }
    let body = &rest[layout.header..end];
    let data_start = identifier_size
        .checked_add(padding_size)
        .filter(|&start| start <= body.len())
        .ok_or("an identifier and padding past the entry's end")?;
    let image = body
        .get(data_start..data_start.saturating_add(data_size))
        .ok_or("data past the entry's end")?;
    let data_checksum = u64_at(header, layout.header - 16).ok_or_else(short)?;
    let header_checksum = u64_at(header, layout.header - 8).ok_or_else(short)?;
    let extension = layout
        .extension
        .then(|| common::text::utf16le_until_nul(&header[16..24]).text)
        .filter(|text| !text.is_empty());
    let dimension = |at: usize| u32_at(header, at).filter(|_| layout.dimensions);
    Ok(Some(Entry {
        offset,
        size,
        hash,
        extension,
        identifier: common::text::utf16le(&body[..identifier_size]).text,
        width: dimension(layout.sizes_at + 12),
        height: dimension(layout.sizes_at + 16),
        data_checksum,
        header_checksum,
        data_checksum_valid: crc64::data(image) == data_checksum,
        header_checksum_valid: crc64::header(&header[..layout.header - 8]) == header_checksum,
        format: ImageFormat::sniff(image),
        data_offset: offset + layout.header + data_start,
        data: image,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_round_trip() {
        for number in [20, 21, 30, 31, 32] {
            assert_eq!(
                Version::from_number(number).map(Version::number),
                Some(number)
            );
        }
        assert_eq!(Version::from_number(33), None);
        assert_eq!(Version::Windows10.size_name(4), Some("256"));
        assert_eq!(Version::Windows7.size_name(4), Some("sr"));
        assert_eq!(Version::Windows81.size_name(11), None);
    }

    #[test]
    fn formats_from_signatures() {
        assert_eq!(ImageFormat::sniff(b"BM6\x00"), ImageFormat::Bmp);
        assert_eq!(
            ImageFormat::sniff(&[0xFF, 0xD8, 0xFF, 0xE0]),
            ImageFormat::Jpeg
        );
        assert_eq!(ImageFormat::sniff(b"\x89PNG\r\n\x1a\n"), ImageFormat::Png);
        assert_eq!(ImageFormat::sniff(b"GIF8"), ImageFormat::Other);
        assert_eq!(ImageFormat::sniff(b""), ImageFormat::Empty);
        assert_eq!(ImageFormat::Png.extension(), Some("png"));
    }

    #[test]
    fn not_a_cache() {
        let cache = read(b"IMMM");
        assert!(cache.version.is_none());
        assert_eq!(cache.problems.len(), 1);
    }
}
