//! Phase 2 gate: the built symbol arrays must match chafa's fully-initialized
//! `chafa_symbols` / `chafa_symbols2` arrays index-for-index — codepoint, tags,
//! popcount and bitmap. This proves the parsed outlines, the ported
//! braille/sextant/octant generators, and the tag derivation all reproduce
//! chafa exactly, neutralizing the GLib-Unicode parity risk for builtins.

mod support;

use chafa_syms_rs::symbol::{builtin_narrow, builtin_wide};
use support::{oracle_available, oracle_dump_symbols};

#[test]
fn narrow_symbols_match_chafa_exactly() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (dump, _) = oracle_dump_symbols();
    let mine = builtin_narrow();

    assert_eq!(mine.len(), dump.len(), "narrow symbol count");

    let mut mismatches = 0;
    for (i, (m, d)) in mine.iter().zip(dump.iter()).enumerate() {
        if m.c as u32 != d.c
            || m.tags.bits() != d.sc
            || m.popcount != d.popcount
            || m.bitmap != d.bitmap
        {
            if mismatches < 20 {
                eprintln!(
                    "narrow[{i}] MISMATCH\n  mine: c=U+{:04X} sc={:#010x} pc={} bm={:#018x}\n  chafa: c=U+{:04X} sc={:#010x} pc={} bm={:#018x}",
                    m.c as u32, m.tags.bits(), m.popcount, m.bitmap,
                    d.c, d.sc, d.popcount, d.bitmap
                );
            }
            mismatches += 1;
        }
    }
    assert_eq!(mismatches, 0, "{mismatches} narrow symbol mismatches");
}

#[test]
fn wide_symbols_match_chafa_exactly() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (_, dump) = oracle_dump_symbols();
    let mine = builtin_wide();

    assert_eq!(mine.len(), dump.len(), "wide symbol count");

    let mut mismatches = 0;
    for (i, (m, d)) in mine.iter().zip(dump.iter()).enumerate() {
        let ok = m.sym[0].c as u32 == d.c
            && m.sym[0].tags.bits() == d.sc
            && m.sym[0].popcount == d.popcount[0]
            && m.sym[1].popcount == d.popcount[1]
            && m.sym[0].bitmap == d.bitmap[0]
            && m.sym[1].bitmap == d.bitmap[1];
        if !ok {
            if mismatches < 20 {
                eprintln!(
                    "wide[{i}] MISMATCH\n  mine: c=U+{:04X} sc={:#010x} pc=[{},{}] bm=[{:#018x},{:#018x}]\n  chafa: c=U+{:04X} sc={:#010x} pc=[{},{}] bm=[{:#018x},{:#018x}]",
                    m.sym[0].c as u32, m.sym[0].tags.bits(), m.sym[0].popcount, m.sym[1].popcount, m.sym[0].bitmap, m.sym[1].bitmap,
                    d.c, d.sc, d.popcount[0], d.popcount[1], d.bitmap[0], d.bitmap[1]
                );
            }
            mismatches += 1;
        }
    }
    assert_eq!(mismatches, 0, "{mismatches} wide symbol mismatches");
}
