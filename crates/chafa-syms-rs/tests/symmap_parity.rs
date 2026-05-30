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

#[test]
fn ascii_symbol_map_matches_chafa() {
    if !oracle_available() {
        eprintln!("SKIP: oracle binary not found");
        return;
    }
    // A different selector set, to exercise char_is_selected + sort more widely.
    let (dump_narrow, _) = oracle_dump_symmap(&["-c", "full", "--symbols", "ascii"]);

    let mut m = SymbolMap::new();
    m.apply_selectors("ascii").unwrap();
    m.prepare();

    assert_eq!(m.n_symbols(), dump_narrow.len(), "ascii symbol-map size");
    for (i, (mine, d)) in m.symbols().iter().zip(dump_narrow.iter()).enumerate() {
        assert_eq!(
            (mine.c as u32, mine.popcount),
            (d.c, d.popcount),
            "ascii symbol-map entry {i}"
        );
    }
}
