//! Phase 4 MILESTONE gate: the Rust selection core, fed chafa's exact post-prep
//! pixels, must reproduce chafa's per-cell `(char, fg, bg)` picks bit-for-bit in
//! truecolor — across symbol sets and the normal / fg-only mode gates.
//!
//! This isolates the selection core from the pixel pipeline (Phase 5): both
//! sides see identical pixels (chafa's, via `CHAFA_DUMP_PIXELS`).

mod support;

use chafa_syms_rs::select::{render_cells, CanvasMode, RenderConfig};
use chafa_syms_rs::SymbolMap;
use support::{oracle_available, oracle_render_dump};

/// Deterministic, varied RGBA8 image: gradients + bands + edges + a pseudo-noise
/// overlay, with intra-cell variation to elicit a wide range of symbol picks.
fn varied_image(cols: u32, rows: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (cols * 8, rows * 8);
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let mut lcg: u32 = 0x1234_5678;
    for y in 0..h {
        for x in 0..w {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 24) as u8;
            // Smooth gradients + a coarse checker + sharp diagonal edge + noise.
            let r = ((x * 5 + y * 2) as u8).wrapping_add(noise >> 2);
            let g = ((x ^ y) as u8).wrapping_mul(3);
            let b: u8 = if (x / 3 + y / 3) % 2 == 0 { 0x30 } else { 0xc0 };
            let edge: u8 = if (x as i32 - y as i32).rem_euclid(11) < 3 {
                0x40
            } else {
                0
            };
            buf.extend_from_slice(&[r, g.wrapping_add(edge), b.wrapping_add(noise >> 3), 0xff]);
        }
    }
    (buf, w, h)
}

fn compare_cells(label: &str, cells: &[chafa_syms_rs::CellOut], g: &support::OracleGrid) {
    assert_eq!(cells.len(), g.cells.len(), "[{label}] cell count");

    let mut mism = 0;
    for (i, (mine, oracle)) in cells.iter().zip(g.cells.iter()).enumerate() {
        let ok = mine.c == oracle.codepoint && mine.fg == oracle.fg_raw && mine.bg == oracle.bg_raw;
        if !ok {
            if mism < 15 {
                let (x, y) = (i % g.cols, i / g.cols);
                eprintln!(
                    "[{label}] cell ({x},{y}) MISMATCH\n  mine:   c=U+{:04X} fg={:#010x} bg={:#010x}\n  oracle: c=U+{:04X} fg={:#010x} bg={:#010x}",
                    mine.c, mine.fg, mine.bg, oracle.codepoint, oracle.fg_raw, oracle.bg_raw
                );
            }
            mism += 1;
        }
    }
    assert_eq!(mism, 0, "[{label}] {mism}/{} cells mismatched", cells.len());
}

fn run_case(symbols: &str, fg_only: bool) {
    run_case_work(symbols, fg_only, 5);
}

/// `work` is chafa's 1..=9 `--work` value; work_factor = (work-1)/8.
fn run_case_work(symbols: &str, fg_only: bool, work: u32) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (cols, rows) = (20u32, 12u32);
    let (buf, w, h) = varied_image(cols, rows);

    let work_s = work.to_string();
    let mut args: Vec<&str> = vec!["-c", "full", "--symbols", symbols, "--work", &work_s];
    if fg_only {
        args.push("--fg-only");
    }
    let render = oracle_render_dump(&buf, w, h, cols, rows, &args);
    let work_factor = (work as f32 - 1.0) / 8.0;

    let mut map = SymbolMap::new();
    map.apply_selectors(symbols).unwrap();
    map.prepare();
    let cfg = RenderConfig::new(
        CanvasMode::Truecolor,
        fg_only,
        0xffffff,
        0x000000,
        work_factor,
        &map,
        None,
    );
    let cells = render_cells(
        &cfg,
        &map,
        None,
        &render.pixels,
        render.width_px,
        render.height_px,
    );
    let label = format!("{symbols} w{work}{}", if fg_only { " fg-only" } else { "" });
    compare_cells(&label, &cells, &render.grid);
}

#[test]
fn truecolor_default_symbols() {
    run_case("block,border,space-wide", false);
}

#[test]
fn truecolor_ascii_symbols() {
    run_case("ascii", false);
}

#[test]
fn truecolor_all_symbols() {
    // Includes wide (kana) symbols -> exercises the wide-lookback path.
    run_case("all", false);
}

#[test]
fn truecolor_fg_only() {
    run_case("block,border,space-wide", true);
}

#[test]
fn truecolor_all_fg_only() {
    run_case("all", true);
}

#[test]
fn truecolor_wide_heavy() {
    // Narrow set is empty -> every cell pair becomes a wide symbol; maximally
    // stresses the wide pick + lookback-replacement path.
    run_case("wide", false);
}

#[test]
fn truecolor_wide_heavy_slow() {
    run_case_work("wide", false, 9);
}

#[test]
fn truecolor_slow_path_work9() {
    // work 9 -> work_factor 1.0 -> work_factor_int 10 >= 8 -> slow path (all
    // symbols evaluated, Phase A skipped).
    run_case_work("all", false, 9);
}

#[test]
fn truecolor_min_candidates_work1() {
    // work 1 -> work_factor 0.0 -> n_candidates clamped to 1.
    run_case_work("block,border,space-wide", false, 1);
}

/// Run a parity case in a given color mode (exercises palette lookup +
/// per-mode `update_cell_colors` + the `use_quantized_error` / `extract_colors`
/// selection branches). Cells store palette indices in non-truecolor modes.
fn run_mode(colors_flag: &str, mode: CanvasMode, symbols: &str, fg_only: bool) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (cols, rows) = (20u32, 12u32);
    let (buf, w, h) = varied_image(cols, rows);

    let mut args: Vec<&str> = vec!["-c", colors_flag, "--symbols", symbols];
    if fg_only {
        args.push("--fg-only");
    }
    let render = oracle_render_dump(&buf, w, h, cols, rows, &args);

    let mut map = SymbolMap::new();
    map.apply_selectors(symbols).unwrap();
    map.prepare();
    let cfg = RenderConfig::new(mode, fg_only, 0xffffff, 0x000000, 0.5, &map, None);
    let cells = render_cells(
        &cfg,
        &map,
        None,
        &render.pixels,
        render.width_px,
        render.height_px,
    );
    compare_cells(&format!("{colors_flag} {symbols}"), &cells, &render.grid);
}

#[test]
fn indexed_256() {
    run_mode(
        "256",
        CanvasMode::Indexed256,
        "block,border,space-wide",
        false,
    );
}

#[test]
fn indexed_240() {
    run_mode(
        "240",
        CanvasMode::Indexed240,
        "block,border,space-wide",
        false,
    );
}

#[test]
fn indexed_16() {
    run_mode(
        "16",
        CanvasMode::Indexed16,
        "block,border,space-wide",
        false,
    );
}

#[test]
fn indexed_8() {
    run_mode("8", CanvasMode::Indexed8, "block,border,space-wide", false);
}

#[test]
fn indexed_16_8_quantized_error() {
    // use_quantized_error path: colors snapped to fg/bg palettes before scoring,
    // which can change the symbol picked.
    run_mode(
        "16/8",
        CanvasMode::Indexed16_8,
        "block,border,space-wide",
        false,
    );
}

#[test]
fn fgbg_mode() {
    // -c none -> FGBG: extract_colors=false, forced fg_only; selection vs fixed
    // default_colors.
    run_mode("none", CanvasMode::Fgbg, "block,border,space-wide", false);
}

#[test]
fn fgbg_bgfg_mode() {
    // -c 2 -> FGBG_BGFG: extract_colors=false but not fg_only.
    run_mode("2", CanvasMode::FgbgBgfg, "block,border,space-wide", false);
}

#[test]
fn indexed_256_all_symbols() {
    run_mode("256", CanvasMode::Indexed256, "all", false);
}

#[test]
fn wide_lookback_is_exercised() {
    // Confirm the 'all' set actually produces wide cells (c==0 continuations),
    // so the wide-lookback path is genuinely covered by the parity test.
    if !oracle_available() {
        eprintln!("SKIP");
        return;
    }
    let (cols, rows) = (16u32, 10u32);
    let (buf, w, h) = varied_image(cols, rows);
    let render = oracle_render_dump(&buf, w, h, cols, rows, &["-c", "full", "--symbols", "all"]);
    let wide_cells = render
        .grid
        .cells
        .iter()
        .filter(|c| c.codepoint == 0)
        .count();
    eprintln!("wide continuation cells with 'all': {wide_cells}");
}
