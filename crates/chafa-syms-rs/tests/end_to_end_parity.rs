//! Capstone end-to-end parity: the high-level `Canvas` API (format conversion
//! -> identity scale -> composite -> selection -> printer) must reproduce
//! chafa's full canonical ANSI byte-for-byte, for pre-sized opaque inputs.
//!
//! Using a `cols*8 x rows*8` input makes the scaler an identity copy on both
//! sides (chafa via `--stretch`, us via the equal-size fast path), so this
//! exercises the whole pipeline *except* resampling (best-effort, D2).

mod support;

use chafa_syms_rs::canvas::{Canvas, CanvasConfig};
use chafa_syms_rs::printer::Optimizations;
use chafa_syms_rs::select::CanvasMode;
use chafa_syms_rs::{PixelType, SymbolMap};
use support::{oracle_available, oracle_render_dump};

fn varied_image(cols: u32, rows: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (cols * 8, rows * 8);
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let mut lcg: u32 = 0x0bad_f00d;
    for y in 0..h {
        for x in 0..w {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (lcg >> 24) as u8;
            let r = ((x * 4 + y * 3) as u8).wrapping_add(n >> 2);
            let g = ((x ^ y) as u8).wrapping_mul(7);
            let b: u8 = if (x / 5 + y / 3) % 2 == 0 { 0x28 } else { 0xb8 };
            buf.extend_from_slice(&[r, g, b.wrapping_add(n >> 3), 0xff]);
        }
    }
    (buf, w, h)
}

fn run(colors_flag: &str, mode: CanvasMode, symbols: &str) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (cols, rows) = (24u32, 14u32);
    let (buf, w, h) = varied_image(cols, rows);
    // -O 5 -> REUSE_ATTRIBUTES (REPEAT is inert with the fallback term).
    let render = oracle_render_dump(
        &buf,
        w,
        h,
        cols,
        rows,
        5,
        &["-c", colors_flag, "--symbols", symbols, "-p", "off"],
    );

    let mut map = SymbolMap::new();
    map.apply_selectors(symbols).unwrap();
    let cfg = CanvasConfig::new(cols as usize, rows as usize)
        .mode(mode)
        .work_factor(0.5)
        .preprocessing(false)
        .optimizations(Optimizations::REUSE_ATTRIBUTES)
        .symbol_map(map);
    let mut canvas = Canvas::new(cfg);
    canvas.draw_all_pixels(
        PixelType::Rgba8,
        &buf,
        w as usize,
        h as usize,
        w as usize * 4,
    );
    let ansi = canvas.print();

    assert_eq!(
        ansi.as_bytes(),
        render.ansi.as_slice(),
        "[{colors_flag} {symbols}] end-to-end ANSI differs from chafa"
    );
}

#[test]
fn e2e_truecolor() {
    run("full", CanvasMode::Truecolor, "block,border,space-wide");
}

#[test]
fn e2e_truecolor_all() {
    run("full", CanvasMode::Truecolor, "all");
}

#[test]
fn e2e_indexed_256() {
    run("256", CanvasMode::Indexed256, "block,border,space-wide");
}

#[test]
fn e2e_indexed_16() {
    run("16", CanvasMode::Indexed16, "block,border,space-wide");
}

#[test]
fn e2e_fgbg() {
    run("none", CanvasMode::Fgbg, "block,border,space-wide");
}

#[test]
fn e2e_fgbg_preprocessing() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (cols, rows) = (24u32, 14u32);
    let (buf, w, h) = varied_image(cols, rows);
    let symbols = "half,stipple";
    let render = oracle_render_dump(
        &buf,
        w,
        h,
        cols,
        rows,
        5,
        &["-c", "none", "--symbols", symbols, "-p", "on"],
    );

    let mut map = SymbolMap::new();
    map.apply_selectors(symbols).unwrap();
    let cfg = CanvasConfig::new(cols as usize, rows as usize)
        .mode(CanvasMode::Fgbg)
        .work_factor(0.5)
        .optimizations(Optimizations::REUSE_ATTRIBUTES)
        .symbol_map(map);
    let mut canvas = Canvas::new(cfg);
    canvas.draw_all_pixels(
        PixelType::Rgba8,
        &buf,
        w as usize,
        h as usize,
        w as usize * 4,
    );

    assert_eq!(
        canvas.print().as_bytes(),
        render.ansi.as_slice(),
        "[none {symbols} preprocess] end-to-end ANSI differs from chafa"
    );
}
