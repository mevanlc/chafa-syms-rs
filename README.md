# chafa-syms-rs

The **symbols** functionality of [chafa](https://hpjansson.org/chafa/) extracted
to a pure-Rust library: fancy and tunable Unicode-symbol rendering of raster
graphics.

- **`chafa-syms-rs`** — the library (the main deliverable).
- **`chafa-syms`** — a thin demo/test CLI over it.

It ports chafa's *novel core*: turning a raster image into a grid of terminal
character cells, where each cell picks the Unicode **symbol** + **foreground/
background colors** that best reconstruct that cell's pixels, then serializes the
grid to ANSI/UTF-8.

## Parity

The port targets **bit-exact parity** with chafa 1.19.0 (sRGB color space). The
selection core is all-integer, so parity is exact rather than approximate. It is
validated against a patched chafa oracle (see [`devdocs/oracle`](devdocs/oracle))
by differential tests:

| Layer | Gate | Status |
|-------|------|--------|
| Builtin symbol set (1261 narrow + 181 wide) | every codepoint, tag, popcount, bitmap vs `chafa_symbols[]` | ✅ exact |
| Symbol map (selectors: tags, ranges, codepoints, `[...]` sets; dedup, sort) | compiled codepoints + order vs `symbol_map->symbols` | ✅ exact |
| Selection core (symbol + colors per cell) | `(char, fg, bg)` vs chafa's cells, all color modes × work levels × fg-only, fed chafa's exact pixels | ✅ exact |
| Printer (cells → ANSI) | byte-exact vs chafa's canonical output, all modes × `-O 0/5/6` | ✅ exact |
| Scaler (smolscale) | RGBA8 resample vs chafa's smolscale, every filter path × axis (incl. alpha) | ✅ exact |
| End-to-end (image → ANSI) | byte-exact vs chafa, incl. real `--stretch` scaling + transparency, all modes (`-p off`) | ✅ exact |

**Determinism note.** Stock chafa breaks equal-popcount symbol ties with an
unstable `qsort` over GLib hashtable order (platform-dependent, non-canonical).
This port uses a deterministic `(popcount, codepoint)` total order; the oracle is
patched to match. On a worst-case noise image, stock vs deterministic chafa
differ on ~3% of cells — all genuinely arbitrary ties. See
[`devdocs/oracle/README.md`](devdocs/oracle/README.md).

## What's in scope

- Per-cell symbol + color selection: narrow & wide (kana) symbols, candidate
  search, fg/bg extraction, wide-symbol lookback, blank normalization.
- chafa's predefined symbol sets + the full `--symbols` selector grammar,
  including the procedurally-generated Braille, Sextant and Octant families.
- All character-cell color modes: truecolor, indexed 256/240/16/8, 16/8,
  fgbg, fgbg-bgfg — with chafa's fixed palettes and nearest-color lookup.
- ANSI/UTF-8 output with the `REUSE_ATTRIBUTES` optimization, aixterm 16-color
  pen math, and 16/8 bold-for-bright.
- Six input pixel formats: RGBA8 / BGRA8 / ARGB8 / ABGR8 / RGB8 / BGR8.
- Bit-exact **smolscale** resampling: a faithful port of chafa's scalar
  resampler (gamma-correct, premultiplied linear-light box/bilinear), as a pure
  stretch to the cell grid.
- `rayon` multithreading (deterministic regardless of thread count).

## Not ported (out of scope or best-effort)

- **Placement / tuck / align:** the scaler is a pure *stretch* (resize to the
  cell grid), matching chafa `--stretch`. chafa's CLI-default FIT-and-center
  placement (and the AVX2 fast paths / CPU-feature multiversioning) are not
  modeled; the underlying resampling math is bit-exact regardless.
- **Preprocessing:** chafa's `normalize_rgb`/saturation preprocessing (applied
  to 16/8/fgbg modes) is not ported; the library behaves as `--preprocess off`.
- **User-imported glyphs:** the selector engine's RTL / zero-width /
  non-printable exclusion branches exist for chafa's user-glyph import, which is
  out of scope here. They are inert for the builtin glyph set (no builtin is
  RTL/combining/non-printable), so builtin selection is exact; per-codepoint
  membership of arbitrary *imported* glyphs is not validated.
- DIN99d / non-sRGB color spaces; dithering; PCA / dynamic palettes; sixel /
  kitty / iterm2 graphics; image *file* decoding in the library; tty probes.

## Library usage

```rust
use chafa_syms_rs::{Canvas, CanvasConfig, CanvasMode, PixelType};

let cfg = CanvasConfig::new(80, 24)        // cells
    .mode(CanvasMode::Truecolor);
let mut canvas = Canvas::new(cfg);
canvas.draw_all_pixels(PixelType::Rgba8, &rgba, width, height, width * 4);
print!("{}", canvas.print());
```

`draw_all_pixels` resamples (bit-exactly, as a stretch) to the cell grid. To
skip resampling entirely, pre-size the image to `width*8 × height*8` and use
`Canvas::draw_rgba_presized`.

## CLI usage

```sh
chafa-syms image.png                       # fit to terminal, truecolor
chafa-syms --size 80x24 -c 256 image.png   # 256-color, fixed size
chafa-syms --symbols ascii -c none in.png  # old-school ASCII art
chafa-syms --fg-only --symbols all pic.png
chafa-syms --symbols help                  # selector grammar and named sets
```

Flags: `--size --scale --colors --fg --bg --work --threads --symbols
--fg-only --invert --font-ratio --optimize --format`.

## Layout

```
crates/chafa-syms-rs/   the library
  src/color.rs          colors + squared-Euclidean diff
  src/geometry.rs       cell constants + MSB-first coverage<->bitmap
  src/symbol/           tags, builtin data (generated), generators, derivation
  src/symbol_map.rs     selectors, char_is_selected, candidate search
  src/work_cell.rs      counting sort, contrasting pair, mean extraction
  src/select.rs         the selection core (the novel part)
  src/palette.rs        fixed palettes + nearest lookup
  src/printer.rs        ANSI/UTF-8 serialization
  src/pixops.rs         input formats + alpha composite
  src/smolscale.rs      bit-exact smolscale resampler (+ smolscale_luts.rs)
  src/canvas.rs         high-level Canvas/CanvasConfig API
crates/chafa-syms-cli/  the CLI (bin name: chafa-syms)
tools/transcode-symbols/ one-shot codegen: chafa headers -> symbol/data.rs
devdocs/                PLAN.md + oracle patch & docs
```

## Building & testing

```sh
cargo build --release
cargo test            # differential tests skip gracefully if the oracle is absent
```

The differential tests need the patched chafa oracle built once; see
[`devdocs/oracle/README.md`](devdocs/oracle/README.md).

## License

LGPL-3.0-only. See [`COPYING`](COPYING), [`COPYING.LESSER`](COPYING.LESSER),
and [`NOTICE`](NOTICE).
