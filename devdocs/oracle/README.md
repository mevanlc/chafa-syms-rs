# Differential-test oracle (chafa 1.19.0, patched)

The parity gate compares this crate's per-cell symbol/color picks against the
real chafa. chafa exposes **no per-cell introspection**, and its printed ANSI
re-encodes cells through optimizations (SGR 7 reverse for solid cells, space
handling) that are awkward to invert. So we run a tiny **patched** chafa that
dumps each cell's internal `(codepoint, fg_color, bg_color)` directly.

This isolates the **selection core** (Phase 4 gate) from the **printer**
(Phase 6, validated separately by byte-exact ANSI comparison against the
*stock* binary).

## Building the oracle

```sh
cd ~/p/gh/chafa            # chafa 1.19.0 checkout (commit ea18ac4)
git apply <this-repo>/devdocs/oracle/chafa-cells-dump.patch
NOCONFIGURE=1 ./autogen.sh
./configure
make -j8
```

The patched library is `chafa/.libs/libchafa.0.dylib`; the CLI is the libtool
wrapper `tools/chafa/chafa` (real binary at `tools/chafa/.libs/chafa`).

Verified: with `-c full --color-space rgb -O 0`, the built 1.19.0 binary
produces **byte-identical** truecolor output to the homebrew 1.18.2 binary, so
1.18.2 is an acceptable fallback for the truecolor core (the only core-affecting
deltas since 1.18.2 are an FGBG color-storage fix and a scaler pixel-type fix;
the renderer change is a pure code move).

## The patch

`chafa-cells-dump.patch` adds `chafa_syms_rs_dump_cells()`, called at the top of
`chafa_canvas_print` and `chafa_canvas_print_rows`. When the `CHAFA_DUMP_CELLS`
env var names a file (and the canvas is in symbol mode), it writes:

```
<width> <height>
<cx> <cy> <codepoint> <fg_AARRGGBB_hex> <bg_AARRGGBB_hex>
...                       (one line per cell, row-major)
```

`codepoint == 0` marks a wide-symbol continuation (right half). Colors are
chafa's packed `0xAARRGGBB` in truecolor; in indexed modes they are palette
indices. No effect unless the env var is set.

The patch also adds `chafa_syms_rs_dump_symbols()`, called at the end of
`chafa_init_symbols`. When `CHAFA_DUMP_SYMBOLS` names a file it writes the
fully-initialized builtin symbol arrays — ground truth for the Phase 2 symbol
gate:

```
NARROW <n>
<codepoint> <sc> <popcount> <bitmap_hex>      (one line per narrow symbol)
...
WIDE <m>
<codepoint> <sc> <pc_left> <pc_right> <bitmap_left_hex> <bitmap_right_hex>
...
```

## Using it from tests

`crates/chafa-syms-rs/tests/support/mod.rs` locates the binary via
`CHAFA_ORACLE_BIN` / `CHAFA_ORACLE_LIB` (build-tree defaults), writes a PNG
fixture, runs the binary with `CHAFA_DUMP_CELLS`, and parses the dump. Tests
skip gracefully if the binary is absent.
