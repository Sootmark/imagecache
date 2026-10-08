//! Stored pixels to 32-bit BGRA (alpha always 255).

/// The 20 colours Windows reserves in a 256-colour palette, as blue,
/// green, red: the first ten take indexes 0 to 9, the last ten 246 to 255.
const STATIC_LOW: [[u8; 3]; 10] = [
    [0, 0, 0],
    [0, 0, 128],
    [0, 128, 0],
    [0, 128, 128],
    [128, 0, 0],
    [128, 0, 128],
    [128, 128, 0],
    [192, 192, 192],
    [192, 220, 192],
    [240, 202, 166],
];
const STATIC_HIGH: [[u8; 3]; 10] = [
    [240, 251, 255],
    [164, 160, 160],
    [128, 128, 128],
    [0, 0, 255],
    [0, 255, 0],
    [0, 255, 255],
    [255, 0, 0],
    [255, 0, 255],
    [255, 255, 0],
    [255, 255, 255],
];

/// The palette an 8-bit tile is drawn with (the cache doesn't keep the
/// session's): the 20 static colours around a cube of 4 blues, 8 greens
/// and 8 reds, index `bbgggrrr`.
fn palette(index: u8) -> [u8; 3] {
    match index {
        0..=9 => STATIC_LOW[usize::from(index)],
        246..=255 => STATIC_HIGH[usize::from(index - 246)],
        _ => [(index >> 6) * 64, ((index >> 3) & 7) * 32, (index & 7) * 32],
    }
}

/// Widen a 5- or 6-bit channel to 8 bits, so that full scale stays full.
fn widen(value: u16, bits: u32) -> u8 {
    let value = value << (8 - bits);
    (value | (value >> bits)) as u8
}

/// One stored pixel of `bytes_per_pixel` bytes (1: palette index, 2:
/// RGB565, 3: BGR, 4: BGRX) as BGRA.
pub(crate) fn to_bgra(pixel: &[u8]) -> [u8; 4] {
    match *pixel {
        [index] => {
            let [b, g, r] = palette(index);
            [b, g, r, 0xFF]
        }
        [low, high] => {
            let value = u16::from_le_bytes([low, high]);
            [
                widen(value & 0x1F, 5),
                widen((value >> 5) & 0x3F, 6),
                widen(value >> 11, 5),
                0xFF,
            ]
        }
        [b, g, r] | [b, g, r, _] => [b, g, r, 0xFF],
        _ => [0, 0, 0, 0xFF],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb565_keeps_full_scale() {
        assert_eq!(to_bgra(&[0xFF, 0xFF]), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(to_bgra(&[0x1F, 0x00]), [0xFF, 0, 0, 0xFF]);
        assert_eq!(to_bgra(&[0xE0, 0x07]), [0, 0xFF, 0, 0xFF]);
        assert_eq!(to_bgra(&[0x00, 0xF8]), [0, 0, 0xFF, 0xFF]);
    }

    #[test]
    fn the_palette_has_static_colours_around_a_cube() {
        assert_eq!(to_bgra(&[0]), [0, 0, 0, 0xFF]);
        assert_eq!(to_bgra(&[255]), [0xFF, 0xFF, 0xFF, 0xFF]);
        assert_eq!(to_bgra(&[10]), [0, 32, 64, 0xFF]);
        assert_eq!(to_bgra(&[245]), [192, 192, 160, 0xFF]);
    }

    #[test]
    fn alpha_is_opaque() {
        assert_eq!(to_bgra(&[1, 2, 3, 0]), [1, 2, 3, 0xFF]);
        assert_eq!(to_bgra(&[1, 2, 3]), [1, 2, 3, 0xFF]);
    }
}
