//! The selection core (the novel part). Port of `chafa-symbol-renderer.c`'s
//! per-cell symbol/color selection, wide-symbol lookback, fill fallback, and
//! blank normalization, plus the mode gates set in `chafa-canvas.c:557-579`.
//!
//! This module covers the truecolor and fg-only paths exactly (the Phase 4
//! core-parity milestone). Palette-quantized color output for indexed/fgbg
//! modes is layered on in Phase 6 via [`CanvasMode`] + a palette.

use crate::color::{color_average_2, color_diff, Color, ColorPair, COLOR_PAIR_BG, COLOR_PAIR_FG};
use crate::geometry::{N_BUF_CELLS, N_CANDIDATES_MAX, SYMBOL_ERROR_MAX, SYMBOL_N_PIXELS};
use crate::symbol::Symbol;
use crate::symbol_map::{Candidate, SymbolMap};
use crate::work_cell::WorkCell;

/// Output color modes. Phase 4 implements `Truecolor`; the rest land in Phase 6.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CanvasMode {
    Truecolor,
    Indexed256,
    Indexed240,
    Indexed16,
    Indexed8,
    Indexed16_8,
    FgbgBgfg,
    Fgbg,
}

/// One output cell: chosen symbol + colors. `c == 0` marks a wide continuation.
/// In truecolor, `fg`/`bg` are packed `0xAARRGGBB`; in indexed modes (Phase 6)
/// they are palette indices.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellOut {
    pub c: u32,
    pub fg: u32,
    pub bg: u32,
}

/// Pack a color as chafa's `0xAARRGGBB` (`chafa_pack_color`).
pub fn pack_color(c: Color) -> u32 {
    ((c.ch[0] as u32) << 16) | ((c.ch[1] as u32) << 8) | (c.ch[2] as u32) | ((c.ch[3] as u32) << 24)
}

/// `transparent_cell_color` (`chafa-symbol-renderer.c:57-69`). Truecolor packs
/// 50%-gray with zero alpha; other modes use a palette transparent index.
fn transparent_cell_color(mode: CanvasMode) -> u32 {
    if mode == CanvasMode::Truecolor {
        pack_color(Color::new(0x80, 0x80, 0x80, 0x00))
    } else {
        // CHAFA_PALETTE_INDEX_TRANSPARENT
        256
    }
}

/// Resolved per-render configuration: the mode gates plus default colors and
/// blank/solid chars. Built once via [`RenderConfig::new`].
#[derive(Clone, Debug)]
pub struct RenderConfig {
    pub mode: CanvasMode,
    pub work_factor_int: i32,
    pub fg_only: bool,
    pub default_colors: ColorPair,
    pub consider_inverted: bool,
    pub extract_colors: bool,
    pub use_quantized_error: bool,
    pub blank_char: u32,
    pub solid_char: u32,
}

impl RenderConfig {
    /// Derive the render configuration the way `chafa-canvas.c` does on canvas
    /// creation. `fg_rgb`/`bg_rgb` are packed `0x00RRGGBB` config colors.
    /// `work_factor` is in `0.0..=1.0`. The (prepared) symbol map and optional
    /// fill map determine the blank/solid characters.
    pub fn new(
        mode: CanvasMode,
        fg_only: bool,
        fg_rgb: u32,
        bg_rgb: u32,
        work_factor: f32,
        symbol_map: &SymbolMap,
        fill_map: Option<&SymbolMap>,
    ) -> Self {
        // FGBG forces fg_only on (chafa-canvas.c:568-569).
        let fg_only = fg_only || mode == CanvasMode::Fgbg;

        let consider_inverted = !(fg_only || mode == CanvasMode::Fgbg);
        let extract_colors = !(mode == CanvasMode::Fgbg || mode == CanvasMode::FgbgBgfg);
        let use_quantized_error = mode == CanvasMode::Indexed16_8 && !fg_only;

        // default_colors (update_display_colors). config colors are 0x00RRGGBB.
        let mut fg = Color::from_rgb_u32(fg_rgb);
        let mut bg = Color::from_rgb_u32(bg_rgb);
        fg.ch[3] = 0xff;
        bg.ch[3] = 0x00;
        if extract_colors && fg_only {
            // 50%-gray stand-in FG; nudge BG away by 5 per channel.
            fg = Color::new(0x7f, 0x7f, 0x7f, 0xff);
            for i in 0..3 {
                bg.ch[i] = differentiate_channel(bg.ch[i], fg.ch[i], 5);
            }
        }
        let mut default_colors = ColorPair::default();
        default_colors.colors[COLOR_PAIR_FG] = fg;
        default_colors.colors[COLOR_PAIR_BG] = bg;

        let blank_char = find_best_blank_char(symbol_map, fill_map);
        let solid_char = find_best_solid_char(symbol_map, fill_map);

        RenderConfig {
            mode,
            work_factor_int: (work_factor * 10.0 + 0.5) as i32,
            fg_only,
            default_colors,
            consider_inverted,
            extract_colors,
            use_quantized_error,
            blank_char,
            solid_char,
        }
    }
}

/// `differentiate_channel` (`chafa-canvas.c:140-151`).
fn differentiate_channel(dest: u8, reference: u8, min_diff: i32) -> u8 {
    let diff = dest as i32 - reference as i32;
    if diff >= -min_diff && diff <= 0 {
        (reference as i32 - min_diff).max(0) as u8
    } else if (0..=min_diff).contains(&diff) {
        (reference as i32 + min_diff).min(255) as u8
    } else {
        dest
    }
}

fn find_best_blank_char(symbol_map: &SymbolMap, fill_map: Option<&SymbolMap>) -> u32 {
    if symbol_map.has_symbol(' ') || fill_map.is_some_and(|f| f.has_symbol(' ')) {
        return 0x20;
    }
    if let Some(f) = fill_map {
        if let Some(c) = nearest_char(f, 0) {
            return c;
        }
    }
    nearest_char(symbol_map, 0).unwrap_or(0x20)
}

fn find_best_solid_char(symbol_map: &SymbolMap, fill_map: Option<&SymbolMap>) -> u32 {
    if symbol_map.has_symbol('\u{2588}') || fill_map.is_some_and(|f| f.has_symbol('\u{2588}')) {
        return 0x2588;
    }
    if let Some(f) = fill_map {
        if let Some((c, hd)) = nearest_char_hd(f, u64::MAX) {
            if hd <= 32 {
                return c;
            }
        }
    }
    if let Some((c, hd)) = nearest_char_hd(symbol_map, u64::MAX) {
        if hd <= 32 {
            return c;
        }
    }
    0
}

fn nearest_char(map: &SymbolMap, bitmap: u64) -> Option<u32> {
    nearest_char_hd(map, bitmap).map(|(c, _)| c)
}

fn nearest_char_hd(map: &SymbolMap, bitmap: u64) -> Option<(u32, u8)> {
    let mut cands = [Candidate {
        symbol_index: 0,
        hamming_distance: 0,
        is_inverted: false,
    }; N_CANDIDATES_MAX];
    let mut n = N_CANDIDATES_MAX;
    map.find_candidates(bitmap, false, &mut cands, &mut n);
    if n == 0 {
        return None;
    }
    let s = &map.symbols_ref()[cands[0].symbol_index];
    Some((s.c as u32, cands[0].hamming_distance))
}

// --- per-symbol evaluation ---

struct SymbolEval {
    colors: ColorPair,
    error: i32,
}

fn calc_cell_error(
    pixels: &[Color; SYMBOL_N_PIXELS],
    pair: &ColorPair,
    cov: &[u8; SYMBOL_N_PIXELS],
) -> i32 {
    let mut error = 0i32;
    for i in 0..SYMBOL_N_PIXELS {
        error += color_diff(pair.colors[cov[i] as usize], pixels[i]);
    }
    error
}

/// Evaluate one narrow symbol against the work cell, updating the running best
/// (strict `<` — first candidate wins ties). Port of `eval_symbol`.
fn eval_symbol(
    cfg: &RenderConfig,
    wcell: &WorkCell,
    sym: &Symbol,
    sym_index: usize,
    best_index: &mut i64,
    best: &mut SymbolEval,
) {
    let colors = if cfg.fg_only {
        cfg.default_colors
    } else {
        wcell.mean_colors_for_symbol(sym)
    };
    // use_quantized_error is false for all Phase 4 modes (truecolor/fgbg/fg-only),
    // so error is computed against the raw extracted colors (no palette snap).
    debug_assert!(
        !cfg.use_quantized_error,
        "quantized error needs Phase 6 palette"
    );
    let error = calc_cell_error(&wcell.pixels, &colors, &sym.coverage());
    if error < best.error {
        *best_index = sym_index as i64;
        *best = SymbolEval { colors, error };
    }
}

struct SymbolEval2 {
    colors: ColorPair,
    error: [i32; 2],
}

#[allow(clippy::too_many_arguments)]
fn eval_symbol_wide(
    cfg: &RenderConfig,
    wa: &WorkCell,
    wb: &WorkCell,
    sym_a: &Symbol,
    sym_b: &Symbol,
    sym_index: usize,
    best_index: &mut i64,
    best: &mut SymbolEval2,
) {
    let colors = if cfg.fg_only {
        cfg.default_colors
    } else {
        let pa = wa.mean_colors_for_symbol(sym_a);
        let pb = wb.mean_colors_for_symbol(sym_b);
        let mut c = ColorPair::default();
        c.colors[COLOR_PAIR_FG] =
            color_average_2(pa.colors[COLOR_PAIR_FG], pb.colors[COLOR_PAIR_FG]);
        c.colors[COLOR_PAIR_BG] =
            color_average_2(pa.colors[COLOR_PAIR_BG], pb.colors[COLOR_PAIR_BG]);
        c
    };
    let e0 = calc_cell_error(&wa.pixels, &colors, &sym_a.coverage());
    let e1 = calc_cell_error(&wb.pixels, &colors, &sym_b.coverage());
    if e0 + e1 < best.error[0] + best.error[1] {
        *best_index = sym_index as i64;
        *best = SymbolEval2 {
            colors,
            error: [e0, e1],
        };
    }
}

/// Port of `pick_symbol_and_colors_fast`/`_slow` (narrow). Returns
/// `(codepoint, colors, error)`.
fn pick_symbol_and_colors(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    wcell: &mut WorkCell,
) -> (u32, ColorPair, i32) {
    let symbols = smap.symbols_ref();
    let mut best_index: i64 = -1;
    let mut best = SymbolEval {
        colors: ColorPair::default(),
        error: SYMBOL_ERROR_MAX,
    };

    if cfg.work_factor_int >= 8 {
        // Slow path: all symbols are candidates.
        for (i, sym) in symbols.iter().enumerate() {
            eval_symbol(cfg, wcell, sym, i, &mut best_index, &mut best);
        }
    } else {
        // Fast path: shortlist by shape, then evaluate by color error.
        let color_pair = if cfg.extract_colors && !cfg.fg_only {
            wcell.contrasting_color_pair()
        } else {
            cfg.default_colors
        };
        let bitmap = wcell.to_bitmap(&color_pair);
        let mut n_candidates = cfg.work_factor_int.clamp(1, N_CANDIDATES_MAX as i32) as usize;
        let mut candidates = [Candidate {
            symbol_index: 0,
            hamming_distance: 0,
            is_inverted: false,
        }; N_CANDIDATES_MAX];
        smap.find_candidates(
            bitmap,
            cfg.consider_inverted,
            &mut candidates,
            &mut n_candidates,
        );
        for cand in &candidates[..n_candidates] {
            let sym = &symbols[cand.symbol_index];
            eval_symbol(
                cfg,
                wcell,
                sym,
                cand.symbol_index,
                &mut best_index,
                &mut best,
            );
        }
    }

    let best_index = best_index as usize;
    // fg-only: re-extract the real colors of the winning symbol.
    if cfg.extract_colors && cfg.fg_only {
        best.colors = wcell.mean_colors_for_symbol(&symbols[best_index]);
    }
    (symbols[best_index].c as u32, best.colors, best.error)
}

/// Port of `pick_symbol_and_colors_wide_fast`/`_slow`.
fn pick_symbol_and_colors_wide(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    wa: &mut WorkCell,
    wb: &mut WorkCell,
) -> Option<(u32, ColorPair, i32, i32)> {
    let wide = smap.wide_symbols_ref();
    if wide.is_empty() {
        return None;
    }
    let mut best_index: i64 = -1;
    let mut best = SymbolEval2 {
        colors: ColorPair::default(),
        error: [SYMBOL_ERROR_MAX, SYMBOL_ERROR_MAX],
    };

    if cfg.work_factor_int >= 8 {
        for (i, w) in wide.iter().enumerate() {
            eval_symbol_wide(
                cfg,
                wa,
                wb,
                &w.sym[0],
                &w.sym[1],
                i,
                &mut best_index,
                &mut best,
            );
        }
    } else {
        let color_pair = if cfg.mode == CanvasMode::Fgbg || cfg.mode == CanvasMode::FgbgBgfg {
            cfg.default_colors
        } else {
            let ca = wa.contrasting_color_pair();
            let cb = wb.contrasting_color_pair();
            let mut c = ColorPair::default();
            c.colors[0] = color_average_2(ca.colors[0], cb.colors[0]);
            c.colors[1] = color_average_2(ca.colors[1], cb.colors[1]);
            c
        };
        let bitmaps = [wa.to_bitmap(&color_pair), wb.to_bitmap(&color_pair)];
        let mut n_candidates = cfg.work_factor_int.clamp(1, N_CANDIDATES_MAX as i32) as usize;
        let mut candidates = [Candidate {
            symbol_index: 0,
            hamming_distance: 0,
            is_inverted: false,
        }; N_CANDIDATES_MAX];
        smap.find_wide_candidates(
            bitmaps,
            cfg.consider_inverted,
            &mut candidates,
            &mut n_candidates,
        );
        for cand in &candidates[..n_candidates] {
            let w = &wide[cand.symbol_index];
            eval_symbol_wide(
                cfg,
                wa,
                wb,
                &w.sym[0],
                &w.sym[1],
                cand.symbol_index,
                &mut best_index,
                &mut best,
            );
        }
    }

    let bi = best_index as usize;
    if cfg.extract_colors && cfg.fg_only {
        let w = &wide[bi];
        let pa = wa.mean_colors_for_symbol(&w.sym[0]);
        let pb = wb.mean_colors_for_symbol(&w.sym[1]);
        best.colors.colors[COLOR_PAIR_FG] =
            color_average_2(pa.colors[COLOR_PAIR_FG], pb.colors[COLOR_PAIR_FG]);
        best.colors.colors[COLOR_PAIR_BG] =
            color_average_2(pa.colors[COLOR_PAIR_BG], pb.colors[COLOR_PAIR_BG]);
    }
    Some((
        wide[bi].sym[0].c as u32,
        best.colors,
        best.error[0],
        best.error[1],
    ))
}

/// Port of `update_cell_colors`. Truecolor packs; fg-only forces transparent BG.
/// Indexed/fgbg palette quantization is added in Phase 6.
fn update_cell_colors(cfg: &RenderConfig, cell: &mut CellOut, pair: &ColorPair) {
    match cfg.mode {
        CanvasMode::Truecolor => {
            cell.fg = pack_color(pair.colors[COLOR_PAIR_FG]);
            cell.bg = pack_color(pair.colors[COLOR_PAIR_BG]);
        }
        _ => unimplemented!("palette-quantized modes land in Phase 6"),
    }
    if cfg.fg_only {
        cell.bg = transparent_cell_color(cfg.mode);
    }
}

fn update_cell(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    wcell: &mut WorkCell,
    cell: &mut CellOut,
) -> i32 {
    if smap.symbols_ref().is_empty() {
        return SYMBOL_ERROR_MAX;
    }
    let (sym, pair, err) = pick_symbol_and_colors(cfg, smap, wcell);
    cell.c = sym;
    update_cell_colors(cfg, cell, &pair);
    err
}

#[allow(clippy::too_many_arguments)]
fn update_cells_wide(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    wa: &mut WorkCell,
    wb: &mut WorkCell,
    cell_a: &mut CellOut,
    cell_b: &mut CellOut,
    err_a: &mut i32,
    err_b: &mut i32,
) {
    *err_a = SYMBOL_ERROR_MAX;
    *err_b = SYMBOL_ERROR_MAX;
    let Some((sym, pair, ea, eb)) = pick_symbol_and_colors_wide(cfg, smap, wa, wb) else {
        return;
    };
    *err_a = ea;
    *err_b = eb;
    cell_a.c = sym;
    cell_b.c = 0;
    update_cell_colors(cfg, cell_a, &pair);
    cell_b.fg = cell_a.fg;
    cell_b.bg = cell_a.bg;
    // solid_char (from 16/8 revert) is narrow; extend to both cells.
    if cell_a.c == cfg.solid_char {
        cell_b.c = cell_a.c;
    }
}

/// Render the full cell grid for `pixels` (RGBA `Color`, `width_pixels` wide).
/// Sequential; threading is added in Phase 7 (output is row-independent).
pub fn render_cells(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    fill_map: Option<&SymbolMap>,
    pixels: &[Color],
    width_pixels: usize,
    height_pixels: usize,
) -> Vec<CellOut> {
    let cols = width_pixels / crate::geometry::SYMBOL_WIDTH_PIXELS;
    let rows = height_pixels / crate::geometry::SYMBOL_HEIGHT_PIXELS;
    let mut cells = vec![
        CellOut {
            c: 0x20,
            fg: 0,
            bg: 0
        };
        cols * rows
    ];
    for row in 0..rows {
        update_cells_row(
            cfg,
            smap,
            fill_map,
            pixels,
            width_pixels,
            &mut cells,
            cols,
            row,
        );
    }
    cells
}

#[allow(clippy::too_many_arguments)]
fn update_cells_row(
    cfg: &RenderConfig,
    smap: &SymbolMap,
    fill_map: Option<&SymbolMap>,
    pixels: &[Color],
    width_pixels: usize,
    cells: &mut [CellOut],
    cols: usize,
    row: usize,
) {
    // Ring buffer of work cells (for wide lookback) + their errors.
    let mut work_cells: Vec<Option<WorkCell>> = (0..N_BUF_CELLS).map(|_| None).collect();
    let mut cell_errors = [0i32; N_BUF_CELLS];
    let base = row * cols;

    for cx in 0..cols {
        let buf_index = cx % N_BUF_CELLS;

        cells[base + cx] = CellOut {
            c: 0x20,
            fg: 0,
            bg: 0,
        };

        let mut wcell = WorkCell::init(pixels, width_pixels, cx, row);
        let mut cell = cells[base + cx];
        cell_errors[buf_index] = update_cell(cfg, smap, &mut wcell, &mut cell);
        cells[base + cx] = cell;
        work_cells[buf_index] = Some(wcell);

        // Wide-symbol lookback over (cx-1, cx).
        if cx >= 1 && cells[base + cx - 1].c != 0 {
            let prev_buf = (cx - 1) % N_BUF_CELLS;
            // Take the two work cells out for &mut access.
            let (mut wa, mut wb) = take_two(&mut work_cells, prev_buf, buf_index);
            let mut wide_a = CellOut { c: 0, fg: 0, bg: 0 };
            let mut wide_b = CellOut { c: 0, fg: 0, bg: 0 };
            let mut wea = SYMBOL_ERROR_MAX;
            let mut web = SYMBOL_ERROR_MAX;
            update_cells_wide(
                cfg,
                smap,
                &mut wa,
                &mut wb,
                &mut wide_a,
                &mut wide_b,
                &mut wea,
                &mut web,
            );
            if wea + web < cell_errors[prev_buf] + cell_errors[buf_index] {
                cells[base + cx - 1] = wide_a;
                cells[base + cx] = wide_b;
                cell_errors[prev_buf] = wea;
                cell_errors[buf_index] = web;
            }
            work_cells[prev_buf] = Some(wa);
            work_cells[buf_index] = Some(wb);
        }

        // Fill fallback for featureless cells (inert when fill_map is empty/None).
        let cur = cells[base + cx];
        if cur.c != 0 && (cur.c == 0x20 || cur.c == 0x2588 || cur.fg == cur.bg) {
            let wcell = work_cells[buf_index].as_ref().unwrap();
            apply_fill(cfg, fill_map, wcell, &mut cells[base + cx]);
        }

        // Blank normalization: still-featureless -> blank_char; ASCII space
        // inherits previous fg to reduce escape churn.
        let cur = cells[base + cx];
        if cur.c != 0 && (cur.c == 0x20 || cur.fg == cur.bg) {
            cells[base + cx].c = cfg.blank_char;
            if cfg.blank_char == 0x20 && cx > 0 {
                let prev_fg = cells[base + cx - 1].fg;
                cells[base + cx].fg = prev_fg;
                if cfg.mode == CanvasMode::Truecolor {
                    cells[base + cx].fg |= 0xff00_0000;
                }
            }
        }
    }
}

/// Take two distinct elements mutably from the ring buffer.
fn take_two(cells: &mut [Option<WorkCell>], i: usize, j: usize) -> (WorkCell, WorkCell) {
    debug_assert_ne!(i, j);
    let a = cells[i].take().unwrap();
    let b = cells[j].take().unwrap();
    (a, b)
}

/// Fill fallback. Inert (no-op) when there is no fill map — chafa returns early
/// when `fill_symbol_map.n_symbols == 0`, which is the default. The non-empty
/// fill path is wired up in a later phase.
fn apply_fill(
    cfg: &RenderConfig,
    fill_map: Option<&SymbolMap>,
    _wcell: &WorkCell,
    _cell: &mut CellOut,
) {
    let Some(fill) = fill_map else { return };
    if fill.symbols_ref().is_empty() {
        return;
    }
    let _ = cfg;
    unimplemented!("non-empty fill map lands in Phase 9 (--fill)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_color_is_aarrggbb() {
        assert_eq!(pack_color(Color::new(0x30, 0x60, 0xa0, 0xff)), 0xff3060a0);
    }

    #[test]
    fn config_gates_truecolor() {
        let mut m = SymbolMap::chafa_default();
        m.prepare();
        let cfg = RenderConfig::new(
            CanvasMode::Truecolor,
            false,
            0xffffff,
            0x000000,
            0.5,
            &m,
            None,
        );
        assert!(cfg.consider_inverted);
        assert!(cfg.extract_colors);
        assert!(!cfg.use_quantized_error);
        assert_eq!(cfg.work_factor_int, 5);
        assert_eq!(cfg.blank_char, 0x20);
        assert_eq!(cfg.solid_char, 0x2588);
    }

    #[test]
    fn config_gates_fg_only() {
        let mut m = SymbolMap::chafa_default();
        m.prepare();
        let cfg = RenderConfig::new(
            CanvasMode::Truecolor,
            true,
            0xffffff,
            0x000000,
            0.5,
            &m,
            None,
        );
        assert!(!cfg.consider_inverted);
        assert!(cfg.extract_colors);
        // Forced 50%-gray FG stand-in.
        assert_eq!(
            cfg.default_colors.colors[COLOR_PAIR_FG].ch,
            [0x7f, 0x7f, 0x7f, 0xff]
        );
    }
}
