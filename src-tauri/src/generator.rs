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
