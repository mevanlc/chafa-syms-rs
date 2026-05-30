//! Differential-oracle test harness.
//!
//! Drives a **patched** chafa 1.19.0 binary that, when `CHAFA_DUMP_CELLS` names
//! a file, writes each cell's internal `(codepoint, fg_color, bg_color)` there
//! (see the `chafa_syms_rs_dump_cells` patch in the chafa source tree). This
//! exposes chafa's actual per-cell symbol/color picks directly, bypassing all
//! printer encoding — the comparison substrate for the Phase 4 core-parity gate.
//!
//! Colors in the dump are packed `0xAARRGGBB` (chafa's `chafa_pack_color`).
//!
//! The oracle binary/lib are located via env vars with build-tree defaults:
//! - `CHAFA_ORACLE_BIN` (default `~/p/gh/chafa/tools/chafa/.libs/chafa`)
//! - `CHAFA_ORACLE_LIB` (default `~/p/gh/chafa/chafa/.libs`, used as `DYLD_LIBRARY_PATH`)
//!
//! If the binary is missing, [`oracle_available`] returns false so tests can
//! skip gracefully on machines without the built oracle.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use chafa_syms_rs::Color;

/// One cell as reported by the patched oracle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OracleCell {
    /// Unicode codepoint of the chosen symbol. `0` marks a wide-symbol
    /// continuation (the right half of a double-width cell).
    pub codepoint: u32,
    /// Raw `0xAARRGGBB` foreground color (truecolor) or palette index (indexed).
    pub fg_raw: u32,
    /// Raw `0xAARRGGBB` background color (truecolor) or palette index (indexed).
    pub bg_raw: u32,
}

impl OracleCell {
    /// The chosen symbol as a `char` (`None` for a wide continuation cell).
    pub fn ch(&self) -> Option<char> {
        if self.codepoint == 0 {
            None
        } else {
            char::from_u32(self.codepoint)
        }
    }

    /// Foreground as an RGB [`Color`], interpreting `fg_raw` as `0xAARRGGBB`.
    pub fn fg(&self) -> Color {
        unpack_aarrggbb(self.fg_raw)
    }

    /// Background as an RGB [`Color`], interpreting `bg_raw` as `0xAARRGGBB`.
    pub fn bg(&self) -> Color {
        unpack_aarrggbb(self.bg_raw)
    }
}

/// Unpack chafa's `0xAARRGGBB` packed color into a [`Color`] (`[R, G, B, A]`).
pub fn unpack_aarrggbb(u: u32) -> Color {
    Color::new(
        ((u >> 16) & 0xff) as u8,
        ((u >> 8) & 0xff) as u8,
        (u & 0xff) as u8,
        ((u >> 24) & 0xff) as u8,
    )
}

/// A grid of oracle cells, row-major.
#[derive(Clone, Debug)]
pub struct OracleGrid {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<OracleCell>,
}

impl OracleGrid {
    pub fn at(&self, x: usize, y: usize) -> OracleCell {
        self.cells[y * self.cols + x]
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/".into()))
}

/// Path to the patched oracle binary.
pub fn oracle_bin() -> PathBuf {
    if let Ok(p) = std::env::var("CHAFA_ORACLE_BIN") {
        return PathBuf::from(p);
    }
    home().join("p/gh/chafa/tools/chafa/.libs/chafa")
}

/// Directory to put on `DYLD_LIBRARY_PATH` so the binary loads the patched lib.
pub fn oracle_lib() -> PathBuf {
    if let Ok(p) = std::env::var("CHAFA_ORACLE_LIB") {
        return PathBuf::from(p);
    }
    home().join("p/gh/chafa/chafa/.libs")
}

/// Whether the oracle binary exists (tests should skip if not).
pub fn oracle_available() -> bool {
    oracle_bin().is_file()
}

static SEQ: AtomicU64 = AtomicU64::new(0);

fn unique_tmp(ext: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    std::env::temp_dir().join(format!("chafa_syms_oracle_{pid}_{n}.{ext}"))
}

/// Render `pixels` (RGBA8, `w`×`h`, tightly packed) through the oracle at a cell
/// geometry of `cols`×`rows`, returning the per-cell dump.
///
/// `extra_args` carries mode flags (e.g. `["-c", "full"]`, `["--fg-only"]`).
/// The fixture is written as a PNG and fed with `--size {cols}x{rows} --stretch`
/// so that, when `w == cols*8 && h == rows*8`, smolscale takes the identity copy
/// path (no resampling) — the Tier-A bypass.
pub fn oracle_render(
    pixels: &[u8],
    w: u32,
    h: u32,
    cols: u32,
    rows: u32,
    extra_args: &[&str],
) -> OracleGrid {
    assert_eq!(pixels.len(), (w * h * 4) as usize, "RGBA8 buffer size");

    let png_path = unique_tmp("png");
    let dump_path = unique_tmp("dump");

    image::save_buffer(&png_path, pixels, w, h, image::ColorType::Rgba8)
        .expect("write PNG fixture");

    let size = format!("{cols}x{rows}");
    let mut args: Vec<String> = vec![
        "-f".into(),
        "symbols".into(),
        "--color-space".into(),
        "rgb".into(),
        "-O".into(),
        "0".into(),
        "--size".into(),
        size,
        "--stretch".into(),
    ];
    for a in extra_args {
        args.push((*a).into());
    }
    args.push(png_path.to_string_lossy().into_owned());

    let output = Command::new(oracle_bin())
        .args(&args)
        .env("CHAFA_DUMP_CELLS", &dump_path)
        .env("DYLD_LIBRARY_PATH", oracle_lib())
        .output()
        .expect("run oracle");

    assert!(
        output.status.success(),
        "oracle failed: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let dump = std::fs::read_to_string(&dump_path)
        .unwrap_or_else(|e| panic!("read dump {}: {e}", dump_path.display()));

    let _ = std::fs::remove_file(&png_path);
    let _ = std::fs::remove_file(&dump_path);

    parse_dump(&dump)
}

/// Parse the `CHAFA_DUMP_CELLS` file format:
/// first line `W H`, then one line per cell `cx cy codepoint fg_hex bg_hex`.
pub fn parse_dump(s: &str) -> OracleGrid {
    let mut lines = s.lines();
    let header = lines.next().expect("dump header");
    let mut hdr = header.split_whitespace();
    let cols: usize = hdr.next().unwrap().parse().unwrap();
    let rows: usize = hdr.next().unwrap().parse().unwrap();

    let mut cells = vec![
        OracleCell {
            codepoint: 0,
            fg_raw: 0,
            bg_raw: 0
        };
        cols * rows
    ];
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let mut p = line.split_whitespace();
        let cx: usize = p.next().unwrap().parse().unwrap();
        let cy: usize = p.next().unwrap().parse().unwrap();
        let codepoint: u32 = p.next().unwrap().parse().unwrap();
        let fg_raw = u32::from_str_radix(p.next().unwrap(), 16).unwrap();
        let bg_raw = u32::from_str_radix(p.next().unwrap(), 16).unwrap();
        cells[cy * cols + cx] = OracleCell {
            codepoint,
            fg_raw,
            bg_raw,
        };
    }

    OracleGrid { cols, rows, cells }
}
