//! The CRC-64 the thumbnail cache checksums its entries with: reflected,
//! polynomial `0x92C64265D32139A4` (`0x259C84CBA6426349` written the
//! usual way round), no final inversion. The header checksum starts from
//! all ones; the data checksum starts from zero and, for data over 1024
//! bytes, covers the first 1024 bytes in full and then only the first
//! four bytes of every 400 that follow, the two CRCs combined by XOR.

const POLYNOMIAL: u64 = 0x92C6_4265_D321_39A4;
/// Data up to this size is checksummed in full.
const FULL_SPAN: usize = 1024;
/// Past [`FULL_SPAN`], one sample per stride.
const STRIDE: usize = 400;
/// The bytes sampled at the start of each stride.
const SAMPLE: usize = 4;

const TABLE: [u64; 256] = table();

const fn table() -> [u64; 256] {
    let mut table = [0; 256];
    let mut byte = 0;
    while byte < 256 {
        let mut crc = byte as u64;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 0 {
                crc >> 1
            } else {
                (crc >> 1) ^ POLYNOMIAL
            };
            bit += 1;
        }
        table[byte] = crc;
        byte += 1;
    }
    table
}

fn update(crc: u64, data: &[u8]) -> u64 {
    data.iter().fold(crc, |crc, &byte| {
        TABLE[((crc ^ u64::from(byte)) & 0xFF) as usize] ^ (crc >> 8)
    })
}

/// The checksum of an entry's header (every field before the checksum).
pub(crate) fn header(bytes: &[u8]) -> u64 {
    update(u64::MAX, bytes)
}

/// The checksum of an entry's data.
pub(crate) fn data(bytes: &[u8]) -> u64 {
    if bytes.len() <= FULL_SPAN {
        return update(0, bytes);
    }
    let (full, sampled) = bytes.split_at(FULL_SPAN);
    let tail_crc = sampled.chunks(STRIDE).fold(0, |crc, chunk| {
        update(crc, &chunk[..chunk.len().min(SAMPLE)])
    });
    update(0, full) ^ tail_crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_thumbcache_dll_holds() {
        // The first entries of the table as published from thumbcache.dll.
        assert_eq!(TABLE[1], 0x0809_E8A2_9694_51E9);
        assert_eq!(TABLE[2], 0x1013_D145_2D28_A3D2);
        assert_eq!(TABLE[128], POLYNOMIAL);
    }

    #[test]
    fn long_data_samples_four_bytes_per_stride() {
        let mut bytes = vec![7_u8; 2000];
        let before = data(&bytes);
        // Past the first 1024 bytes, only each stride's first four count.
        bytes[1024 + 4] = 0;
        bytes[1024 + 400 + 399] = 0;
        assert_eq!(data(&bytes), before);
        bytes[1024 + 400 + 3] = 0;
        assert_ne!(data(&bytes), before);
    }
}
