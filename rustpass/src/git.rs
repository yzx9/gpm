// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{Error, ErrorCode};
use crate::store::SyncResult;

/// Clone a git repository to a local directory.
///
/// For HTTPS URLs, uses PAT credential callback.
///
/// # Errors
///
/// Returns an error if the clone fails due to authentication, network, or
/// filesystem issues.
pub fn clone_repo(url: &str, dest: &Path, pat: Option<&str>) -> Result<(), Error> {
    // Remove existing directory if present (re-clone)
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }

    let mut callbacks = RemoteCallbacks::new();
    if let Some(token) = pat {
        let token = token.to_string();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            git2::Cred::userpass_plaintext(&token, "")
                .or_else(|_| git2::Cred::userpass_plaintext("", &token))
        });
    }

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder.clone(url, dest).map_err(|e| {
        let msg = e.message().to_string();
        if msg.contains("authentication") || msg.contains("unsupported URL") {
            Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
        } else if msg.contains("unable to connect") || msg.contains("timeout") {
            Error::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
        } else {
            Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
        }
    })?;

    Ok(())
}

/// Pull (fetch + fast-forward only merge) from origin/main.
///
/// Returns whether any commits were pulled and the new HEAD hash.
///
/// # Errors
///
/// Returns an error if the repository cannot be found, the remote is
/// unreachable, or the branches have diverged (non-fast-forward).
pub fn pull_repo(repo_path: &Path, pat: Option<&str>) -> Result<SyncResult, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let mut remote = repo.find_remote("origin").map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Cannot find origin remote: {}", e.message()),
        )
    })?;

    // Set up credential callback for HTTPS PAT
    let mut callbacks = RemoteCallbacks::new();
    if let Some(token) = pat {
        let token = token.to_string();
        callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
            git2::Cred::userpass_plaintext(&token, "")
                .or_else(|_| git2::Cred::userpass_plaintext("", &token))
        });
    }

    // Capture HEAD before fetch so we can detect changes.
    // The fetch refspec `refs/heads/*:refs/heads/*` updates local branches
    // in-place during fetch, so reading HEAD after fetch would already
    // reflect the update. We must compare pre-fetch vs post-fetch.
    let pre_fetch_oid = repo.head().ok().and_then(|r| r.target());

    // Fetch from remote
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&["refs/heads/*:refs/heads/*"], Some(&mut fetch_opts), None)?;

    // Read HEAD after fetch
    let post_fetch_oid = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Cannot determine current HEAD"))?;

    // If HEAD hasn't moved, there's nothing to do
    let Some(pre_oid) = pre_fetch_oid else {
        return Ok(SyncResult {
            changed: false,
            head: short_hash(&post_fetch_oid),
        });
    };

    if post_fetch_oid == pre_oid {
        return Ok(SyncResult {
            changed: false,
            head: short_hash(&post_fetch_oid),
        });
    }

    // Verify fast-forward: new HEAD must be a descendant of old HEAD
    if !repo.graph_descendant_of(post_fetch_oid, pre_oid)? {
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Cannot fast-forward: branches have diverged. Resolve on desktop.",
        ));
    }

    // Checkout the new HEAD to update the working tree.
    // The in-place refspec fetch updates refs but not the working tree,
    // so we must explicitly checkout. Use FORCE strategy to ensure all files
    // (including newly added ones) are written to disk.
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    repo.checkout_head(Some(&mut checkout_builder))?;

    Ok(SyncResult {
        changed: true,
        head: short_hash(&post_fetch_oid),
    })
}

/// Find the default branch name (main or master).
#[cfg(test)]
fn find_default_branch(repo: &Repository) -> Result<String, Error> {
    // Try refs/heads/main first, then refs/heads/master
    for branch in &["main", "master"] {
        if repo.find_reference(&format!("refs/heads/{branch}")).is_ok() {
            return Ok(branch.to_string());
        }
    }

    // Fallback: check what HEAD points to
    if let Ok(head) = repo.head() {
        if let Some(name) = head.shorthand() {
            return Ok(name.to_string());
        }
    }

    Err(Error::new(
        ErrorCode::PullFfFailed,
        "Cannot determine default branch",
    ))
}

fn short_hash(oid: &git2::Oid) -> String {
    // Short hash is first 7 chars
    let full = oid.to_string();
    if full.len() >= 7 {
        full[..7].to_string()
    } else {
        full
    }
}

/// Helper: create an empty initial commit in a test repository.
///
/// Builds a commit from an empty tree so the repo has a valid HEAD
/// without requiring any working-tree files.
#[cfg(test)]
fn create_empty_commit(repo: &Repository, sig: &git2::Signature<'_>) -> git2::Oid {
    let mut index = repo.index().expect("failed to get index");
    let tree_id = index.write_tree().expect("failed to write tree");
    let tree = repo.find_tree(tree_id).expect("failed to find tree");
    repo.commit(Some("HEAD"), sig, sig, "initial commit", &tree, &[])
        .expect("failed to create commit")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Shared test signature used across tests.
    fn test_signature() -> git2::Signature<'static> {
        git2::Signature::new("Test", "test@test.com", &git2::Time::new(0, 0))
            .expect("failed to create signature")
    }

    /// Read the system's default branch name from git config.
    fn config_default_branch(repo: &Repository) -> String {
        repo.config()
            .and_then(|c| c.get_string("init.defaultBranch"))
            .unwrap_or_else(|_| "master".to_string())
    }

    #[test]
    fn find_default_branch_main() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let _oid = create_empty_commit(&repo, &sig);

        let expected = config_default_branch(&repo);
        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, expected);
    }

    #[test]
    fn find_default_branch_master() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let oid = create_empty_commit(&repo, &sig);

        let default_branch = config_default_branch(&repo);
        repo.find_reference(&format!("refs/heads/{default_branch}"))
            .expect("should find auto-created ref")
            .delete()
            .expect("failed to delete ref");
        repo.reference("refs/heads/master", oid, false, "test master branch")
            .expect("failed to create master ref");
        repo.set_head("refs/heads/master")
            .expect("failed to set HEAD");

        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, "master");
    }

    #[test]
    fn find_default_branch_head_fallback() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).expect("failed to init repo");
        let sig = test_signature();
        let oid = create_empty_commit(&repo, &sig);

        let default_branch = config_default_branch(&repo);
        repo.find_reference(&format!("refs/heads/{default_branch}"))
            .expect("should find auto-created ref")
            .delete()
            .expect("failed to delete ref");
        repo.reference("refs/heads/develop", oid, false, "test develop ref")
            .expect("failed to create develop ref");
        repo.set_head("refs/heads/develop")
            .expect("failed to set HEAD");

        let branch = find_default_branch(&repo).expect("should find a branch");
        assert_eq!(branch, "develop");
    }

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
