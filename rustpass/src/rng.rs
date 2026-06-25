// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Cryptographically-strong randomness primitives.
//!
//! Thin wrapper over the OS CSPRNG ([`getrandom`]) plus a uniform
//! rejection-sampled index helper. Shared by at-rest key generation and the
//! password generator — the analogue of gopass's `pkg/pwgen/rand.go`.

use crate::error::{Error, ErrorCode};

/// Fill `out` with cryptographically-strong random bytes from the OS RNG.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails (essentially never on a
/// booted device; the same mapping the at-rest path has always used).
pub fn fill_random(out: &mut [u8]) -> Result<(), Error> {
    getrandom::getrandom(out)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("OS RNG failed: {e}")))
}

/// Draw a uniformly-distributed integer in `[0, n)` over the OS CSPRNG.
///
/// Uses rejection sampling over a 32-bit draw to avoid the modulo bias of a
/// naive `byte % n`, and works for any practical `n` (alphabets, the BIP39
/// wordlist, …). The analogue of gopass's `randomInteger`.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the OS RNG fails.
///
/// # Panics
///
/// Panics if `n == 0`.
pub fn uniform_index(n: usize) -> Result<usize, Error> {
    assert!(n > 0, "uniform_index: n must be non-zero");
    if n == 1 {
        return Ok(0);
    }

    let n_u64 = u64::try_from(n).expect("n fits in u64");
    // 32-bit draw space; reject the partial top block so `v % n` stays unbiased.
    let space: u64 = 1u64 << 32;
    let limit = space - (space % n_u64);

    let mut buf = [0u8; 4];
    loop {
        fill_random(&mut buf)?;
        let v = u64::from(u32::from_le_bytes(buf));
        if v < limit {
            return Ok(usize::try_from(v % n_u64).expect("index < n fits usize"));
        }
    }
}
