//! Phase 7: output must be byte-identical regardless of thread count.

use chafa_syms_rs::select::{render_cells, CanvasMode, RenderConfig};
use chafa_syms_rs::{print_cells, Color, Optimizations, SymbolMap};

fn pixels(cols: usize, rows: usize) -> (Vec<Color>, usize, usize) {
    let (w, h) = (cols * 8, rows * 8);
    let mut lcg: u32 = 0xc0ffee;
    let mut px = Vec::with_capacity(w * h);
    for _ in 0..(w * h) {
        lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let n = lcg.to_le_bytes();
        px.push(Color::new(n[0], n[1], n[2], 0xff));
    }
    (px, w, h)
}

fn render_with_threads(n: usize, px: &[Color], w: usize, h: usize) -> String {
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(n)
        .build()
        .unwrap();
    pool.install(|| {
        let mut map = SymbolMap::chafa_default();
        map.prepare();
        let cfg = RenderConfig::new(
            CanvasMode::Truecolor,
            false,
            0xffffff,
            0x000000,
            0.5,
            &map,
            None,
        );
        let cells = render_cells(&cfg, &map, None, px, w, h);
        print_cells(
            &cfg,
            &map,
            &cells,
            w / 8,
            h / 8,
            Optimizations::REUSE_ATTRIBUTES,
        )
    })
}

#[test]
fn output_is_thread_count_independent() {
    let (px, w, h) = pixels(40, 24);
    let a = render_with_threads(1, &px, w, h);
    let b = render_with_threads(4, &px, w, h);
    let c = render_with_threads(8, &px, w, h);
    assert_eq!(a, b, "1 vs 4 threads");
    assert_eq!(a, c, "1 vs 8 threads");
    assert!(!a.is_empty());
}
