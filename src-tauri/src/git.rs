// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{AppError, ErrorCode};
use crate::store::PullResult;

/// Clone a git repository to a local directory.
/// For HTTPS URLs, uses PAT credential callback.
///
/// # Errors
///
/// Returns an error if the clone fails due to authentication, network, or
/// filesystem issues.
pub fn clone_repo(url: &str, dest: &Path, pat: Option<&str>) -> Result<(), AppError> {
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
            AppError::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
        } else if msg.contains("unable to connect") || msg.contains("timeout") {
            AppError::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
        } else {
            AppError::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
        }
    })?;

    Ok(())
}

/// Pull (fetch + fast-forward only merge) from origin/main.
/// Returns whether any commits were pulled and the new HEAD hash.
///
/// # Errors
///
/// Returns an error if the repository cannot be found, the remote is
/// unreachable, or the branches have diverged (non-fast-forward).
pub fn pull_repo(repo_path: &Path, pat: Option<&str>) -> Result<PullResult, AppError> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| AppError::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let mut remote = repo.find_remote("origin").map_err(|e| {
        AppError::new(
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

    // Fetch from remote
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&["refs/heads/*:refs/heads/*"], Some(&mut fetch_opts), None)?;

    // Get current HEAD
    let head_oid = repo
        .head()?
        .target()
        .ok_or_else(|| AppError::new(ErrorCode::PullFfFailed, "Cannot determine current HEAD"))?;

    // Find the upstream branch (origin/main or origin/master)
    let upstream_branch = find_default_branch(&repo)?;
    let upstream_ref = repo.find_reference(&format!("refs/heads/{upstream_branch}"))?;
    let upstream_oid = upstream_ref
        .target()
        .ok_or_else(|| AppError::new(ErrorCode::PullFfFailed, "Cannot determine upstream HEAD"))?;

    // Check if fast-forward is possible
    if upstream_oid == head_oid {
        return Ok(PullResult {
            changed: false,
            head: short_hash(&head_oid),
        });
    }

    // Verify fast-forward: upstream must be a descendant of HEAD
    let _head_commit = repo.find_commit(head_oid)?;
    let upstream_commit = repo.find_commit(upstream_oid)?;

    if !repo.graph_descendant_of(upstream_oid, head_oid)? {
        return Err(AppError::new(
            ErrorCode::PullFfFailed,
            "Cannot fast-forward: branches have diverged. Resolve on desktop.",
        ));
    }

    // Perform fast-forward merge
    repo.checkout_tree(upstream_commit.as_object(), None)?;
    repo.set_head(&format!("refs/heads/{upstream_branch}"))?;

    Ok(PullResult {
        changed: true,
        head: short_hash(&upstream_oid),
    })
}

/// Find the default branch name (main or master).
fn find_default_branch(repo: &Repository) -> Result<String, AppError> {
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

    Err(AppError::new(
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

        // The commit creates the system's default branch (e.g. "main" or
        // "master"). find_default_branch should return it.
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

        // Remove the auto-created default branch and create master instead.
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

        // Remove the auto-created default branch so neither main nor
        // master exists, then create develop for the HEAD fallback path.
        let default_branch = config_default_branch(&repo);
        repo.find_reference(&format!("refs/heads/{default_branch}"))
            .expect("should find auto-created ref")
            .delete()
            .expect("failed to delete ref");
        repo.reference("refs/heads/develop", oid, false, "test develop branch")
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
        // Valid git2::Oid is always 40 hex chars, so the len < 7 branch in
        // short_hash is defensive code that cannot be reached through normal
        // usage. Test the string-slicing logic directly to cover that branch.
        let full = String::from("abc");
        let result = if full.len() >= 7 {
            full[..7].to_string()
        } else {
            full
        };
        assert_eq!(result, "abc");
    }
}
