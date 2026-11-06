//! Validated input-format conversion. Resampling and linear-light background
//! compositing live in [`crate::smolscale`].

use crate::color::Color;
use crate::geometry::pixel_count;

/// Supported raw input pixel formats (the six in-scope `ChafaPixelType`s).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelType {
    Rgba8,
    Bgra8,
    Argb8,
    Abgr8,
    Rgb8,
    Bgr8,
}

impl PixelType {
    /// Bytes per pixel.
    pub fn bpp(self) -> usize {
        match self {
            PixelType::Rgba8 | PixelType::Bgra8 | PixelType::Argb8 | PixelType::Abgr8 => 4,
            PixelType::Rgb8 | PixelType::Bgr8 => 3,
        }
    }

    /// Decode one pixel at `p` into RGBA.
    #[inline]
    fn decode(self, p: &[u8]) -> Color {
        match self {
            PixelType::Rgba8 => Color::new(p[0], p[1], p[2], p[3]),
            PixelType::Bgra8 => Color::new(p[2], p[1], p[0], p[3]),
            PixelType::Argb8 => Color::new(p[1], p[2], p[3], p[0]),
            PixelType::Abgr8 => Color::new(p[3], p[2], p[1], p[0]),
            PixelType::Rgb8 => Color::new(p[0], p[1], p[2], 0xff),
            PixelType::Bgr8 => Color::new(p[2], p[1], p[0], 0xff),
        }
    }
}

/// Convert a raw buffer (`w`×`h`, `rowstride` bytes/row) to a tight RGBA `Color`
/// grid. Faithful to `ChafaPixelType` channel orders.
///
/// # Panics
///
/// Panics for dimensions outside `1..=65535`, an undersized rowstride or buffer,
/// or overflowing buffer arithmetic.
pub fn to_rgba(ptype: PixelType, data: &[u8], w: usize, h: usize, rowstride: usize) -> Vec<Color> {
    let bpp = ptype.bpp();
    let count = pixel_count(w, h);
    let row_bytes = w * bpp;
    assert!(
        rowstride >= row_bytes,
        "rowstride is shorter than a pixel row"
    );
    let required = (h - 1)
        .checked_mul(rowstride)
        .and_then(|n| n.checked_add(row_bytes))
        .expect("source buffer size overflow");
    assert!(data.len() >= required, "source buffer is too short");
    let mut out = Vec::with_capacity(count);
    for y in 0..h {
        let row = &data[y * rowstride..];
        for x in 0..w {
            out.push(ptype.decode(&row[x * bpp..x * bpp + bpp]));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_orders() {
        let p = [0x11u8, 0x22, 0x33, 0x44];
        assert_eq!(
            PixelType::Rgba8.decode(&p),
            Color::new(0x11, 0x22, 0x33, 0x44)
        );
        assert_eq!(
            PixelType::Bgra8.decode(&p),
            Color::new(0x33, 0x22, 0x11, 0x44)
        );
        assert_eq!(
            PixelType::Argb8.decode(&p),
            Color::new(0x22, 0x33, 0x44, 0x11)
        );
        assert_eq!(
            PixelType::Abgr8.decode(&p),
            Color::new(0x44, 0x33, 0x22, 0x11)
        );
        assert_eq!(
            PixelType::Rgb8.decode(&p[..3]),
            Color::new(0x11, 0x22, 0x33, 0xff)
        );
        assert_eq!(
            PixelType::Bgr8.decode(&p[..3]),
            Color::new(0x33, 0x22, 0x11, 0xff)
        );
    }

    #[test]
    fn padded_rows_need_no_trailing_padding() {
        let data = [1, 2, 3, 255, 0, 0, 0, 0, 4, 5, 6, 255];
        assert_eq!(
            to_rgba(PixelType::Rgba8, &data, 1, 2, 8),
            [Color::new(1, 2, 3, 255), Color::new(4, 5, 6, 255)]
        );
    }

    #[test]
    #[should_panic(expected = "source buffer size overflow")]
    fn rejects_overflowing_rowstride() {
        to_rgba(PixelType::Rgba8, &[], 1, 2, usize::MAX);
    }

    #[test]
    #[should_panic(expected = "source buffer is too short")]
    fn rejects_short_buffer_before_decoding() {
        to_rgba(PixelType::Rgba8, &[0; 7], 1, 2, 4);
    }
}
