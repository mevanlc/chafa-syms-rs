//! High-level [`Canvas`] / [`CanvasConfig`] API tying together the pixel
//! pipeline, selection core, and printer.
//!
//! Builder-style configuration (idiomatic Rust) over chafa-tracking semantics.
//! Geometry is in **cells**; the pixel grid is `width*8 × height*8`. Input is
//! resampled to that grid (best-effort, see [`crate::pixops`]); pass a
//! pre-sized buffer for bit-exact selection.

use crate::color::Color;
use crate::geometry::{SYMBOL_HEIGHT_PIXELS, SYMBOL_WIDTH_PIXELS};
use crate::pixops::{self, PixelType};
use crate::printer::{print_cells, Optimizations};
use crate::select::{render_cells, CanvasMode, CellOut, RenderConfig};
use crate::smolscale;
use crate::symbol_map::SymbolMap;

/// Canvas configuration. Construct with [`CanvasConfig::new`] then use the
/// builder setters.
#[derive(Clone, Debug)]
pub struct CanvasConfig {
    /// Width in cells.
    pub width: usize,
    /// Height in cells.
    pub height: usize,
    pub mode: CanvasMode,
    /// Foreground color, packed `0x00RRGGBB`.
    pub fg_rgb: u32,
    /// Background color, packed `0x00RRGGBB`.
    pub bg_rgb: u32,
    /// Work factor `0.0..=1.0` (higher = more thorough, slower).
    pub work_factor: f32,
    pub fg_only: bool,
    pub preprocessing_enabled: bool,
    pub stipple_mode: StippleMode,
    pub optimizations: Optimizations,
    pub symbol_map: SymbolMap,
}

/// Optional post-processing for stipple-style featureless cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StippleMode {
    #[default]
    Off,
    /// Intended for grayscale/FGBG output: transparent cells become spaces, and
    /// featureless non-transparent cells are normalized globally onto `░▒▓█`.
    GrayscaleFill,
}

impl CanvasConfig {
    /// A config with chafa's library defaults (truecolor, white-on-black, work
    /// 0.5, all optimizations, `block+border+space-wide` symbols).
    pub fn new(width: usize, height: usize) -> Self {
        CanvasConfig {
            width,
            height,
            mode: CanvasMode::Truecolor,
            fg_rgb: 0xffffff,
            bg_rgb: 0x000000,
            work_factor: 0.5,
            fg_only: false,
            preprocessing_enabled: true,
            stipple_mode: StippleMode::Off,
            optimizations: Optimizations::all(),
            symbol_map: SymbolMap::chafa_default(),
        }
    }

    pub fn geometry(mut self, width: usize, height: usize) -> Self {
        self.width = width;
        self.height = height;
        self
    }
    pub fn mode(mut self, mode: CanvasMode) -> Self {
        self.mode = mode;
        self
    }
    pub fn fg_color(mut self, rgb: u32) -> Self {
        self.fg_rgb = rgb;
        self
    }
    pub fn bg_color(mut self, rgb: u32) -> Self {
        self.bg_rgb = rgb;
        self
    }
    pub fn work_factor(mut self, w: f32) -> Self {
        self.work_factor = w;
        self
    }
    pub fn fg_only(mut self, on: bool) -> Self {
        self.fg_only = on;
        self
    }
    pub fn preprocessing(mut self, on: bool) -> Self {
        self.preprocessing_enabled = on;
        self
    }
    pub fn stipple_mode(mut self, mode: StippleMode) -> Self {
        self.stipple_mode = mode;
        self
    }
    pub fn optimizations(mut self, o: Optimizations) -> Self {
        self.optimizations = o;
        self
    }
    pub fn symbol_map(mut self, m: SymbolMap) -> Self {
        self.symbol_map = m;
        self
    }
}

/// A drawable canvas: holds the resolved render config and the rendered cells.
pub struct Canvas {
    cfg: CanvasConfig,
    render_cfg: RenderConfig,
    cells: Vec<CellOut>,
}

impl Canvas {
    /// Create a canvas, compiling the symbol map and resolving the render
    /// configuration (mode gates, palettes, blank/solid chars).
    pub fn new(mut cfg: CanvasConfig) -> Self {
        cfg.symbol_map.prepare();
        let render_cfg = RenderConfig::new(
            cfg.mode,
            cfg.fg_only,
            cfg.fg_rgb,
            cfg.bg_rgb,
            cfg.work_factor,
            &cfg.symbol_map,
            None,
        );
        let cells = vec![
            CellOut {
                c: 0x20,
                fg: 0,
                bg: 0
            };
            cfg.width * cfg.height
        ];
        Canvas {
            cfg,
            render_cfg,
            cells,
        }
    }

    fn width_pixels(&self) -> usize {
        self.cfg.width * SYMBOL_WIDTH_PIXELS
    }
    fn height_pixels(&self) -> usize {
        self.cfg.height * SYMBOL_HEIGHT_PIXELS
    }

    /// Draw raw pixels onto the canvas: convert format → resample to the cell
    /// grid (a bit-exact port of chafa's smolscale, gamma-correct in
    /// premultiplied linear light) → composite alpha over the background → run
    /// the selection core.
    ///
    /// The resample is a pure stretch to `width*8 × height*8`, matching chafa
    /// run with `--stretch`. Placement/tuck/align (chafa's CLI default
    /// centering) is not modeled here.
    pub fn draw_all_pixels(
        &mut self,
        ptype: PixelType,
        data: &[u8],
        w: usize,
        h: usize,
        rowstride: usize,
    ) {
        // Decode the input format to tightly-packed RGBA8 bytes, then resample
        // with the smolscale port (which does its own unpack/premul/gamma).
        let src = pixops::to_rgba(ptype, data, w, h, rowstride);
        let src_bytes: Vec<u8> = src.iter().flat_map(|c| c.ch).collect();
        let (dw, dh) = (self.width_pixels(), self.height_pixels());
        let scaled = smolscale::scale_rgba8(&src_bytes, w, h, dw, dh);
        let mut grid: Vec<Color> = scaled
            .chunks_exact(4)
            .map(|p| Color::new(p[0], p[1], p[2], p[3]))
            .collect();
        preprocess_for_symbols(&mut grid, self.cfg.mode, self.cfg.preprocessing_enabled);
        if pixops::has_alpha(&grid) {
            composite_grid(&mut grid, self.cfg.bg_rgb);
        }
        self.cells = render_cells(&self.render_cfg, &self.cfg.symbol_map, None, &grid, dw, dh);
        apply_stipple_mode(
            self.cfg.stipple_mode,
            &self.render_cfg,
            &grid,
            dw,
            &mut self.cells,
        );
    }

    /// Draw a pre-sized RGBA grid (`width*8 × height*8`) directly — no
    /// resampling, so selection is bit-exact for the given pixels.
    pub fn draw_rgba_presized(&mut self, grid: &[Color]) {
        let (dw, dh) = (self.width_pixels(), self.height_pixels());
        assert_eq!(
            grid.len(),
            dw * dh,
            "pre-sized grid must be width*8 x height*8"
        );
        let mut grid = grid.to_vec();
        preprocess_for_symbols(&mut grid, self.cfg.mode, self.cfg.preprocessing_enabled);
        if pixops::has_alpha(&grid) {
            composite_grid(&mut grid, self.cfg.bg_rgb);
        }
        self.cells = render_cells(&self.render_cfg, &self.cfg.symbol_map, None, &grid, dw, dh);
        apply_stipple_mode(
            self.cfg.stipple_mode,
            &self.render_cfg,
            &grid,
            dw,
            &mut self.cells,
        );
    }

    /// The rendered cells.
    pub fn cells(&self) -> &[CellOut] {
        &self.cells
    }

    /// Serialize to an ANSI/UTF-8 string (rows joined by newlines).
    pub fn print(&self) -> String {
        print_cells(
            &self.render_cfg,
            &self.cfg.symbol_map,
            &self.cells,
            self.cfg.width,
            self.cfg.height,
            self.cfg.optimizations,
        )
    }
}

fn composite_grid(grid: &mut [Color], bg_rgb: u32) {
    let mut bg = Color::from_rgb_u32(bg_rgb);
    bg.ch[3] = 0xff;
    pixops::composite_over_bg(grid, bg);
}

fn apply_stipple_mode(
    mode: StippleMode,
    cfg: &RenderConfig,
    pixels: &[Color],
    width_pixels: usize,
    cells: &mut [CellOut],
) {
    match mode {
        StippleMode::Off => {}
        StippleMode::GrayscaleFill => {
            apply_grayscale_stipple_fill(cfg, pixels, width_pixels, cells)
        }
    }
}

fn apply_grayscale_stipple_fill(
    cfg: &RenderConfig,
    pixels: &[Color],
    width_pixels: usize,
    cells: &mut [CellOut],
) {
    let cols = width_pixels / SYMBOL_WIDTH_PIXELS;
    let mut fill_cells = Vec::new();
    let mut min_intensity = i32::MAX;
    let mut max_intensity = i32::MIN;

    for (i, cell) in cells.iter_mut().enumerate() {
        let cx = i % cols;
        let cy = i / cols;
        let Some(intensity) = cell_intensity(pixels, width_pixels, cx, cy, cfg.alpha_threshold)
        else {
            cell.c = 0x20;
            continue;
        };

        if is_stipple_fill_candidate(*cell) {
            min_intensity = min_intensity.min(intensity);
            max_intensity = max_intensity.max(intensity);
            fill_cells.push((i, intensity));
        }
    }

    for (i, intensity) in fill_cells {
        cells[i].c = grayscale_stipple_char(intensity, min_intensity, max_intensity);
    }
}

fn is_stipple_fill_candidate(cell: CellOut) -> bool {
    matches!(cell.c, 0x20 | 0x2588 | 0x2591 | 0x2592 | 0x2593) || cell.fg == cell.bg
}

fn cell_intensity(
    pixels: &[Color],
    width_pixels: usize,
    cx: usize,
    cy: usize,
    alpha_threshold: u8,
) -> Option<i32> {
    let mut sum = 0;
    let mut n = 0;

    for row in 0..SYMBOL_HEIGHT_PIXELS {
        let start = (cy * SYMBOL_HEIGHT_PIXELS + row) * width_pixels + cx * SYMBOL_WIDTH_PIXELS;
        for p in &pixels[start..start + SYMBOL_WIDTH_PIXELS] {
            if p.ch[3] >= alpha_threshold {
                sum += rgb_to_intensity_fast(*p);
                n += 1;
            }
        }
    }

    if n > 0 {
        Some(sum / n)
    } else {
        None
    }
}

fn grayscale_stipple_char(intensity: i32, min_intensity: i32, max_intensity: i32) -> u32 {
    if max_intensity <= min_intensity {
        return 0x2591;
    }

    let numerator = (intensity - min_intensity) * 3;
    let denominator = max_intensity - min_intensity;
    match (numerator * 2 + denominator) / (denominator * 2) {
        0 => 0x2591, // ░
        1 => 0x2592, // ▒
        2 => 0x2593, // ▓
        _ => 0x2588, // █
    }
}

const FIXED_MULT: i32 = 4096;
const INTENSITY_MAX: usize = 256 * 8;

fn preprocess_for_symbols(grid: &mut [Color], mode: CanvasMode, enabled: bool) {
    if !enabled {
        return;
    }

    let Some(crop_pct) = preprocessing_crop_pct(mode) else {
        return;
    };

    if matches!(
        mode,
        CanvasMode::Indexed16 | CanvasMode::Indexed16_8 | CanvasMode::Indexed8
    ) {
        for p in grid.iter_mut() {
            boost_saturation_rgb(p);
        }
    }

    let mut hist = Histogram::default();
    for p in grid.iter() {
        if p.ch[3] > 127 {
            hist.c[rgb_to_intensity_fast(*p) as usize] += 1;
            hist.n_samples += 1;
        }
    }
    hist.calc_bounds(crop_pct);
    normalize_rgb(grid, &hist);
}

fn preprocessing_crop_pct(mode: CanvasMode) -> Option<i32> {
    match mode {
        CanvasMode::Indexed16 | CanvasMode::Indexed16_8 => Some(5),
        CanvasMode::Indexed8 => Some(10),
        CanvasMode::FgbgBgfg | CanvasMode::Fgbg => Some(20),
        CanvasMode::Truecolor | CanvasMode::Indexed256 | CanvasMode::Indexed240 => None,
    }
}

#[derive(Clone)]
struct Histogram {
    c: [i32; INTENSITY_MAX],
    n_samples: i32,
    min: i32,
    max: i32,
}

impl Default for Histogram {
    fn default() -> Self {
        Histogram {
            c: [0; INTENSITY_MAX],
            n_samples: 0,
            min: 0,
            max: 0,
        }
    }
}

impl Histogram {
    fn calc_bounds(&mut self, crop_pct: i32) {
        let pixels_crop = (self.n_samples as i64 * ((crop_pct as i64 * 1024) / 100)) / 1024;

        let mut t = pixels_crop as i32;
        for i in 0..INTENSITY_MAX {
            t -= self.c[i];
            if t <= 0 {
                self.min = i as i32;
                break;
            }
        }

        t = pixels_crop as i32;
        for i in (0..INTENSITY_MAX).rev() {
            t -= self.c[i];
            if t <= 0 {
                self.max = i as i32;
                break;
            }
        }
    }
}

fn rgb_to_intensity_fast(color: Color) -> i32 {
    color.ch[0] as i32 * 3 + color.ch[1] as i32 * 4 + color.ch[2] as i32
}

fn normalize_ch(v: u8, min: i32, factor: i32) -> u8 {
    let mut vt = v as i32;
    vt -= min;
    vt *= factor;
    vt /= FIXED_MULT;
    vt.clamp(0, 255) as u8
}

fn normalize_rgb(grid: &mut [Color], hist: &Histogram) {
    if hist.min == hist.max {
        return;
    }

    let factor = ((INTENSITY_MAX as i32 - 1) * FIXED_MULT) / (hist.max - hist.min);
    let min = hist.min / 8;
    for p in grid {
        p.ch[0] = normalize_ch(p.ch[0], min, factor);
        p.ch[1] = normalize_ch(p.ch[1], min, factor);
        p.ch[2] = normalize_ch(p.ch[2], min, factor);
    }
}

fn boost_saturation_rgb(color: &mut Color) {
    let p = (color.ch[0] as f32 * color.ch[0] as f32 * 0.299
        + color.ch[1] as f32 * color.ch[1] as f32 * 0.587
        + color.ch[2] as f32 * color.ch[2] as f32 * 0.144)
        .sqrt();

    for ch in 0..3 {
        let v = p + (color.ch[ch] as f32 - p) * 2.0;
        color.ch[ch] = (v as i32).clamp(0, 255) as u8;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::SYMBOL_N_PIXELS;

    #[test]
    fn end_to_end_truecolor_runs() {
        let cfg = CanvasConfig::new(10, 5);
        let mut canvas = Canvas::new(cfg);
        // A 40x20 RGBA image (will be downscaled to 80x40).
        let data: Vec<u8> = (0..40 * 20 * 4).map(|i| (i % 256) as u8).collect();
        canvas.draw_all_pixels(PixelType::Rgba8, &data, 40, 20, 40 * 4);
        let out = canvas.print();
        assert!(!out.is_empty());
        assert_eq!(canvas.cells().len(), 50);
    }

    #[test]
    fn grayscale_stipple_fill_maps_transparency_and_global_range() {
        let mut map = SymbolMap::new();
        map.apply_selectors("half,stipple").unwrap();
        let cfg = CanvasConfig::new(5, 1)
            .mode(CanvasMode::Fgbg)
            .preprocessing(false)
            .stipple_mode(StippleMode::GrayscaleFill)
            .symbol_map(map);
        let mut canvas = Canvas::new(cfg);
        let cells = [
            [0, 0, 0, 0],
            [20, 20, 20, 255],
            [95, 95, 95, 255],
            [170, 170, 170, 255],
            [245, 245, 245, 255],
        ];
        let mut grid = Vec::with_capacity(5 * SYMBOL_N_PIXELS);
        for _row in 0..SYMBOL_HEIGHT_PIXELS {
            for rgba in cells {
                for _ in 0..SYMBOL_WIDTH_PIXELS {
                    grid.push(Color::new(rgba[0], rgba[1], rgba[2], rgba[3]));
                }
            }
        }

        canvas.draw_rgba_presized(&grid);

        assert_eq!(canvas.print(), " ░▒▓█");
    }
}
