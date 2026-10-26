//! Reed-Solomon codec, ported from `thirdparty/fec` (a reduced copy of
//! Phil Karn KA9Q's `libfec`, LGPL-2.1), instantiated exactly the way
//! `DabPlusServiceComponentDecoder::RSDecoder` does:
//! `init_rs_char(8, 0x11D, 0, 1, 10, 135)` -- an 8-bit-symbol RS(255,245)
//! code shortened, via 135 bytes of implicit leading padding, to a
//! 120-symbol block (110 data symbols + 10 parity symbols).
//!
//! Unlike the reference (which builds a generic codec instance at
//! runtime), this crate only ever needs this one parameterization, so the
//! constants below are hardcoded rather than carried as struct fields --
//! `RsCodec` only holds the derived Galois-field tables, exactly what
//! `init_rs_char` computes into `struct rs` (`thirdparty/fec/rs-common.h`).
//! `thirdparty/fec/init_rs.h`, `decode_rs.h`, and `encode_rs.h` are the
//! reference this module transcribes -- their macro-templated C is
//! `#include`d into a function body wired to `struct rs`'s fields; ported
//! here as plain Rust functions on `RsCodec` with the same variable names
//! and control flow, since none of that genericity is needed for this
//! single fixed parameterization.
//!
//! Deliberately keeps the reference's index-based loops (`for i in
//! 0..N { arr1[i] = ...; arr2[i] = ... }`) rather than clippy's preferred
//! iterator-based rewrites: several loops here index two or three parallel
//! arrays (`s`/`lambda`/`b`/`reg`/...) by the same counter, sometimes with
//! that counter also feeding the Galois-field arithmetic itself (not just
//! an array offset) -- an idiomatic Rust rewrite of any one of them stops
//! being a line-for-line match against `decode_rs.h`/`encode_rs.h`, which
//! is the entire point of this module.
#![allow(clippy::needless_range_loop, clippy::explicit_counter_loop)]

/// Symbol size, bits.
const SYMSIZE: u32 = 8;
/// Galois field generator polynomial.
const GFPOLY: u32 = 0x11D;
/// First consecutive root of the RS generator polynomial, index form.
const FCR: u32 = 0;
/// Primitive element used to generate polynomial roots.
const PRIM: u32 = 1;
/// Number of generator-polynomial roots (= parity symbols per block).
const NROOTS: usize = 10;
/// Padding symbols implicitly present at the front of the shortened block.
pub(crate) const PAD: usize = 135;
/// Symbols per full (unshortened) RS block: `(1 << SYMSIZE) - 1`.
const NN: usize = 255;
/// "Index of zero" sentinel, matching `char.h`'s `#define A0 (NN)`.
const A0: u8 = NN as u8;

/// Symbols actually carried in one shortened block (`NN - PAD`): 110 data
/// symbols + 10 parity symbols. This is the `rs_packet[120]`/`data[]` size
/// throughout the reference.
pub const BLOCK_LEN: usize = NN - PAD;
/// Data (non-parity) symbols per block.
pub const DATA_LEN: usize = BLOCK_LEN - NROOTS;

/// Reduces `x` modulo `NN` (`rs-common.h`'s `modnn`).
fn modnn(mut x: u32) -> usize {
    let nn = NN as u32;
    while x >= nn {
        x -= nn;
        x = (x >> SYMSIZE) + (x & nn);
    }
    x as usize
}

/// The derived Galois-field log/antilog tables and generator polynomial
/// for this codec's fixed parameterization. Mirrors `struct rs`.
pub struct RsCodec {
    /// Index (log) form -> Galois field element. `alpha_to[A0] == 0`
    /// (alpha**-inf = 0).
    alpha_to: [u8; NN + 1],
    /// Galois field element -> index (log) form. `index_of[0] == A0`.
    index_of: [u8; NN + 1],
    /// Generator polynomial, in index form (as `init_rs.h` leaves it after
    /// its own final conversion pass).
    genpoly: [u8; NROOTS + 1],
}

impl Default for RsCodec {
    fn default() -> Self {
        Self::new()
    }
}

impl RsCodec {
    /// `init_rs_char(8, 0x11D, 0, 1, 10, 135)` / `init_rs.h`.
    pub fn new() -> Self {
        let mut alpha_to = [0u8; NN + 1];
        let mut index_of = [0u8; NN + 1];

        // Generate Galois field lookup tables.
        index_of[0] = A0;
        alpha_to[A0 as usize] = 0;
        let mut sr: u32 = 1;
        for i in 0..NN {
            index_of[sr as usize] = i as u8;
            alpha_to[i] = sr as u8;
            sr <<= 1;
            if sr & (1 << SYMSIZE) != 0 {
                sr ^= GFPOLY;
            }
            sr &= NN as u32;
        }
        debug_assert_eq!(
            sr, 1,
            "GFPOLY 0x11D is not a primitive polynomial over GF(2^8) -- \
             this would mean the transcribed constant itself is wrong, \
             not a runtime condition"
        );

        // Form the RS code generator polynomial from its roots.
        let mut genpoly = [0u8; NROOTS + 1];
        genpoly[0] = 1;
        let mut root = FCR * PRIM;
        for i in 0..NROOTS {
            genpoly[i + 1] = 1;

            // Multiply genpoly[] by (@**(root + x)).
            for j in (1..=i).rev() {
                if genpoly[j] != 0 {
                    genpoly[j] = genpoly[j - 1]
                        ^ alpha_to[modnn(index_of[genpoly[j] as usize] as u32 + root)];
                } else {
                    genpoly[j] = genpoly[j - 1];
                }
            }
            // genpoly[0] can never be zero.
            genpoly[0] = alpha_to[modnn(index_of[genpoly[0] as usize] as u32 + root)];

            root += PRIM;
        }
        // Convert genpoly[] to index form for quicker encoding.
        let mut genpoly_idx = [0u8; NROOTS + 1];
        for i in 0..=NROOTS {
            genpoly_idx[i] = index_of[genpoly[i] as usize];
        }

        Self {
            alpha_to,
            index_of,
            genpoly: genpoly_idx,
        }
    }

    /// Reed-Solomon-encodes `data` (`DATA_LEN` symbols), writing `NROOTS`
    /// parity symbols to `parity` (`encode_rs_char`/`encode_rs.h`).
    ///
    /// Not used by the DAB+ receive path itself (only a decoder is needed
    /// to receive an already-encoded broadcast) -- ported alongside decode
    /// because a self-consistent encode/corrupt/decode round trip is the
    /// only practical way to verify this decoder's correctness without a
    /// captured reference bitstream to compare against (see this module's
    /// tests).
    pub fn encode(&self, data: &[u8; DATA_LEN], parity: &mut [u8; NROOTS]) {
        parity.fill(0);

        for i in 0..DATA_LEN {
            let feedback = self.index_of[(data[i] ^ parity[0]) as usize];
            if feedback != A0 {
                for j in 1..NROOTS {
                    parity[j] ^=
                        self.alpha_to[modnn(feedback as u32 + self.genpoly[NROOTS - j] as u32)];
                }
            }
            // Shift.
            parity.copy_within(1..NROOTS, 0);
            if feedback != A0 {
                parity[NROOTS - 1] = self.alpha_to[modnn(feedback as u32 + self.genpoly[0] as u32)];
            } else {
                parity[NROOTS - 1] = 0;
            }
        }
    }

    /// Reed-Solomon-decodes one `BLOCK_LEN`-symbol shortened block in
    /// place (`decode_rs_char`/`decode_rs.h`, called with `no_eras = 0` --
    /// this decoder is always run blind, with no a priori erasure
    /// positions, matching `RSDecoder::DecodeSuperframe`'s own call).
    ///
    /// Returns the corrected symbol positions as raw Chien-search location
    /// indices (same representation as the reference's `eras_pos`/
    /// `corr_pos` output) on success -- some of these can land in the
    /// codec's virtual padding region rather than `data` itself (matching
    /// `DecodeSuperframe`'s own separate `pos < 0` guard after subtracting
    /// `PAD` from them, ported as-is in [`crate::dabplus`]) -- or `None` if
    /// the block has more errors than the code can correct.
    pub fn decode(&self, data: &mut [u8; BLOCK_LEN]) -> Option<Vec<u32>> {
        let alpha_to = &self.alpha_to;
        let index_of = &self.index_of;

        // Form the syndromes: evaluate data(x) at roots of g(x).
        let mut s = [0u8; NROOTS];
        for si in s.iter_mut() {
            *si = data[0];
        }
        for j in 1..BLOCK_LEN {
            for i in 0..NROOTS {
                if s[i] == 0 {
                    s[i] = data[j];
                } else {
                    s[i] = data[j]
                        ^ alpha_to[modnn(index_of[s[i] as usize] as u32 + (FCR + i as u32) * PRIM)];
                }
            }
        }

        // Convert syndromes to index form, checking for nonzero condition.
        let mut syn_error = 0u8;
        for si in s.iter_mut() {
            syn_error |= *si;
            *si = index_of[*si as usize];
        }

        if syn_error == 0 {
            // data[] is already a codeword: no errors to correct.
            return Some(Vec::new());
        }

        let mut lambda = [0u8; NROOTS + 1];
        lambda[0] = 1;

        let mut b = [0u8; NROOTS + 1];
        for i in 0..=NROOTS {
            b[i] = index_of[lambda[i] as usize];
        }

        // Berlekamp-Massey algorithm to determine the error locator
        // polynomial. `no_eras` is always 0 here (see this fn's doc
        // comment), so `r`/`el` both start at 0.
        let mut r = 0usize;
        let mut el = 0usize;
        loop {
            r += 1;
            if r > NROOTS {
                break;
            }

            // Compute discrepancy at the r-th step, in poly form.
            let mut discr_r = 0u8;
            for i in 0..r {
                if lambda[i] != 0 && s[r - i - 1] != A0 {
                    discr_r ^=
                        alpha_to[modnn(index_of[lambda[i] as usize] as u32 + s[r - i - 1] as u32)];
                }
            }
            let discr_r = index_of[discr_r as usize]; // Index form.

            if discr_r == A0 {
                // B(x) <- x*B(x)
                for i in (1..=NROOTS).rev() {
                    b[i] = b[i - 1];
                }
                b[0] = A0;
            } else {
                // T(x) <- lambda(x) - discr_r*x*b(x)
                let mut t = [0u8; NROOTS + 1];
                t[0] = lambda[0];
                for i in 0..NROOTS {
                    if b[i] != A0 {
                        t[i + 1] = lambda[i + 1] ^ alpha_to[modnn(discr_r as u32 + b[i] as u32)];
                    } else {
                        t[i + 1] = lambda[i + 1];
                    }
                }
                if 2 * el < r {
                    el = r - el;
                    // B(x) <- inv(discr_r) * lambda(x)
                    for i in 0..=NROOTS {
                        b[i] = if lambda[i] == 0 {
                            A0
                        } else {
                            modnn(index_of[lambda[i] as usize] as u32 + NN as u32 - discr_r as u32)
                                as u8
                        };
                    }
                } else {
                    // B(x) <- x*B(x)
                    for i in (1..=NROOTS).rev() {
                        b[i] = b[i - 1];
                    }
                    b[0] = A0;
                }
                lambda = t;
            }
        }

        // Convert lambda to index form and compute deg(lambda(x)).
        let mut deg_lambda = 0usize;
        for i in 0..=NROOTS {
            lambda[i] = index_of[lambda[i] as usize];
            if lambda[i] != A0 {
                deg_lambda = i;
            }
        }

        // Find roots of the error locator polynomial by Chien search.
        let mut reg = [0u8; NROOTS + 1];
        reg[1..=NROOTS].copy_from_slice(&lambda[1..=NROOTS]);
        let mut root = [0u32; NROOTS];
        let mut loc = [0u32; NROOTS];
        let mut count = 0usize;
        // IPRIM: prim-th root of 1, index form. `init_rs.h` computes this
        // as `iprim = (smallest positive multiple of prim that's == 1 mod
        // nn) / prim`; for this codec's PRIM=1, that's trivially 1, so
        // IPRIM (`rs->iprim`) is 1 -- computed inline here rather than
        // stored, since it never varies for this fixed parameterization.
        const IPRIM: u32 = 1;
        let mut i = 1u32;
        let mut k: u32 = IPRIM - 1;
        while i <= NN as u32 {
            let mut q = 1u8;
            for j in (1..=deg_lambda).rev() {
                if reg[j] != A0 {
                    reg[j] = modnn(reg[j] as u32 + j as u32) as u8;
                    q ^= alpha_to[reg[j] as usize];
                }
            }
            if q == 0 {
                root[count] = i;
                loc[count] = k;
                count += 1;
                if count == deg_lambda {
                    break;
                }
            }
            i += 1;
            k = modnn(k + IPRIM) as u32;
        }

        if deg_lambda != count {
            // deg(lambda) != number of roots: uncorrectable.
            return None;
        }

        // Compute the error/erasure evaluator polynomial
        // omega(x) = s(x)*lambda(x) (mod x**NROOTS), in index form.
        // `deg_lambda == 0` implies `count == 0` (checked above), which
        // makes every loop below that uses `deg_omega` a no-op -- so a
        // saturating (rather than the reference's signed -1) fallback here
        // is never actually observed, just avoids an underflow panic.
        let deg_omega = deg_lambda.saturating_sub(1);
        let mut omega = [0u8; NROOTS + 1];
        if deg_lambda > 0 {
            for i in 0..=deg_omega {
                let mut tmp = 0u8;
                for j in (0..=i).rev() {
                    if s[i - j] != A0 && lambda[j] != A0 {
                        tmp ^= alpha_to[modnn(s[i - j] as u32 + lambda[j] as u32)];
                    }
                }
                omega[i] = index_of[tmp as usize];
            }
        }

        // Compute error values in poly form and apply them to `data`.
        for j in (0..count).rev() {
            let mut num1 = 0u8;
            for i in (0..=deg_omega).rev() {
                if omega[i] != A0 {
                    num1 ^= alpha_to[modnn(omega[i] as u32 + i as u32 * root[j])];
                }
            }
            // `root[j] * (FCR - 1) + NN`: `FCR - 1` is negative for this
            // codec's FCR=0, so this needs signed arithmetic to transcribe
            // faithfully -- computed in i64 and cast back down, which is
            // sound because the full expression is always non-negative for
            // every `root[j]` the Chien search can produce (1..=NN): NN
            // alone already dominates `root[j] * (FCR - 1)`'s minimum
            // (`-root[j]`, bounded by `-NN`).
            let num2_arg = root[j] as i64 * (FCR as i64 - 1) + NN as i64;
            let num2 = alpha_to[modnn(num2_arg as u32)];

            let mut den = 0u8;
            // lambda[i+1] for i even is the formal derivative lambda_pr of
            // lambda[i].
            let mut i = std::cmp::min(deg_lambda, NROOTS - 1) & !1;
            loop {
                if lambda[i + 1] != A0 {
                    den ^= alpha_to[modnn(lambda[i + 1] as u32 + i as u32 * root[j])];
                }
                if i < 2 {
                    break;
                }
                i -= 2;
            }

            if num1 != 0 && loc[j] as usize >= PAD {
                let correction = modnn(
                    index_of[num1 as usize] as u32 + index_of[num2 as usize] as u32 + NN as u32
                        - index_of[den as usize] as u32,
                );
                data[loc[j] as usize - PAD] ^= alpha_to[correction];
            }
        }

        Some(loc[..count].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn galois_field_tables_are_self_consistent() {
        let codec = RsCodec::new();
        // alpha**0 == 1, and alpha_to/index_of are inverses of each other
        // over the nonzero field elements.
        assert_eq!(codec.alpha_to[0], 1);
        for i in 0..NN {
            let element = codec.alpha_to[i];
            assert_eq!(codec.index_of[element as usize], i as u8);
        }
    }

    #[test]
    fn decodes_uncorrupted_block_as_error_free() {
        let codec = RsCodec::new();
        let data: [u8; DATA_LEN] = std::array::from_fn(|i| (i * 7 + 3) as u8);
        let mut parity = [0u8; NROOTS];
        codec.encode(&data, &mut parity);

        let mut block = [0u8; BLOCK_LEN];
        block[..DATA_LEN].copy_from_slice(&data);
        block[DATA_LEN..].copy_from_slice(&parity);

        let corrected = codec
            .decode(&mut block)
            .expect("valid codeword must decode");
        assert!(corrected.is_empty());
        assert_eq!(&block[..DATA_LEN], &data);
    }

    #[test]
    fn corrects_up_to_half_the_parity_symbols() {
        let codec = RsCodec::new();
        let data: [u8; DATA_LEN] =
            std::array::from_fn(|i| (i as u8).wrapping_mul(31).wrapping_add(17));
        let mut parity = [0u8; NROOTS];
        codec.encode(&data, &mut parity);

        let mut block = [0u8; BLOCK_LEN];
        block[..DATA_LEN].copy_from_slice(&data);
        block[DATA_LEN..].copy_from_slice(&parity);

        // This RS code can correct up to floor(NROOTS/2) = 5 symbol errors.
        for &pos in &[0usize, 20, 50, 90, 115] {
            block[pos] ^= 0xFF;
        }

        let corrected = codec
            .decode(&mut block)
            .expect("5 errors must be within correction capacity");
        assert_eq!(corrected.len(), 5);
        assert_eq!(
            &block[..DATA_LEN],
            &data,
            "data symbols must be fully corrected"
        );
    }

    #[test]
    fn reports_uncorrectable_when_error_count_exceeds_capacity() {
        let codec = RsCodec::new();
        let data: [u8; DATA_LEN] = std::array::from_fn(|i| (i * 3 + 1) as u8);
        let mut parity = [0u8; NROOTS];
        codec.encode(&data, &mut parity);

        let mut block = [0u8; BLOCK_LEN];
        block[..DATA_LEN].copy_from_slice(&data);
        block[DATA_LEN..].copy_from_slice(&parity);

        // More errors than floor(NROOTS/2) = 5 can reliably be corrected;
        // scatter enough corruption that it's detected as uncorrectable
        // rather than (rarely, for RS) silently miscorrected.
        for pos in (0..BLOCK_LEN).step_by(13) {
            block[pos] ^= 0xA5;
        }

        assert!(codec.decode(&mut block).is_none());
    }
}
