// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The git "write" surface — gopass's `gitCommitAndPush` half: `clone_repo`,
//! `init_repo`, `remote_add`, stage+commit (add / remove / initial), and `push`.
//! All blocking, adapted to async by [`backend::GitStorage`](super::backend::GitStorage)'s
//! `StorageBackend` impl via `spawn_blocking`.

use std::path::Path;

use git2::Repository;

use crate::error::{Error, ErrorCode};
use crate::storage::GitAuth;

use super::{transport, util};

/// Clone a git repository to a local directory.
///
/// Supports HTTPS (PAT) and SSH key authentication via [`GitAuth`].
///
/// # Errors
///
/// Returns an error if the clone fails due to authentication, network, or
/// filesystem issues.
pub(super) fn clone_repo(
    url: &str,
    dest: &Path,
    auth: &GitAuth,
    cancel: Option<&crate::storage::CancelToken>,
    progress: Option<&crate::storage::ProgressSender>,
) -> Result<(), Error> {
    #[cfg(target_os = "android")]
    if url.starts_with("https://") {
        transport::ensure_https_ca_loaded()?;
    }
    // Remove existing directory if present (re-clone)
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }

    let callbacks = transport::build_remote_callbacks(auth, cancel, progress);

    let mut fetch_opts = git2::FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    if let Err(e) = builder.clone(url, dest) {
        // A failed or cancelled clone leaves a partial `dest` on disk (notably
        // `config_dir/repo` after a user cancel). Remove it so the next attempt
        // starts clean, mirroring `Store::create_store`'s failure cleanup.
        let _ = std::fs::remove_dir_all(dest);
        return Err(if transport::cancelled(cancel) {
            Error::new(ErrorCode::Cancelled, "Clone cancelled")
        } else {
            transport::classify_git_error(e.message())
        });
    }

    Ok(())
}

/// Initialize a new git repository at `dest` (gopass's `gitInit`).
///
/// Creates the repo on the system default branch (main/master per
/// `init.defaultBranch`). Makes no commits and adds no remote — the create-store
/// flow writes `.age-recipients`, makes the initial commit, and (optionally)
/// adds a remote afterwards.
///
/// # Errors
///
/// Returns an error if `Repository::init` fails.
pub(super) fn init_repo(dest: &Path) -> Result<(), Error> {
    Repository::init(dest)?;
    Ok(())
}

/// Stage `rel_paths` (paths relative to the worktree root), create a commit on
/// the current branch, and return the new commit OID.
fn add_and_commit(
    repo: &Repository,
    rel_paths: &[String],
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
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

    let sig = util::gpm_signature(name, email)?;
    Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?)
}

/// Like [`add_and_commit`] but stages **removals**: the worktree file is already
/// gone (the caller removes it), and this drops the index entry via
/// `index.remove_path` so the commit records the deletion. The delete-path
/// sibling of [`add_and_commit`] (`git rm` vs `git add`).
fn remove_and_commit(
    repo: &Repository,
    rel_paths: &[String],
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
) -> Result<git2::Oid, Error> {
    let mut index = repo.index()?;
    for p in rel_paths {
        index.remove_path(Path::new(p)).map_err(|e| {
            Error::new(
                ErrorCode::StoreError,
                format!("Failed to stage removal of {p}: {e}"),
            )
        })?;
    }
    index.write()?;
    let tree_id = index.write_tree()?;
    let tree = repo.find_tree(tree_id)?;

    let head_oid = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit to build on"))?;
    let parent = repo.find_commit(head_oid)?;

    let sig = util::gpm_signature(name, email)?;
    Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, &[&parent])?)
}

/// Like [`add_and_commit`] but with **no parent commit** — the first commit on a
/// freshly initialized repo. [`add_and_commit`] looks up HEAD as the parent,
/// which fails before the first commit exists (gopass's "Initialized Store"
/// commit has no ancestors).
fn commit_initial_inner(
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

    // The "Initialized Store" commit is authored under the app default
    // identity: the create flow runs at first run, before any commit identity
    // is configured, so there is no `name`/`email` to thread yet.
    let sig = util::gpm_signature(None, None)?;
    let parents: &[&git2::Commit<'_>] = &[];
    Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, parents)?)
}

/// Push the current branch to `origin` using `auth`.
///
/// `Err` here means the push was rejected — most commonly a non-fast-forward
/// because the remote advanced (the write-path conflict case).
pub(super) fn push_current_branch(repo: &Repository, auth: &GitAuth) -> Result<(), Error> {
    // No `origin` → a local-only store (created with no remote). Push is a no-op:
    // there is nothing to push to. Mirrors the `pull_repo` no-op.
    //
    // LOAD-BEARING INVARIANT: this returns `Ok(())`, so `Store::push_locked`
    // (called by `autosync_write` after a local write, and by `sync_repo`) sees
    // a successful push for a local-only store. That keeps the orchestrator on
    // its happy path — `autosync_write` returns `Written`, `sync_repo` returns
    // `FastForwarded` — instead of misreading "no origin" as a rejection and
    // surfacing a spurious divergence. Do not change this to surface "no origin"
    // as a rejection without reworking the orchestrator's success/divergence
    // branching.
    let Ok(mut remote) = repo.find_remote("origin") else {
        return Ok(());
    };

    let branch = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot push"))?
        .to_string();

    let refspec = format!("refs/heads/{branch}:refs/heads/{branch}");
    let mut opts = git2::PushOptions::new();
    opts.remote_callbacks(transport::build_remote_callbacks(auth, None, None));
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(|e| transport::classify_push_error(&e.to_string()))
}

/// Stage `rel_paths` and commit on the current branch. Returns the short hash
/// of the new HEAD commit. (Commit half of gopass's `gitCommitAndPush`.)
///
/// # Errors
///
/// Returns an error if the repo cannot be opened or staging/committing fails.
pub(super) fn commit(
    repo_path: &Path,
    rel_paths: &[String],
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    add_and_commit(&repo, rel_paths, message, name, email)?;
    let head = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD after commit"))?;
    Ok(util::short_hash(&head))
}

/// Stage the **removal** of `rel_paths` (the worktree files are already gone) and
/// commit on the current branch. The delete-path sibling of [`commit`]: identical
/// signature, but [`remove_and_commit`] (index `remove_path`) instead of
/// [`add_and_commit`] (index `add_path`). Returns the short hash of the new HEAD.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened or staging/committing fails
/// (e.g. a path that isn't tracked, or a stale `index.lock`).
pub(super) fn commit_removal(
    repo_path: &Path,
    rel_paths: &[String],
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    remove_and_commit(&repo, rel_paths, message, name, email)?;
    let head = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD after commit"))?;
    Ok(util::short_hash(&head))
}

/// Stage `rel_paths` and create the **initial** commit (no parent) — gopass's
/// "Initialized Store" commit on a freshly `git init`ed repo. [`commit`] (and
/// [`add_and_commit`]) build on an existing HEAD; this commits with no parents,
/// the only valid first commit. Returns the short hash of the new HEAD commit.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened, staging fails, or the commit
/// cannot be created.
pub(super) fn commit_initial(
    repo_path: &Path,
    rel_paths: &[String],
    message: &str,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let oid = commit_initial_inner(&repo, rel_paths, message)?;
    Ok(util::short_hash(&oid))
}

/// Push the current branch to `origin`. (Push half of gopass's
/// `gitCommitAndPush`.)
///
/// # Errors
///
/// Returns `PushRejected` when the remote has diverged (non-fast-forward), or a
/// network/auth error otherwise.
pub(super) fn push(repo_path: &Path, auth: &GitAuth) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    transport::ensure_https_ca_for_origin(&repo)?;
    push_current_branch(&repo, auth)
}

/// Add a remote named `name` pointing at `url` (gopass's `addRemote`). This is
/// **local only** — it records the remote config without contacting it. The
/// first push to the remote happens later, after the store's identity is durable
/// (the deferred-push step of the create flow).
///
/// # Errors
///
/// Returns an error if the repo cannot be opened, or the remote cannot be added
/// (e.g. a remote of that name already exists).
pub(super) fn remote_add(repo_path: &Path, name: &str, url: &str) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    repo.remote(name, url)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::recipient::RECIPIENTS_FILE;
    use crate::storage::git::test_support::{create_empty_commit, test_signature};

    use super::*;

    #[test]
    fn init_repo_and_commit_initial_create_first_commit_no_parent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        init_repo(dir.path()).expect("init_repo");

        // Write a recipients file, then make the no-parent initial commit.
        std::fs::write(dir.path().join(RECIPIENTS_FILE), "age1abc\n").unwrap();
        let message = "Initialized Store for age1abc";
        let head = commit_initial(dir.path(), &[RECIPIENTS_FILE.to_string()], message)
            .expect("commit_initial");
        assert!(!head.is_empty());

        let repo = Repository::open(dir.path()).unwrap();
        let oid = repo.head().unwrap().target().unwrap();
        let head_commit = repo.find_commit(oid).unwrap();
        assert_eq!(head_commit.message(), Some(message));
        assert_eq!(
            head_commit.parent_count(),
            0,
            "initial commit must have no parents"
        );
        // .age-recipients is recorded in the commit tree.
        let tree = head_commit.tree().unwrap();
        assert!(tree.get_path(Path::new(RECIPIENTS_FILE)).is_ok());

        // A follow-up commit (which needs a parent HEAD) works after the initial.
        std::fs::write(dir.path().join("foo.age"), b"x").unwrap();
        let second = commit(dir.path(), &["foo.age".to_string()], "second", None, None).unwrap();
        assert_ne!(second, head);
    }

    #[test]
    fn push_current_branch_noops_without_origin() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).unwrap();
        let _oid = create_empty_commit(&repo, &test_signature());

        // No `origin` configured → push is a no-op (Ok), not an error.
        push_current_branch(&repo, &GitAuth::None).expect("push no-ops without origin");
    }

    #[test]
    fn remote_add_records_origin_locally() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        let repo = Repository::init(dir.path()).unwrap();
        drop(repo);

        // remote_add is local only — a bogus URL never touches the network.
        remote_add(dir.path(), "origin", "https://example.invalid/repo.git").unwrap();

        let repo = Repository::open(dir.path()).unwrap();
        let remote = repo.find_remote("origin").expect("origin should exist");
        assert_eq!(remote.name(), Some("origin"));
        assert_eq!(remote.url(), Some("https://example.invalid/repo.git"));
    }
}
