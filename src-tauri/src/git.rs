use std::path::Path;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{AppError, ErrorCode};
use crate::store::PullResult;

/// Clone a git repository to a local directory.
/// For HTTPS URLs, uses PAT credential callback.
pub fn clone_repo(url: &str, dest: &Path, pat: Option<&str>) -> Result<(), AppError> {
    // Remove existing directory if present (re-clone)
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }

    let mut callbacks = RemoteCallbacks::new();
    if let Some(_token) = pat {
        let token = _token.to_string();
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
            AppError::new(ErrorCode::CloneFailed, format!("Clone failed: {}", msg))
        } else if msg.contains("unable to connect") || msg.contains("timeout") {
            AppError::new(ErrorCode::NetworkError, format!("Network error: {}", msg))
        } else {
            AppError::new(ErrorCode::CloneFailed, format!("Clone failed: {}", msg))
        }
    })?;

    Ok(())
}

/// Pull (fetch + fast-forward only merge) from origin/main.
/// Returns whether any commits were pulled and the new HEAD hash.
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
    if let Some(_token) = pat {
        let token = _token.to_string();
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
    let upstream_ref = repo.find_reference(&format!("refs/heads/{}", upstream_branch))?;
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
    repo.checkout_tree(&upstream_commit.as_object(), None)?;
    repo.set_head(&format!("refs/heads/{}", upstream_branch))?;

    Ok(PullResult {
        changed: true,
        head: short_hash(&upstream_oid),
    })
}

/// Find the default branch name (main or master).
fn find_default_branch(repo: &Repository) -> Result<String, AppError> {
    // Try refs/heads/main first, then refs/heads/master
    for branch in &["main", "master"] {
        if repo
            .find_reference(&format!("refs/heads/{}", branch))
            .is_ok()
        {
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
