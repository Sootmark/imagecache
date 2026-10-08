//! A 32-bit Windows bitmap from top-down BGRA pixels.

/// The file header and `BITMAPINFOHEADER` before the pixels.
const HEADERS: usize = 14 + 40;
/// 72 dots per inch, in pixels per metre.
const PIXELS_PER_METRE: u32 = 2835;

/// `width` by `height` BGRA pixels, top row first, as a `.bmp` file
/// (uncompressed, rows stored bottom-up as usual).
pub(crate) fn encode(width: u16, height: u16, pixels: &[u8]) -> Vec<u8> {
    let row = usize::from(width) * 4;
    let image_size = row * usize::from(height);
    let mut file = Vec::with_capacity(HEADERS + image_size);
    file.extend_from_slice(b"BM");
    file.extend_from_slice(&((HEADERS + image_size) as u32).to_le_bytes());
    file.extend_from_slice(&[0; 4]);
    file.extend_from_slice(&(HEADERS as u32).to_le_bytes());
    file.extend_from_slice(&40_u32.to_le_bytes());
    file.extend_from_slice(&u32::from(width).to_le_bytes());
    file.extend_from_slice(&u32::from(height).to_le_bytes());
    file.extend_from_slice(&1_u16.to_le_bytes());
    file.extend_from_slice(&32_u16.to_le_bytes());
    file.extend_from_slice(&0_u32.to_le_bytes());
    file.extend_from_slice(&(image_size as u32).to_le_bytes());
    file.extend_from_slice(&PIXELS_PER_METRE.to_le_bytes());
    file.extend_from_slice(&PIXELS_PER_METRE.to_le_bytes());
    file.extend_from_slice(&[0; 8]);
    if row > 0 {
        pixels
            .chunks_exact(row)
            .rev()
            .for_each(|line| file.extend_from_slice(line));
    }
    file
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_are_stored_bottom_up() {
        let top = [1, 2, 3, 255];
        let bottom = [4, 5, 6, 255];
        let file = encode(1, 2, &[top, bottom].concat());
        assert_eq!(file.len(), HEADERS + 8);
        assert_eq!(&file[..2], b"BM");
        assert_eq!(&file[HEADERS..], [bottom, top].concat());
        assert_eq!(file[28], 32, "bits per pixel");
    }
}
