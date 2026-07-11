//! `chafa-syms` — demo/test CLI over [`chafa_syms_rs`]: render an image as
//! tunable Unicode symbol art.
//!
//! Flags track chafa's where practical:
//! `--size --scale --colors --fg --bg --work --threads --symbols --fg-only
//! --invert --font-ratio --optimize --format`.

use std::{ffi::OsString, process::ExitCode};

use chafa_syms_rs::canvas::{Canvas, CanvasConfig, StippleMode};
use chafa_syms_rs::printer::Optimizations;
use chafa_syms_rs::select::CanvasMode;
use chafa_syms_rs::{PixelType, SymbolMap};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "chafa-syms",
    about = "Render images as tunable Unicode symbol art (pure-Rust chafa symbol core)."
)]
struct Args {
    /// Image file to render.
    file: String,

    /// Output size in cells: `WxH`, `W`, or `xH`. Defaults to the terminal size.
    #[arg(long)]
    size: Option<String>,

    /// Scale factor applied to the natural (1 px/cell-block) size. `max` fits
    /// the view. Ignored when `--size` is given.
    #[arg(long)]
    scale: Option<String>,

    /// Color mode: none, 2, 8, 16, 16/8, 240, 256, full.
    #[arg(short = 'c', long, default_value = "full")]
    colors: String,

    /// Foreground color (hex `rrggbb`/`#rrggbb` or a basic name).
    #[arg(long, default_value = "white")]
    fg: String,

    /// Background color.
    #[arg(long, default_value = "black")]
    bg: String,

    /// Work/quality factor 1–9 (higher is slower, more thorough).
    #[arg(short = 'w', long, default_value_t = 5)]
    work: u32,

    /// Worker threads (-1 = auto).
    #[arg(long, default_value_t = -1)]
    threads: i32,

    /// Symbol selector string (e.g. `block+border`, `ascii`, `all`), or `help`.
    #[arg(long, value_name = "SPEC")]
    symbols: Option<String>,

    /// Use foreground colors only (transparent background).
    #[arg(long)]
    fg_only: bool,

    /// Swap foreground/background colors.
    #[arg(long)]
    invert: bool,

    /// Image preprocessing: on/off. Defaults to on, matching chafa.
    #[arg(short = 'p', long, default_value = "on")]
    preprocess: String,

    /// Stipple post-processing mode: off, grayscale-fill.
    #[arg(long, default_value = "off")]
    stipple_mode: String,

    /// Font width/height ratio for aspect correction (default 0.5).
    #[arg(long, default_value_t = 0.5)]
    font_ratio: f32,

    /// Output optimization level 0–9 (0 = none).
    #[arg(short = 'O', long, default_value_t = 5)]
    optimize: u32,

    /// Output format. Only `symbols` is supported.
    #[arg(short = 'f', long, default_value = "symbols")]
    format: String,
}

fn main() -> ExitCode {
    let args = parse_args();
    if args.symbols.as_deref() == Some("help") {
        print!("{SYMBOLS_HELP}");
        return ExitCode::SUCCESS;
    }
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("chafa-syms: {e}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Args {
    let raw_args: Vec<OsString> = std::env::args_os().collect();
    let symbols_help_requested = raw_args.iter().any(|arg| arg == "--symbols=help")
        || raw_args
            .windows(2)
            .any(|args| args[0] == "--symbols" && args[1] == "help");

    match Args::try_parse_from(raw_args.clone()) {
        Ok(args) => args,
        Err(error)
            if symbols_help_requested
                && error.kind() == clap::error::ErrorKind::MissingRequiredArgument =>
        {
            let mut args_with_placeholder = raw_args;
            args_with_placeholder.push(OsString::new());
            Args::parse_from(args_with_placeholder)
        }
        Err(error) => error.exit(),
    }
}

fn run(args: &Args) -> Result<(), String> {
    if args.format != "symbols" {
        return Err(format!(
            "unsupported --format '{}': only 'symbols'",
            args.format
        ));
    }

    let mode = parse_mode(&args.colors)?;
    let (mut fg, mut bg) = (parse_color(&args.fg)?, parse_color(&args.bg)?);
    let preprocess = parse_bool(&args.preprocess, "--preprocess")?;
    let stipple_mode = parse_stipple_mode(&args.stipple_mode)?;
    if args.invert {
        std::mem::swap(&mut fg, &mut bg);
    }

    // Threads.
    if args.threads >= 0 {
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(args.threads as usize)
            .build_global();
    } else {
        let n = std::thread::available_parallelism()
            .map(|p| p.get())
            .unwrap_or(1)
            .min(24);
        let _ = rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global();
    }

    // Decode image.
    let img = image::open(&args.file)
        .map_err(|e| format!("cannot open {}: {e}", args.file))?
        .to_rgba8();
    let (iw, ih) = (img.width() as usize, img.height() as usize);

    // Geometry.
    let (max_cols, max_rows) = view_size();
    let (cols, rows) = geometry(args, iw, ih, max_cols, max_rows);

    // Config.
    let mut symbol_map = cli_symbols(mode, args.symbols.as_deref())?;
    symbol_map.prepare();

    let cfg = CanvasConfig::new(cols, rows)
        .mode(mode)
        .fg_color(fg)
        .bg_color(bg)
        .work_factor((args.work.clamp(1, 9) as f32 - 1.0) / 8.0)
        .fg_only(args.fg_only)
        .preprocessing(preprocess)
        .stipple_mode(stipple_mode)
        .optimizations(optimizations(args.optimize))
        .symbol_map(symbol_map);

    let mut canvas = Canvas::new(cfg);
    canvas.draw_all_pixels(PixelType::Rgba8, img.as_raw(), iw, ih, iw * 4);

    // Cursor framing like the chafa CLI: hide cursor, output, show cursor.
    print!("\x1b[?25l{}\n\x1b[?25h", canvas.print());
    Ok(())
}

const SYMBOLS_HELP: &str = r#"NAME
    chafa-syms --symbols - select the characters used for symbol rendering

SYNOPSIS
    chafa-syms --symbols SPEC IMAGE
    chafa-syms --symbols help

DESCRIPTION
    SPEC is an ordered list of symbol selectors. If its first selector has no
    initial '+' or '-', it replaces the default symbol map. An initial '+' or
    '-' modifies the default map instead. Selectors are applied from left to
    right: '+' adds matching symbols and '-' removes them.

GRAMMAR
    SPEC      := [OP] ITEM { [SEPARATOR] [OP] ITEM }
    OP        := '+' | '-'
    SEPARATOR := comma or space
    ITEM      := NAME | CODEPOINT | RANGE | SET
    CODEPOINT := hexadecimal code point: 2580, u2580, or 0x2580
    RANGE     := CODEPOINT '..' CODEPOINT
    SET       := '[' literal characters ']'

    Within a SET, backslash escapes the next character, so '\]' selects a
    closing bracket and '\\' selects a backslash. An OP remains in effect until
    another OP changes it; commas and spaces only separate items. Named sets
    are case-insensitive.

NAMED SYMBOL SETS
    all         All normal symbols; excludes extra, ambiguous, and ugly sets.
    none        No symbols (useful as the base/replacement set).
    space       The space character.
    solid       The solid cell character (the inverse of space).
    stipple     Stipple and shade symbols.
    block       Block-element symbols.
    border      Border and box-drawing symbols.
    diagonal    Diagonal border symbols.
    dot         Isolated dot symbols, excluding Braille.
    quad        Quadrant block symbols (2x2 mosaics).
    half        Horizontal and vertical half-block symbols.
    hhalf       Horizontal half-block symbols (upper and lower halves).
    vhalf       Vertical half-block symbols (left and right halves).
    inverted    Symbols that are inverses of simpler symbols.
    braille     Braille symbols (2x4 dot mosaics).
    sextant     Sextant symbols (2x3 mosaics).
    octant      Octant symbols (2x4 mosaics).
    wedge       Wedge shapes that align with sextants.
    technical   Miscellaneous technical symbols.
    geometric   Geometric shapes.
    ascii       Printable ASCII characters.
    alpha       Letters.
    digit       Digits.
    alnum       Letters and digits (the union of alpha and digit).
    narrow      Characters that are one terminal cell wide.
    wide        Characters that are two terminal cells wide.
    ambiguous   Characters of uncertain width; otherwise excluded
                automatically.
    ugly        Characters unlikely to render well; otherwise excluded
                automatically.
    bad         The union of ambiguous and ugly.
    legacy      Legacy computing symbols, including sextants and wedges.
    latin       Latin and Latin-like symbols (a superset of ASCII).
    import      User-imported glyphs (none are available in this build).
    imported    Alias for import.
    extra       Symbols not in another category; excluded from all.

DEFAULT
    The default is block+border+space-wide. In color modes other than 'none'
    and '2', inverted symbols are also removed.

EXAMPLES
    --symbols ascii
        Replace the default with printable ASCII characters.

    --symbols block+border-solid
        Use block and border symbols, excluding the solid cell.

    --symbols +braille-dot
        Add Braille symbols to the default and remove isolated dots.

    --symbols u2580..u259f
        Use Unicode Block Elements by code point range.

    --symbols '[ .oO@]'
        Use exactly the five literal characters between the brackets.
"#;

/// chafa's CLI default symbol set: block+border+space-wide, minus `inverted`
/// for non-FGBG modes (`chicle-options.c`).
fn default_cli_symbols(mode: CanvasMode) -> SymbolMap {
    let mut m = SymbolMap::chafa_default();
    if mode != CanvasMode::Fgbg && mode != CanvasMode::FgbgBgfg {
        m.apply_selectors("-inverted").unwrap();
    }
    m
}

fn cli_symbols(mode: CanvasMode, selectors: Option<&str>) -> Result<SymbolMap, String> {
    let mut m = default_cli_symbols(mode);
    if let Some(s) = selectors {
        m.apply_selectors(s).map_err(|e| e.0)?;
    }
    Ok(m)
}

fn parse_mode(s: &str) -> Result<CanvasMode, String> {
    Ok(match s {
        "none" => CanvasMode::Fgbg,
        "2" => CanvasMode::FgbgBgfg,
        "8" => CanvasMode::Indexed8,
        "16" => CanvasMode::Indexed16,
        "16/8" | "16-8" => CanvasMode::Indexed16_8,
        "240" => CanvasMode::Indexed240,
        "256" => CanvasMode::Indexed256,
        "full" | "truecolor" | "tc" => CanvasMode::Truecolor,
        _ => return Err(format!("unknown color mode '{s}'")),
    })
}

fn parse_bool(s: &str, name: &str) -> Result<bool, String> {
    match s.to_ascii_lowercase().as_str() {
        "on" | "yes" | "true" | "1" => Ok(true),
        "off" | "no" | "false" | "0" => Ok(false),
        _ => Err(format!("{name} must be on or off")),
    }
}

fn parse_stipple_mode(s: &str) -> Result<StippleMode, String> {
    match s.to_ascii_lowercase().as_str() {
        "off" | "none" => Ok(StippleMode::Off),
        "grayscale-fill" | "grayscale" | "gray-fill" | "grey-fill" => {
            Ok(StippleMode::GrayscaleFill)
        }
        _ => Err(format!("unknown --stipple-mode '{s}'")),
    }
}

fn optimizations(level: u32) -> Optimizations {
    let mut o = Optimizations::empty();
    if level >= 1 {
        o |= Optimizations::REUSE_ATTRIBUTES;
    }
    if level >= 6 {
        o |= Optimizations::REPEAT_CELLS;
    }
    o
}

/// Parse a hex (`rrggbb`/`#rrggbb`) or basic named color to packed `0x00RRGGBB`.
fn parse_color(s: &str) -> Result<u32, String> {
    let t = s.trim();
    let named = match t.to_ascii_lowercase().as_str() {
        "black" => Some(0x000000),
        "red" => Some(0xff0000),
        "green" => Some(0x00ff00),
        "yellow" => Some(0xffff00),
        "blue" => Some(0x0000ff),
        "magenta" => Some(0xff00ff),
        "cyan" => Some(0x00ffff),
        "white" => Some(0xffffff),
        "gray" | "grey" => Some(0x808080),
        _ => None,
    };
    if let Some(v) = named {
        return Ok(v);
    }
    let hex = t.strip_prefix('#').unwrap_or(t);
    if hex.len() == 6 {
        if let Ok(v) = u32::from_str_radix(hex, 16) {
            return Ok(v & 0x00ff_ffff);
        }
    }
    Err(format!("invalid color '{s}'"))
}

fn view_size() -> (usize, usize) {
    use terminal_size::{terminal_size, Height, Width};
    if let Some((Width(w), Height(h))) = terminal_size() {
        // Leave the bottom row for the shell prompt, like chafa.
        (w as usize, (h as usize).saturating_sub(1).max(1))
    } else {
        (80, 24)
    }
}

/// Compute the cell grid. Honors `--size` (`WxH`/`W`/`xH`), else fits the image
/// into the view preserving aspect using `font_ratio`.
fn geometry(args: &Args, iw: usize, ih: usize, max_cols: usize, max_rows: usize) -> (usize, usize) {
    let fr = if args.font_ratio > 0.0 {
        args.font_ratio
    } else {
        0.5
    };

    if let Some(sz) = &args.size {
        if let Some((c, r)) = parse_size(sz) {
            match (c, r) {
                (Some(c), Some(r)) => return (c.max(1), r.max(1)),
                (Some(c), None) => {
                    let rows = ((c as f32) * (ih as f32) / (iw as f32) * fr).ceil() as usize;
                    return (c.max(1), rows.max(1));
                }
                (None, Some(r)) => {
                    let cols = ((r as f32) * (iw as f32) / (ih as f32) / fr).ceil() as usize;
                    return (cols.max(1), r.max(1));
                }
                (None, None) => {}
            }
        }
    }

    // --scale: multiply the natural (1 cell = 8x8 px) size, or "max" to fit.
    if let Some(scale) = &args.scale {
        if scale != "max" {
            if let Ok(n) = scale.parse::<f32>() {
                if n > 0.0 {
                    let cols = ((iw as f32 / 8.0) * n).ceil() as usize;
                    let rows = ((ih as f32 / 8.0) * n * fr).ceil() as usize;
                    return (cols.clamp(1, max_cols), rows.clamp(1, max_rows));
                }
            }
        }
    }

    fit_aspect(iw, ih, max_cols, max_rows, fr)
}

/// Fit image into the `max` cell box preserving on-screen aspect.
fn fit_aspect(iw: usize, ih: usize, max_cols: usize, max_rows: usize, fr: f32) -> (usize, usize) {
    let img_aspect = iw as f32 / ih as f32;
    // On screen a cell is `fr` wide per 1 tall, so cols*fr : rows should match aspect.
    let mut cols = max_cols as f32;
    let mut rows = cols * fr / img_aspect;
    if rows > max_rows as f32 {
        rows = max_rows as f32;
        cols = rows * img_aspect / fr;
    }
    (
        (cols.ceil() as usize).clamp(1, max_cols),
        (rows.ceil() as usize).clamp(1, max_rows),
    )
}

/// Parse `WxH`, `W`, or `xH` into optional dims.
fn parse_size(s: &str) -> Option<(Option<usize>, Option<usize>)> {
    if let Some((a, b)) = s.split_once('x') {
        let c = if a.is_empty() { None } else { a.parse().ok() };
        let r = if b.is_empty() { None } else { b.parse().ok() };
        Some((c, r))
    } else {
        s.parse().ok().map(|c| (Some(c), None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plus_symbols_augments_cli_default_map() {
        let mut map = cli_symbols(CanvasMode::Truecolor, Some("+sextant")).unwrap();
        map.prepare();

        assert!(map.has_symbol('\u{1fb00}'));
        assert!(map.has_symbol('\u{2574}'));
    }

    #[test]
    fn bare_symbols_replace_cli_default_map() {
        let mut map = cli_symbols(CanvasMode::Truecolor, Some("sextant")).unwrap();
        map.prepare();

        assert!(map.has_symbol('\u{1fb00}'));
        assert!(!map.has_symbol('\u{2574}'));
    }

    #[test]
    fn fit_aspect_uses_chafa_ceil_geometry() {
        assert_eq!(fit_aspect(1952, 2158, 80, 24, 0.5), (44, 24));
    }

    #[test]
    fn parse_stipple_mode_aliases() {
        assert_eq!(parse_stipple_mode("off").unwrap(), StippleMode::Off);
        assert_eq!(
            parse_stipple_mode("grayscale-fill").unwrap(),
            StippleMode::GrayscaleFill
        );
    }
}
