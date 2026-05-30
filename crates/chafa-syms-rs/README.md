# chafa-syms-rs

`chafa-syms-rs` is a pure-Rust port of the symbol-rendering core from
[chafa](https://hpjansson.org/chafa/). It turns raster pixels into a grid of
terminal character cells, choosing the Unicode symbol plus foreground and
background colors that best reconstruct each cell.

This crate is the library package. The companion CLI package is
`chafa-syms-cli`, which installs the `chafa-syms` binary.

## What It Does

- Selects narrow and wide Unicode symbols for each terminal cell.
- Extracts foreground and background colors per cell.
- Supports chafa-style symbol maps, including tag selectors, ranges,
  codepoints, and bracketed character sets.
- Renders ANSI/UTF-8 output with chafa-compatible attribute reuse.
- Supports truecolor, indexed, 16/8-color, and foreground/background modes.
- Resamples RGBA input to the target cell grid with a pure-Rust smolscale port.
- Runs deterministically across Rayon thread counts.

The library does not decode image files. Pass raw pixels from your image loader
of choice, or use the `chafa-syms` CLI if you want a ready-to-run image command.

## Install

```sh
cargo add chafa-syms-rs
```

## Basic Usage

```rust
use chafa_syms_rs::{Canvas, CanvasConfig, CanvasMode, PixelType};

let width = 320;
let height = 240;
let rgba: Vec<u8> = load_rgba_somehow();

let cfg = CanvasConfig::new(80, 24).mode(CanvasMode::Truecolor);
let mut canvas = Canvas::new(cfg);

canvas.draw_all_pixels(PixelType::Rgba8, &rgba, width, height, width * 4);

print!("{}", canvas.print());
```

`CanvasConfig::new(width, height)` uses terminal-cell dimensions, not pixel
dimensions. `draw_all_pixels` stretches the source image to `width * 8` by
`height * 8` pixels before selecting symbols.

## Pre-Sized Input

If you already have pixels sized exactly to the cell grid, use
`draw_rgba_presized` to skip resampling:

```rust
use chafa_syms_rs::{Canvas, CanvasConfig, Color};

let cfg = CanvasConfig::new(80, 24);
let mut canvas = Canvas::new(cfg);

let pixels: Vec<Color> = make_640_by_192_rgba_grid();
canvas.draw_rgba_presized(&pixels);

let ansi = canvas.print();
```

The pre-sized grid must contain `width * 8 * height * 8` pixels.

## Symbol Maps

The default symbol map matches chafa's library defaults. You can replace it
with selectors when you want a narrower character set:

```rust
use chafa_syms_rs::{CanvasConfig, SymbolMap};

let mut symbols = SymbolMap::new();
symbols.apply_selectors("ascii").unwrap();

let cfg = CanvasConfig::new(80, 24).symbol_map(symbols);
```

The selector grammar follows chafa's `--symbols` style: named tags, ranges,
individual codepoints, and bracketed character sets are supported.

## Parity

The port targets bit-exact parity with chafa 1.19.0 for the sRGB
symbol-rendering path. The repository validates the library against a patched
chafa oracle with differential tests for:

- builtin symbol data
- symbol-map selection
- per-cell symbol and color choice
- ANSI printer output
- smolscale resampling
- end-to-end image-to-ANSI rendering

One intentional difference is tie-breaking. Stock chafa can break equal-score
symbol ties through platform-dependent sort behavior; this crate uses a stable
deterministic order.

## Scope

Implemented:

- builtin chafa symbols, including generated Braille, Sextant, and Octant sets
- narrow and wide-symbol selection
- `RGBA8`, `BGRA8`, `ARGB8`, `ABGR8`, `RGB8`, and `BGR8` input pixels
- truecolor, indexed 256/240/16/8, 16/8, `fgbg`, and `fgbg-bgfg` modes
- ANSI/UTF-8 serialization
- pure stretch scaling to the cell grid

Not implemented:

- image-file decoding
- terminal probing
- placement, tuck, and alignment
- chafa CLI preprocessing
- dithering, dynamic palettes, sixel, Kitty, or iTerm2 graphics
- user-imported glyphs

## License

LGPL-3.0-only.
