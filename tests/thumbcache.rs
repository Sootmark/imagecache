//! Thumbnail caches against dissect.thumbcache (run, never vendored) and
//! checksums computed independently (`tests/oracle/thumbcache_oracle.py`):
//! the synthetic caches of `tests/fixtures/synthetic/thumbcache/` (see
//! `tests/oracle/make_thumbcache.py`) always, and the real Windows caches
//! dissect.thumbcache tests with when fetched into
//! `tests/fixtures-real/dissect/` (`tests/oracle/fetch-dissect-samples.sh`,
//! then `cargo test -- --ignored`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use imagecache::thumbcache::{self, Version};
use proptest::prelude::*;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

/// An oracle table's rows by file, header dropped.
fn rows_by_file(tsv: &str) -> BTreeMap<String, Vec<String>> {
    let mut rows: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for line in tsv.lines().skip(1) {
        let (file, rest) = line.split_once('\t').unwrap();
        rows.entry(file.to_owned())
            .or_default()
            .push(rest.to_owned());
    }
    rows
}

/// A cache's entries as the oracle prints them.
fn cache_rows(data: &[u8]) -> Vec<String> {
    let cache = thumbcache::read(data);
    assert_eq!(cache.problems, Vec::<String>::new());
    cache
        .entries
        .iter()
        .map(|e| {
            let format = match e.format {
                thumbcache::ImageFormat::Bmp => "bmp",
                thumbcache::ImageFormat::Jpeg => "jpeg",
                thumbcache::ImageFormat::Png => "png",
                thumbcache::ImageFormat::Other => "other",
                thumbcache::ImageFormat::Empty => "empty",
            };
            [
                e.offset.to_string(),
                e.cache_id(),
                e.extension.clone().unwrap_or_default(),
                e.identifier.clone(),
                e.data.len().to_string(),
                common::sha256::hex(&common::sha256::Sha256::digest(e.data)),
                format.to_owned(),
                e.header_checksum_valid.to_string(),
                e.data_checksum_valid.to_string(),
            ]
            .join("\t")
        })
        .collect()
}

/// An index's entries as the oracle prints them.
fn index_rows(data: &[u8]) -> Vec<String> {
    let index = thumbcache::read_index(data);
    assert_eq!(index.problems, Vec::<String>::new());
    index
        .entries
        .iter()
        .map(|e| {
            let locations: Vec<String> = e
                .locations
                .iter()
                .map(|l| format!("{}:{}", l.cache_type, l.offset))
                .collect();
            format!("{}\t{}\t{}", e.cache_id(), e.flags, locations.join(","))
        })
        .collect()
}

fn check_against(folder: &Path, caches_tsv: &str, index_tsv: &str) -> usize {
    let mut entries = 0;
    for (file, expected) in rows_by_file(caches_tsv) {
        let data = std::fs::read(folder.join(&file)).unwrap();
        assert_eq!(cache_rows(&data), expected, "{file}");
        entries += expected.len();
    }
    for (file, expected) in rows_by_file(index_tsv) {
        let data = std::fs::read(folder.join(&file)).unwrap();
        assert_eq!(index_rows(&data), expected, "{file}");
    }
    entries
}

/// Every place the index names holds an entry with the index's hash.
fn index_matches_caches(folder: &Path) -> usize {
    let index = thumbcache::read_index(&std::fs::read(folder.join("thumbcache_idx.db")).unwrap());
    assert_eq!(index.problems, Vec::<String>::new());
    let mut checked = 0;
    for entry in &index.entries {
        for location in &entry.locations {
            let name = format!("thumbcache_{}.db", location.size_name.unwrap());
            let data = std::fs::read(folder.join(&name)).unwrap();
            let cache = thumbcache::read(&data);
            let found = cache
                .entries
                .iter()
                .find(|e| e.offset == location.offset as usize)
                .unwrap_or_else(|| panic!("{name}: no entry at {}", location.offset));
            assert_eq!(found.hash, entry.hash, "{name} at {}", location.offset);
            checked += 1;
        }
    }
    checked
}

#[test]
fn synthetic_caches_as_dissect_reads_them() {
    let folder = root().join("fixtures/synthetic/thumbcache");
    let entries = check_against(
        &folder,
        include_str!("oracle/synthetic-caches.tsv"),
        include_str!("oracle/synthetic-index.tsv"),
    );
    assert_eq!(entries, 72);
}

#[test]
fn synthetic_indexes_point_at_their_entries() {
    let folder = root().join("fixtures/synthetic/thumbcache");
    for version in ["vista", "windows7", "windows8", "windows81", "windows10"] {
        assert_eq!(index_matches_caches(&folder.join(version)), 16, "{version}");
    }
}

#[test]
fn windows8_layout_unverified_but_consistent() {
    // dissect.thumbcache doesn't know format version 30 and no Windows 8.0
    // cache is openly available: only the generator's reading of the
    // documented format, its checksums and its index agree here.
    let data =
        std::fs::read(root().join("fixtures/synthetic/thumbcache/windows8/thumbcache_256.db"))
            .unwrap();
    let cache = thumbcache::read(&data);
    assert_eq!(cache.problems, Vec::<String>::new());
    assert_eq!(cache.version, Some(Version::Windows8));
    assert_eq!(cache.size_name(), Some("256"));
    let valid: Vec<(bool, bool)> = cache
        .entries
        .iter()
        .map(|e| (e.header_checksum_valid, e.data_checksum_valid))
        .collect();
    let mut expected = vec![(true, true); 8];
    expected[6] = (true, false);
    expected[7] = (false, true);
    assert_eq!(valid, expected);
    assert_eq!(
        (cache.entries[2].width, cache.entries[2].height),
        (Some(8), Some(8))
    );
}

#[test]
fn entry_fields() {
    let folder = root().join("fixtures/synthetic/thumbcache");
    let vista = std::fs::read(folder.join("vista/thumbcache_32.db")).unwrap();
    let cache = thumbcache::read(&vista);
    assert_eq!(cache.size_name(), Some("32"));
    let jpeg = &cache.entries[2];
    assert_eq!(jpeg.extension.as_deref(), Some("jpg"));
    assert_eq!(jpeg.width, None);
    assert_eq!(
        &vista[jpeg.data_offset..jpeg.data_offset + jpeg.data.len()],
        jpeg.data
    );
    assert_eq!(
        cache.entries[3].identifier,
        "::{645FF040-5081-101B-9F08-00AA002F954E}"
    );
    assert_eq!(cache.entries[3].format, thumbcache::ImageFormat::Empty);

    let icons = std::fs::read(folder.join("windows10/iconcache_16.db")).unwrap();
    let cache = thumbcache::read(&icons);
    assert_eq!(
        (cache.version, cache.size_name()),
        (Some(Version::Windows10), Some("16"))
    );
    assert_eq!(cache.entries[0].cache_id(), "1111000000000203");
    assert_eq!(cache.entries[0].identifier, "1111000000000203");
    assert_eq!(
        (cache.entries[1].width, cache.entries[1].height),
        (Some(5), Some(3))
    );

    let index =
        thumbcache::read_index(&std::fs::read(folder.join("vista/thumbcache_idx.db")).unwrap());
    assert_eq!((index.used_entries, index.total_entries), (8, 11));
    assert!(index.entries[0].modified.is_some());
}

#[test]
#[ignore = "needs tests/fixtures-real/dissect (tests/oracle/fetch-dissect-samples.sh)"]
fn real_windows_caches() {
    let folder = root().join("fixtures-real/dissect");
    let entries = check_against(
        &folder,
        include_str!("oracle/dissect-caches.tsv"),
        include_str!("oracle/dissect-index.tsv"),
    );
    assert_eq!(entries, 323);
    // dissect.thumbcache misplaces Windows 8.1's first index entry, so
    // that index is checked against the caches it points into instead.
    for (version, places) in [
        ("windows_vista", 78),
        ("windows_7", 1),
        ("windows_81", 51),
        ("windows_10", 106),
        ("windows_11", 56),
    ] {
        assert_eq!(
            index_matches_caches(&folder.join(version)),
            places,
            "{version}"
        );
    }
}

proptest! {
    #[test]
    fn damaged_caches_read_without_panic(
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..64),
        cut in any::<usize>(),
        version in 0_usize..5,
    ) {
        let folder = ["vista", "windows7", "windows8", "windows81", "windows10"][version];
        let path = root().join("fixtures/synthetic/thumbcache").join(folder);
        for name in ["thumbcache_256.db", "thumbcache_idx.db"] {
            let mut data = std::fs::read(path.join(name)).unwrap();
            for &(at, byte) in &flips {
                let at = at % data.len();
                data[at] = byte;
            }
            data.truncate(cut % (data.len() + 1));
            let cache = thumbcache::read(&data);
            prop_assert!(cache.entries.iter().all(|e| e.data.len() < data.len()));
            let _ = thumbcache::read_index(&data);
        }
    }

    #[test]
    fn real_headers_with_random_tails(tail in proptest::collection::vec(any::<u8>(), 0..512)) {
        for (head, signature_at) in [
            (&b"CMMM\x14\0\0\0\x02\0\0\0\x18\0\0\0\x00\x01\0\0\x05\0\0\0"[..], 0),
            (&b"CMMM\x20\0\0\0\x04\0\0\0\0\0\0\0\x18\0\0\0\x00\x01\0\0"[..], 0),
            (&b"\x0c\0\x30\x20IMMM\x20\0\0\0\0\0\0\0\0\0\0\0\x05\0\0\0\x09\0\0\0"[..], 4),
            (&b"IMMM\x15\0\0\0\x02\0\0\0\x01\0\0\0\x65\0\0\0\0\0\0\0"[..], 0),
        ] {
            let data = [head, &tail].concat();
            if signature_at == 0 && data.starts_with(b"CMMM") {
                let cache = thumbcache::read(&data);
                prop_assert!(cache.version.is_some());
            }
            let _ = thumbcache::read_index(&data);
        }
    }

    #[test]
    fn arbitrary_bytes_read_without_panic(data in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = thumbcache::read(&data);
        let _ = thumbcache::read_index(&data);
        let mut cache_like = b"CMMM\x15\0\0\0".to_vec();
        cache_like.extend_from_slice(&data);
        let _ = thumbcache::read(&cache_like);
    }
}
