//! Interleaved RLE, the bitmap compression of the Remote Desktop Protocol
//! ([MS-RDPBCGR] 2.2.9.1.1.3.1.2.4 and 3.1.9), as `.bmc` tiles use it
//! for 8-, 16- and 32-bit pixels.
//!
//! The stream is a run of orders, each a header byte (an order code and,
//! for most, a run length, longer runs taking one or two more bytes) and
//! the pixels or bit masks it needs. Rows are drawn bottom-up, and most
//! orders draw relative to the row before (the pixel "above", in memory
//! order): a background run copies it, a foreground run XORs it with the
//! foreground colour. On the first row there is no row before, so black
//! stands in for it. Two background runs in a row are separated by one
//! foreground pixel, implied rather than written.
//!
//! [MS-RDPBCGR]: https://learn.microsoft.com/en-us/openspecs/windows_protocols/ms-rdpbcgr/

/// A pixel, its first `bytes_per_pixel` bytes used.
type Pixel = [u8; 4];

const BLACK: Pixel = [0; 4];
const WHITE: Pixel = [0xFF; 4];

/// What an order draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Order {
    /// Copy the pixel above (black on the first row).
    BackgroundRun,
    /// The pixel above XOR the foreground colour.
    ForegroundRun { sets_colour: bool },
    /// Two colours alternating.
    DitheredRun,
    /// One colour repeated.
    ColourRun,
    /// Pixels as they are.
    ColourImage,
    /// A bit mask choosing, per pixel, background or foreground.
    ForegroundBackgroundImage { sets_colour: bool },
    /// Eight pixels of a fixed bit mask.
    FixedMask(u8),
    /// One pixel.
    Single(Pixel),
}

/// How an order's run length is coded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Length {
    /// The low 5 bits; 0: the next byte plus 32.
    Regular,
    /// The low 5 bits times 8; 0: the next byte plus 1.
    RegularMask,
    /// The low 4 bits; 0: the next byte plus 16.
    Lite,
    /// The low 4 bits times 8; 0: the next byte plus 1.
    LiteMask,
    /// The next two bytes.
    Mega,
    /// Fixed by the order.
    Fixed(usize),
}

/// The order and run length coding a header byte names.
fn decode_header(header: u8) -> Option<(Order, Length)> {
    use Length::{Fixed, Lite, LiteMask, Mega, Regular, RegularMask};
    use Order::{
        BackgroundRun, ColourImage, ColourRun, DitheredRun, FixedMask, ForegroundBackgroundImage,
        ForegroundRun, Single,
    };
    let fg = |sets_colour| ForegroundRun { sets_colour };
    let fgbg = |sets_colour| ForegroundBackgroundImage { sets_colour };
    Some(match header {
        0x00..=0x1F => (BackgroundRun, Regular),
        0x20..=0x3F => (fg(false), Regular),
        0x40..=0x5F => (fgbg(false), RegularMask),
        0x60..=0x7F => (ColourRun, Regular),
        0x80..=0x9F => (ColourImage, Regular),
        0xC0..=0xCF => (fg(true), Lite),
        0xD0..=0xDF => (fgbg(true), LiteMask),
        0xE0..=0xEF => (DitheredRun, Lite),
        0xF0 => (BackgroundRun, Mega),
        0xF1 => (fg(false), Mega),
        0xF2 => (fgbg(false), Mega),
        0xF3 => (ColourRun, Mega),
        0xF4 => (ColourImage, Mega),
        0xF6 => (fg(true), Mega),
        0xF7 => (fgbg(true), Mega),
        0xF8 => (DitheredRun, Mega),
        0xF9 => (FixedMask(0x03), Fixed(8)),
        0xFA => (FixedMask(0x05), Fixed(8)),
        0xFD => (Single(WHITE), Fixed(1)),
        0xFE => (Single(BLACK), Fixed(1)),
        _ => return None,
    })
}

/// Decompress `stream` into `width` by `height` pixels of
/// `bytes_per_pixel` bytes, bottom-up as RDP draws them.
pub(crate) fn decompress(
    stream: &[u8],
    width: usize,
    height: usize,
    bytes_per_pixel: usize,
) -> Result<Vec<u8>, String> {
    let row = width * bytes_per_pixel;
    let mut decoder = Decoder {
        stream,
        at: 0,
        out: Vec::with_capacity(row * height),
        capacity: row * height,
        row,
        bytes_per_pixel,
        foreground: WHITE,
    };
    decoder.decode()?;
    if decoder.out.len() == decoder.capacity {
        Ok(decoder.out)
    } else {
        Err(format!(
            "{} of {} bytes drawn",
            decoder.out.len(),
            decoder.capacity
        ))
    }
}

struct Decoder<'a> {
    stream: &'a [u8],
    at: usize,
    out: Vec<u8>,
    capacity: usize,
    /// A row's size in bytes.
    row: usize,
    bytes_per_pixel: usize,
    foreground: Pixel,
}

impl Decoder<'_> {
    fn decode(&mut self) -> Result<(), String> {
        let mut first_row = true;
        let mut insert_foreground = false;
        while self.at < self.stream.len() {
            if first_row && self.out.len() >= self.row {
                first_row = false;
                insert_foreground = false;
            }
            let header = self.byte()?;
            let (order, length) =
                decode_header(header).ok_or_else(|| format!("an unknown order {header:#04x}"))?;
            let run = self.run_length(header, length)?;
            self.draw(order, run, first_row, insert_foreground)?;
            insert_foreground = order == Order::BackgroundRun;
        }
        Ok(())
    }

    fn draw(
        &mut self,
        order: Order,
        run: usize,
        first_row: bool,
        insert_foreground: bool,
    ) -> Result<(), String> {
        match order {
            Order::BackgroundRun => self.background_run(run, first_row, insert_foreground),
            Order::ForegroundRun { sets_colour } => {
                if sets_colour {
                    self.foreground = self.pixel()?;
                }
                (0..run).try_for_each(|_| self.push_foreground(first_row))
            }
            Order::DitheredRun => {
                let (a, b) = (self.pixel()?, self.pixel()?);
                (0..run).try_for_each(|_| {
                    self.push(a)?;
                    self.push(b)
                })
            }
            Order::ColourRun => {
                let colour = self.pixel()?;
                (0..run).try_for_each(|_| self.push(colour))
            }
            Order::ColourImage => (0..run).try_for_each(|_| {
                let pixel = self.pixel()?;
                self.push(pixel)
            }),
            Order::ForegroundBackgroundImage { sets_colour } => {
                if sets_colour {
                    self.foreground = self.pixel()?;
                }
                let mut left = run;
                while left > 0 {
                    let count = left.min(8);
                    let mask = self.byte()?;
                    self.masked(mask, count, first_row)?;
                    left -= count;
                }
                Ok(())
            }
            Order::FixedMask(mask) => self.masked(mask, run, first_row),
            Order::Single(pixel) => self.push(pixel),
        }
    }

    fn background_run(
        &mut self,
        run: usize,
        first_row: bool,
        insert_foreground: bool,
    ) -> Result<(), String> {
        let mut run = run;
        if insert_foreground && run > 0 {
            self.push_foreground(first_row)?;
            run -= 1;
        }
        (0..run).try_for_each(|_| {
            let pixel = if first_row { BLACK } else { self.above()? };
            self.push(pixel)
        })
    }

    /// Pixels chosen by the low `count` bits of `mask`, least significant
    /// first: set, foreground; clear, background.
    fn masked(&mut self, mask: u8, count: usize, first_row: bool) -> Result<(), String> {
        (0..count).try_for_each(|bit| {
            if mask & (1 << bit) == 0 {
                let pixel = if first_row { BLACK } else { self.above()? };
                self.push(pixel)
            } else {
                self.push_foreground(first_row)
            }
        })
    }

    /// The foreground colour, XOR the pixel above past the first row.
    fn push_foreground(&mut self, first_row: bool) -> Result<(), String> {
        let mut pixel = self.foreground;
        if !first_row {
            let above = self.above()?;
            pixel.iter_mut().zip(above).for_each(|(p, a)| *p ^= a);
        }
        self.push(pixel)
    }

    fn run_length(&mut self, header: u8, length: Length) -> Result<usize, String> {
        let low5 = usize::from(header & 0x1F);
        let low4 = usize::from(header & 0x0F);
        Ok(match length {
            Length::Regular if low5 == 0 => usize::from(self.byte()?) + 32,
            Length::Regular => low5,
            Length::RegularMask if low5 == 0 => usize::from(self.byte()?) + 1,
            Length::RegularMask => low5 * 8,
            Length::Lite if low4 == 0 => usize::from(self.byte()?) + 16,
            Length::Lite => low4,
            Length::LiteMask if low4 == 0 => usize::from(self.byte()?) + 1,
            Length::LiteMask => low4 * 8,
            Length::Mega => usize::from(u16::from_le_bytes([self.byte()?, self.byte()?])),
            Length::Fixed(run) => run,
        })
    }

    fn byte(&mut self) -> Result<u8, String> {
        let byte = *self
            .stream
            .get(self.at)
            .ok_or("the stream ends inside an order")?;
        self.at += 1;
        Ok(byte)
    }

    fn pixel(&mut self) -> Result<Pixel, String> {
        let bytes = self
            .stream
            .get(self.at..self.at + self.bytes_per_pixel)
            .ok_or("the stream ends inside a pixel")?;
        self.at += self.bytes_per_pixel;
        let mut pixel = BLACK;
        pixel[..bytes.len()].copy_from_slice(bytes);
        Ok(pixel)
    }

    fn above(&self) -> Result<Pixel, String> {
        let at = self.out.len().checked_sub(self.row).ok_or("no row above")?;
        let bytes = self
            .out
            .get(at..at + self.bytes_per_pixel)
            .ok_or("no row above")?;
        let mut pixel = BLACK;
        pixel[..bytes.len()].copy_from_slice(bytes);
        Ok(pixel)
    }

    fn push(&mut self, pixel: Pixel) -> Result<(), String> {
        if self.out.len() + self.bytes_per_pixel > self.capacity {
            return Err("more pixels than the tile holds".to_owned());
        }
        self.out.extend_from_slice(&pixel[..self.bytes_per_pixel]);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_runs_on_the_first_row_are_black_and_separated_by_foreground() {
        // Two 2-pixel background runs: black, black, then white (the
        // implied foreground pixel) and black.
        let out = decompress(&[0x02, 0x02], 4, 1, 1).unwrap();
        assert_eq!(out, [0, 0, 0xFF, 0]);
    }

    #[test]
    fn later_rows_copy_and_xor_the_row_above() {
        // Row 1: a colour image; row 2: a background run (copies), then a
        // foreground run with colour 0x0F (XORs).
        let stream = [0x82, 0x11, 0x22, 0x01, 0xC1, 0x0F];
        let out = decompress(&stream, 2, 2, 1).unwrap();
        assert_eq!(out, [0x11, 0x22, 0x11, 0x22 ^ 0x0F]);
    }

    #[test]
    fn foreground_background_masks_and_long_runs() {
        // Lite set-foreground mask of 8 pixels, mask 0b1010_0101.
        let out = decompress(&[0xD1, 0x07, 0xA5], 8, 1, 1).unwrap();
        assert_eq!(out, [7, 0, 7, 0, 0, 7, 0, 7]);
        // A mega colour run of 300 two-byte pixels.
        let out = decompress(&[0xF3, 0x2C, 0x01, 0x34, 0x12], 300, 1, 2).unwrap();
        assert!(out.chunks(2).all(|p| p == [0x34, 0x12]));
    }

    #[test]
    fn damage_is_an_error() {
        assert!(decompress(&[0xF5], 1, 1, 1).is_err());
        assert!(
            decompress(&[0x63], 2, 1, 1).is_err(),
            "a colour run without its colour"
        );
        assert!(
            decompress(&[0x05], 2, 1, 1).is_err(),
            "more pixels than fit"
        );
        assert!(
            decompress(&[0x01], 2, 1, 1).is_err(),
            "fewer pixels than fit"
        );
    }
}
