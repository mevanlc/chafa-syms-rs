# chafa-syms-rs — Port Plan

> A pure-Rust port of the **symbol-rendering core** of [chafa](https://hpjansson.org/chafa/):
> fancy, tunable Unicode symbol rendering of raster graphics.

- **`chafa-syms-rs`** — the library (the main course).
- **`chafa-syms`** — a thin demo/test CLI over the library.

Source of truth for the port: the chafa C source at `~/p/gh/chafa/` (version **1.19.0**,
per `configure.ac`). A homebrew `chafa` **1.18.2** binary is also on `PATH`; the symbol
core is stable across these, but for precise differential testing we build 1.19.0 from
source (see §10). All `path:line` references below point into `~/p/gh/chafa/`.

---

## 1. Goal & fidelity target

Port the "novel part" of chafa: turning a raster image into a grid of terminal character
cells, where each cell picks the Unicode **symbol** + **foreground/background colors** that
best reconstruct that cell's pixels, then serialize the grid to ANSI/UTF-8.

**Fidelity bar (decided): _core numerical parity_.** Given an *identical input pixel grid*
(`cols*8 × rows*8`), the chosen symbol and colors per cell must match chafa bit-for-bit.
The scaler is ported too (see §2 decisions) and should get us close to end-to-end parity,
but exactness through the scaling stage is best-effort, not the validation gate.

This split is deliberate and load-bearing: **the selection core depends only on
color + symbol-map + work-cell — _not_ on the scaler** — so it can be built and validated
first, in isolation, by feeding pre-sized images (§10 tier A).

**Why bit-exact core parity is achievable (not aspirational):** dropping DIN99d (D1)
removed the only transcendental/float math from the *matching* path. The gated core is now
**all-integer** — counting sort on `u8` channels, integer squared-distance, `u64` Hamming,
integer mean extraction. There is no float reproducibility hazard, so the core is
platform-independent and can match chafa exactly rather than approximately.

"Idiomatic Rust over 1:1 API replication" applies to **API shape**, not numerical output.
Those are separate axes: the public API may diverge from the C API freely; the per-cell
math must not.

---

## 2. Resolved decisions

These were genuine ambiguities/contradictions in the brief; resolved up front so they don't
get re-litigated mid-port.

| # | Decision | Resolution | Consequence |
|---|----------|------------|-------------|
| D1 | **Color metric / colorspace** | **sRGB only.** No DIN99d. | Matches chafa's *default* (`--color-space` defaults to `rgb`). The color module is much simpler: skip the RGB→DIN99d transform entirely, and with it the two nastiest parity landmines (the nonstandard `1.044` sRGB compand divisor and the truncating-non-clamping `double→u8` wrap). `color_diff` is plain squared-Euclidean over RGB channels 0–2. |
| D2 | **Fidelity bar** | **Core parity** (see §1). | Validation gate = cell-by-cell match on pre-sized input. Scaler exactness is best-effort. |
| D3 | **Scaler** | **Port smolscale to Rust.** | Pure-Rust requirement satisfied without an external resampler; also buys us *near* end-to-end parity as a bonus. Includes smolscale's filter selection + its reversible sRGB LUTs. Phased so the core lands first (§9, Phase 5 is after the core milestone). |
| D4 | **Threading** | **`rayon`** (pure Rust, idiomatic). | Maps chafa's row-batch model to parallel iterators. `--threads` sizes the pool; auto = logical CPUs capped at 24 (matches `AUTO_THREAD_COUNT_MAX`). |
| D5 | **Terminal detection** | **None in the lib.** CLI may read window size via the `terminal_size` crate (passive `TIOCGWINSZ` ioctl, *not* a call/response tty probe) with a fixed `80×24` fallback. Default canvas mode = `Truecolor` (the lib config default); no auto-detection of "best" mode. | Honors "no tty probes." `--size`/`--scale` still work; auto-fit uses the ioctl-or-default view size. |
| D6 | **Image decoding** | **Lib is decode-agnostic** (takes raw buffers in the 6 in-scope formats). CLI uses the pure-Rust `image` crate to decode files → RGBA8 → lib. | Keeps the lib boundary clean and dependency-light. |
| D7 | **Input formats** | The **six 8-bit integer formats** (RGBA8/BGRA8/ARGB8/ABGR8/RGB8/BGR8) are the concrete deliverable. float32 ingest is optional sugar (convert→RGBA8 at the boundary), not core. | These map 1:1 to `ChafaPixelType`/`SmolPixelType` and the u8 internal pipeline. |

**Scope-closure note (verified):** every in-scope `--colors` mode
(`none/2/8/16/16-8/240/256/full`) resolves to a **fixed palette or direct RGB** —
`INDEXED_256 → FIXED_256` in symbol mode, etc. (`chafa-canvas.c:237-273`). None needs the
PCA/kd-tree `DYNAMIC_256` generator. So "PCA out of scope" is fully consistent with all
color modes being in scope. `ChafaColorTable` (the dynamic-palette accelerator) is **not
ported**.

---

## 3. Scope

### In scope
- Per-cell symbol + color selection (the novel core): narrow & wide symbols, candidate
  search, fg/bg extraction, fill fallback, blank normalization, wide-symbol lookback.
- chafa's predefined symbol sets + the full `--symbols` selector grammar.
- Procedurally-generated symbol families: Braille, Sextant, Octant.
- Color modes: `Truecolor`, `Indexed256`, `Indexed240`, `Indexed16`, `Indexed8`,
  `Indexed16_8`, `FgbgBgfg`, `Fgbg`. Fixed ANSI/256 palettes + nearest-color lookup.
- Pixel pipeline: the 6 input formats, alpha-over-background compositing, scaling
  (smolscale port), two-pass prep.
- Geometry: `--scale`, `--size`, font-ratio aspect correction.
- ANSI/UTF-8 output (SGR sequences) for all in-scope modes, incl. the `--optimize`
  reuse/repeat optimizations.
- CLI flags: `--scale`, `--size`, `--bg`, `--colors`, `--fg`, `--threads`, `--work`,
  `--symbols`, `--format=symbols` (+ `--invert`, `--font-ratio`, `--fg-only`, `--optimize`
  as natural companions).

### Out of scope
- sixel / kitty / iterm2 pixel graphics protocols.
- Image *file* decoding in the lib (CLI only, via `image`).
- DIN99d / any non-sRGB colorspace (D1).
- Dithering (Floyd-Steinberg, ordered, noise).
- PCA, dynamic palette generation, `ChafaColorTable`.
- Reading from stdin/pty/tty; terminal call/response probes.
- Non-pure-Rust dependencies.
- x86 SIMD paths (AVX2/SSE4.1/MMX). We port the **scalar** kernels only; Rust autovec +
  `u64::count_ones()` cover the gap.
- The unused `chafa-symbols-ascii-ibm.h` alternate set (present in tree, not `#include`d).

---

## 4. Crate layout

A Cargo workspace keeps the lib dependency-light while letting the CLI pull in `image`/`clap`:

```
chafa-syms-rs/                  (repo root)
├── Cargo.toml                  # [workspace]
├── crates/
│   ├── chafa-syms-rs/          # the library  (lib name: chafa_syms_rs)
│   │   ├── Cargo.toml          # deps: rayon, unicode-width, unicode-properties (see §11)
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── color.rs        # ChafaColor, color_diff (Phase 1)
│   │       ├── symbol/         # symbol struct, tags, builtin data, generators (Phase 2)
│   │       │   ├── mod.rs
│   │       │   ├── tags.rs
│   │       │   ├── data.rs     # GENERATED (committed) — see tools/transcode-symbols
│   │       │   └── generated/  # braille/sextant/octant algorithms (Phase 2)
│   │       ├── symbol_map.rs   # selectors, compile, candidate search (Phase 3)
│   │       ├── work_cell.rs    # per-cell scratch math (Phase 4)
│   │       ├── select.rs       # the selection core / renderer (Phase 4)  ← NOVEL PART
│   │       ├── pixops/         # input formats, alpha, two-pass prep (Phase 5)
│   │       ├── smolscale/      # ported scaler (Phase 5)
│   │       ├── palette.rs      # fixed palettes + nearest lookup (Phase 6)
│   │       ├── canvas.rs       # Canvas + CanvasConfig + modes (Phase 6)
│   │       └── printer.rs      # ANSI/UTF-8 serialization (Phase 6)
│   └── chafa-syms/             # the CLI (bin name: chafa-syms)
│       ├── Cargo.toml          # deps: chafa-syms-rs, clap, image, terminal_size, anstyle
│       └── src/main.rs
├── tools/
│   └── transcode-symbols/      # one-shot generator: parses chafa-symbols-*.h → data.rs
├── tests/                      # integration + differential (oracle) tests
└── devdocs/
    └── PLAN.md
```

**Symbol-data codegen:** `tools/transcode-symbols` is a standalone binary (run manually,
output committed to `symbol/data.rs`). It parses the `CHAFA_SYMBOL_OUTLINE_8X8/16X8(...)`
string-literal tables out of `~/p/gh/chafa/chafa/internal/chafa-symbols-{ascii,latin,block,
kana,misc-narrow}.h` (the 5 `#include`d in `chafa-symbols.c:152-156`) into Rust arrays.
This keeps the lib build pure (no C parsing at build time) and the data auditable in git.
Braille/Sextant/Octant are **not** data — they port as runtime generator functions (§ Phase 2).

---

## 5. Geometry & key constants

From `chafa-symbol-map.h:29-30` / `chafa-private.h`:

```
SYMBOL_WIDTH_PIXELS  = 8
SYMBOL_HEIGHT_PIXELS = 8
SYMBOL_N_PIXELS      = 64        // one symbol = 64 bits = one u64
N_CANDIDATES_MAX     = 8
SYMBOL_ERROR_MAX     = i32::MAX / 8
AUTO_THREAD_COUNT_MAX= 24
```

- Canvas pixel grid: `width_pixels = cols*8`, `height_pixels = rows*8`
  (`chafa-canvas.c:539-540`).
- **Bitmap bit-order (load-bearing):** pixel `(x,y)` occupies bit `63 - (y*8 + x)`.
  Bit 63 = top-left, bit 0 = bottom-right, row-major, MSB-first
  (`coverage_to_bitmap`, `chafa-symbols.c:217-234`). The coverage array is the inverse:
  `coverage[i] = (bitmap >> (63-i)) & 1`, and `coverage[i] ∈ {0,1}` is used **directly** as
  an index into `color_pair.colors[]` (because `COLOR_PAIR_BG=0`, `COLOR_PAIR_FG=1`).
- Wide symbol = two side-by-side 8×8 cells (16×8), stored as `[left, right]`, matched over
  128 bits.

---

## 6. Core data model (sketch)

Idiomatic Rust; field semantics mirror chafa exactly.

```rust
// color.rs
#[derive(Clone, Copy)] pub struct Color { pub ch: [u8; 4] }   // R,G,B,A
#[derive(Clone, Copy)] pub struct ColorPair { pub colors: [Color; 2] } // [BG, FG]
#[inline] pub fn color_diff(a: Color, b: Color) -> i32 {       // squared-Euclidean, ch 0..2
    let d0 = a.ch[0] as i32 - b.ch[0] as i32;
    let d1 = a.ch[1] as i32 - b.ch[1] as i32;
    let d2 = a.ch[2] as i32 - b.ch[2] as i32;
    d0*d0 + d1*d1 + d2*d2
}

// symbol/mod.rs
pub struct Symbol {
    pub tags: SymbolTags,
    pub c: char,                // Unicode scalar (0 = sentinel in C; use Option/explicit end)
    pub coverage: [u8; 64],     // 0/1 per pixel, row-major
    pub bitmap: u64,            // packed, MSB = top-left
    pub popcount: u32,
    pub fg_weight: u32,         // == popcount
    pub bg_weight: u32,         // == 64 - popcount
}
pub struct WideSymbol { pub sym: [Symbol; 2] }  // [left, right]

// symbol_map.rs
pub struct SymbolMap {
    selectors: Vec<Selector>,           // ordered; later overrides earlier
    use_builtin_glyphs: bool,
    // compiled (sorted ascending by popcount):
    symbols: Vec<Symbol>,
    packed_bitmaps: Vec<u64>,           // parallel to symbols, for the Hamming sweep
    symbols_wide: Vec<WideSymbol>,
    packed_bitmaps_wide: Vec<u64>,      // interleaved [l0,r0, l1,r1, ...]
    dirty: bool,
}

// work_cell.rs  (transient per-cell scratch; a ring of N_BUF_CELLS=4 for wide lookback)
pub struct WorkCell {
    pixels: [Pixel; 64],
    sorted_index: [[u8; 64]; 4],        // per-channel counting-sort, lazy
    have_sorted: [bool; 4],
    dominant_channel: i32,              // -1 until computed
}

pub struct Candidate { pub symbol_index: u32, pub hamming_distance: u8, pub is_inverted: bool }
```

Public API surface (chafa-tracking but idiomatic — builder-style config, `Result` errors):

```rust
pub enum PixelType { Rgba8, Bgra8, Argb8, Abgr8, Rgb8, Bgr8 }   // the 6 in scope
pub enum CanvasMode { Truecolor, Indexed256, Indexed240, Indexed16,
                      Indexed8, Indexed16_8, FgbgBgfg, Fgbg }

pub struct CanvasConfig { /* width, height (cells), cell_w/h, mode, fg/bg, alpha_threshold,
                             work_factor (0.0..=1.0), fg_only, optimizations,
                             symbol_map, fill_symbol_map */ }
impl CanvasConfig { /* set_geometry, set_canvas_mode, set_fg_color, set_bg_color,
                       set_work_factor, set_symbol_map, ... (builder/setters) */ }

pub struct Canvas { /* cells, pixel grid, palettes, config */ }
impl Canvas {
    pub fn new(cfg: &CanvasConfig) -> Self;
    pub fn draw_all_pixels(&mut self, t: PixelType, data: &[u8],
                           w: usize, h: usize, rowstride: usize) -> Result<(), Error>;
    pub fn print(&self) -> String;     // ANSI/UTF-8
}
```

---

## 7. The selection core (the novel part) — algorithm reference

This is the heart of the port (`chafa-symbol-renderer.c` + `chafa-work-cell.c`). Two-phase
per narrow cell:

**Phase A — shortlist by shape (fast path).** Pick the cell's two contrasting colors, threshold
the 64 pixels into a `u64` bitmap, then Hamming-rank all symbol bitmaps to get ≤N candidates.

**Phase B — choose by color error.** For each candidate, extract its fg/bg from the cell under
its coverage mask, then sum squared per-pixel color error. Lowest wins.

Per-cell sequence (`update_cells_row` @ `chafa-symbol-renderer.c:797`):
1. `WorkCell::init` — copy the cell's 8×8 pixel block from the big grid.
2. `update_cell`:
   - `work_factor_int = (work_factor*10 + 0.5)` → if `>= 8` use **slow** path (evaluate
     *all* symbols by color error, skip Phase A); else **fast** path
     (`n_candidates = clamp(work_factor_int, 1, 8)`).
   - Fast path: contrasting pair (`work_cell_get_contrasting_color_pair`,
     `chafa-work-cell.c:293` — widest-range channel via counting sort, darkest pixel = BG,
     brightest = FG) → `work_cell_to_bitmap` (`:121`, per pixel set bit if closer to FG) →
     `find_candidates` (Hamming sweep + 8-slot sorted insertion, optional inverse) →
     `eval_symbol` over the shortlist.
   - `eval_symbol` (`:187`): extract per-symbol colors (AVERAGE extractor —
     mean of covered pixels = FG, mean of uncovered = BG; `extract_cell_mean_colors_plain`
     `chafa-work-cell.c:59`) then `calc_cell_error_plain` (`:98`: sum `color_diff(colors[cov[i]],
     pixel[i])` over 64). Keep running min.
   - `update_cell_colors` — quantize fg/bg per mode (§ Phase 6).
3. **Wide lookback** (`update_cells_wide`): if prev cell non-empty, evaluate the best
   double-width symbol spanning `(cx-1, cx)`; if `err[left]+err[right]` beats the two narrow
   cells, replace both. Ring buffer of 4 work cells.
4. **Fill fallback** (`apply_fill` `:641`): if the cell is featureless (`' '`, solid block, or
   fg==bg), pick a single shade/gradient symbol from `fill_symbol_map` by interpolating the
   cell mean color between two palette colors over 65 steps and matching popcount. Honors
   `is_inverted` (swaps pens). Inert when the fill map is empty (the default).
5. **Blank normalization:** still-featureless → `blank_char`; ASCII space inherits prev fg to
   reduce escape churn.

**Mode-dependent gates** (set in `chafa-canvas.c:561-572`) — these are the *only* places the
shape selection branches by color mode:
- `consider_inverted = !(fg_only || mode==Fgbg)` — whether Phase A also tries each symbol's
  negative (`hd' = 64 - hd`). The candidate's `is_inverted` flag is *dropped* in the symbol
  path (only widens the shortlist); it is used only in `apply_fill`.
- `extract_colors = !(mode==Fgbg || mode==FgbgBgfg)` — else colors = fixed `default_colors`.
- `use_quantized_error = (mode==Indexed16_8 && !fg_only)` — Phase B error computed against
  palette-snapped colors so the pick accounts for the limited palette.

**fg-only:** matches against `default_colors` (with `default_colors[FG]` forced to 50% gray
`0x7f7f7f` as an "average color" stand-in, `chafa-canvas.c:186-198`); the final winning
symbol's color is re-extracted once.

Scalar-only: replace the SWAR popcount with `u64::count_ones()`; ignore `mask_u32` (AVX2-only).

---

## 8. Symbol data, tags & selector grammar

**Tags** (`chafa-symbol-map.h:32-69`) — port as `bitflags`. 28 single-bit flags
(`SPACE=1<<0 … OCTANT=1<<26`, `EXTRA=1<<30`) plus composites
`HALF=HHALF|VHALF`, `ALNUM=ALPHA|DIGIT`, `BAD=AMBIGUOUS|UGLY`, `ALL=~(EXTRA|BAD)`.

**Builtin glyphs** (`chafa-symbols.c`): ~712 narrow + 181 wide (kana) from the 5 header tables,
authored as 64/128-char `'X'`/`' '` outline strings → coverage → bitmap. Plus generated at init:
- Braille: U+2800–28FF, 256 symbols, 2×4 dot pattern from low byte (`gen_braille_sym`).
- Sextant: U+1FB00–1FB3A, 59, skip indices colliding with full/half blocks.
- Octant: 256 masks → codepoints via `octant_map[26]`, skip those colliding with block chars.

These three port as **exact algorithms** (no data tables). `def_to_symbol` also auto-derives
tags from the codepoint (`get_default_tags_for_char` `:520-560`): WIDE/NARROW, ASCII/TECHNICAL/
GEOMETRIC/BRAILLE/SEXTANT by range, ALPHA/DIGIT, and AMBIGUOUS/UGLY — but AMBIGUOUS is masked
off the auto-derived set (`& ~AMBIGUOUS`, see the `iswide_cjk` FIXME at `:567`).

**Selector grammar** (`parse_selectors` `:924`, `parse_symbol_tag` `:833`): tokens split on
space/comma; leading `+`/`-` set add/remove mode (persists across tokens); first token without
an operator triggers `do_clear` (replace vs. amend). Token forms: a **named class** (33-entry
case-insensitive vocabulary — `all none space solid stipple block border diagonal dot quad half
hhalf vhalf inverted braille sextant wedge technical geometric ascii alpha digit narrow wide
ambiguous ugly extra alnum bad legacy latin import imported octant`), a **code point**
(`u`/`U`/`0x` prefix + hex), a **range** (`first..last`), or a **literal set** `[...]`
(`\` escapes). Use the **code map**, not the man page (the docs omit ~11 valid classes).

**Selection eval** (`char_is_selected` `:477`): selectors applied in order, later wins.
Always-excluded: non-printable, zero-width, tab, RTL scripts (Arabic/Hebrew/Thaana/Syriac).
`auto_exclude_tags` starts as `BAD`; a matched *tag* selector clears its own bits — so ranges
don't pull in ugly/ambiguous unless explicitly named.

**Compile** (`rebuild_symbols` `:560`): dedup by codepoint (hashmap), filter via
`char_is_selected`, deep-copy coverage, **sort ascending by popcount** (enables binary-search
fill matching), build `packed_bitmaps`. Wide symbols mirror this with interleaved
`packed_bitmaps2`.

**GLib-Unicode parity risk:** routing/tagging uses `g_unichar_iswide/iswide_cjk/get_script/
ismark/isprint/iszerowidth/isalpha/isdigit`. Rust crates (`unicode-width`,
`unicode-properties`) may key off a different Unicode version → **symbol-set membership can
diverge** from chafa. Mitigation: lean on the builtin glyph set (fixed codepoints, so width is
known per-symbol from the source header — narrow vs. wide is *already decided* in the data) and
only need live classification for user-supplied ranges. Flagged in §12.

---

## 9. Port phases (dependency-ordered; each independently testable)

> Build the core first and validate it scaler-free (the milestone), then the pipeline around it.

**Phase 0 — Scaffolding & oracle.** Workspace, crates, CI (`cargo check/test/clippy`). Stand up
the oracle: build chafa 1.19.0 from `~/p/gh/chafa` (glib 2.88.1 + pkg-config present). Write the
**test-harness ANSI parser** (§10) — chafa exposes no per-cell introspection, so the parser is
how we observe chafa's chosen `(char, fg, bg)` per cell. It decodes a chafa-emitted ANSI stream
produced with `-O 0` (no REUSE/REPEAT optimizations → every cell emits a full SGR + glyph) back
into a cell grid. This parser, *not* our Phase 6 printer, is the comparison substrate for the
Phase 4 milestone. Wire the differential harness skeleton around it.

**Phase 1 — Color.** `Color`, `ColorPair`, `color_diff` (squared-Euclidean RGB). *No DIN99d* (D1).
Trivial; unit-test against hand-computed values. Unblocks everything.

**Phase 2 — Symbol data & tags.** `SymbolTags` (bitflags); `Symbol`/`WideSymbol`; coverage↔bitmap
(MSB-first); popcount/weights. Write `tools/transcode-symbols`, generate & commit `data.rs`.
Port the 3 procedural generators. Test: counts (256/59/~230), spot-check known glyphs
(e.g. U+25AE), bitmap round-trips.

**Phase 3 — Symbol map.** Selector parser (full grammar + 33-class vocab), `char_is_selected`
(ordering, auto-exclude, RTL/zero-width exclusion), `rebuild_symbols` (dedup, popcount sort,
packed bitmaps), candidate search (`find_candidates`/`find_wide_candidates` — Hamming sweep,
8-slot sorted insertion, inverse, sentinels 65/129). Test: selector strings → expected
codepoint sets; candidate ordering vs. a C dump.

**Phase 4 — Work-cell + selection core (🎯 MILESTONE: core parity).** `WorkCell` (counting sort,
dominant channel, `to_bitmap`, contrasting pair, mean-colors-for-symbol); `eval_symbol`,
fast/slow pick, wide lookback, fill fallback, blank normalization; the mode gates (without yet
needing palettes — use raw extracted colors / `default_colors`). **Milestone validation is
printer-free and palette-free:** drive chafa in **truecolor** (`-c full --color-space rgb -O 0`)
on pre-sized, fully-opaque images; parse its per-cell truecolor SGR back to `(char, fg_rgb,
bg_rgb)` with the Phase 0 parser; diff against our `cells` (in truecolor, `update_cell_colors`
is just an identity RGB pack, so no palette is needed). The `fgbg` and `fg-only` mode gates are
*also* reachable here (they select against fixed `default_colors`, not a palette) and are added
to tier A at this phase. The `Indexed16_8` gate (`use_quantized_error` snaps colors to the
fg/bg palettes *before* scoring, which can change the symbol picked) needs palettes → its tier-A
coverage completes in Phase 6. See §10. This is where the port earns its keep.

**Phase 5 — Pixel pipeline.** The 6 input formats → normalize to RGBA8 (R,G,B,A in `ch[0..3]`);
**port smolscale** (D3: filter selection `pick_filter_params` `smolscale.c:684`, bilinear+box
halvings, the two reversible sRGB LUTs `smolscale.c:98,124`, premul↔unassoc); two-pass prep
(`prepare_pixels_pass_1/2`); alpha-over-bg composite (`(c*a + bg*(255-a))/255`, gated on
have-alpha, no premul/threshold); `chafa_tuck_and_align` placement. Skip the saturation boost
(palette-gen only) and dithering (out of scope). Test: scaler unit tests; tier B e2e (§10).

**Phase 6 — Canvas, config, palettes, printer.** `CanvasConfig` (defaults: 80×24, cell 8×8,
fg `0xffffff`, bg `0x000000`, alpha_threshold 127, work_factor 0.5, optimizations ALL),
mode→palette mapping, fixed palettes (16-ANSI + 216-cube + 24-gray, exact values/formulas in
`chafa-palette.c:131-191`), `lookup_nearest` (cube-index formula + gray walk + ANSI scan;
brute-force for 16/8/240/fgbg), `update_cell_colors` per mode (incl. the `Indexed16_8`
fg-16/bg-8 split). Printer (`chafa-canvas-printer.c`): per-mode SGR emitters, wide-cell skip
(`c==0`), transparent→swap+invert, `flush_chars`/`REPEAT_CHAR`, `REUSE_ATTRIBUTES` SGR
suppression, aixterm 16-color pen arithmetic, `Indexed16_8` bold-for-bright. Test: ANSI string
diff vs. chafa (§10).

**Phase 7 — Threading.** `rayon` over row batches (D4); `--threads` → pool size; deterministic
output regardless of thread count (assert byte-identical across `--threads 1` vs N).

**Phase 8 — CLI (`chafa-syms`).** `clap` flags mapping to config:
`--colors` (`parse_colors_arg` table), `--work` (1–9 → `(n-1)/8.0`), `--threads` (-1 auto),
`--scale`/`--size`/`--font-ratio` (geometry §`chafa-util.c:60-159` + the `pixel_to_cell_dimensions`
scale hack `chafa.c:243-269`), `--fg`/`--bg` (named colors + hex), `--invert`, `--fg-only`,
`--format=symbols` (only mode accepted), `--optimize`. `image` crate decodes files → RGBA8.
Default symbol set: `block+border+space-wide` (lib config default), CLI removes `inverted` for
non-FGBG modes (matches chafa). View size via `terminal_size` or 80×24 (D5).

**Phase 9 — Polish.** Docs, examples, error types, `--symbols` round-trip, `--fill` (machinery
already exists from Phase 4 — wire the CLI flag), README.

---

## 10. Testing & the differential oracle

Two tiers, both diffing against the C chafa. **Observability constraint:** chafa has no
per-cell introspection — no `--dump-cells`, `cells` is internal, the only public output is the
rendered ANSI. So we cannot "dump both sides"; instead the Phase 0 **harness ANSI parser**
decodes chafa's output back into a `(char, fg, bg)` grid that we diff against our `cells`. We do
*not* need our own printer to run the gate.

**Oracle invocation rules (apply to every run):**
- Always pass `--color-space rgb` **explicitly** — never rely on the default. We checked the
  1.18.2 binary's default while porting 1.19.0; the default is version-dependent and unverified
  for the target. Passing it explicitly makes D1 parity version-proof.
- Always pass a fixed **`-O 0`** (disables `REUSE_ATTRIBUTES`/`REPEAT_CHAR`) so each cell emits a
  full, trivially-parseable SGR + glyph.
- Use **fully-opaque** inputs for the gate (avoids the transparent→swap+invert printer path
  muddying the diff).

- **Tier A — core parity (the gate).** Feed images sized *exactly* `cols*8 × rows*8` with
  `--size cols×rows` (+ stretch / `--scale` so smolscale takes the identity `COPY` path) → no
  resampling. Diff the parsed chafa grid against our `cells`. Tier A must **span the mode gates**,
  because the *selection* (not just the output) branches by mode — Truecolor alone leaves the
  branches unvalidated:
  | Mode flag | Gate it exercises | Dep |
  |-----------|-------------------|-----|
  | `-c full` (truecolor) | baseline Phase A+B + wide lookback + fill + blank | Phase 4 |
  | `-c none` (fgbg) | `extract_colors=false` → select vs fixed `default_colors` | Phase 4 |
  | `--fg-only` | `consider_inverted` + forced 50% gray FG | Phase 4 |
  | `-c 16/8` (indexed-16/8) | `use_quantized_error` → colors snapped to palette *before* scoring → can change the symbol picked | Phase 6 |
  | `-c 16`, `-c 256` | palette nearest-lookup + per-mode `update_cell_colors` | Phase 6 |
- **Tier B — end-to-end.** Arbitrary images + scaling; expect close/near-identical (smolscale is
  ported, so this can approach exact, but it is not the gate).

Harness: a Rust integration test (or script) that shells out to the built `chafa` 1.19.0,
captures stdout for a matrix of `{image} × {mode/flags}`, parses, and compares. Pin chafa
version; homebrew 1.18.2 is an approximate fallback only. Also: unit tests per module (color,
coverage round-trips, selector→set, candidate ordering, palette lookup, scaler) and a
determinism test asserting byte-identical output across `--threads 1` vs N.

---

## 11. Dependencies (all pure Rust)

- **lib `chafa-syms-rs`:** `rayon` (threads), `bitflags` (tags), `unicode-width` +
  `unicode-properties` (Unicode classification — parity risk §12). Aim to keep this set
  minimal; the core math is std-only.
- **CLI `chafa-syms`:** `chafa-syms-rs`, `clap`, `image` (decode), `terminal_size` (size),
  `anstyle`/raw writes (output). 
- **`tools/transcode-symbols`:** std-only (or a light parser); not a runtime dep.

---

## 12. Parity risks / landmines (annotated for the sRGB-only + core-parity target)

| Risk | Matters? | Note |
|------|----------|------|
| **MSB-first bitmap order** (`(x,y) → bit 63-(y*8+x)`) | ✅ critical | Get coverage↔bitmap exactly right or every match is wrong. |
| **Counting-sort & 8-slot candidate insertion tie-breaks** | ✅ critical | Ties must resolve identically to C (stable sort order, `memmove` insertion, sentinel 65/129) or candidate sets drift. |
| **Symbols sorted ascending by popcount** | ✅ | Affects candidate iteration order *and* binary-search fill matching. |
| **AVERAGE color extractor; weights = popcount / 64-popcount** | ✅ | Default extractor; MEDIAN is out (chafa itself calls it "extremely slow, almost no difference"). |
| **Mode gates** (`consider_inverted`/`extract_colors`/`use_quantized_error`) | ✅ | The only branch points by color mode. |
| **Fixed palette values** (chafa's *non-standard* 16-ANSI, e.g. green=`0x007000`) | ✅ | Use chafa's table verbatim, not real xterm values. |
| **256-cube index formula + midpoint cutoffs; gray ramp `8+10k`** | ✅ | Exact nearest-color reproduction. |
| **aixterm 16-color pen math; Indexed16_8 bold-for-bright** | ✅ | Output-string parity for those modes. |
| **`--work` mapping `(n-1)/8`; fast/slow threshold at `work_factor_int>=8`** | ✅ | Wrong threshold → wrong path → different picks. |
| **smolscale filter selection + reversible sRGB LUTs** | ⚠️ best-effort | Only affects Tier B (e2e); Tier A bypasses the scaler. Port faithfully but it's not the gate. |
| **GLib Unicode classification version** | ⚠️ | Can shift symbol-set membership for user ranges; builtin set is fixed-codepoint so mostly safe. |
| **sRGB compand `1.044` divisor; truncating `double→u8` DIN99d wrap** | ❌ N/A | Eliminated by D1 (no DIN99d). Recorded so a future DIN99d add-on knows the trap. |
| **Saturation boost; dithering; DYNAMIC_256/PCA** | ❌ N/A | Out of scope; do not port. |

---

## 13. Key C source map (for implementers)

| Area | Files |
|------|-------|
| Selection core / renderer | `chafa/internal/chafa-symbol-renderer.c`, `chafa-work-cell.c`, `chafa-canvas.c:539-578` |
| Symbol structs / popcount | `chafa/internal/chafa-private.h`, `chafa-bitfield.h`, `chafa-popcnt.c` |
| Symbol map / selectors / candidates | `chafa/chafa-symbol-map.c`, `.h` |
| Builtin glyph data | `chafa/internal/chafa-symbols.c`, `chafa-symbols-{ascii,latin,block,kana,misc-narrow}.h` |
| Color / diff | `chafa/internal/chafa-color.{c,h}` (`chafa_color_diff_fast` macro `:145`) |
| Pixel pipeline | `chafa/internal/chafa-pixops.c`, `chafa-batch.c`, `chafa-math-util.c` |
| Scaler | `chafa/internal/smolscale/` (`smolscale.h` API; `smolscale.c:684` filters; `:98,124` LUTs) |
| Frame / image / placement | `chafa/chafa-frame.c`, `chafa-image.c`, `chafa-placement.c`; structs `chafa-private.h:124-153` |
| Config / modes / formats | `chafa/chafa-canvas-config.c`, `chafa-common.h` (enums), `chafa-features.c` (threads) |
| Palettes | `chafa/internal/chafa-palette.c:131-191` (tables), lookup `:955-1037` |
| ANSI printer | `chafa/internal/chafa-canvas-printer.c`; SGR seqs `chafa-term-db.c`, pen math `chafa-term-info.c:1712-1746` |
| CLI option parsing | `tools/chafa/chicle-options.c`, geometry `chafa-util.c:60-159` + `chafa.c:243-269` |

---

## 14. Open items (non-blocking)

- Default canvas mode with no terminal detection: chosen `Truecolor` (D5); revisit if a more
  conservative default (e.g. `Indexed256`) is preferred for compatibility.
- `--fill` CLI flag: machinery lands in Phase 4; expose in Phase 9.
- float32 input: optional boundary conversion (D7), deferred.
- Whether to vendor a pinned Unicode data version to fully neutralize §12's GLib risk.
