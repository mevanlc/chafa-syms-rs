//! Phase 3 gate: the compiled default symbol map (selection + dedup + sort)
//! must match chafa's prepared `symbol_map->symbols` exactly — same codepoints
//! in the same `(popcount, codepoint)` order. Run against the deterministic
//! (tiebreak) oracle.

mod support;

use chafa_syms_rs::SymbolMap;
use support::{oracle_available, oracle_dump_symmap};

#[test]
fn default_symbol_map_matches_chafa() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    // Match the *library* default (block+border+space-wide). The chafa CLI's
    // no-flag default additionally removes `inverted` (a CLI-layer tweak,
    // replicated in Phase 8), so we pass the equivalent selector explicitly.
    let (dump_narrow, dump_wide) =
        oracle_dump_symmap(&["-c", "full", "--symbols", "block,border,space-wide"]);

    let mut m = SymbolMap::chafa_default();
    m.prepare();

    assert_eq!(m.n_symbols(), dump_narrow.len(), "narrow symbol-map size");
    assert_eq!(m.n_wide_symbols(), dump_wide.len(), "wide symbol-map size");

    for (i, (mine, d)) in m.symbols().iter().zip(dump_narrow.iter()).enumerate() {
        assert_eq!(
            (mine.c as u32, mine.popcount),
            (d.c, d.popcount),
            "narrow symbol-map entry {i}"
        );
    }
}

/// Compile `selectors` on both sides and require identical compiled narrow maps
/// (codepoints + popcount order).
fn assert_selector_parity(selectors: &str) {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    let (dump_narrow, _) = oracle_dump_symmap(&["-c", "full", "--symbols", selectors]);
    let mut m = SymbolMap::new();
    m.apply_selectors(selectors).unwrap();
    m.prepare();

    assert_eq!(
        m.n_symbols(),
        dump_narrow.len(),
        "[{selectors}] symbol-map size"
    );
    for (i, (mine, d)) in m.symbols().iter().zip(dump_narrow.iter()).enumerate() {
        assert_eq!(
            (mine.c as u32, mine.popcount),
            (d.c, d.popcount),
            "[{selectors}] symbol-map entry {i}"
        );
    }
}

#[test]
fn ascii_symbol_map_matches_chafa() {
    assert_selector_parity("ascii");
}

#[test]
fn range_selector_matches_chafa() {
    // A bare range (clear + add) over a builtin-dense span: box drawing +
    // block + geometric. Exercises parse_code_point + SELECTOR_RANGE.
    assert_selector_parity("0x2500..0x259f");
}

#[test]
fn codepoint_and_u_prefix_match_chafa() {
    // Single codepoints with the `0x` and `u` prefixes.
    assert_selector_parity("0x2588 0x2580 u2584");
}

#[test]
fn literal_set_matches_chafa() {
    // `[...]` literal set (each char added as a c..c range).
    assert_selector_parity("[ABCabc0123#@]");
}

#[test]
fn tag_plus_range_remove_matches_chafa() {
    // Tag add then range remove: block minus the full block + braille range.
    assert_selector_parity("block-0x2588-0x2800..0x28ff");
}

#[test]
fn wide_range_with_exclusions_matches_chafa() {
    // A broad range spanning RTL (Hebrew/Arabic) and combining-mark codepoints.
    // No *builtin* falls in those excluded ranges, so this both stresses the
    // range path and confirms membership still matches chafa exactly.
    assert_selector_parity("0..0x2fff");
}
