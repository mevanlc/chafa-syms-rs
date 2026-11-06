# Differential-test oracle (chafa 1.19.0, upstream c897fce)

The current rendering baseline is upstream commit
`c897fce60cc4e947a254770ea1c2e320d58e7dce`, synchronized November 6, 2026.
The sibling `../chafa` fork includes this baseline and the dump hooks at merge
commit `db17c15`. Its instrumentation exposes prepared pixels, cells, symbol
maps, builtin symbols, and canonical ANSI without changing rendering behavior.

The version number alone does not identify a compatible oracle: earlier
1.19.0 snapshots and Homebrew 1.18.2 use different scaling, compositing, and
fixed-palette arithmetic.

## Building

With the sibling fork already at `db17c15`:

```sh
cd ~/p/my/chafa
./autogen.sh
make -j4
```

Alternatively, use a separate upstream checkout and the bundled patch:

```sh
cd ~/p/my
git clone https://github.com/hpjansson/chafa.git chafa-oracle
cd chafa-oracle
git checkout --detach c897fce60cc4e947a254770ea1c2e320d58e7dce
git apply ../chafa-syms-rs/devdocs/oracle/chafa-cells-dump.patch
./autogen.sh
make -j4
```

The real CLI is `tools/chafa/.libs/chafa`; its library is in `chafa/.libs`.
Tests default to the sibling `../chafa` build. For another checkout:

```sh
export CHAFA_ORACLE_BIN="$HOME/p/my/chafa-oracle/tools/chafa/.libs/chafa"
export CHAFA_ORACLE_LIB="$HOME/p/my/chafa-oracle/chafa/.libs"
export CHAFA_SMOLSCALE_DIR="$HOME/p/my/chafa-oracle/chafa/internal/smolscale"
cd ~/p/my/chafa-syms-rs
cargo nextest run --workspace --locked
```

The tests skip their oracle assertions if the binary/source tree is absent.
When sources are available, a reference-harness compilation failure fails the
test. The scalar harness cache is keyed by its C wrapper, source, and headers,
so switching source snapshots cannot silently reuse an older reference.

## Dump formats

Set an environment variable to the destination filename to enable each hook:

| Variable | Contents |
|----------|----------|
| `CHAFA_DUMP_CELLS` | `W H`, then `x y codepoint fg_hex bg_hex` for every cell |
| `CHAFA_DUMP_PIXELS` | `W H\n`, then `W*H*4` prepared RGBA bytes |
| `CHAFA_DUMP_SYMMAP` | `SYMMAP_NARROW n`, then `codepoint popcount`; `SYMMAP_WIDE m`, then `codepoint left_popcount right_popcount` |
| `CHAFA_DUMP_SYMBOLS` | `NARROW n`, then `codepoint tags popcount bitmap_hex`; `WIDE m`, then `codepoint tags left_popcount right_popcount left_bitmap_hex right_bitmap_hex` |
| `CHAFA_DUMP_ANSI` | Canonical canvas bytes, rows joined by newlines without CLI framing |

In the cell dump, codepoint zero marks a wide-symbol continuation. Truecolor
colors are packed `0xAARRGGBB`; indexed modes store palette indices. The
symbol-map and ANSI hooks run through `chafa_canvas_print_rows`, used by the CLI.

## Rendering changes covered by this sync

- Upstream now breaks equal-popcount symbol ties by codepoint unconditionally.
  `CHAFA_SYMS_RS_TIEBREAK` is no longer needed or set by the harness.
- Fixed-palette lookup corrects the first cube midpoint and bright-gray index.
- Contrasting colors use a linear extrema scan, retaining first-minimum,
  last-maximum, and first-channel tie behavior.
- Scaling uses upstream's reversible gamma-2 lookup tables, lossless
  `(alpha + 1)` encoding, a 14x box-filter cutoff, and conservative box weights.
  Extremely large reductions switch to compressed channels as upstream does.
- Background compositing runs inside smolscale before preprocessing and retains
  source alpha. Low-work sampling uses the new nearest-neighbor filter.

The tests compare symbol data/order, per-cell choices, ANSI bytes, isolated
scaling/compositing, and end-to-end output. They exercise transparency, colored
backgrounds, work levels, preprocessing, and scaling boundaries. The isolated
scaler gate compiles upstream's scalar implementation without SIMD; the port
does not add C decoder dependencies, DIN99d, dynamic palettes, dithering, pixel
graphics protocols, terminal probing, or animation playback.
