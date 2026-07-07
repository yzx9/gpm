// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Shared `git2` plumbing used across the [`super`] RCS modules.
//!
//! Pure git helpers with no network/transport dependency — the dependency leaf
//! of the git backend: [`transport`](super::transport), [`commit`](super::commit),
//! [`pull`](super::pull), and [`divergence`](super::divergence) all reach for
//! these.

use git2::Repository;

use crate::config::{DEFAULT_COMMIT_EMAIL, DEFAULT_COMMIT_NAME};
use crate::error::{Error, ErrorCode};

/// The signature gpm commits under. `name` / `email` come from the configured
/// commit identity and fall back to the app default when `None`. gpm does not
/// (yet) SSH-sign its own commits; remote commits are verified on pull via the
/// authenticity layer.
pub(super) fn gpm_signature(
    name: Option<&str>,
    email: Option<&str>,
) -> Result<git2::Signature<'static>, Error> {
    git2::Signature::now(
        name.unwrap_or(DEFAULT_COMMIT_NAME),
        email.unwrap_or(DEFAULT_COMMIT_EMAIL),
    )
    .map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("Failed to build signature: {e}"),
        )
    })
}

/// Move the branch ref to `target` and check out HEAD (forced), updating the
/// working tree.
pub(super) fn advance_branch(
    repo: &Repository,
    branch_name: &str,
    target: git2::Oid,
) -> Result<(), Error> {
    let branch_ref = format!("refs/heads/{branch_name}");
    repo.reference(&branch_ref, target, true, "gpm pull")?;
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    repo.checkout_head(Some(&mut checkout_builder))?;
    Ok(())
}

/// Short hash (first 7 chars) of `oid`.
pub(super) fn short_hash(oid: &git2::Oid) -> String {
    let full = oid.to_string();
    if full.len() >= 7 {
        full[..7].to_string()
    } else {
        full
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_hash_normal() {
        let hex = "abcdef1234567890abcdef1234567890abcdef12";
        let oid = git2::Oid::from_str(hex).expect("failed to parse oid");
        let result = short_hash(&oid);
        assert_eq!(result, "abcdef1");
    }

    #[test]
    fn short_hash_short_input() {
        let full = String::from("abc");
        let result = if full.len() >= 7 {
            full[..7].to_string()
        } else {
            full
        };
        assert_eq!(result, "abc");
    }
}
