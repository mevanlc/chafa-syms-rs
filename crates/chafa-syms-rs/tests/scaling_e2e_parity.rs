//! End-to-end parity *through real scaling*: feed an arbitrary-sized image and
//! require the full `Canvas` pipeline (format → smolscale resample → composite
//! → selection → printer) to reproduce chafa's canonical ANSI byte-for-byte.
//!
//! Unlike `end_to_end_parity.rs` (which pre-sizes the input so the scaler is an
//! identity copy), this exercises chafa's **smolscale** resampler via the real
//! canvas pipeline (`smol_scale_new_full` + `SRC_CLEAR_DEST`, full placement),
//! confirming the port matches it end-to-end. We use opaque, high-contrast
//! imagery downscaled hard — the regime where gamma-correct linear-light
//! averaging diverges most from naive sRGB resampling.

mod support;

use chafa_syms_rs::canvas::{Canvas, CanvasConfig};
use chafa_syms_rs::printer::Optimizations;
use chafa_syms_rs::select::CanvasMode;
use chafa_syms_rs::{PixelType, SymbolMap};
use support::{oracle_available, oracle_render_dump};

/// Opaque, high-contrast, structured image (checkerboard + gradients) at an
/// arbitrary size — forces real resampling on both axes.
fn hi_contrast_image(w: u32, h: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let mut lcg: u32 = 0x1234_5678;
    for y in 0..h {
        for x in 0..w {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (lcg >> 24) as u8;
            let checker = if (x / 3 + y / 3) % 2 == 0 { 0xff } else { 0x00 };
            let r = checker;
            let g = ((x * 7 + y * 3) as u8) ^ (n & 0x40);
            let b = if (x + y) % 2 == 0 { 0x20 } else { 0xd0 };
            buf.extend_from_slice(&[r, g, b, 0xff]);
        }
    }
    buf
}

fn run(w: u32, h: u32, cols: u32, rows: u32, colors_flag: &str, mode: CanvasMode, symbols: &str) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let buf = hi_contrast_image(w, h);
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
        "[{w}x{h} -> {cols}x{rows} {colors_flag}] scaled end-to-end ANSI differs from chafa"
    );
}

#[test]
fn scale_downscale_truecolor() {
    // 150x90 -> 24x14 cells (192x112 px): a real reduction on both axes.
    run(
        150,
        90,
        24,
        14,
        "full",
        CanvasMode::Truecolor,
        "block,border,space-wide",
    );
}

#[test]
fn scale_upscale_truecolor() {
    // 20x12 -> 24x14 cells (192x112 px): magnification on both axes.
    run(20, 12, 24, 14, "full", CanvasMode::Truecolor, "all");
}

#[test]
fn scale_mixed_256() {
    // Wide + short source: downscale width hard, upscale height.
    run(
        300,
        7,
        20,
        10,
        "256",
        CanvasMode::Indexed256,
        "block,border,space-wide",
    );
}

#[test]
fn scale_downscale_indexed_16() {
    run(
        111,
        83,
        16,
        9,
        "16",
        CanvasMode::Indexed16,
        "block,border,space-wide",
    );
}
