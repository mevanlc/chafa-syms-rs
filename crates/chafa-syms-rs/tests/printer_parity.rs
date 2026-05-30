//! Phase 6 printer gate: **cells -> ANSI** byte-exact parity.
//!
//! Feeds chafa's exact post-prep pixels through the (proven) Rust selection
//! core to produce cells, serializes them with the Rust printer, and compares
//! the bytes to chafa's canonical canvas ANSI (`CHAFA_DUMP_ANSI`, no CLI
//! framing). This is *cells->ANSI* parity; *image->ANSI* awaits the Phase 5
//! scaler (best-effort per D2).
//!
//! Covered: all color modes x optimization levels -O 0 (none), 5 (REUSE), 6
//! (REUSE+REPEAT). -O>=7 (SKIP_CELLS) is out of scope here.

mod support;

use chafa_syms_rs::printer::{print_cells, Optimizations};
use chafa_syms_rs::select::{render_cells, CanvasMode, RenderConfig};
use chafa_syms_rs::SymbolMap;
use support::{oracle_available, oracle_render_dump};

fn varied_image(cols: u32, rows: u32) -> (Vec<u8>, u32, u32) {
    let (w, h) = (cols * 8, rows * 8);
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    let mut lcg: u32 = 0x9e37_79b9;
    for y in 0..h {
        for x in 0..w {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let noise = (lcg >> 24) as u8;
            let r = ((x * 3 + y * 7) as u8).wrapping_add(noise >> 2);
            let g = ((x ^ (y * 2)) as u8).wrapping_mul(5);
            let b: u8 = if (x / 4 + y / 2) % 2 == 0 { 0x20 } else { 0xd0 };
            buf.extend_from_slice(&[r, g, b.wrapping_add(noise >> 3), 0xff]);
        }
    }
    (buf, w, h)
}

fn opts_for(level: i32) -> Optimizations {
    let mut o = Optimizations::empty();
    if level >= 1 {
        o |= Optimizations::REUSE_ATTRIBUTES;
    }
    if level >= 6 {
        o |= Optimizations::REPEAT_CELLS;
    }
    o
}

fn show(bytes: &[u8]) -> String {
    let mut s = String::new();
    for &b in bytes {
        match b {
            0x1b => s.push_str("\\e"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(b as char),
            _ => s.push_str(&format!("\\x{b:02x}")),
        }
    }
    s
}

fn run(colors_flag: &str, mode: CanvasMode, symbols: &str, fg_only: bool, opt: i32) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (cols, rows) = (16u32, 8u32);
    let (buf, w, h) = varied_image(cols, rows);

    let mut args: Vec<&str> = vec!["-c", colors_flag, "--symbols", symbols];
    if fg_only {
        args.push("--fg-only");
    }
    let render = oracle_render_dump(&buf, w, h, cols, rows, opt, &args);

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
    let ansi = print_cells(
        &cfg,
        &map,
        &cells,
        render.width_px / 8,
        render.height_px / 8,
        opts_for(opt),
    );

    let label = format!(
        "{colors_flag} {symbols}{} -O{opt}",
        if fg_only { " fg-only" } else { "" }
    );
    if ansi.as_bytes() != render.ansi.as_slice() {
        // Find first divergence.
        let mine = ansi.as_bytes();
        let theirs = render.ansi.as_slice();
        let at = mine
            .iter()
            .zip(theirs.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(mine.len().min(theirs.len()));
        let lo = at.saturating_sub(20);
        eprintln!(
            "[{label}] ANSI MISMATCH at byte {at} (mine {} bytes, oracle {} bytes)\n  mine:   ...{}\n  oracle: ...{}",
            mine.len(),
            theirs.len(),
            show(&mine[lo..(at + 30).min(mine.len())]),
            show(&theirs[lo..(at + 30).min(theirs.len())]),
        );
        panic!("[{label}] printer output differs from chafa");
    }
}

macro_rules! mode_tests {
    ($($name:ident => ($flag:expr, $mode:expr, $syms:expr, $fg:expr);)*) => {
        $(
            #[test]
            fn $name() {
                for opt in [0, 5, 6] {
                    run($flag, $mode, $syms, $fg, opt);
                }
            }
        )*
    };
}

mode_tests! {
    truecolor       => ("full", CanvasMode::Truecolor,  "block,border,space-wide", false);
    truecolor_all   => ("full", CanvasMode::Truecolor,  "all",                     false);
    truecolor_fgonly=> ("full", CanvasMode::Truecolor,  "block,border,space-wide", true);
    indexed_256     => ("256",  CanvasMode::Indexed256,  "block,border,space-wide", false);
    indexed_240     => ("240",  CanvasMode::Indexed240,  "block,border,space-wide", false);
    indexed_16      => ("16",   CanvasMode::Indexed16,   "block,border,space-wide", false);
    indexed_8       => ("8",    CanvasMode::Indexed8,    "block,border,space-wide", false);
    indexed_16_8    => ("16/8", CanvasMode::Indexed16_8, "block,border,space-wide", false);
    fgbg            => ("none", CanvasMode::Fgbg,        "block,border,space-wide", false);
    fgbg_bgfg       => ("2",    CanvasMode::FgbgBgfg,    "block,border,space-wide", false);
}
