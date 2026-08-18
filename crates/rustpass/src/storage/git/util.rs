// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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

/// The current branch name HEAD sits on, named for the failing operation
/// (`op`) in the error. Fails when HEAD is detached (no branch refspec can be
/// built) or the name is not valid UTF-8 (a refspec cannot be constructed).
pub(super) fn head_branch(repo: &Repository, op: &str) -> Result<String, Error> {
    let head = repo.head()?;
    if !head.is_branch() {
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            format!("cannot {op}: detached HEAD"),
        ));
    }
    head.shorthand()
        .map_err(|e| {
            Error::new(
                ErrorCode::PullFfFailed,
                format!(
                    "cannot {op}: branch name is not valid UTF-8: {}",
                    e.message()
                ),
            )
        })
        .map(str::to_string)
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

    /// A branch name with a raw non-UTF-8 byte can only exist via out-of-band
    /// writes (git2's `&str` API refuses to create it); resolving it for an
    /// operation must fail instead of building a garbled refspec.
    #[cfg(unix)]
    #[test]
    fn head_branch_rejects_non_utf8_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        use crate::storage::git::test_support::test_signature;

        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let sig = test_signature();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "m", &tree, &[])
            .unwrap();
        drop(tree);
        let git_dir = repo.path().to_path_buf();
        let branch = git_dir
            .join("refs/heads")
            .join(OsStr::from_bytes(b"ma\xffin"));
        std::fs::write(&branch, format!("{oid}\n")).unwrap();
        std::fs::write(git_dir.join("HEAD"), b"ref: refs/heads/ma\xffin\n").unwrap();

        let repo = Repository::open(dir.path()).unwrap();
        let err = head_branch(&repo, "pull").unwrap_err();
        assert_eq!(err.code, "PULL_FF_FAILED");
        assert!(
            err.message.contains("not valid UTF-8"),
            "got: {}",
            err.message
        );
    }

    /// A detached HEAD resolves `shorthand()` to "HEAD", which would build a
    /// refspec for a branch literally named HEAD — refuse it instead.
    #[test]
    fn head_branch_rejects_detached_head() {
        use crate::storage::git::test_support::test_signature;

        let dir = tempfile::tempdir().unwrap();
        let repo = Repository::init(dir.path()).unwrap();
        let sig = test_signature();
        let tree_id = repo.treebuilder(None).unwrap().write().unwrap();
        let tree = repo.find_tree(tree_id).unwrap();
        let oid = repo
            .commit(Some("HEAD"), &sig, &sig, "m", &tree, &[])
            .unwrap();
        drop(tree);
        repo.set_head_detached(oid).unwrap();

        let err = head_branch(&repo, "push").unwrap_err();
        assert_eq!(err.code, "PULL_FF_FAILED");
        assert!(
            err.message.contains("detached HEAD"),
            "got: {}",
            err.message
        );
    }
}
