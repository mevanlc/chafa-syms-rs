//! `transcode-symbols` — one-shot generator (run manually; output committed).
//!
//! Parses the `CHAFA_SYMBOL_OUTLINE_8X8 / 16X8(...)` outline tables from
//! `chafa-symbols-{ascii,latin,block,kana,misc-narrow}.h` (the 5 headers
//! `#include`d by `chafa-symbols.c`, in that order) and emits
//! `crates/chafa-syms-rs/src/symbol/data.rs`.
//!
//! Each entry yields `(author_tags, codepoint, bitmap)`; the bitmap is computed
//! with chafa's MSB-first, row-major bit order (pixel `(x,y)` -> bit
//! `63 - (y*8 + x)`). Narrow defs (64-char outlines) and wide defs (128-char,
//! 16x8) are emitted to separate arrays, preserving cross-file order so the
//! built arrays match chafa's `chafa_symbols`/`chafa_symbols2` ordering.
//!
//! Usage:
//!   transcode-symbols [CHAFA_INTERNAL_DIR] [OUT_DATA_RS]
//! Defaults:
//!   CHAFA_INTERNAL_DIR = ~/p/gh/chafa/chafa/internal
//!   OUT_DATA_RS        = crates/chafa-syms-rs/src/symbol/data.rs (relative to CWD)

use std::fmt::Write as _;
use std::path::PathBuf;

const FILES: &[&str] = &[
    "chafa-symbols-ascii.h",
    "chafa-symbols-latin.h",
    "chafa-symbols-block.h",
    "chafa-symbols-kana.h",
    "chafa-symbols-misc-narrow.h",
];

/// Map a `CHAFA_SYMBOL_TAG_<NAME>` name to its bit value. Single-bit flags only
/// (composites like HALF/ALNUM/BAD/ALL never appear in the def tables).
fn tag_bit(name: &str) -> u32 {
    match name {
        "NONE" => 0,
        "SPACE" => 1 << 0,
        "SOLID" => 1 << 1,
        "STIPPLE" => 1 << 2,
        "BLOCK" => 1 << 3,
        "BORDER" => 1 << 4,
        "DIAGONAL" => 1 << 5,
        "DOT" => 1 << 6,
        "QUAD" => 1 << 7,
        "HHALF" => 1 << 8,
        "VHALF" => 1 << 9,
        "HALF" => (1 << 8) | (1 << 9),
        "INVERTED" => 1 << 10,
        "BRAILLE" => 1 << 11,
        "TECHNICAL" => 1 << 12,
        "GEOMETRIC" => 1 << 13,
        "ASCII" => 1 << 14,
        "ALPHA" => 1 << 15,
        "DIGIT" => 1 << 16,
        "ALNUM" => (1 << 15) | (1 << 16),
        "NARROW" => 1 << 17,
        "WIDE" => 1 << 18,
        "AMBIGUOUS" => 1 << 19,
        "UGLY" => 1 << 20,
        "LEGACY" => 1 << 21,
        "SEXTANT" => 1 << 22,
        "WEDGE" => 1 << 23,
        "LATIN" => 1 << 24,
        "IMPORTED" => 1 << 25,
        "OCTANT" => 1 << 26,
        "EXTRA" => 1 << 30,
        other => panic!("unknown tag CHAFA_SYMBOL_TAG_{other}"),
    }
}

/// Strip C `/* ... */` block comments.
fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < b.len() {
        if i + 1 < b.len() && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

/// Pack a coverage slice (`'X'`/space per pixel, row-major, 8 wide) into a u64,
/// MSB-first (pixel index 0 -> bit 63). Matches `coverage_to_bitmap`.
fn coverage_to_bitmap(cov: &[u8; 64]) -> u64 {
    let mut bm = 0u64;
    for (i, &c) in cov.iter().enumerate() {
        if c != 0 {
            bm |= 1u64 << (63 - i);
        }
    }
    bm
}

struct NarrowDef {
    author: u32,
    c: u32,
    bitmap: u64,
}
struct WideDef {
    author: u32,
    c: u32,
    left: u64,
    right: u64,
}

fn parse_file(text: &str, narrow: &mut Vec<NarrowDef>, wide: &mut Vec<WideDef>) {
    let text = strip_comments(text);
    // Each entry contains exactly one OUTLINE macro. Walk the macro sites.
    let bytes = text.as_bytes();
    let mut search = 0usize;
    while let Some(macro_rel) = text[search..].find("CHAFA_SYMBOL_OUTLINE_") {
        let macro_pos = search + macro_rel;
        let is_wide = text[macro_pos..].starts_with("CHAFA_SYMBOL_OUTLINE_16X8");

        // Entry start = the '{' preceding this macro (after any prior '}').
        let brace = text[..macro_pos].rfind('{').expect("entry brace");
        let header = &text[brace..macro_pos]; // "{ TAGS , 0xCP ,"

        // Codepoint: the hex literal in the header.
        let cp = parse_codepoint(header);

        // Author tags: every CHAFA_SYMBOL_TAG_<NAME> in the header.
        let author = parse_tags(header);

        // Outline: concatenated string literals from the '(' after the macro to
        // its matching ')'.
        let open = text[macro_pos..].find('(').unwrap() + macro_pos;
        let close = matching_paren(bytes, open);
        let outline = collect_string_literals(&text[open + 1..close]);

        if is_wide {
            assert_eq!(
                outline.len(),
                128,
                "wide outline for U+{cp:04X} must be 128 chars, got {}",
                outline.len()
            );
            let (left, right) = wide_bitmaps(&outline);
            wide.push(WideDef {
                author,
                c: cp,
                left,
                right,
            });
        } else {
            assert_eq!(
                outline.len(),
                64,
                "narrow outline for U+{cp:04X} must be 64 chars, got {}",
                outline.len()
            );
            let mut cov = [0u8; 64];
            for (i, ch) in outline.iter().enumerate() {
                cov[i] = (*ch == b'X') as u8;
            }
            narrow.push(NarrowDef {
                author,
                c: cp,
                bitmap: coverage_to_bitmap(&cov),
            });
        }

        search = close + 1;
    }
}

fn parse_codepoint(header: &str) -> u32 {
    let pos = header
        .find("0x")
        .or_else(|| header.find("0X"))
        .expect("codepoint");
    let hex: String = header[pos + 2..]
        .chars()
        .take_while(|c| c.is_ascii_hexdigit())
        .collect();
    u32::from_str_radix(&hex, 16).expect("hex codepoint")
}

fn parse_tags(header: &str) -> u32 {
    // Tags appear before the codepoint; restrict to that region.
    let end = header
        .find("0x")
        .or_else(|| header.find("0X"))
        .unwrap_or(header.len());
    let region = &header[..end];
    let mut bits = 0u32;
    let needle = "CHAFA_SYMBOL_TAG_";
    let mut i = 0;
    while let Some(rel) = region[i..].find(needle) {
        let start = i + rel + needle.len();
        let name: String = region[start..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        bits |= tag_bit(&name);
        i = start;
    }
    bits
}

fn matching_paren(bytes: &[u8], open: usize) -> usize {
    let mut depth = 0i32;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return i;
                }
            }
            _ => {}
        }
        i += 1;
    }
    panic!("unmatched paren");
}

/// Concatenate the contents of all `"..."` literals in `s` (handling `\"`).
fn collect_string_literals(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'"' {
            i += 1;
            while i < b.len() && b[i] != b'"' {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                }
                out.push(b[i]);
                i += 1;
            }
            i += 1; // closing quote
        } else {
            i += 1;
        }
    }
    out
}

/// Split a 128-char (16x8) outline into left/right 8x8 bitmaps.
fn wide_bitmaps(outline: &[u8]) -> (u64, u64) {
    let mut left = [0u8; 64];
    let mut right = [0u8; 64];
    for y in 0..8usize {
        for x in 0..8usize {
            left[y * 8 + x] = (outline[y * 16 + x] == b'X') as u8;
            right[y * 8 + x] = (outline[y * 16 + 8 + x] == b'X') as u8;
        }
    }
    (coverage_to_bitmap(&left), coverage_to_bitmap(&right))
}

fn main() {
    let mut args = std::env::args().skip(1);
    let internal_dir = args.next().map(PathBuf::from).unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap();
        PathBuf::from(home).join("p/gh/chafa/chafa/internal")
    });
    let out_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates/chafa-syms-rs/src/symbol/data.rs"));

    let mut narrow = Vec::new();
    let mut wide = Vec::new();
    for f in FILES {
        let path = internal_dir.join(f);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        parse_file(&text, &mut narrow, &mut wide);
    }

    let mut out = String::new();
    out.push_str(
        "// @generated by tools/transcode-symbols — DO NOT EDIT.\n\
         //\n\
         // Parsed from chafa-symbols-{ascii,latin,block,kana,misc-narrow}.h.\n\
         // Each narrow def is (author_tags, codepoint, bitmap); each wide def is\n\
         // (author_tags, codepoint, left_bitmap, right_bitmap). Bitmaps are\n\
         // MSB-first row-major (pixel (x,y) -> bit 63-(y*8+x)).\n\n",
    );
    let _ = write!(
        out,
        "/// (author_tags, codepoint, bitmap)\n\
         pub static NARROW_DEFS: &[(u32, u32, u64)] = &[\n"
    );
    for d in &narrow {
        let _ = writeln!(
            out,
            "    (0x{:08x}, 0x{:04x}, 0x{:016x}),",
            d.author, d.c, d.bitmap
        );
    }
    out.push_str("];\n\n");
    let _ = write!(
        out,
        "/// (author_tags, codepoint, left_bitmap, right_bitmap)\n\
         pub static WIDE_DEFS: &[(u32, u32, u64, u64)] = &[\n"
    );
    for d in &wide {
        let _ = writeln!(
            out,
            "    (0x{:08x}, 0x{:04x}, 0x{:016x}, 0x{:016x}),",
            d.author, d.c, d.left, d.right
        );
    }
    out.push_str("];\n");

    std::fs::write(&out_path, out).unwrap_or_else(|e| panic!("write {}: {e}", out_path.display()));
    eprintln!(
        "wrote {} ({} narrow, {} wide)",
        out_path.display(),
        narrow.len(),
        wide.len()
    );
}
