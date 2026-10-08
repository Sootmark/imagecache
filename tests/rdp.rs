//! RDP bitmap caches against ANSSI's bmc-tools (run, never vendored:
//! `tests/oracle/rdp_oracle.py`) and against the ground truth of a cache
//! tesserae generated from a known image (MIT, `tests/fixtures/tesserae/`,
//! `tests/oracle/rdp_truth.py`). The `.bmc` files and a small `.bin` are
//! synthetic (`tests/oracle/make_rdp.py`).

use std::collections::BTreeMap;
use std::path::PathBuf;

use imagecache::rdp::{self, Container};
use proptest::prelude::*;

fn fixture(path: &str) -> Vec<u8> {
    std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(path),
    )
    .unwrap()
}

fn sha256(data: &[u8]) -> String {
    common::sha256::hex(&common::sha256::Sha256::digest(data))
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

/// A cache's tiles as the oracle prints them.
fn tile_rows(cache: &rdp::BitmapCache) -> Vec<String> {
    cache
        .tiles
        .iter()
        .map(|t| {
            format!(
                "{}\t{:016x}\t{}\t{}\t{}\t{}",
                t.offset,
                t.key,
                t.width,
                t.height,
                t.compressed,
                sha256(&t.pixels)
            )
        })
        .collect()
}

#[test]
fn synthetic_caches_as_bmc_tools_decodes_them() {
    let expected = rows_by_file(include_str!("oracle/synthetic-tiles.tsv"));
    let mut tiles = 0;
    for (file, rows) in &expected {
        let cache = rdp::read(&fixture(&format!("synthetic/rdp/{file}")), file);
        assert_eq!(cache.problems, Vec::<String>::new(), "{file}");
        assert_eq!(&tile_rows(&cache), rows, "{file}");
        tiles += rows.len();
    }
    assert_eq!(tiles, 21);
}

#[test]
fn bmc_depths_found_without_the_name() {
    for (file, depth) in [
        ("bcache2.bmc", 1),
        ("bcache22.bmc", 2),
        ("bcache24.bmc", 4),
        ("bcache-depth3.bmc", 3),
    ] {
        let data = fixture(&format!("synthetic/rdp/{file}"));
        let guessed = rdp::read_bmc(&data, None);
        assert_eq!(guessed.bytes_per_pixel, Some(depth), "{file}");
        assert_eq!(guessed, rdp::read(&data, file), "{file}");
        assert_eq!(guessed.container, Some(Container::Bmc));
    }
}

#[test]
fn a_generated_cache_holds_its_source_image() {
    let cache = rdp::read(&fixture("tesserae/Cache0001.bin"), "Cache0001.bin");
    assert_eq!(cache.problems, Vec::<String>::new());
    assert_eq!(
        (cache.container, cache.version),
        (Some(Container::Bin), Some(3))
    );
    let expected = rows_by_file(include_str!("oracle/tesserae-tiles.tsv"));
    assert_eq!(tile_rows(&cache), expected["Cache0001.bin"]);

    // Independently of bmc-tools: each tile is the piece of the source
    // image the generator cut, with the key it gave it.
    let truth: Vec<String> = include_str!("oracle/tesserae-truth.tsv")
        .lines()
        .skip(1)
        .map(|line| line.split_once('\t').unwrap().1.to_owned())
        .collect();
    let ours: Vec<String> = cache
        .tiles
        .iter()
        .map(|t| format!("{:016x}\t{}", t.key, sha256(&t.pixels)))
        .collect();
    assert_eq!(ours, truth);
    assert_eq!(ours.len(), 51);
}

#[test]
fn repeated_tiles_listed_once() {
    let cache = rdp::read(&fixture("synthetic/rdp/bcache22.bmc"), "bcache22.bmc");
    let once = rdp::distinct(&cache.tiles);
    assert_eq!((cache.tiles.len(), once.len()), (5, 4));
    let other = rdp::read(&fixture("synthetic/rdp/bcache24.bmc"), "bcache24.bmc");
    assert_eq!(
        rdp::distinct(cache.tiles.iter().chain(&other.tiles)).len(),
        8
    );
}

#[test]
fn a_tile_as_a_bitmap() {
    let cache = rdp::read(&fixture("synthetic/rdp/Cache0002.bin"), "Cache0002.bin");
    let tile = &cache.tiles[3];
    assert_eq!((tile.width, tile.height), (5, 3));
    let bmp = tile.to_bmp();
    assert_eq!(&bmp[..2], b"BM");
    assert_eq!(
        u32::from_le_bytes(bmp[2..6].try_into().unwrap()) as usize,
        bmp.len()
    );
    assert_eq!(i32::from_le_bytes(bmp[18..22].try_into().unwrap()), 5);
    assert_eq!(i32::from_le_bytes(bmp[22..26].try_into().unwrap()), 3);
    // Bottom-up: the file's first row is the tile's last.
    assert_eq!(&bmp[54..54 + 20], &tile.pixels[40..60]);
}

#[test]
fn empty_slots_skipped_and_bad_tiles_reported() {
    let data = fixture("synthetic/rdp/bcache22.bmc");
    let stride = 20 + 64 * 64 * 2;
    let mut with_empty = vec![0; stride];
    with_empty.extend_from_slice(&data);
    let cache = rdp::read(&with_empty, "bcache22.bmc");
    assert_eq!(cache.problems, Vec::<String>::new());
    assert_eq!(cache.tiles.len(), 5);
    assert_eq!(cache.tiles[0].offset, stride);

    // Damage the first compressed tile's stream: only that tile is lost.
    let mut damaged = data.clone();
    damaged[2 * stride + 20] = 0xF5;
    let cache = rdp::read(&damaged, "bcache22.bmc");
    assert_eq!(cache.tiles.len(), 4);
    assert_eq!(cache.problems.len(), 1);
    assert!(
        cache.problems[0].contains("unknown order"),
        "{:?}",
        cache.problems
    );
}

#[test]
fn a_bin_cut_short() {
    let data = fixture("synthetic/rdp/Cache0002.bin");
    let cache = rdp::read_bin(&data[..20_000]);
    assert_eq!(cache.tiles.len(), 1);
    assert_eq!(cache.problems.len(), 1);
    assert!(rdp::read_bin(b"RDP8bmp").tiles.is_empty());
    assert_eq!(rdp::read_bin(b"not a cache").problems.len(), 1);
}

proptest! {
    #[test]
    fn damaged_caches_read_without_panic(
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 0..64),
        cut in any::<usize>(),
        file in 0_usize..5,
    ) {
        let name = ["bcache2.bmc", "bcache22.bmc", "bcache24.bmc", "bcache-depth3.bmc", "Cache0002.bin"][file];
        let mut data = fixture(&format!("synthetic/rdp/{name}"));
        for (at, byte) in flips {
            let at = at % data.len();
            data[at] = byte;
        }
        data.truncate(cut % (data.len() + 1));
        for cache in [rdp::read(&data, name), rdp::read_bmc(&data, None)] {
            for tile in &cache.tiles {
                prop_assert_eq!(tile.pixels.len(), usize::from(tile.width) * usize::from(tile.height) * 4);
            }
        }
    }

    #[test]
    fn real_headers_with_random_tails(
        tail in proptest::collection::vec(any::<u8>(), 0..20_000),
        depth in 1_u8..=4,
    ) {
        let bin = [&b"RDP8bmp\0\x06\0\0\0\x01\0\0\0\x02\0\0\0\x40\0\x40\0"[..], &tail].concat();
        let _ = rdp::read_bin(&bin);
        let compressed = [&b"\x01\0\0\0\x02\0\0\0\x40\0\x40\0\x00\x01\0\0\x08\0\0\0"[..], &tail].concat();
        let _ = rdp::read_bmc(&compressed, Some(depth));
        let _ = rdp::read_bmc(&compressed, None);
    }

    #[test]
    fn arbitrary_bytes_read_without_panic(data in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let _ = rdp::read(&data, "bcache24.bmc");
        let _ = rdp::read_bmc(&data, None);
        let _ = rdp::read_bmc(&data, Some(9));
        let _ = rdp::read_bin(&data);
    }
}
