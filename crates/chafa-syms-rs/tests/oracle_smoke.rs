//! Phase 0 smoke test: prove the differential-oracle harness works end-to-end
//! and that the PNG fixture round-trips pixel-exactly through chafa's decoder
//! (so the oracle and the Rust port see identical input pixels).

mod support;

use support::{oracle_available, oracle_render, unpack_aarrggbb};

fn solid_rgba(cols: u32, rows: u32, rgba: [u8; 4]) -> (Vec<u8>, u32, u32) {
    let (w, h) = (cols * 8, rows * 8);
    let mut buf = Vec::with_capacity((w * h * 4) as usize);
    for _ in 0..(w * h) {
        buf.extend_from_slice(&rgba);
    }
    (buf, w, h)
}

#[test]
fn harness_runs_and_dimensions_match() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found (set CHAFA_ORACLE_BIN)");
        return;
    }
    let (buf, w, h) = solid_rgba(3, 2, [0x30, 0x60, 0xa0, 0xff]);
    let grid = oracle_render(&buf, w, h, 3, 2, &["-c", "full"]);
    assert_eq!(grid.cols, 3);
    assert_eq!(grid.rows, 2);
    assert_eq!(grid.cells.len(), 6);
}

#[test]
fn flat_fixture_decodes_pixel_exact() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    // A solid, fully-opaque fill. An interior cell's background color (the
    // dominant/mean color) must equal the input exactly, proving chafa's PNG
    // decode introduces no color transform.
    let input = [0x30u8, 0x60, 0xa0, 0xff];
    let (buf, w, h) = solid_rgba(3, 2, input);
    let grid = oracle_render(&buf, w, h, 3, 2, &["-c", "full"]);

    // Every cell is featureless -> blank/space; background carries the color.
    for y in 0..grid.rows {
        for x in 0..grid.cols {
            let c = grid.at(x, y);
            let bg = unpack_aarrggbb(c.bg_raw);
            assert_eq!(
                [bg.ch[0], bg.ch[1], bg.ch[2]],
                [input[0], input[1], input[2]],
                "cell ({x},{y}) bg should equal input color exactly; got {bg:?}"
            );
        }
    }
}

#[test]
fn gradient_picks_are_reported() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    // Vertical 2-band gradient across 2x2 cells to elicit a half-block symbol.
    let (w, h) = (16u32, 16u32);
    let mut buf = Vec::new();
    for y in 0..h {
        let v = if y < 8 { 0x20 } else { 0xe0 };
        for _ in 0..w {
            buf.extend_from_slice(&[v, v, v, 0xff]);
        }
    }
    let grid = oracle_render(&buf, w, h, 2, 2, &["-c", "full"]);
    // Just assert we got plausible cells (non-panicking parse, valid chars).
    for c in &grid.cells {
        assert!(c.codepoint == 0 || char::from_u32(c.codepoint).is_some());
    }
    eprintln!(
        "gradient picks: {:?}",
        grid.cells
            .iter()
            .map(|c| c.ch().map(|ch| format!("U+{:04X}", ch as u32)))
            .collect::<Vec<_>>()
    );
}
