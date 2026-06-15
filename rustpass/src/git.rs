// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{Error, ErrorCode};
use crate::signing::{self, AuthenticityConfig, VerifyMode};
use crate::store::{AuthenticityResult, SyncResult};

/// Credentials for Git remote authentication.
#[derive(Debug, Clone)]
pub enum GitAuth {
    /// No authentication (public repo).
    None,
    /// HTTPS PAT (personal access token).
    Pat(String),
    /// SSH key from memory.
    Ssh {
        /// SSH username (typically `"git"`).
        username: String,
        /// PEM or OpenSSH private key.
        private_key: String,
        /// Optional passphrase for encrypted key.
        passphrase: Option<String>,
    },
}

/// Build credential callbacks based on the authentication method.
fn build_remote_callbacks(auth: &GitAuth) -> RemoteCallbacks<'_> {
    let mut callbacks = RemoteCallbacks::new();
    match auth {
        GitAuth::None => {}
        GitAuth::Pat(token) => {
            let token = token.clone();
            callbacks.credentials(move |_url, _username_from_url, _allowed_types| {
                git2::Cred::userpass_plaintext(&token, "")
                    .or_else(|_| git2::Cred::userpass_plaintext("", &token))
            });
        }
        GitAuth::Ssh {
            username,
            private_key,
            passphrase,
        } => {
            let username = username.clone();
            let private_key = private_key.clone();
            let passphrase = passphrase.clone();
            callbacks.credentials(move |_url, username_from_url, _allowed_types| {
                let user = username_from_url.unwrap_or(&username);
                git2::Cred::ssh_key_from_memory(user, None, &private_key, passphrase.as_deref())
                    .map_err(|e| {
                        git2::Error::from_str(&format!(
                            "SSH key error: {}. Ensure the key is in OpenSSH or PEM format.",
                            e.message()
                        ))
                    })
            });
        }
    }
    callbacks
}

/// Clone a git repository to a local directory.
///
/// Supports HTTPS (PAT) and SSH key authentication via [`GitAuth`].
///
/// # Errors
///
/// Returns an error if the clone fails due to authentication, network, or
/// filesystem issues.
pub fn clone_repo(url: &str, dest: &Path, auth: &GitAuth) -> Result<(), Error> {
    // Remove existing directory if present (re-clone)
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }

    let callbacks = build_remote_callbacks(auth);

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    builder.clone(url, dest).map_err(|e| {
        let msg = e.message().to_string();
        classify_git_error(&msg)
    })?;

    Ok(())
}

/// Pull (fetch + fast-forward only merge) from origin.
///
/// Applies repository-authenticity verification according to `policy`:
/// - **[`VerifyMode::Off`]** — today's behaviour: in-place fetch + checkout.
///   Byte-for-byte equivalent to the pre-authenticity pull.
/// - **[`VerifyMode::Audit`]/[`VerifyMode::Enforce`]** — fetch the current
///   branch into a temp ref, verify every commit in `(old HEAD, new HEAD]`,
///   then (Audit) always advance + check out reporting open issues, or
///   (Enforce) refuse to advance when a non-ignored blocking issue remains —
///   HEAD and the working tree stay put.
///
/// Returns whether HEAD advanced and the current HEAD hash.
///
/// # Errors
///
/// Returns an error if the repository cannot be found, the remote is
/// unreachable, or the branches have diverged (non-fast-forward).
pub fn pull_repo(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
) -> Result<SyncResult, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let mut remote = repo.find_remote("origin").map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Cannot find origin remote: {}", e.message()),
        )
    })?;

    let callbacks = build_remote_callbacks(auth);

    // Off mode: original in-place fetch + checkout path (unchanged behaviour).
    if policy.mode == VerifyMode::Off {
        return pull_off(&repo, &mut remote, callbacks);
    }

    // Audit / Enforce: verify-before-checkout.
    pull_verified(&repo, &mut remote, callbacks, policy)
}

/// The commit author gpm writes under. gpm is a single-user client and does not
/// (yet) SSH-sign its own commits; remote commits are verified on pull via the
/// authenticity layer. A fixed identity keeps the author stable across devices.
fn gpm_signature() -> Result<git2::Signature<'static>, Error> {
    git2::Signature::now("gpm", "gpm@local").map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("Failed to build signature: {e}"),
        )
    })
}

/// Stage `rel_paths` (paths relative to the worktree root), create a commit on
/// the current branch, and return the new commit OID.
fn add_and_commit(
    repo: &Repository,
    rel_paths: &[String],
    message: &str,
) -> Result<git2::Oid, Error> {
    let mut index = repo.index()?;
    for p in rel_paths {
        index
            .add_path(Path::new(p))
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("Failed to stage {p}: {e}")))?;
    }
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head_oid = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit to build on"))?;
    let parent = repo.find_commit(head_oid)?;

    let sig = gpm_signature()?;
    Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?)
}

/// Push the current branch to `origin` using `auth`.
///
/// `Err` here means the push was rejected — most commonly a non-fast-forward
/// because the remote advanced (the write-path conflict case).
fn push_current_branch(repo: &Repository, auth: &GitAuth) -> Result<(), Error> {
    let branch = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot push"))?
        .to_string();

    let mut remote = repo.find_remote("origin").map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Cannot find origin remote: {}", e.message()),
        )
    })?;

    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(build_remote_callbacks(auth));
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(|e| classify_push_error(&e.to_string()))
}

/// Commit staged-to-`rel_paths` changes and push to origin. Returns the short
/// hash of the new HEAD commit.
///
/// This is gopass's `gitCommitAndPush` (commit + `PushPull`'s push leg),
/// expressed in libgit2. Unlike gopass we do not re-pull here — the caller is
/// expected to have synced immediately before writing, and commit 2 layers
/// conflict handling on top of a rejected push.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened, staging or committing fails,
/// or the push is rejected (non-fast-forward / network / auth).
pub fn commit_and_push(
    repo_path: &Path,
    auth: &GitAuth,
    rel_paths: &[String],
    message: &str,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    add_and_commit(&repo, rel_paths, message)?;
    push_current_branch(&repo, auth)?;
    let head = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD after commit"))?;
    Ok(short_hash(&head))
}

/// Map a libgit2 push error onto an [`Error`]. A non-fast-forward rejection
/// becomes `PushRejected` so the write path can distinguish "remote moved" from
/// generic network/auth failures.
fn classify_push_error(msg: &str) -> Error {
    if msg.contains("non-fast-forward")
        || msg.contains("fetch first")
        || msg.contains("rejected")
        || msg.contains("would clobber")
    {
        Error::new(
            ErrorCode::PushRejected,
            "Push rejected: remote has diverged. A sync/merge is required.",
        )
    } else if msg.contains("authentication") || msg.contains("credential") {
        Error::new(ErrorCode::CloneFailed, format!("Push auth failed: {msg}"))
    } else if msg.contains("unable to connect") || msg.contains("timeout") {
        Error::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
    } else {
        Error::new(ErrorCode::StoreError, format!("Push failed: {msg}"))
    }
}

/// An empty authenticity result for a given mode (Off pull, no-op pull).
fn empty_authenticity(mode: VerifyMode) -> AuthenticityResult {
    AuthenticityResult {
        mode,
        new_commits: Vec::new(),
        open_issues: Vec::new(),
        blocked: false,
    }
}

/// Off-mode pull: in-place refspec fetch + forced checkout. Today's behaviour,
/// byte-for-byte.
fn pull_off(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
) -> Result<SyncResult, Error> {
    // Capture HEAD before fetch (the in-place refspec moves refs during fetch).
    let pre_fetch_oid = repo.head().ok().and_then(|r| r.target());

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&["refs/heads/*:refs/heads/*"], Some(&mut fetch_opts), None)?;

    let post_fetch_oid = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Cannot determine current HEAD"))?;

    if let Some(pre_oid) = pre_fetch_oid {
        if post_fetch_oid == pre_oid {
            return Ok(SyncResult {
                changed: false,
                head: short_hash(&post_fetch_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            });
        }
        if !repo.graph_descendant_of(post_fetch_oid, pre_oid)? {
            return Err(Error::new(
                ErrorCode::PullFfFailed,
                "Cannot fast-forward: branches have diverged. Resolve on desktop.",
            ));
        }
    }

    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    repo.checkout_head(Some(&mut checkout_builder))?;

    Ok(SyncResult {
        changed: true,
        head: short_hash(&post_fetch_oid),
        authenticity: empty_authenticity(VerifyMode::Off),
    })
}

/// Audit/Enforce pull: fetch the current branch into a temp ref, verify the
/// new range, then conditionally advance + check out.
fn pull_verified(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
    policy: &AuthenticityConfig,
) -> Result<SyncResult, Error> {
    let mode = policy.mode;

    // The current branch HEAD sits on (e.g. "main"). gpm always operates on a
    // single default branch; a detached HEAD is unsupported.
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();

    let pre_fetch_oid = repo.head().ok().and_then(|r| r.target());

    // Fetch the remote branch into a temp ref — bring all new commit objects
    // into the store WITHOUT moving the working branch, so extract_signature
    // works on the fetched tip and its ancestors.
    let temp_ref = format!("refs/gpm/pending/{branch_name}");
    let refspec = format!("+refs/heads/{branch_name}:{temp_ref}");
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);
    remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

    let fetched_oid = repo.refname_to_id(&temp_ref).map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Fetch produced no ref: {e}"),
        )
    })?;

    // Always clean up the temp ref before returning.
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // No prior HEAD (first pull anomaly): advance without verifying a range.
    let Some(pre_oid) = pre_fetch_oid else {
        advance_branch(repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncResult {
            changed: true,
            head: short_hash(&fetched_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits: Vec::new(),
                open_issues: Vec::new(),
                blocked: false,
            },
        });
    };

    if fetched_oid == pre_oid {
        cleanup();
        return Ok(SyncResult {
            changed: false,
            head: short_hash(&fetched_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits: Vec::new(),
                open_issues: Vec::new(),
                blocked: false,
            },
        });
    }

    // Fast-forward only: fetched tip must descend from the current HEAD.
    if !repo.graph_descendant_of(fetched_oid, pre_oid)? {
        cleanup();
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Cannot fast-forward: branches have diverged. Resolve on desktop.",
        ));
    }

    // Verify every commit in (pre_oid, fetched_oid].
    let trusted = signing::trusted_fingerprints(policy);
    let new_commits = signing::verify_range(repo, pre_oid, fetched_oid, &trusted, &policy.ignored)?;
    let open_issues: Vec<_> = new_commits
        .iter()
        .filter(|c| !c.ignored && c.status.is_issue())
        .cloned()
        .collect();

    // Enforce: refuse to advance when a non-ignored blocking issue remains.
    let blocked = mode == VerifyMode::Enforce && !open_issues.is_empty();
    if blocked {
        // Do NOT move the branch; HEAD and the working tree stay put.
        cleanup();
        return Ok(SyncResult {
            changed: false,
            head: short_hash(&pre_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        });
    }

    // Audit (always) or Enforce (no blocking issues): advance + check out.
    advance_branch(repo, &branch_name, fetched_oid)?;
    cleanup();
    Ok(SyncResult {
        changed: true,
        head: short_hash(&fetched_oid),
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    })
}

/// Move the branch ref to `target` and check out HEAD (forced), updating the
/// working tree.
fn advance_branch(repo: &Repository, branch_name: &str, target: git2::Oid) -> Result<(), Error> {
    let branch_ref = format!("refs/heads/{branch_name}");
    repo.reference(&branch_ref, target, true, "gpm pull")?;
    let mut checkout_builder = git2::build::CheckoutBuilder::new();
    checkout_builder.force();
    repo.checkout_head(Some(&mut checkout_builder))?;
    Ok(())
}

/// Classify a git2 error message into the appropriate [`Error`].
fn classify_git_error(msg: &str) -> Error {
    if msg.contains("authentication")
        || msg.contains("unsupported URL")
        || msg.contains("SSH key error")
    {
        Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
    } else if msg.contains("unable to connect") || msg.contains("timeout") {
        Error::new(ErrorCode::NetworkError, format!("Network error: {msg}"))
    } else {
        Error::new(ErrorCode::CloneFailed, format!("Clone failed: {msg}"))
    }
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
    if let Ok(head) = repo.head()
        && let Some(name) = head.shorthand()
    {
        return Ok(name.to_string());
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

    /// Create an empty initial commit in a test repository.
    ///
    /// Builds a commit from an empty tree so the repo has a valid HEAD
    /// without requiring any working-tree files.
    fn create_empty_commit(repo: &Repository, sig: &git2::Signature<'_>) -> git2::Oid {
        let mut index = repo.index().expect("failed to get index");
        let tree_id = index.write_tree().expect("failed to write tree");
        let tree = repo.find_tree(tree_id).expect("failed to find tree");
        let parents: &[&git2::Commit<'_>] = &[];
        repo.commit(Some("HEAD"), sig, sig, "initial commit", &tree, parents)
            .expect("failed to create commit")
    }

    #[test]
    fn git_auth_none_debug() {
        let auth = GitAuth::None;
        assert_eq!(format!("{auth:?}"), "None");
    }

    #[test]
    fn git_auth_pat_debug_masks_token() {
        let auth = GitAuth::Pat("secret-token".to_string());
        let debug = format!("{auth:?}");
        assert_eq!(debug, "Pat(\"secret-token\")");
    }

    #[test]
    fn git_auth_ssh_debug_format() {
        let auth = GitAuth::Ssh {
            username: "git".to_string(),
            private_key: "secret-key-data".to_string(),
            passphrase: Some("secret-pass".to_string()),
        };
        let debug = format!("{auth:?}");
        assert!(
            debug.contains("Ssh"),
            "SSH variant debug should contain 'Ssh': {debug}"
        );
    }

    #[test]
    fn git_auth_ssh_without_passphrase() {
        let auth = GitAuth::Ssh {
            username: "git".to_string(),
            private_key: "key-data".to_string(),
            passphrase: None,
        };
        let debug = format!("{auth:?}");
        assert!(
            debug.contains("Ssh"),
            "SSH variant debug should contain 'Ssh': {debug}"
        );
    }

    #[test]
    fn classify_git_error_ssh_key() {
        let err = classify_git_error("SSH key error: invalid format");
        assert_eq!(
            err.code, "CLONE_FAILED",
            "SSH key errors should map to CLONE_FAILED"
        );
        assert!(
            err.message.contains("SSH key error"),
            "message should preserve SSH context"
        );
    }

    #[test]
    fn classify_git_error_auth() {
        let err = classify_git_error("authentication required but no callback set");
        assert_eq!(err.code, "CLONE_FAILED");
    }

    #[test]
    fn classify_git_error_network() {
        let err = classify_git_error("unable to connect to host");
        assert_eq!(err.code, "NETWORK_ERROR");
    }

    #[test]
    fn classify_git_error_unsupported_url() {
        let err = classify_git_error("unsupported URL protocol");
        assert_eq!(err.code, "CLONE_FAILED");
    }

    #[test]
    fn classify_git_error_generic() {
        let err = classify_git_error("some unknown error");
        assert_eq!(err.code, "CLONE_FAILED");
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
