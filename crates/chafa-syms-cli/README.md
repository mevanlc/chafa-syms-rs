# chafa-syms-cli

`chafa-syms-cli` provides the `chafa-syms` command: a small image-to-terminal
renderer built on the `chafa-syms-rs` library.

It decodes an image file, renders it through the pure-Rust chafa symbol core,
and writes ANSI/UTF-8 symbol art to stdout.

## Install

```sh
cargo install chafa-syms-cli
```

This installs the binary as `chafa-syms`.

## Examples

```sh
chafa-syms image.png
chafa-syms --size 80x24 image.png
chafa-syms --size 120x image.png
chafa-syms --scale 2 image.png
chafa-syms --colors 256 image.jpg
chafa-syms --colors none --symbols ascii image.png
chafa-syms --fg-only --symbols all image.webp
chafa-syms --fg cyan --bg black --invert image.bmp
```

Supported input formats are the formats enabled in this package's `image`
dependency: PNG, JPEG, GIF, BMP, and WebP.

## Size

By default, `chafa-syms` fits the image to the current terminal size while
preserving its visible aspect ratio. If terminal size is unavailable, it falls
back to an 80x24 cell view.

Use `--size` to choose output dimensions in terminal cells:

```sh
chafa-syms --size 80x24 image.png  # fixed width and height
chafa-syms --size 80 image.png     # fixed width, inferred height
chafa-syms --size x24 image.png    # inferred width, fixed height
```

Use `--scale` to multiply the natural size, where one terminal cell represents
an 8x8 pixel block:

```sh
chafa-syms --scale 1 image.png
chafa-syms --scale 2 image.png
chafa-syms --scale max image.png
```

`--size` takes precedence over `--scale`.

## Colors

Use `--colors` or `-c` to select the output color mode:

```sh
chafa-syms -c full image.png
chafa-syms -c 256 image.png
chafa-syms -c 16 image.png
chafa-syms -c none image.png
```

Supported values:

- `full`, `truecolor`, `tc`
- `256`
- `240`
- `16`
- `16/8`, `16-8`
- `8`
- `2`
- `none`

Foreground and background colors accept `rrggbb`, `#rrggbb`, or basic color
names:

```sh
chafa-syms --fg '#00ffff' --bg black image.png
chafa-syms --fg yellow --bg blue image.png
```

Named colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`,
`white`, `gray`, and `grey`.

## Symbols

Use `--symbols` to choose the character set:

```sh
chafa-syms --symbols ascii image.png
chafa-syms --symbols block+border image.png
chafa-syms --symbols all image.png
chafa-syms --symbols '+sextant' image.png
```

The selector syntax follows chafa-style symbol selectors, including tags,
ranges, codepoints, and bracketed character sets.

## Options

```text
--size WxH        Output size in terminal cells
--scale N|max     Scale natural size, ignored when --size is set
-c, --colors MODE Color mode
--fg COLOR        Foreground color, default white
--bg COLOR        Background color, default black
-w, --work 1-9    Quality/work factor, default 5
--threads N       Worker threads, -1 for auto
--symbols SPEC    Symbol selector string
--fg-only         Use foreground colors only
--invert          Swap foreground and background colors
--font-ratio N    Font width/height ratio, default 0.5
-O, --optimize N  Output optimization level, default 5
-f, --format FMT  Output format; only symbols is supported
```

## Scope

`chafa-syms` is a focused CLI for the `chafa-syms-rs` symbol renderer. It is not
a full replacement for the upstream chafa command.

Implemented:

- image-file decoding for PNG, JPEG, GIF, BMP, and WebP
- terminal-cell sizing and aspect correction
- truecolor and indexed terminal color modes
- foreground-only rendering
- chafa-style symbol selectors
- ANSI/UTF-8 symbol output

Not implemented:

- sixel, Kitty, or iTerm2 graphics output
- terminal capability probing
- dynamic palettes or dithering
- placement, tuck, and alignment controls
- non-symbol output formats

## Library

For embedding the renderer in another Rust program, use the `chafa-syms-rs`
crate directly.

## License

LGPL-3.0-only.
