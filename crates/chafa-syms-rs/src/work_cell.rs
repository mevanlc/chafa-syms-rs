//! Per-cell scratch math. Port of `chafa-work-cell.c` (AVERAGE extractor only;
//! the MEDIAN extractor is out of scope — chafa itself calls it "extremely slow
//! and makes almost no difference").
//!
//! The per-channel `for ch in 0..4` / `for c in 0..4` loops mirror chafa's
//! fixed RGBA channel iteration and read clearest with explicit indices.
#![allow(clippy::needless_range_loop)]

use crate::color::{color_diff, Color, ColorPair, COLOR_PAIR_BG, COLOR_PAIR_FG};
use crate::geometry::{SYMBOL_HEIGHT_PIXELS, SYMBOL_N_PIXELS, SYMBOL_WIDTH_PIXELS};
use crate::symbol::Symbol;

/// Transient 8x8 pixel block.
/// Mirrors `ChafaWorkCell`.
pub struct WorkCell {
    /// The cell's 64 pixels, row-major.
    pub pixels: [Color; SYMBOL_N_PIXELS],
}

impl WorkCell {
    /// Copy the 8x8 block at cell `(cx, cy)` out of the `src_width`-wide pixel
    /// grid. Port of `chafa_work_cell_init` + `fetch_canvas_pixel_block`.
    pub fn init(src_image: &[Color], src_width: usize, cx: usize, cy: usize) -> Self {
        let mut pixels = [Color::default(); SYMBOL_N_PIXELS];
        let mut i = 0;
        for row in 0..SYMBOL_HEIGHT_PIXELS {
            let start = (cy * SYMBOL_HEIGHT_PIXELS + row) * src_width + cx * SYMBOL_WIDTH_PIXELS;
            pixels[i..i + SYMBOL_WIDTH_PIXELS]
                .copy_from_slice(&src_image[start..start + SYMBOL_WIDTH_PIXELS]);
            i += SYMBOL_WIDTH_PIXELS;
        }
        WorkCell { pixels }
    }

    /// Threshold the 64 pixels into a `u64` bitmap: set a bit where the pixel is
    /// closer to FG than BG (strict `error_bg > error_fg`). MSB-first.
    /// Port of `chafa_work_cell_to_bitmap`.
    pub fn to_bitmap(&self, color_pair: &ColorPair) -> u64 {
        let mut bitmap = 0u64;
        for p in &self.pixels {
            bitmap <<= 1;
            let e0 = color_diff(*p, color_pair.colors[0]);
            let e1 = color_diff(*p, color_pair.colors[1]);
            if e0 > e1 {
                bitmap |= 1;
            }
        }
        bitmap
    }

    /// Two contrasting colors via median cut along the dominant channel:
    /// BG = darkest pixel, FG = brightest. Port of
    /// `chafa_work_cell_get_contrasting_color_pair`.
    pub fn contrasting_color_pair(&self) -> ColorPair {
        // Only extrema are needed. Preserve the old stable-sort endpoints:
        // first minimum, last maximum, and first channel on equal ranges.
        let mut min_index = [0; 4];
        let mut max_index = [0; 4];
        for (i, pixel) in self.pixels.iter().enumerate().skip(1) {
            for ch in 0..4 {
                if pixel.ch[ch] < self.pixels[min_index[ch]].ch[ch] {
                    min_index[ch] = i;
                }
                if pixel.ch[ch] >= self.pixels[max_index[ch]].ch[ch] {
                    max_index[ch] = i;
                }
            }
        }
        let mut best_ch = 0;
        let mut best_range = 0;
        for ch in 0..4 {
            let range = self.pixels[max_index[ch]].ch[ch] - self.pixels[min_index[ch]].ch[ch];
            if range > best_range {
                best_range = range;
                best_ch = ch;
            }
        }
        let bg = self.pixels[min_index[best_ch]];
        let fg = self.pixels[max_index[best_ch]];
        let mut pair = ColorPair::default();
        pair.colors[COLOR_PAIR_BG] = bg;
        pair.colors[COLOR_PAIR_FG] = fg;
        pair
    }

    /// AVERAGE color extractor: mean of covered pixels = FG, mean of uncovered =
    /// BG. Port of `chafa_work_cell_get_mean_colors_for_symbol` +
    /// `extract_cell_mean_colors_plain`. Division is integer floor; weights ≤ 1
    /// skip division (matching chafa).
    pub fn mean_colors_for_symbol(&self, sym: &Symbol) -> ColorPair {
        let coverage = sym.coverage();
        // accums[0] = BG (uncovered), accums[1] = FG (covered). i16 channels.
        let mut accums = [[0i32; 4]; 2];
        for (i, p) in self.pixels.iter().enumerate() {
            let pen = coverage[i] as usize;
            for c in 0..4 {
                accums[pen][c] += p.ch[c] as i32;
            }
        }
        if sym.fg_weight > 1 {
            for c in 0..4 {
                accums[1][c] /= sym.fg_weight as i32;
            }
        }
        if sym.bg_weight > 1 {
            for c in 0..4 {
                accums[0][c] /= sym.bg_weight as i32;
            }
        }
        let mut pair = ColorPair::default();
        pair.colors[COLOR_PAIR_BG] = accum_to_color(&accums[0]);
        pair.colors[COLOR_PAIR_FG] = accum_to_color(&accums[1]);
        pair
    }

    /// Mean color of the whole cell (all 64 pixels). Port of
    /// `chafa_work_cell_calc_mean_color`.
    pub fn calc_mean_color(&self) -> Color {
        let mut accum = [0i32; 4];
        for p in &self.pixels {
            for c in 0..4 {
                accum[c] += p.ch[c] as i32;
            }
        }
        for c in 0..4 {
            accum[c] /= SYMBOL_N_PIXELS as i32;
        }
        accum_to_color(&accum)
    }
}

/// Truncate an accumulator (i16 semantics) to a [`Color`]. The accumulator
/// holds non-negative sums/means that fit in `u8` post-division, matching
/// chafa's `accum_to_color` (`color->ch[i] = accum->ch[i]`).
fn accum_to_color(accum: &[i32; 4]) -> Color {
    Color::new(
        accum[0] as u8,
        accum[1] as u8,
        accum[2] as u8,
        accum[3] as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_block(rgba: [u8; 4]) -> [Color; SYMBOL_N_PIXELS] {
        [Color::new(rgba[0], rgba[1], rgba[2], rgba[3]); SYMBOL_N_PIXELS]
    }

    #[test]
    fn init_copies_block() {
        let grid = solid_block([1, 2, 3, 255]);
        let wc = WorkCell::init(&grid, 8, 0, 0);
        assert_eq!(wc.pixels[0], Color::new(1, 2, 3, 255));
        assert_eq!(wc.pixels[63], Color::new(1, 2, 3, 255));
    }

    #[test]
    fn contrasting_pair_preserves_stable_endpoint_ties() {
        let mut grid = solid_block([128, 128, 0, 255]);
        grid[0] = Color::new(0, 10, 1, 255);
        grid[1] = Color::new(0, 0, 2, 255);
        grid[62] = Color::new(255, 255, 3, 255);
        grid[63] = Color::new(255, 240, 4, 255);
        let wc = WorkCell::init(&grid, 8, 0, 0);
        let pair = wc.contrasting_color_pair();
        assert_eq!(pair.colors[COLOR_PAIR_BG], grid[0]);
        assert_eq!(pair.colors[COLOR_PAIR_FG], grid[63]);
    }

    #[test]
    fn mean_color_of_flat_cell() {
        let grid = solid_block([40, 80, 120, 255]);
        let wc = WorkCell::init(&grid, 8, 0, 0);
        assert_eq!(wc.calc_mean_color(), Color::new(40, 80, 120, 255));
    }
}
