//! Pixel ingestion pipeline: input-format conversion, scaling, and
//! alpha-over-background compositing.
//!
//! The **alpha composite** and **format conversion** are faithful ports of
//! `chafa-pixops.c` (`composite_alpha_on_bg`, the `ChafaPixelType` byte orders).
//! The **scaler** is a self-contained pure-Rust box/bilinear resampler — it is
//! *not* a bit-exact port of smolscale (D2: scaling is best-effort, not the
//! parity gate). The selection core is validated against chafa's exact
//! post-prep pixels regardless (see `tests/selection_parity.rs`).
//!
//! Per-channel `for c in 0..4` loops mirror chafa's fixed RGBA iteration.
#![allow(clippy::needless_range_loop)]

use crate::color::Color;

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
pub fn to_rgba(ptype: PixelType, data: &[u8], w: usize, h: usize, rowstride: usize) -> Vec<Color> {
    let bpp = ptype.bpp();
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        let row = &data[y * rowstride..];
        for x in 0..w {
            out.push(ptype.decode(&row[x * bpp..x * bpp + bpp]));
        }
    }
    out
}

/// Whether any pixel is non-opaque (`have_alpha`).
pub fn has_alpha(pixels: &[Color]) -> bool {
    pixels.iter().any(|p| p.ch[3] != 0xff)
}

/// Composite straight-alpha pixels over an opaque background:
/// `(c*a + bg*(255-a)) / 255` per channel, alpha forced opaque. Port of
/// `composite_alpha_on_bg` (`chafa-pixops.c:643-664`).
pub fn composite_over_bg(pixels: &mut [Color], bg: Color) {
    for p in pixels.iter_mut() {
        let a = p.ch[3] as u32;
        for c in 0..3 {
            p.ch[c] = ((p.ch[c] as u32 * a + bg.ch[c] as u32 * (255 - a)) / 255) as u8;
        }
        p.ch[3] = 0xff;
    }
}

/// Resample `src` (`sw`×`sh`) to `dw`×`dh`. Box-averages when shrinking an axis
/// and bilinearly interpolates when growing it, channel-wise in sRGB space.
/// Best-effort (not smolscale-exact).
pub fn scale(src: &[Color], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<Color> {
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    // Two-pass separable resample: horizontal then vertical.
    let horiz = resample_axis_rows(src, sw, sh, dw);
    resample_axis_cols(&horiz, dw, sh, dh)
}

/// Resample each row from `sw` to `dw` samples.
fn resample_axis_rows(src: &[Color], sw: usize, sh: usize, dw: usize) -> Vec<Color> {
    let mut out = vec![Color::default(); dw * sh];
    for y in 0..sh {
        let srow = &src[y * sw..y * sw + sw];
        let drow = &mut out[y * dw..y * dw + dw];
        resample_line(srow, drow, sw, dw);
    }
    out
}

/// Resample each column from `sh` to `dh` samples (operating on `w`-wide rows).
fn resample_axis_cols(src: &[Color], w: usize, sh: usize, dh: usize) -> Vec<Color> {
    let mut out = vec![Color::default(); w * dh];
    // Gather a column, resample, scatter back.
    let mut col_in = vec![Color::default(); sh];
    let mut col_out = vec![Color::default(); dh];
    for x in 0..w {
        for y in 0..sh {
            col_in[y] = src[y * w + x];
        }
        resample_line(&col_in, &mut col_out, sh, dh);
        for y in 0..dh {
            out[y * w + x] = col_out[y];
        }
    }
    out
}

/// Resample a 1-D line of `n` samples to `m` samples.
fn resample_line(src: &[Color], dst: &mut [Color], n: usize, m: usize) {
    if m <= n {
        // Downscale: average each output's source span (box filter).
        for (j, d) in dst.iter_mut().enumerate() {
            let lo = j * n / m;
            let hi = ((j + 1) * n).div_ceil(m).max(lo + 1).min(n);
            let mut acc = [0u32; 4];
            let cnt = (hi - lo) as u32;
            for s in &src[lo..hi] {
                for c in 0..4 {
                    acc[c] += s.ch[c] as u32;
                }
            }
            *d = Color::new(
                (acc[0] / cnt) as u8,
                (acc[1] / cnt) as u8,
                (acc[2] / cnt) as u8,
                (acc[3] / cnt) as u8,
            );
        }
    } else {
        // Upscale: linear interpolation between neighbouring source samples.
        for (j, d) in dst.iter_mut().enumerate() {
            let pos = if m > 1 {
                j as f32 * (n as f32 - 1.0) / (m as f32 - 1.0)
            } else {
                0.0
            };
            let i0 = pos.floor() as usize;
            let i1 = (i0 + 1).min(n - 1);
            let t = pos - i0 as f32;
            let a = src[i0];
            let b = src[i1];
            let mut ch = [0u8; 4];
            for c in 0..4 {
                ch[c] = (a.ch[c] as f32 * (1.0 - t) + b.ch[c] as f32 * t).round() as u8;
            }
            *d = Color::new(ch[0], ch[1], ch[2], ch[3]);
        }
    }
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
    fn composite_opaque_is_identity() {
        let mut px = [Color::new(10, 20, 30, 255)];
        composite_over_bg(&mut px, Color::new(0, 0, 0, 255));
        assert_eq!(px[0], Color::new(10, 20, 30, 255));
    }

    #[test]
    fn composite_transparent_is_bg() {
        let mut px = [Color::new(10, 20, 30, 0)];
        composite_over_bg(&mut px, Color::new(7, 8, 9, 255));
        assert_eq!(px[0], Color::new(7, 8, 9, 255));
    }

    #[test]
    fn scale_identity() {
        let src = vec![Color::new(1, 2, 3, 255); 4];
        assert_eq!(scale(&src, 2, 2, 2, 2), src);
    }

    #[test]
    fn scale_downscale_averages() {
        // 2x1 -> 1x1 averages the two pixels.
        let src = vec![Color::new(0, 0, 0, 255), Color::new(100, 100, 100, 255)];
        let out = scale(&src, 2, 1, 1, 1);
        assert_eq!(out[0], Color::new(50, 50, 50, 255));
    }
}
