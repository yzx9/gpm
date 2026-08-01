// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Password generator command. Stateless — pure CSPRNG work over the rustpass
//! generator, no store access. The result is a [`Zeroizing<String>`] so the
//! secret is wiped on drop in the app process.

use rustpass::{Error, GenerateMode, GenerateOptions};
use zeroize::Zeroizing;

/// Generate a password for the chosen mode and (optional) per-field charset,
/// mirroring gopass's create-wizard generation. When `charset` is set the
/// result is drawn from that alphabet (e.g. a digits-only PIN); otherwise the
/// selected `mode` (random/memorable/xkcd) drives it.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn generate_password(
    mode: GenerateMode,
    charset: Option<String>,
    min_len: Option<usize>,
    max_len: Option<usize>,
    strict: bool,
) -> Result<Zeroizing<String>, Error> {
    rustpass::generate_password(&GenerateOptions {
        mode,
        charset: charset.as_deref(),
        min_len,
        max_len,
        strict,
    })
}

/// Upper bound on a client-requested batch size, so a buggy/malicious caller
/// can't ask for `usize::MAX`. The frontend defaults to 10.
const MAX_BATCH_COUNT: usize = 32;

/// Clamp a client-requested batch size to a sane, non-zero bound.
fn clamp_count(count: usize) -> usize {
    count.clamp(1, MAX_BATCH_COUNT)
}

/// Generate up to [`MAX_BATCH_COUNT`] passwords in one call — a gopass-`pwgen`-
/// style "give me a batch to pick from" affordance for the standalone generator
/// page. Same options as [`generate_password`]; `count` is clamped to
/// `[1, MAX_BATCH_COUNT]`. Each entry is [`Zeroizing<String>`], so the Rust-side
/// `Vec` is wiped when it is dropped here. The plaintext also crosses IPC to the
/// `WebView` (the page displays the batch so the user can pick one) and is
/// cleared there on lock/unmount/regenerate like other shown secrets — the
/// `WebView`'s copy is ordinary JS strings, not `Zeroizing`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn generate_password_batch(
    mode: GenerateMode,
    charset: Option<String>,
    min_len: Option<usize>,
    max_len: Option<usize>,
    strict: bool,
    count: usize,
) -> Result<Vec<Zeroizing<String>>, Error> {
    let opts = GenerateOptions {
        mode,
        charset: charset.as_deref(),
        min_len,
        max_len,
        strict,
    };
    let n = clamp_count(count);
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(rustpass::generate_password(&opts)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_count_bounds_request_size() {
        assert_eq!(clamp_count(0), 1);
        assert_eq!(clamp_count(1), 1);
        assert_eq!(clamp_count(10), 10);
        assert_eq!(clamp_count(MAX_BATCH_COUNT), MAX_BATCH_COUNT);
        assert_eq!(clamp_count(MAX_BATCH_COUNT + 1), MAX_BATCH_COUNT);
        assert_eq!(clamp_count(usize::MAX), MAX_BATCH_COUNT);
    }
}
