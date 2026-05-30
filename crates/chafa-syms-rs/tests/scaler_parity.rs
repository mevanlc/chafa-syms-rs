//! Bit-exact parity gate for the smolscale scalar port (`smolscale::scale_rgba8`).
//!
//! The reference is an **isolated** golden harness: chafa's own
//! `smolscale.c` + `smolscale-generic.c` compiled with no SIMD, exposing
//! `smol_scale_simple(RGBA8_UNASSOCIATED -> RGBA8_UNASSOCIATED, NO_FLAGS)` —
//! exactly the function this module ports. This validates the resampler in
//! isolation from the rest of chafa's pixel pipeline.
//!
//! The harness is built once (cached under the crate's target dir) from the
//! chafa source tree. If neither the built harness nor the sources are
//! available, the test skips gracefully (like the other oracle gates).

use std::path::{Path, PathBuf};
use std::process::Command;

use chafa_syms_rs::smolscale::scale_rgba8;

const HARNESS_C: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include "smolscale.h"
int main(int argc, char **argv) {
    if (argc != 7) return 2;
    int sw=atoi(argv[1]), sh=atoi(argv[2]), dw=atoi(argv[3]), dh=atoi(argv[4]);
    FILE *fi=fopen(argv[5],"rb"); if(!fi) return 1;
    size_t sn=(size_t)sw*sh*4, dn=(size_t)dw*dh*4;
    unsigned char *src=malloc(sn), *dst=malloc(dn);
    if (fread(src,1,sn,fi)!=sn) return 1; fclose(fi);
    smol_scale_simple(src, SMOL_PIXEL_RGBA8_UNASSOCIATED, sw, sh, sw*4,
                      dst, SMOL_PIXEL_RGBA8_UNASSOCIATED, dw, dh, dw*4,
                      SMOL_NO_FLAGS);
    FILE *fo=fopen(argv[6],"wb"); if(!fo) return 1;
    fwrite(dst,1,dn,fo); fclose(fo);
    return 0;
}
"#;

/// Locate the chafa smolscale source directory.
fn smolscale_src_dir() -> Option<PathBuf> {
    if let Ok(d) = std::env::var("CHAFA_SMOLSCALE_DIR") {
        let p = PathBuf::from(d);
        if p.join("smolscale.c").is_file() {
            return Some(p);
        }
    }
    let home = std::env::var("HOME").ok()?;
    let p = PathBuf::from(home).join("p/gh/chafa/chafa/internal/smolscale");
    if p.join("smolscale.c").is_file() {
        Some(p)
    } else {
        None
    }
}

/// Build (or reuse) the golden-reference harness; returns its path or `None`.
fn ensure_harness() -> Option<PathBuf> {
    let out_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let harness = out_dir.join("smolref_harness");
    let src_dir = smolscale_src_dir()?;

    // Rebuild if missing or older than the sources.
    let needs_build = !harness.is_file()
        || newer(&src_dir.join("smolscale.c"), &harness)
        || newer(&src_dir.join("smolscale-generic.c"), &harness);

    if needs_build {
        let c_path = out_dir.join("smolref_harness.c");
        std::fs::write(&c_path, HARNESS_C).ok()?;
        let status = Command::new("cc")
            .args(["-O2", "-I"])
            .arg(&src_dir)
            .arg("-o")
            .arg(&harness)
            .arg(&c_path)
            .arg(src_dir.join("smolscale.c"))
            .arg(src_dir.join("smolscale-generic.c"))
            .status()
            .ok()?;
        if !status.success() {
            eprintln!("SKIP: failed to compile smolscale golden harness");
            return None;
        }
    }
    Some(harness)
}

fn newer(a: &Path, b: &Path) -> bool {
    match (
        a.metadata().and_then(|m| m.modified()),
        b.metadata().and_then(|m| m.modified()),
    ) {
        (Ok(ta), Ok(tb)) => ta > tb,
        _ => true,
    }
}

/// Run the harness to produce the reference output for `src`.
fn oracle_scale(harness: &Path, src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let inf = dir.join("smolref_in.bin");
    let outf = dir.join("smolref_out.bin");
    std::fs::write(&inf, src).unwrap();
    let status = Command::new(harness)
        .args([
            sw.to_string(),
            sh.to_string(),
            dw.to_string(),
            dh.to_string(),
        ])
        .arg(&inf)
        .arg(&outf)
        .status()
        .expect("run harness");
    assert!(status.success(), "harness failed for {sw}x{sh}->{dw}x{dh}");
    std::fs::read(&outf).unwrap()
}

/// Deterministic test image with structured high-contrast edges and varying
/// alpha — the conditions under which linear-light premultiplied scaling
/// diverges from naive sRGB resampling.
fn make_image(sw: usize, sh: usize, seed: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(sw * sh * 4);
    let mut lcg = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    for y in 0..sh {
        for x in 0..sw {
            lcg = lcg.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let n = (lcg >> 24) as u8;
            // High-contrast checker plus a noisy gradient.
            let checker = if (x / 2 + y / 2) % 2 == 0 { 0xff } else { 0x00 };
            let r = checker ^ (n & 0x80);
            let g = ((x * 9 + y * 5) as u8).wrapping_add(n >> 3);
            let b = if (x + y) % 3 == 0 { 0x10 } else { 0xe0 };
            // Alpha: mostly opaque, some transparent and partial pixels.
            let a = match (x + y * 3 + (n as usize >> 5)) % 7 {
                0 => 0x00,
                1 => 0x40,
                2 => 0xc0,
                _ => 0xff,
            };
            buf.extend_from_slice(&[r, g, b, a]);
        }
    }
    buf
}

#[test]
fn scaler_matches_smolscale_golden() {
    let harness = match ensure_harness() {
        Some(h) => h,
        None => {
            eprintln!("SKIP: smolscale golden harness unavailable (set CHAFA_SMOLSCALE_DIR)");
            return;
        }
    };

    // Cover every filter path on each axis:
    //   copy  (src == dest)
    //   one   (src dim == 1)
    //   bilinear magnify (dest > src)
    //   bilinear + halvings (src a few× dest)
    //   box   (src > dest*8)
    // plus mixed per-axis combinations and awkward odd sizes.
    let cases: &[(usize, usize, usize, usize)] = &[
        (64, 64, 64, 64),   // copy / copy
        (1, 1, 1, 1),       // 1x1 identity
        (1, 32, 16, 16),    // one (h) + downscale (v)
        (32, 1, 16, 16),    // downscale (h) + one (v)
        (10, 10, 40, 40),   // magnify both
        (40, 40, 10, 10),   // bilinear+halvings both
        (37, 41, 8, 5),     // odd downscale
        (8, 5, 37, 41),     // odd upscale
        (200, 3, 5, 3),     // box (h) huge downscale
        (3, 200, 3, 5),     // box (v) huge downscale
        (512, 7, 3, 3),     // box past dest*255 (h)
        (100, 100, 24, 14), // typical canvas-ish reduction
        (17, 9, 80, 24),    // mixed up/box
        (256, 256, 1, 1),   // reduce to a single pixel
        (5, 5, 1, 7),       // tiny -> thin strip
        (63, 65, 64, 64),   // near-equal, non-copy
    ];

    for &(sw, sh, dw, dh) in cases {
        for seed in [1u32, 7, 99] {
            let src = make_image(sw, sh, seed);
            let want = oracle_scale(&harness, &src, sw, sh, dw, dh);
            let got = scale_rgba8(&src, sw, sh, dw, dh);
            assert_eq!(got.len(), want.len(), "size mismatch {sw}x{sh}->{dw}x{dh}");
            if got != want {
                // Report the first differing pixel for diagnosis.
                let mut first = None;
                for (i, (a, b)) in got.iter().zip(want.iter()).enumerate() {
                    if a != b {
                        first = Some((i / 4, i % 4, *a, *b));
                        break;
                    }
                }
                panic!(
                    "scaler mismatch {sw}x{sh}->{dw}x{dh} seed={seed}: \
                     first diff at pixel {:?} (channel,got,want); {} of {} bytes differ",
                    first,
                    got.iter().zip(&want).filter(|(a, b)| a != b).count(),
                    got.len()
                );
            }
        }
    }
}
