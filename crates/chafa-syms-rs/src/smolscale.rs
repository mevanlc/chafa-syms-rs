//! A faithful, bit-exact port of the **scalar** subset of chafa's bundled
//! [smolscale](https://github.com/hpjansson/chafa) resampler — exactly the path
//! chafa exercises when scaling images for symbol rendering on a
//! little-endian host with no SIMD acceleration.
//!
//! ## Scope
//!
//! Only the single format path chafa uses is ported:
//! `RGBA8_UNASSOCIATED → RGBA8_UNASSOCIATED`. With sRGB
//! linearization enabled (the default), this forces **128bpp** storage,
//! **PREMUL16** alpha and **SRGB_LINEAR** gamma. Upstream uses a reversible
//! gamma-2 approximation and lossless `(alpha + 1)` channel encoding. At
//! reductions beyond 8191x it switches to compressed channels to prevent
//! overflow. Both paths, optional background compositing with source coverage,
//! and low-work nearest-neighbor sampling are ported here.
//!
//! We implement a pure **stretch** (resize) — placement covers the whole
//! destination with no offset — which is exactly `smol_scale_simple`'s
//! semantics. That eliminates the clear / clip and
//! edge-opacity machinery (all no-ops when the placement fills the dest), but
//! every *filter* is ported faithfully (copy / one / nearest / bilinear-0h..3h / box,
//! both axes), since which one runs depends on the per-axis scale ratio.
//!
//! ### Endianness note
//!
//! smolscale fetches whole pixels as `u32`. On little-endian it remaps
//! `RGBA8_*` to the reversed-byte `ABGR8_*` channel order so the generic
//! integer logic stays correct. Because our source and destination pixel types
//! are identical, the unpack→pack channel permutation cancels and every color
//! lane receives identical treatment (alpha stays in its dedicated lane). We
//! therefore use chafa's `1234` unpack/pack pair with big-endian byte framing
//! (`from_be_bytes`/`to_be_bytes`), which keeps `R,G,B,A` in place and is
//! provably bit-identical to chafa's little-endian `ABGR` run. The identity
//! (COPY) roundtrip is the direct check of this claim.
//!
//! All kernel arithmetic uses wrapping operations to match C's defined
//! unsigned-overflow (and intentional underflow, e.g. `(p - q)` in the bilinear
//! lerp) semantics.

#![allow(clippy::needless_range_loop)]

use crate::geometry::pixel_count;
use crate::smolscale_luts::{FROM_SRGB, INV_DIV_P16, INV_DIV_P16L, TO_SRGB};

// ---------------------------------------------------------------------------
// Constants (smolscale.h / smolscale-private.h)
// ---------------------------------------------------------------------------

const SUBPIXEL_SHIFT: u32 = 8;
const SUBPIXEL_MUL: u64 = 1 << SUBPIXEL_SHIFT; // 256
const SMALL_MUL: u64 = 256;
const BIG_MUL: u64 = 65536;
const BOXES_MULTIPLIER: u64 = BIG_MUL * SMALL_MUL; // 2^24
const BILIN_MULTIPLIER: u64 = BIG_MUL * BIG_MUL; // 2^32
const INVERTED_DIV_SHIFT_P16L: u32 = 30 - 11; // 19

/// 24-bit-per-32-bit-lane mask used throughout the 128bpp kernels.
const MASK24: u64 = 0x00ff_ffff_00ff_ffff;

#[inline]
fn spx_to_px(spx: u64) -> u64 {
    spx.div_ceil(SUBPIXEL_MUL)
}

/// `SMOL_SUBPIXEL_MOD(n)` with C's wrap-into-range behavior, on a signed input.
#[inline]
fn subpixel_mod(n: i64) -> i64 {
    ((n % SUBPIXEL_MUL as i64) + SUBPIXEL_MUL as i64) % SUBPIXEL_MUL as i64
}

// ---------------------------------------------------------------------------
// Filter selection
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FilterType {
    Copy,
    One,
    Nearest,
    /// Bilinear with `n` box-halvings (`0..=3`).
    Bilinear(u32),
    Box,
}

// ---------------------------------------------------------------------------
// Per-dimension precalc + parameters (SmolDim, stretch-only)
// ---------------------------------------------------------------------------

struct Dim {
    src_size_px: u32,
    src_size_spx: u64,
    dest_size_px: u32,
    // Placement == whole destination (stretch).
    placement_size_px: u32,
    placement_size_spx: u64,
    placement_size_prehalving_px: u32,
    placement_size_prehalving_spx: u64,
    n_halvings: u32,
    filter_type: FilterType,
    first_opacity: u16,
    last_opacity: u16,

    /// Bilinear precalc: pairs of `(pixel_ofs, fraction)` as `u16`.
    precalc_bilin: Vec<u16>,
    /// Box precalc: one `u32` per destination sample.
    precalc_box: Vec<u32>,
    span_step: u32,
    span_mul: u32,
}

impl Dim {
    /// Build a dimension for a pure stretch from `src_px` to `dest_px`.
    fn new(src_px: u32, dest_px: u32, nearest: bool) -> Dim {
        let src_size_spx = src_px as u64 * SUBPIXEL_MUL;
        let dest_size_spx = dest_px as u64 * SUBPIXEL_MUL;
        // Placement: offset 0, size == dest.
        let placement_ofs_spx: i64 = 0;
        let placement_size_px = dest_px;
        let placement_size_spx = dest_size_spx;

        let mut dim = Dim {
            src_size_px: src_px,
            src_size_spx,
            dest_size_px: dest_px,
            placement_size_px,
            placement_size_spx,
            placement_size_prehalving_px: placement_size_px,
            placement_size_prehalving_spx: placement_size_spx,
            n_halvings: 0,
            filter_type: FilterType::Copy,
            first_opacity: 256,
            last_opacity: 256,
            precalc_bilin: Vec::new(),
            precalc_box: Vec::new(),
            span_step: 0,
            span_mul: 0,
        };

        dim.pick_filter_params(placement_ofs_spx);
        if nearest && !matches!(dim.filter_type, FilterType::Copy | FilterType::One) {
            dim.filter_type = FilterType::Nearest;
        }
        dim.init_precalc(placement_ofs_spx);
        dim
    }

    /// Port of `pick_filter_params` (stretch: `dest_dim == placement_size`).
    fn pick_filter_params(&mut self, dest_ofs_spx: i64) {
        let src_dim = self.src_size_px;
        let src_dim_spx = self.src_size_spx;
        let dest_dim = self.placement_size_px;
        let dest_dim_spx = self.placement_size_spx;

        self.placement_size_prehalving_px = dest_dim;

        self.first_opacity = (subpixel_mod(-dest_ofs_spx - 1) + 1) as u16;
        self.last_opacity = (subpixel_mod(dest_ofs_spx + dest_dim_spx as i64 - 1) + 1) as u16;

        if dest_dim == 1 {
            self.first_opacity = dest_dim_spx as u16;
            self.last_opacity = 256;
        }

        // chafa splits this into `> dest*255` (which also forces 128bpp storage)
        // and `>= dest*14`; both pick BOX, and storage is always 128bpp on this
        // path, so the two cases collapse.
        if src_dim as u64 >= dest_dim as u64 * 14 {
            self.filter_type = FilterType::Box;
        } else if src_dim <= 1 {
            self.filter_type = FilterType::One;
            self.last_opacity =
                (((dest_ofs_spx + dest_dim_spx as i64 - 1) % SUBPIXEL_MUL as i64) + 1) as u16;
        } else if (dest_ofs_spx & 0xff) == 0 && src_dim_spx == dest_dim_spx {
            self.filter_type = FilterType::Copy;
            self.first_opacity = 256;
            self.last_opacity = 256;
        } else {
            let mut n_halvings = 0u32;
            let mut d = dest_dim_spx;
            loop {
                d *= 2;
                if d >= src_dim_spx {
                    break;
                }
                n_halvings += 1;
            }
            self.placement_size_prehalving_px = dest_dim << n_halvings;
            self.placement_size_prehalving_spx = dest_dim_spx << n_halvings;
            self.filter_type = FilterType::Bilinear(n_halvings);
            self.n_halvings = n_halvings;
        }
    }

    /// Port of `init_dim` (the precalc population in smolscale-generic.c).
    fn init_precalc(&mut self, placement_ofs_spx: i64) {
        match self.filter_type {
            FilterType::Copy | FilterType::One => {}
            FilterType::Nearest => {
                self.precalc_bilin = (0..self.dest_size_px)
                    .map(|i| {
                        (((u64::from(i) * 2 + 1) * u64::from(self.src_size_px))
                            / (u64::from(self.dest_size_px) * 2))
                            .min(u64::from(self.src_size_px) - 1) as u16
                    })
                    .collect();
            }
            FilterType::Box => {
                self.precalc_box = vec![0u32; self.placement_size_px as usize + 1];
                precalc_boxes_array(
                    &mut self.precalc_box,
                    &mut self.span_step,
                    &mut self.span_mul,
                    self.src_size_spx,
                    self.placement_size_px as i64,
                    placement_ofs_spx as u64,
                    self.placement_size_spx,
                    0, // clip_before_px
                );
            }
            FilterType::Bilinear(_) => {
                self.precalc_bilin =
                    vec![0u16; (self.placement_size_prehalving_px as usize + 1) * 2];
                precalc_bilinear_array(
                    &mut self.precalc_bilin,
                    self.src_size_spx,
                    placement_ofs_spx as u64,
                    self.placement_size_prehalving_spx,
                    self.placement_size_prehalving_px,
                    self.n_halvings,
                    0, // clip_before_px
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Precalc (smolscale-generic.c)
// ---------------------------------------------------------------------------

/// Port of `precalc_linear_range`.
#[allow(clippy::too_many_arguments)]
fn precalc_linear_range(
    array_out: &mut [u16],
    first_index: i32,
    last_index: i32,
    first_sample_ofs: u64,
    sample_step: u64,
    sample_ofs_px_max: i32,
    dest_clip_before_px: i32,
    array_i: &mut usize,
) {
    let mut sample_ofs = first_sample_ofs;

    for i in first_index..last_index {
        let sample_ofs_px = (sample_ofs / BILIN_MULTIPLIER) as i64; // u16 in C

        if sample_ofs_px >= sample_ofs_px_max as i64 - 1 {
            if i >= dest_clip_before_px {
                array_out[*array_i * 2] = (sample_ofs_px_max - 2) as u16;
                array_out[*array_i * 2 + 1] = 0;
                *array_i += 1;
            }
            continue;
        }

        if i >= dest_clip_before_px {
            array_out[*array_i * 2] = sample_ofs_px as u16;
            array_out[*array_i * 2 + 1] =
                (SMALL_MUL - ((sample_ofs / (BILIN_MULTIPLIER / SMALL_MUL)) % SMALL_MUL)) as u16;
            *array_i += 1;
        }

        sample_ofs = sample_ofs.wrapping_add(sample_step);
    }
}

/// Port of `precalc_bilinear_array`.
fn precalc_bilinear_array(
    array: &mut [u16],
    src_dim_spx: u64,
    mut dest_ofs_spx: u64,
    dest_dim_spx: u64,
    dest_dim_prehalving_px: u32,
    n_halvings: u32,
    dest_clip_before_px: i32,
) {
    let src_dim_px = spx_to_px(src_dim_spx);
    debug_assert!(src_dim_px > 1);

    dest_ofs_spx %= SUBPIXEL_MUL;

    let mut first_sample_ofs = [0u64; 3];
    let sample_step;

    if src_dim_spx > dest_dim_spx {
        // Minification
        sample_step = (src_dim_spx * BILIN_MULTIPLIER) / dest_dim_spx;
        first_sample_ofs[0] = (sample_step - BILIN_MULTIPLIER) / 2;
        first_sample_ofs[1] = ((sample_step - BILIN_MULTIPLIER) / 2)
            + ((sample_step * (SUBPIXEL_MUL - dest_ofs_spx) * (1u64 << n_halvings)) / SUBPIXEL_MUL);
    } else {
        // Magnification
        sample_step = ((src_dim_spx - SUBPIXEL_MUL) * BILIN_MULTIPLIER)
            / if dest_dim_spx > SUBPIXEL_MUL {
                dest_dim_spx - SUBPIXEL_MUL
            } else {
                1
            };
        first_sample_ofs[0] = 0;
        first_sample_ofs[1] = (sample_step * (SUBPIXEL_MUL - dest_ofs_spx)) / SUBPIXEL_MUL;
    }

    first_sample_ofs[2] = (((src_dim_spx * BILIN_MULTIPLIER * 2) / SUBPIXEL_MUL) + sample_step
        - BILIN_MULTIPLIER)
        / 2
        - sample_step * (1u64 << n_halvings);

    let mut i = 0usize;

    // Left fringe
    precalc_linear_range(
        array,
        0,
        1 << n_halvings,
        first_sample_ofs[0],
        sample_step,
        src_dim_px as i32,
        dest_clip_before_px,
        &mut i,
    );

    // Prevent overruns when the output size is exactly 1
    if dest_dim_prehalving_px > (1u32 << n_halvings) {
        // Main range
        precalc_linear_range(
            array,
            1 << n_halvings,
            (dest_dim_prehalving_px - (1 << n_halvings)) as i32,
            first_sample_ofs[1],
            sample_step,
            src_dim_px as i32,
            dest_clip_before_px,
            &mut i,
        );

        // Right fringe
        precalc_linear_range(
            array,
            (dest_dim_prehalving_px - (1 << n_halvings)) as i32,
            dest_dim_prehalving_px as i32,
            first_sample_ofs[2],
            sample_step,
            src_dim_px as i32,
            dest_clip_before_px,
            &mut i,
        );
    }
}

/// Port of `precalc_boxes_array`.
#[allow(clippy::too_many_arguments)]
fn precalc_boxes_array(
    array: &mut [u32],
    span_step: &mut u32,
    span_mul: &mut u32,
    src_dim_spx: u64,
    dest_dim: i64,
    mut dest_ofs_spx: u64,
    mut dest_dim_spx: u64,
    dest_clip_before_px: i64,
) {
    dest_ofs_spx %= SUBPIXEL_MUL;

    // Output sample can't be less than a pixel.
    if dest_dim_spx < 256 {
        dest_dim_spx = 256;
    }

    let frac_step_f: u64 = (src_dim_spx * BIG_MUL) / dest_dim_spx;

    let stride = frac_step_f / BIG_MUL;
    let f = (frac_step_f / SMALL_MUL) % SMALL_MUL;

    let a = BOXES_MULTIPLIER * 255;
    let b = (stride * 255) + ((f * 255) / 256);
    *span_step = (frac_step_f / SMALL_MUL) as u32;
    // Undershoot: rounding can overflow the alpha lane on very wide spans.
    *span_mul = (a / (b + 1)) as u32;

    // Left fringe
    let mut i = 0usize;
    let mut dest_i: i64 = 0;

    if dest_i >= dest_clip_before_px {
        array[i] = 0;
        i += 1;
    }

    // Main range
    let mut frac_f: u64 = (frac_step_f * (SUBPIXEL_MUL - dest_ofs_spx)) / SUBPIXEL_MUL;
    dest_i = 1;
    while dest_i < dest_dim - 1 {
        if dest_i >= dest_clip_before_px {
            array[i] = (frac_f / SMALL_MUL) as u32;
            i += 1;
        }
        frac_f = frac_f.wrapping_add(frac_step_f);
        dest_i += 1;
    }

    // Right fringe
    if dest_dim > 1 && dest_i >= dest_clip_before_px {
        array[i] = ((src_dim_spx * SMALL_MUL - frac_step_f) / SMALL_MUL) as u32;
    }
}

/// Port of `unpack_box_precalc`. Returns `(ofs0, ofs1, f0, f1, n)`.
#[inline]
fn unpack_box_precalc(precalc: u32, step: u32) -> (u32, u32, u32, u32, u32) {
    let mut ofs0 = precalc;
    let mut ofs1 = ofs0 + step;
    let f0 = 256 - (ofs0 % SUBPIXEL_MUL as u32);
    let f1 = ofs1 % SUBPIXEL_MUL as u32;
    ofs0 /= SUBPIXEL_MUL as u32;
    ofs1 /= SUBPIXEL_MUL as u32;
    let n = ofs1 - ofs0 - 1;
    (ofs0, ofs1, f0, f1, n)
}

// ---------------------------------------------------------------------------
// sRGB / premul helpers (128bpp)
// ---------------------------------------------------------------------------

/// `from_srgb_pixel_xxxa_128bpp`.
#[inline]
fn from_srgb_pixel(pixel: &mut [u64; 2]) {
    let part = pixel[0];
    pixel[0] = ((FROM_SRGB[(part >> 32) as usize] as u64) << 32)
        | FROM_SRGB[(part & 0xff) as usize] as u64;

    let part = pixel[1];
    pixel[1] =
        ((FROM_SRGB[(part >> 32) as usize] as u64) << 32) | ((part & 0xffff_ffff) << 8) | 0xff;
}

/// `to_srgb_pixel_xxxa_128bpp`.
#[inline]
fn to_srgb_pixel(pixel_in: &[u64; 2], pixel_out: &mut [u64; 2]) {
    pixel_out[0] = ((TO_SRGB[(pixel_in[0] >> 32) as usize] as u64) << 32)
        | TO_SRGB[(pixel_in[0] & 0xffff) as usize] as u64;
    pixel_out[1] =
        ((TO_SRGB[(pixel_in[1] >> 32) as usize] as u64) << 32) | (pixel_in[1] & 0xffff_ffff);
}

/// `unpremul_p16l_to_ul_128bpp`.
#[inline]
fn unpremul_pixel(input: &[u64; 2], out: &mut [u64; 2], alpha: u8, linear: bool) {
    let (m, shift, mask) = if linear {
        (
            INV_DIV_P16L[alpha as usize],
            INVERTED_DIV_SHIFT_P16L,
            0x0000_07ff_0000_07ff,
        )
    } else {
        (INV_DIV_P16[alpha as usize], 16, 0x0000_00ff_0000_00ff)
    };
    out[0] = (input[0].wrapping_mul(u64::from(m)) >> shift) & mask;
    out[1] = (input[1].wrapping_mul(u64::from(m)) >> shift) & mask;
}

// ---------------------------------------------------------------------------
// Repacking (the single RGBA8-unassoc <-> 128bpp p16l path)
// ---------------------------------------------------------------------------

/// Unpack one `RGBA8_UNASSOCIATED` pixel (read as big-endian `R,G,B,A`) into
/// 128bpp PREMUL16 / SRGB_LINEAR storage. Port of
/// `unpack_pixel_123a_u_to_123a_p16l_128bpp`.
#[inline]
fn unpack_pixel(p: u32, out: &mut [u64; 2], linear: bool) {
    let p64 = p as u64;
    let alpha = (p & 0xff) as u8;

    out[0] = ((p64 & 0xff00_0000) << 8) | ((p64 & 0x00ff_0000) >> 16);
    out[1] = (p64 & 0x0000_ff00) << 24;

    if linear {
        from_srgb_pixel(out);
    }
    // Lossless 19-bit linear channels, encoded with (alpha + 1).
    out[0] = out[0].wrapping_mul(alpha as u64 + 1);
    out[1] = out[1].wrapping_mul(alpha as u64 + 1);

    out[1] = (out[1] & 0xffff_ffff_0000_0000) | ((alpha as u64) << 8) | 0xff;
}

/// Unpack a whole source row of `n` pixels (big-endian framed).
fn unpack_row(src: &[u8], dest: &mut [u64], n: usize, linear: bool) {
    for i in 0..n {
        let b = &src[i * 4..i * 4 + 4];
        let p = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
        let mut px = [0u64; 2];
        unpack_pixel(p, &mut px, linear);
        dest[i * 2] = px[0];
        dest[i * 2 + 1] = px[1];
    }
}

/// `PACK_FROM_1234_128BPP(in, 1, 2, 3, 4)`.
#[inline]
fn pack_1234(t: &[u64; 2]) -> u32 {
    (((t[0] >> 8) & 0xff00_0000)
        | ((t[0] << 16) & 0x00ff_0000)
        | ((t[1] >> 24) & 0x0000_ff00)
        | (t[1] & 0x0000_00ff)) as u32
}

/// Pack a finished 128bpp p16l/linear row back to `RGBA8_UNASSOCIATED`
/// (written big-endian, keeping `R,G,B,A`). Port of the
/// `PREMUL16,LINEAR -> 1234,UNASSOCIATED,COMPRESSED` repack.
fn pack_row(src: &[u64], dest: &mut [u8], n: usize, linear: bool) {
    for i in 0..n {
        let in_px = [src[i * 2], src[i * 2 + 1]];
        let alpha = (in_px[1] >> 8) as u8;
        let mut t = [0u64; 2];
        unpremul_pixel(&in_px, &mut t, alpha, linear);
        if linear {
            let t_in = t;
            to_srgb_pixel(&t_in, &mut t);
        }
        t[1] = (t[1] & 0xffff_ffff_0000_0000) | alpha as u64;
        let packed = pack_1234(&t);
        dest[i * 4..i * 4 + 4].copy_from_slice(&packed.to_be_bytes());
    }
}

// ---------------------------------------------------------------------------
// Filter math helpers (128bpp)
// ---------------------------------------------------------------------------

/// `weight_pixel_128bpp`.
#[inline]
fn weight_pixel(p: &[u64], out: &mut [u64], w: u64) {
    out[0] = (p[0].wrapping_mul(w) >> 8) & MASK24;
    out[1] = (p[1].wrapping_mul(w) >> 8) & MASK24;
}

/// `scale_128bpp_half`.
#[inline]
fn scale_128bpp_half(accum: u64, multiplier: u64) -> u64 {
    let mut a = accum & 0x0000_0000_ffff_ffff;
    a = (a
        .wrapping_mul(multiplier)
        .wrapping_add(BOXES_MULTIPLIER / 2))
        / BOXES_MULTIPLIER;

    let mut b = (accum & 0xffff_ffff_0000_0000) >> 32;
    b = (b
        .wrapping_mul(multiplier)
        .wrapping_add(BOXES_MULTIPLIER / 2))
        / BOXES_MULTIPLIER;

    a | (b << 32)
}

/// `apply_subpixel_opacity_128bpp_half`.
#[inline]
fn apply_subpixel_opacity_half(v: u64, opacity: u16) -> u64 {
    (v.wrapping_mul(opacity as u64) >> SUBPIXEL_SHIFT) & MASK24
}

// ---------------------------------------------------------------------------
// Scale context
// ---------------------------------------------------------------------------

struct ScaleCtx<'a> {
    src_pixels: &'a [u8],
    src_rowstride: usize,
    hdim: Dim,
    vdim: Dim,
    background: Option<[u64; 2]>,
    linear: bool,
}

/// Per-batch scratch ring (`SmolLocalCtx`), 128bpp (2 u64/pixel).
struct LocalCtx {
    rows: [Vec<u64>; 4],
    src_ofs: u32,
}

impl LocalCtx {
    fn new(row_len: usize) -> LocalCtx {
        LocalCtx {
            rows: [
                vec![0u64; row_len],
                vec![0u64; row_len],
                vec![0u64; row_len],
                vec![0u64; row_len],
            ],
            // Must be one less than UINT_MAX so the `src_ofs + 1` test in
            // update_local_ctx_bilinear can't wrap around.
            src_ofs: u32::MAX - 1,
        }
    }
}

impl ScaleCtx<'_> {
    #[inline]
    fn src_row(&self, row: u32) -> &[u8] {
        let ofs = self.src_rowstride * row as usize;
        &self.src_pixels[ofs..]
    }

    // -- Horizontal filters -------------------------------------------------

    /// `interp_horizontal_bilinear_{0..3}h_128bpp`, unified over `n_halvings`.
    fn hfilter_bilinear(&self, src: &[u64], dest: &mut [u64], n_halvings: u32) {
        let precalc = &self.hdim.precalc_bilin;
        let width = self.hdim.placement_size_px as usize;
        let mut pi = 0usize;
        for di in 0..width {
            let mut accum = [0u64; 2];
            for _ in 0..(1u32 << n_halvings) {
                let pixel_ofs = precalc[pi] as usize * 2;
                let f = precalc[pi + 1] as u64;
                pi += 2;

                let p = src[pixel_ofs];
                let q = src[pixel_ofs + 2];
                accum[0] = accum[0].wrapping_add(
                    ((p.wrapping_sub(q).wrapping_mul(f)) >> 8).wrapping_add(q) & MASK24,
                );

                let p = src[pixel_ofs + 1];
                let q = src[pixel_ofs + 3];
                accum[1] = accum[1].wrapping_add(
                    ((p.wrapping_sub(q).wrapping_mul(f)) >> 8).wrapping_add(q) & MASK24,
                );
            }
            dest[di * 2] = (accum[0] >> n_halvings) & MASK24;
            dest[di * 2 + 1] = (accum[1] >> n_halvings) & MASK24;
        }
    }

    /// `interp_horizontal_boxes_128bpp`.
    fn hfilter_box(&self, src: &[u64], dest: &mut [u64]) {
        let precalc = &self.hdim.precalc_box;
        let span_step = self.hdim.span_step;
        let span_mul = self.hdim.span_mul as u64;
        let width = self.hdim.placement_size_px as usize;

        for di in 0..width {
            let (ofs0, _ofs1, f0, f1, n) = unpack_box_precalc(precalc[di], span_step);
            let mut base = ofs0 as usize * 2;

            let mut accum = [0u64; 2];
            weight_pixel(&src[base..base + 2], &mut accum, f0 as u64);
            base += 2;

            for _ in 0..n {
                accum[0] = accum[0].wrapping_add(src[base]);
                accum[1] = accum[1].wrapping_add(src[base + 1]);
                base += 2;
            }

            let mut t = [0u64; 2];
            weight_pixel(&src[base..base + 2], &mut t, f1 as u64);
            accum[0] = accum[0].wrapping_add(t[0]);
            accum[1] = accum[1].wrapping_add(t[1]);

            dest[di * 2] = scale_128bpp_half(accum[0], span_mul);
            dest[di * 2 + 1] = scale_128bpp_half(accum[1], span_mul);
        }
    }

    /// `interp_horizontal_one_128bpp`.
    fn hfilter_one(&self, src: &[u64], dest: &mut [u64]) {
        let width = self.hdim.placement_size_px as usize;
        let (a, b) = (src[0], src[1]);
        for di in 0..width {
            dest[di * 2] = a;
            dest[di * 2 + 1] = b;
        }
    }

    /// `scale_horizontal`: unpack `src_row` into the scratch row, run the
    /// horizontal filter into `dest_idx`, then apply edge opacity.
    fn scale_horizontal(&self, local: &mut LocalCtx, src_row_index: u32, dest_idx: usize) {
        debug_assert!(dest_idx != 3);
        let src = self.src_row(src_row_index);
        if self.hdim.filter_type == FilterType::Copy {
            // No filtering or edge opacity is needed: unpack directly into
            // the output row, bypassing the scratch-row copy.
            unpack_row(
                src,
                &mut local.rows[dest_idx],
                self.hdim.src_size_px as usize,
                self.linear,
            );
            return;
        }
        unpack_row(
            src,
            &mut local.rows[3],
            self.hdim.src_size_px as usize,
            self.linear,
        );

        let mut dest = std::mem::take(&mut local.rows[dest_idx]);
        match self.hdim.filter_type {
            FilterType::Copy => unreachable!("copy handled while unpacking"),
            FilterType::One => self.hfilter_one(&local.rows[3], &mut dest),
            FilterType::Nearest => {
                for (i, &offset) in self.hdim.precalc_bilin.iter().enumerate() {
                    let offset = usize::from(offset) * 2;
                    dest[i * 2..i * 2 + 2].copy_from_slice(&local.rows[3][offset..offset + 2]);
                }
            }
            FilterType::Bilinear(n) => self.hfilter_bilinear(&local.rows[3], &mut dest, n),
            FilterType::Box => self.hfilter_box(&local.rows[3], &mut dest),
        }
        self.apply_horiz_edge_opacity(&mut dest);
        local.rows[dest_idx] = dest;
    }

    /// `apply_horiz_edge_opacity` (128bpp). No-op when opacities are 256.
    fn apply_horiz_edge_opacity(&self, row: &mut [u64]) {
        let first = self.hdim.first_opacity;
        let last = self.hdim.last_opacity;
        if first != 256 {
            row[0] = apply_subpixel_opacity_half(row[0], first);
            row[1] = apply_subpixel_opacity_half(row[1], first);
        }
        if last != 256 {
            let i = (self.hdim.placement_size_px as usize - 1) * 2;
            row[i] = apply_subpixel_opacity_half(row[i], last);
            row[i + 1] = apply_subpixel_opacity_half(row[i + 1], last);
        }
    }

    // -- Vertical driving ---------------------------------------------------

    /// `update_local_ctx_bilinear`: ensure rows[0]/rows[1] hold the
    /// horizontally-scaled source rows bracketing `dest_row_index`.
    fn update_local_ctx_bilinear(&self, local: &mut LocalCtx, dest_row_index: u32) {
        let new_src_ofs = self.vdim.precalc_bilin[dest_row_index as usize * 2] as u32;

        if new_src_ofs == local.src_ofs {
            return;
        }

        if new_src_ofs == local.src_ofs.wrapping_add(1) {
            local.rows.swap(0, 1);
            self.scale_horizontal(local, new_src_ofs + 1, 1);
        } else {
            self.scale_horizontal(local, new_src_ofs, 0);
            self.scale_horizontal(local, new_src_ofs + 1, 1);
        }

        local.src_ofs = new_src_ofs;
    }

    /// Bilinear vertical scaling for one destination row. Returns the index of
    /// the parts row holding the finished pixels (`2`). Unified over
    /// `n_halvings` (`scale_dest_row_bilinear_{0..3}h_128bpp`).
    fn scale_dest_row_bilinear(
        &self,
        local: &mut LocalCtx,
        dest_row_index: u32,
        n_halvings: u32,
    ) -> usize {
        let width = self.hdim.placement_size_px as usize * 2;
        let last_row = self.vdim.placement_size_px - 1;

        if n_halvings == 0 {
            self.update_local_ctx_bilinear(local, dest_row_index);
            let f = self.vdim.precalc_bilin[dest_row_index as usize * 2 + 1] as u64;
            let opacity = self.edge_opacity_v(dest_row_index, last_row);
            self.v_store(local, f, width, opacity);
            return 2;
        }

        let mut bilin_index = dest_row_index << n_halvings;

        // First sub-sample: store.
        self.update_local_ctx_bilinear(local, bilin_index);
        let f = self.vdim.precalc_bilin[bilin_index as usize * 2 + 1] as u64;
        self.v_store(local, f, width, None);
        bilin_index += 1;

        // Middle sub-samples: add.
        for _ in 0..((1u32 << n_halvings) - 2) {
            self.update_local_ctx_bilinear(local, bilin_index);
            let f = self.vdim.precalc_bilin[bilin_index as usize * 2 + 1] as u64;
            self.v_add(local, f, width);
            bilin_index += 1;
        }

        // Final sub-sample: combine + halve.
        self.update_local_ctx_bilinear(local, bilin_index);
        let f = self.vdim.precalc_bilin[bilin_index as usize * 2 + 1] as u64;
        let opacity = self.edge_opacity_v(dest_row_index, last_row);
        self.v_final(local, f, width, n_halvings, opacity);

        2
    }

    /// Opacity to apply for vertical edge rows, or `None` for full opacity.
    fn edge_opacity_v(&self, dest_row_index: u32, last_row: u32) -> Option<u16> {
        if dest_row_index == 0 && self.vdim.first_opacity < 256 {
            Some(self.vdim.first_opacity)
        } else if dest_row_index == last_row && self.vdim.last_opacity < 256 {
            Some(self.vdim.last_opacity)
        } else {
            None
        }
    }

    /// `interp_vertical_bilinear_store[_with_opacity]_128bpp`: rows[0],rows[1] -> rows[2].
    fn v_store(&self, local: &mut LocalCtx, f: u64, width: usize, opacity: Option<u16>) {
        let mut dest = std::mem::take(&mut local.rows[2]);
        let top = &local.rows[0];
        let bot = &local.rows[1];
        for i in 0..width {
            let p = top[i];
            let q = bot[i];
            let mut v = ((p.wrapping_sub(q).wrapping_mul(f)) >> 8).wrapping_add(q) & MASK24;
            if let Some(op) = opacity {
                v = apply_subpixel_opacity_half(v, op);
            }
            dest[i] = v;
        }
        local.rows[2] = dest;
    }

    /// `interp_vertical_bilinear_add_128bpp`: rows[0],rows[1] += into rows[2].
    fn v_add(&self, local: &mut LocalCtx, f: u64, width: usize) {
        let mut dest = std::mem::take(&mut local.rows[2]);
        let top = &local.rows[0];
        let bot = &local.rows[1];
        for i in 0..width {
            let p = top[i];
            let q = bot[i];
            let v = ((p.wrapping_sub(q).wrapping_mul(f)) >> 8).wrapping_add(q) & MASK24;
            dest[i] = dest[i].wrapping_add(v);
        }
        local.rows[2] = dest;
    }

    /// `interp_vertical_bilinear_final_{n}h[_with_opacity]_128bpp`:
    /// combine rows[0],rows[1] with the accumulator rows[2], then `>> n`.
    fn v_final(
        &self,
        local: &mut LocalCtx,
        f: u64,
        width: usize,
        n_halvings: u32,
        opacity: Option<u16>,
    ) {
        let mut dest = std::mem::take(&mut local.rows[2]);
        let top = &local.rows[0];
        let bot = &local.rows[1];
        for i in 0..width {
            let p = top[i];
            let q = bot[i];
            let mut v = ((p.wrapping_sub(q).wrapping_mul(f)) >> 8).wrapping_add(q) & MASK24;
            v = ((v.wrapping_add(dest[i])) >> n_halvings) & MASK24;
            if let Some(op) = opacity {
                v = apply_subpixel_opacity_half(v, op);
            }
            dest[i] = v;
        }
        local.rows[2] = dest;
    }

    /// `scale_dest_row_box_128bpp`. Returns parts-row index `2`.
    fn scale_dest_row_box(&self, local: &mut LocalCtx, dest_row_index: u32) -> usize {
        let (mut ofs_y, _ofs_y_max, w1, w2, n) = unpack_box_precalc(
            self.vdim.precalc_box[dest_row_index as usize],
            self.vdim.span_step,
        );
        let width_px = self.hdim.placement_size_px as usize;
        let width = width_px * 2;

        // First input row -> weighted into rows[1].
        if ofs_y != local.src_ofs {
            self.scale_horizontal(local, ofs_y, 0);
            local.src_ofs = ofs_y;
        }
        {
            let mut acc = std::mem::take(&mut local.rows[1]);
            let row0 = &local.rows[0];
            for i in 0..width {
                acc[i] = (row0[i].wrapping_mul(w1 as u64) >> 8) & MASK24;
            }
            local.rows[1] = acc;
        }
        ofs_y += 1;

        // Whole input rows -> add into rows[1].
        for _ in 0..n {
            self.scale_horizontal(local, ofs_y, 0);
            local.src_ofs = ofs_y;
            let mut acc = std::mem::take(&mut local.rows[1]);
            let row0 = &local.rows[0];
            for i in 0..width {
                acc[i] = acc[i].wrapping_add(row0[i]);
            }
            local.rows[1] = acc;
            ofs_y += 1;
        }

        // Last input row -> weighted add into rows[1].
        if w2 > 0 && ofs_y < self.vdim.src_size_px {
            self.scale_horizontal(local, ofs_y, 0);
            local.src_ofs = ofs_y;
            let mut acc = std::mem::take(&mut local.rows[1]);
            let row0 = &local.rows[0];
            for px in 0..width_px {
                let mut t = [0u64; 2];
                weight_pixel(&row0[px * 2..px * 2 + 2], &mut t, w2 as u64);
                acc[px * 2] = acc[px * 2].wrapping_add(t[0]);
                acc[px * 2 + 1] = acc[px * 2 + 1].wrapping_add(t[1]);
            }
            local.rows[1] = acc;
        }

        // Keep the boundary source row in rows[0] for the next output span.
        // Finalize rows[1] -> rows[2], which can then be composited in place.
        let span_mul = self.vdim.span_mul as u64;
        let opacity = self.edge_opacity_v(dest_row_index, self.vdim.placement_size_px - 1);
        {
            let mut dest = std::mem::take(&mut local.rows[2]);
            let acc = &local.rows[1];
            match opacity {
                None => {
                    for i in 0..width {
                        dest[i] = scale_128bpp_half(acc[i], span_mul);
                    }
                }
                Some(op) => {
                    for px in 0..width_px {
                        dest[px * 2] = scale_128bpp_half(acc[px * 2], span_mul);
                        dest[px * 2 + 1] = scale_128bpp_half(acc[px * 2 + 1], span_mul);
                        dest[px * 2] = apply_subpixel_opacity_half(dest[px * 2], op);
                        dest[px * 2 + 1] = apply_subpixel_opacity_half(dest[px * 2 + 1], op);
                    }
                }
            }
            local.rows[2] = dest;
        }

        2
    }

    /// `scale_dest_row_one_128bpp`. Returns parts-row index `1`.
    fn scale_dest_row_one(&self, local: &mut LocalCtx, row_index: u32) -> usize {
        if local.src_ofs != 0 {
            self.scale_horizontal(local, 0, 0);
            local.src_ofs = 0;
        }

        let width_px = self.hdim.placement_size_px as usize;
        let last_row = self.vdim.placement_size_px - 1;
        let opacity = self.edge_opacity_v(row_index, last_row);

        let mut dest = std::mem::take(&mut local.rows[1]);
        let src = &local.rows[0];
        match opacity {
            None => dest[..width_px * 2].copy_from_slice(&src[..width_px * 2]),
            Some(op) => {
                for i in 0..width_px * 2 {
                    dest[i] = apply_subpixel_opacity_half(src[i], op);
                }
            }
        }
        local.rows[1] = dest;
        1
    }

    /// `scale_dest_row_copy`. Returns parts-row index `0`.
    fn scale_dest_row_copy(&self, local: &mut LocalCtx, row_index: u32) -> usize {
        self.scale_horizontal(local, row_index, 0);
        0
    }

    /// Dispatch the vertical filter for one destination row.
    fn vfilter(&self, local: &mut LocalCtx, dest_row_index: u32) -> usize {
        match self.vdim.filter_type {
            FilterType::Copy => self.scale_dest_row_copy(local, dest_row_index),
            FilterType::One => self.scale_dest_row_one(local, dest_row_index),
            FilterType::Nearest => {
                let src_row = u32::from(self.vdim.precalc_bilin[dest_row_index as usize]);
                self.scale_horizontal(local, src_row, 0);
                0
            }
            FilterType::Bilinear(n) => self.scale_dest_row_bilinear(local, dest_row_index, n),
            FilterType::Box => self.scale_dest_row_box(local, dest_row_index),
        }
    }

    /// `do_rows`: scale every destination row and pack it out.
    fn do_rows(&self, dest: &mut [u8]) {
        let row_len =
            (self.hdim.src_size_px as usize + 1).max(self.hdim.placement_size_px as usize) * 2;
        let mut local = LocalCtx::new(row_len);

        let dest_rowstride = self.hdim.dest_size_px as usize * 4;
        let placement_px = self.hdim.placement_size_px as usize;

        for row in 0..self.vdim.dest_size_px {
            let scaled = self.vfilter(&mut local, row);
            if let Some(bg) = self.background {
                composite_row_over_bg(&mut local.rows[scaled][..placement_px * 2], bg);
            }
            let drow = &mut dest[row as usize * dest_rowstride..];
            pack_row(&local.rows[scaled], drow, placement_px, self.linear);
        }
    }
}

// ---------------------------------------------------------------------------
// Public entry
// ---------------------------------------------------------------------------

/// Resample an `RGBA8` (unassociated alpha) image from `sw`×`sh` to `dw`×`dh`,
/// bit-exactly matching chafa's `smol_scale_simple` on a little-endian,
/// non-accelerated host. Input/output are tightly packed `R,G,B,A` bytes.
///
/// # Panics
///
/// Panics for dimensions outside `1..=65535` or an incorrectly sized buffer.
pub fn scale_rgba8(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    scale_rgba8_inner(src, sw, sh, dw, dh, None, false)
}

/// Stretch and composite over an opaque background in linear light, retaining
/// source coverage as alpha. Matches chafa's symbol preparation pipeline.
/// Set `nearest` for chafa's low-work (`work_factor_int < 3`) sampling.
///
/// # Panics
///
/// Panics for dimensions outside `1..=65535` or an incorrectly sized buffer.
pub fn scale_rgba8_with_background(
    src: &[u8],
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    bg_rgb: u32,
    nearest: bool,
) -> Vec<u8> {
    scale_rgba8_inner(src, sw, sh, dw, dh, Some(bg_rgb), nearest)
}

fn scale_rgba8_inner(
    src: &[u8],
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
    background: Option<u32>,
    nearest: bool,
) -> Vec<u8> {
    let source_size = pixel_count(sw, sh) * 4;
    let dest_size = pixel_count(dw, dh) * 4;
    assert_eq!(src.len(), source_size, "source buffer size mismatch");
    let mut dest = vec![0u8; dest_size];

    // Background compositing still needs the linear-light unpack/pack path.
    if sw == dw && sh == dh && background.is_none() {
        dest.copy_from_slice(src);
        return dest;
    }

    // Match smolscale's precision guard: enormous box spans use compressed
    // channels so the packed 32-bit accumulators cannot overflow.
    let linear = sw <= dw * 8191 && sh <= dh * 8191;
    let background = background.map(|rgb| {
        let bytes = [
            ((rgb >> 16) & 0xff) as u8,
            ((rgb >> 8) & 0xff) as u8,
            rgb as u8,
            255,
        ];
        let mut parts = [0; 2];
        unpack_pixel(u32::from_be_bytes(bytes), &mut parts, linear);
        parts
    });
    let ctx = ScaleCtx {
        src_pixels: src,
        src_rowstride: sw * 4,
        hdim: Dim::new(sw as u32, dw as u32, nearest),
        vdim: Dim::new(sh as u32, dh as u32, nearest),
        background,
        linear,
    };
    ctx.do_rows(&mut dest);
    dest
}

/// `composite_over_color_src_alpha_p16_128bpp_span` at full layer opacity.
fn composite_row_over_bg(row: &mut [u64], background: [u64; 2]) {
    for pixel in row.chunks_exact_mut(2) {
        let alpha = (pixel[1] >> 8) & 0xff;
        let nonzero = u64::from(alpha != 0);
        let weight = 256 - alpha - nonzero;
        for c in 0..2 {
            let blended = pixel[c] * nonzero
                + ((background[c]
                    .wrapping_mul(weight)
                    .wrapping_add(0x0000_0080_0000_0080)
                    >> 8)
                    & MASK24);
            pixel[c] = ((blended >> 8) & 0x0000_ffff_0000_ffff) * (alpha + 1);
        }
        pixel[1] = (pixel[1] & 0xffff_ffff_0000_0000) | (alpha << 8) | 0xff;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_1x1_roundtrips() {
        let src = [0x12, 0x34, 0x56, 0xff];
        let out = scale_rgba8(&src, 1, 1, 1, 1);
        assert_eq!(out, src);
    }

    #[test]
    fn downscale_2x1_is_linear_average() {
        // Red + blue -> 0xb4 under upstream's reversible gamma-2 curve.
        let src = [0xff, 0x00, 0x00, 0xff, 0x00, 0x00, 0xff, 0xff];
        let out = scale_rgba8(&src, 2, 1, 1, 1);
        assert_eq!(out, [0xb4, 0x00, 0xb4, 0xff]);
    }

    #[test]
    fn constant_colors_roundtrip_at_every_alpha() {
        for value in 0..=255 {
            for alpha in 0..=255 {
                let src = [value, 255 - value, value / 2, alpha];
                assert_eq!(scale_rgba8(&src, 1, 1, 2, 1), src.repeat(2));
            }
        }
    }

    #[test]
    #[should_panic(expected = "pixel dimensions must be in 1..=65535")]
    fn oversized_destination_is_rejected_before_allocating() {
        scale_rgba8(&[0; 4], 1, 1, usize::MAX, 1);
    }
}
