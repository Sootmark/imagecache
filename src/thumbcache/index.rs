//! The index, `thumbcache_idx.db`: for each cache entry hash, where its
//! thumbnail is in each size's cache file.
//!
//! Vista and Windows 7 start the file with `IMMM`, the format version,
//! the number of entries used and the number allocated, in a 24-byte
//! header. Windows 8 and later put four bytes before the signature, and
//! after the 28-byte header each cache file's size and entry count (32
//! bits each, all sizes first), padded to 8 bytes.
//! Each entry then holds the hash, (Vista: a FILETIME), flags and one
//! 32-bit offset per cache file, in cache type order: 0 or `0xFFFFFFFF`
//! where that size has no thumbnail.

use common::time::Ts;

use super::{Version, INDEX_SIGNATURE};
use crate::le::{u32_at, u64_at};

/// An offset left unset.
const UNSET: [u32; 2] = [0, u32::MAX];

/// One place a thumbnail is: a cache file and an offset in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Location {
    /// The cache file's type (see [`Version::size_name`]).
    pub cache_type: u32,
    /// The part of the cache file's name after `thumbcache_` (`"256"`).
    pub size_name: Option<&'static str>,
    /// The entry's offset in that file.
    pub offset: u32,
}

/// An index entry: a cache entry hash and where its thumbnails are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    /// Its offset in the index file.
    pub offset: usize,
    /// The cache entry hash (`ThumbnailCacheId`).
    pub hash: u64,
    /// Last modified (Vista only).
    pub modified: Option<Ts>,
    /// Flags, undocumented.
    pub flags: u32,
    /// The cache files that hold a thumbnail for it, and where.
    pub locations: Vec<Location>,
}

impl IndexEntry {
    /// The hash as Windows Search shows `System_ThumbnailCacheId`.
    #[must_use]
    pub fn cache_id(&self) -> String {
        format!("{:016x}", self.hash)
    }
}

/// The index's entries in use and what couldn't be read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    /// The format version, if the header could be read.
    pub version: Option<Version>,
    /// The entries in use, as the header counts them.
    pub used_entries: u32,
    /// The entries allocated, as the header counts them.
    pub total_entries: u32,
    /// The entries in use (a hash other than zero), in file order.
    pub entries: Vec<IndexEntry>,
    /// What couldn't be read.
    pub problems: Vec<String>,
}

/// Where a version's index fields are.
struct IndexLayout {
    /// Where the format version is (the signature is 4 bytes before).
    version_at: usize,
    /// Where the used and total entry counts are.
    counts_at: usize,
    /// Where the first entry is.
    entries_at: usize,
    /// An entry's size.
    entry_size: usize,
    /// Where an entry's flags are.
    flags_at: usize,
    /// Where an entry's offsets are.
    offsets_at: usize,
}

impl IndexLayout {
    fn of(version: Version) -> Self {
        let caches = version.sizes().len();
        match version {
            Version::Vista => Self {
                version_at: 4,
                counts_at: 12,
                entries_at: 24,
                entry_size: 20 + 4 * caches,
                flags_at: 16,
                offsets_at: 20,
            },
            Version::Windows7 => Self {
                version_at: 4,
                counts_at: 12,
                entries_at: 24,
                entry_size: 12 + 4 * caches,
                flags_at: 8,
                offsets_at: 12,
            },
            Version::Windows8 | Version::Windows81 | Version::Windows10 => Self {
                version_at: 8,
                counts_at: 20,
                entries_at: (28 + 8 * caches).next_multiple_of(8),
                entry_size: (16 + 4 * caches).next_multiple_of(8),
                flags_at: 8,
                offsets_at: 16,
            },
        }
    }
}

/// Whether `head` starts like an index file.
#[must_use]
pub fn is_index(head: &[u8]) -> bool {
    head.starts_with(INDEX_SIGNATURE)
        || head
            .get(4..)
            .is_some_and(|h| h.starts_with(INDEX_SIGNATURE))
}

/// Read `thumbcache_idx.db`: each hash in use and the cache files that
/// hold its thumbnails.
#[must_use]
pub fn read_index(data: &[u8]) -> Index {
    let mut index = Index::default();
    let version_at = if data.starts_with(INDEX_SIGNATURE) {
        4
    } else if is_index(data) {
        8
    } else {
        index
            .problems
            .push("not an index file (no IMMM)".to_owned());
        return index;
    };
    let Some(number) = u32_at(data, version_at) else {
        index.problems.push("a header cut short".to_owned());
        return index;
    };
    let version = match Version::from_number(number) {
        Some(version) if IndexLayout::of(version).version_at == version_at => version,
        _ => {
            index.problems.push(format!(
                "format version {number} at {version_at:#x}, not one known"
            ));
            return index;
        }
    };
    let layout = IndexLayout::of(version);
    let (Some(used), Some(total)) = (
        u32_at(data, layout.counts_at),
        u32_at(data, layout.counts_at + 4),
    ) else {
        index.problems.push("a header cut short".to_owned());
        return index;
    };
    index.version = Some(version);
    index.used_entries = used;
    index.total_entries = total;
    let table = data.get(layout.entries_at..).unwrap_or_default();
    let chunks = table.chunks_exact(layout.entry_size);
    let available = chunks.len();
    for (number, raw) in chunks.take(total as usize).enumerate() {
        let offset = layout.entries_at + number * layout.entry_size;
        if let Some(entry) = index_entry(raw, offset, version, &layout) {
            index.entries.push(entry);
        }
    }
    if available < total as usize {
        index.problems.push(format!(
            "{total} entries allocated, room for {available} in the file"
        ));
    }
    index
}

/// The entry in `raw`, `None` when unused.
fn index_entry(
    raw: &[u8],
    offset: usize,
    version: Version,
    layout: &IndexLayout,
) -> Option<IndexEntry> {
    let hash = u64_at(raw, 0).filter(|&hash| hash != 0)?;
    let modified = match version {
        Version::Vista => u64_at(raw, 8).filter(|&t| t != 0).map(Ts::from_filetime),
        _ => None,
    };
    let locations = (0..version.sizes().len())
        .filter_map(|cache| {
            let at = u32_at(raw, layout.offsets_at + 4 * cache)?;
            let cache_type = cache as u32;
            (!UNSET.contains(&at)).then(|| Location {
                cache_type,
                size_name: version.size_name(cache_type),
                offset: at,
            })
        })
        .collect();
    Some(IndexEntry {
        offset,
        hash,
        modified,
        flags: u32_at(raw, layout.flags_at)?,
        locations,
    })
}
