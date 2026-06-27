// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::Path;
use std::sync::OnceLock;

use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::error::{Error, ErrorCode};
use crate::signing::{self, AuthenticityConfig, VerifyMode};
use crate::store::{AuthenticityResult, SyncDivergence, SyncOutcome, SyncResult};

/// Mozilla (curl) root CA bundle, embedded so the libgit2 OpenSSL backend has a
/// trust anchor on targets with no discoverable system CA store — notably
/// Android, where the vendored OpenSSL build cannot read the system keystore and
/// HTTPS git otherwise fails with `SSL certificate is invalid`. Refreshed via
/// the `refresh-ca` just recipe; a unit test guards against a truncated/corrupt
/// bundle shipping.
pub const EMBEDDED_CA_BUNDLE: &str = include_str!("../data/cacert.pem");

/// Caches the one-time libgit2 CA-location setup so the "exactly once" safety
/// contract is enforced in code, not just in the SAFETY comment below: a repeat
/// call is a safe no-op that returns the cached outcome.
static CA_BUNDLE_RESULT: OnceLock<Result<(), Error>> = OnceLock::new();

/// Point libgit2's OpenSSL backend at `path` (a concatenated PEM of trusted
/// roots) for HTTPS certificate verification.
///
/// This is the only way to give the vendored OpenSSL backend a trust store on
/// Android (`SSL_CERT_FILE` is not honored), so it sets libgit2's
/// process-global CA location and must be called exactly once, at startup,
/// before any git network operation. The caller writes the bundle bytes — e.g.
/// [`EMBEDDED_CA_BUNDLE`] — to `path` first and applies any platform gating.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if libgit2 rejects the path.
#[allow(unsafe_code)]
pub fn set_ca_bundle(path: &Path) -> Result<(), Error> {
    CA_BUNDLE_RESULT
        .get_or_init(|| {
            // SAFETY: `git2::opts::set_ssl_cert_file` mutates a libgit2
            // process-global without synchronization. `OnceLock::get_or_init`
            // runs this closure at most once process-wide, and the sole call
            // site is app startup before any git network operation, so no
            // concurrent libgit2 TLS access is in flight (the global is read
            // only during a TLS handshake). A repeat call skips the closure
            // and returns the cached outcome — a safe no-op.
            unsafe { git2::opts::set_ssl_cert_file(path) }.map_err(|e| {
                Error::new(
                    ErrorCode::StoreError,
                    format!("set_ssl_cert_file failed: {e}"),
                )
            })
        })
        .clone()
}

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
pub fn init_repo(dest: &Path) -> Result<(), Error> {
    Repository::init(dest)?;
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
/// Returns a [`SyncOutcome`]: [`SyncOutcome::FastForwarded`] for a normal
/// pull, or [`SyncOutcome::Diverged`] when the branches have diverged — the
/// caller surfaces this for resolution instead of erroring.
///
/// # Errors
///
/// Returns an error if the repository cannot be found, the remote is
/// unreachable, or the branches have diverged (non-fast-forward).
pub fn pull_repo(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
) -> Result<SyncOutcome, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    // No `origin` → a local-only store (e.g. created with no remote). Pull is a
    // no-op: there is nothing to fetch. This mirrors the push no-op so the
    // gopass-style pre-write sync (`Store::set` → `sync`) never errors on a
    // local-only store. See `push_current_branch` for the matching case.
    let Ok(mut remote) = repo.find_remote("origin") else {
        let head = repo
            .head()
            .ok()
            .and_then(|r| r.target())
            .map_or_else(String::new, |oid| short_hash(&oid));
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: false,
            head,
            authenticity: empty_authenticity(policy.mode),
        }));
    };

    let callbacks = build_remote_callbacks(auth);

    // Off mode: temp-ref fetch + fast-forward (divergence surfaced, not errored).
    if policy.mode == VerifyMode::Off {
        return pull_off(&repo, &mut remote, callbacks);
    }

    // Audit / Enforce: verify-before-checkout.
    pull_verified(&repo, &mut remote, callbacks, policy)
}

/// The signature gpm commits under. `name` / `email` come from the configured
/// commit identity and fall back to the app default when `None`. gpm does not
/// (yet) SSH-sign its own commits; remote commits are verified on pull via the
/// authenticity layer.
fn gpm_signature(
    name: Option<&str>,
    email: Option<&str>,
) -> Result<git2::Signature<'static>, Error> {
    git2::Signature::now(
        name.unwrap_or(crate::config::DEFAULT_COMMIT_NAME),
        email.unwrap_or(crate::config::DEFAULT_COMMIT_EMAIL),
    )
    .map_err(|e| {
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

    let sig = gpm_signature(name, email)?;
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

    let sig = gpm_signature(name, email)?;
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
    let sig = gpm_signature(None, None)?;
    let parents: &[&git2::Commit<'_>] = &[];
    Ok(repo.commit(Some("HEAD"), &sig, &sig, message, &tree, parents)?)
}

/// Push the current branch to `origin` using `auth`.
///
/// `Err` here means the push was rejected — most commonly a non-fast-forward
/// because the remote advanced (the write-path conflict case).
fn push_current_branch(repo: &Repository, auth: &GitAuth) -> Result<(), Error> {
    // No `origin` → a local-only store (created with no remote). Push is a no-op:
    // there is nothing to push to. Mirrors the `pull_repo` no-op.
    //
    // LOAD-BEARING INVARIANT: this returns `Ok(())`, so `Store::write_commit_push`
    // returns `Ok(Some(head))` (not `Ok(None)`) for a local-only write. That is
    // what keeps `Store::set`'s conflict branch — `fetch_remote_blob` /
    // `fast_forward_to_remote`, both of which also call `find_remote("origin")`
    // — unreachable by construction for a local-only store: the happy path is
    // taken, so the conflict/replay branch never runs. Do not change this to
    // surface "no origin" as a rejection without reworking that branch.
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
    opts.remote_callbacks(build_remote_callbacks(auth));
    remote
        .push(&[&refspec], Some(&mut opts))
        .map_err(|e| classify_push_error(&e.to_string()))
}

/// Stage `rel_paths` and commit on the current branch. Returns the short hash
/// of the new HEAD commit. (Commit half of gopass's `gitCommitAndPush`.)
///
/// # Errors
///
/// Returns an error if the repo cannot be opened or staging/committing fails.
pub fn commit(
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
    Ok(short_hash(&head))
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
pub fn commit_removal(
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
    Ok(short_hash(&head))
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
pub fn commit_initial(
    repo_path: &Path,
    rel_paths: &[String],
    message: &str,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let oid = commit_initial_inner(&repo, rel_paths, message)?;
    Ok(short_hash(&oid))
}

/// Push the current branch to `origin`. (Push half of gopass's
/// `gitCommitAndPush`.)
///
/// # Errors
///
/// Returns `PushRejected` when the remote has diverged (non-fast-forward), or a
/// network/auth error otherwise.
pub fn push(repo_path: &Path, auth: &GitAuth) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    push_current_branch(&repo, auth)
}

/// Stage, commit, and push in one shot. Returns the new HEAD short hash.
/// Convenience wrapper kept for the simple write paths.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened, staging/committing fails, or
/// the push is rejected.
pub fn commit_and_push(
    repo_path: &Path,
    auth: &GitAuth,
    rel_paths: &[String],
    message: &str,
    name: Option<&str>,
    email: Option<&str>,
) -> Result<String, Error> {
    let head = commit(repo_path, rel_paths, message, name, email)?;
    push(repo_path, auth)?;
    Ok(head)
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
pub fn remote_add(repo_path: &Path, name: &str, url: &str) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    repo.remote(name, url)?;
    Ok(())
}

/// Hard-reset the current branch and worktree to `oid_str` (a full commit
/// hash). Used to roll back an unpushed write when its push is rejected.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened, the hash is invalid, or the
/// reset fails.
pub fn reset_hard_to(repo_path: &Path, oid_str: &str) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let oid = git2::Oid::from_str(oid_str)?;
    let target = repo.find_commit(oid)?;
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.force();
    repo.reset(target.as_object(), git2::ResetType::Hard, Some(&mut opts))?;
    Ok(())
}

/// Fetch `origin`'s current branch into a temp ref and return
/// `(branch_name, temp_ref_name, fetched_oid)`. The caller **must** delete the
/// temp ref when done. Mirrors the verified-pull probe so we can inspect remote
/// objects without moving the working branch.
fn fetch_remote_into_temp(
    repo: &Repository,
    auth: &GitAuth,
) -> Result<(String, String, git2::Oid), Error> {
    let branch = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot fetch"))?
        .to_string();

    let temp_ref = format!("refs/gpm/probe/{branch}");
    let refspec = format!("+refs/heads/{branch}:{temp_ref}");

    let mut remote = repo.find_remote("origin").map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Cannot find origin remote: {}", e.message()),
        )
    })?;
    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(build_remote_callbacks(auth));
    remote.fetch(&[&refspec], Some(&mut fetch_opts), None)?;

    let oid = repo.refname_to_id(&temp_ref).map_err(|e| {
        Error::new(
            ErrorCode::NetworkError,
            format!("Fetch produced no ref: {e}"),
        )
    })?;
    Ok((branch, temp_ref, oid))
}

/// Drop a temp ref if it exists (best-effort cleanup).
fn delete_temp_ref(repo: &Repository, temp_ref: &str) {
    drop(repo.find_reference(temp_ref).and_then(|mut r| r.delete()));
}

/// Read the blob content of `rel_path` at `commit_oid`, or `None` if the path
/// is absent from that commit's tree.
fn blob_at_commit(repo: &Repository, commit_oid: git2::Oid, rel_path: &str) -> Option<Vec<u8>> {
    let commit = repo.find_commit(commit_oid).ok()?;
    let tree = commit.tree().ok()?;
    let entry = tree.get_path(Path::new(rel_path)).ok()?;
    let blob = repo.find_blob(entry.id()).ok()?;
    Some(blob.content().to_vec())
}

/// Fetch `origin` and return the content of `rel_path` at the remote branch
/// tip, or `None` if the remote has no such file.
///
/// Used by the write path to detect a same-name remote entry and to assess
/// whether it is decryptable to us.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened or the fetch fails.
pub fn fetch_remote_blob(
    repo_path: &Path,
    auth: &GitAuth,
    rel_path: &str,
) -> Result<Option<Vec<u8>>, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let (_branch, temp_ref, tip) = fetch_remote_into_temp(&repo, auth)?;
    let blob = blob_at_commit(&repo, tip, rel_path);
    delete_temp_ref(&repo, &temp_ref);
    Ok(blob)
}

/// Fast-forward the current branch and worktree to `origin`'s branch tip
/// (discarding any local-only commits). Used by conflict resolution to adopt
/// the remote state (`KeepRemote`) or as the base for replaying our write.
///
/// # Errors
///
/// Returns an error if the repo cannot be opened or the fetch fails.
pub fn fast_forward_to_remote(repo_path: &Path, auth: &GitAuth) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let (_branch, temp_ref, tip) = fetch_remote_into_temp(&repo, auth)?;
    let target = repo.find_commit(tip)?;
    let mut opts = git2::build::CheckoutBuilder::new();
    opts.force();
    repo.reset(target.as_object(), git2::ResetType::Hard, Some(&mut opts))?;
    delete_temp_ref(&repo, &temp_ref);
    Ok(())
}

/// Map a libgit2 push error onto an [`Error`]. A non-fast-forward rejection
/// becomes `PushRejected` so the write path can distinguish "remote moved" from
/// generic network/auth failures.
fn classify_push_error(msg: &str) -> Error {
    // libgit2 reports divergence variously: "non-fast-forward",
    // "cannot push non-fastforwardable reference", code "NotFastForward", plus
    // the server-side "rejected" / "fetch first". Match case-insensitively.
    let lower = msg.to_ascii_lowercase();
    let rejected = lower.contains("non-fast-forward")
        || lower.contains("non-fastforward")
        || lower.contains("fastforwardable")
        || lower.contains("notfastforward")
        || lower.contains("fetch first")
        || lower.contains("rejected");
    if rejected {
        Error::new(
            ErrorCode::PushRejected,
            "Push rejected: remote has diverged. A sync/merge is required.",
        )
    } else if lower.contains("authentication") || lower.contains("credential") {
        Error::new(ErrorCode::CloneFailed, format!("Push auth failed: {msg}"))
    } else if lower.contains("unable to connect") || lower.contains("timeout") {
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

/// Off-mode pull: fetch the current branch into a temp ref, then fast-forward
/// (or report divergence). gpm operates on a single default branch, so fetching
/// one branch is correct; using a temp ref (like the verified path) means
/// divergence is detected reliably, and the working branch is never moved
/// speculatively — so a `Diverged` result leaves the repo byte-identical.
fn pull_off(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
) -> Result<SyncOutcome, Error> {
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();
    let pre_oid = repo.head().ok().and_then(|r| r.target());

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
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    let Some(pre_oid) = pre_oid else {
        advance_branch(repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: true,
            head: short_hash(&fetched_oid),
            authenticity: empty_authenticity(VerifyMode::Off),
        }));
    };

    if fetched_oid == pre_oid {
        cleanup();
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: false,
            head: short_hash(&fetched_oid),
            authenticity: empty_authenticity(VerifyMode::Off),
        }));
    }

    // Fetched tip is not a descendant of HEAD → diverged; surface for resolution.
    if !repo.graph_descendant_of(fetched_oid, pre_oid)? {
        let div = divergence_info(repo, pre_oid, fetched_oid)?;
        cleanup();
        return Ok(SyncOutcome::Diverged(div));
    }

    advance_branch(repo, &branch_name, fetched_oid)?;
    cleanup();
    Ok(SyncOutcome::FastForwarded(SyncResult {
        changed: true,
        head: short_hash(&fetched_oid),
        authenticity: empty_authenticity(VerifyMode::Off),
    }))
}

/// Audit/Enforce pull: fetch the current branch into a temp ref, verify the
/// new range, then conditionally advance + check out.
fn pull_verified(
    repo: &Repository,
    remote: &mut git2::Remote<'_>,
    callbacks: RemoteCallbacks<'_>,
    policy: &AuthenticityConfig,
) -> Result<SyncOutcome, Error> {
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
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: true,
            head: short_hash(&fetched_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits: Vec::new(),
                open_issues: Vec::new(),
                blocked: false,
            },
        }));
    };

    if fetched_oid == pre_oid {
        cleanup();
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: false,
            head: short_hash(&fetched_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits: Vec::new(),
                open_issues: Vec::new(),
                blocked: false,
            },
        }));
    }

    // Fast-forward only: fetched tip must descend from the current HEAD,
    // otherwise the branches have diverged — surface it for resolution.
    if !repo.graph_descendant_of(fetched_oid, pre_oid)? {
        let div = divergence_info(repo, pre_oid, fetched_oid)?;
        cleanup();
        return Ok(SyncOutcome::Diverged(div));
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
        return Ok(SyncOutcome::FastForwarded(SyncResult {
            changed: false,
            head: short_hash(&pre_oid),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        }));
    }

    // Audit (always) or Enforce (no blocking issues): advance + check out.
    advance_branch(repo, &branch_name, fetched_oid)?;
    cleanup();
    Ok(SyncOutcome::FastForwarded(SyncResult {
        changed: true,
        head: short_hash(&fetched_oid),
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    }))
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

/// Count commits reachable from `tip` but not from `base` (first-parent only).
fn count_ahead(repo: &Repository, tip: git2::Oid, base: git2::Oid) -> Result<usize, Error> {
    let mut walk = repo.revwalk()?;
    walk.push(tip)?;
    walk.hide(base)?;
    walk.simplify_first_parent()?;
    Ok(walk.filter_map(Result::ok).count())
}

/// Classify one local-side file loss for the divergence preview: `.age` files
/// become entry names (suffix stripped) and land in `secrets`; anything else
/// lands in `other` by path.
fn classify_loss(path: &Path, secrets: &mut Vec<String>, other: &mut Vec<String>) {
    let is_age = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("age"));
    let s = path.to_string_lossy().into_owned();
    if is_age {
        secrets.push(s.trim_end_matches(".age").to_string());
    } else {
        other.push(s);
    }
}

/// Build the divergence preview for a local-vs-remote split: ahead counts plus
/// the full set of local-side tracked-file changes an "adopt remote" would
/// discard/overwrite. Pure git tree diff — no decryption (so identical-plaintext
/// re-encryptions are over-reported as `modified` until a future enhancement).
fn divergence_info(
    repo: &Repository,
    local_oid: git2::Oid,
    remote_oid: git2::Oid,
) -> Result<SyncDivergence, Error> {
    let base = repo.merge_base(local_oid, remote_oid)?;
    let local_ahead = count_ahead(repo, local_oid, base)?;
    let remote_ahead = count_ahead(repo, remote_oid, base)?;

    let local_tree = repo.find_commit(local_oid)?.tree()?;
    let remote_tree = repo.find_commit(remote_oid)?.tree()?;
    // diff_tree_to_tree(old=local, new=remote): old_file() is the local side.
    let diff = repo.diff_tree_to_tree(Some(&local_tree), Some(&remote_tree), None)?;

    let mut local_only = Vec::new();
    let mut modified = Vec::new();
    let mut other = Vec::new();
    for delta in diff.deltas() {
        match delta.status() {
            // Present locally, absent remotely → deleted by an adopt.
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path() {
                    classify_loss(p, &mut local_only, &mut other);
                }
            }
            // Present on both sides but differing (incl. rename/copy) → overwritten.
            git2::Delta::Modified | git2::Delta::Renamed | git2::Delta::Copied => {
                if let Some(p) = delta.old_file().path() {
                    classify_loss(p, &mut modified, &mut other);
                }
            }
            // Added remotely (absent locally): adopting remote gains it — not a loss.
            _ => {}
        }
    }

    Ok(SyncDivergence {
        local_ahead,
        remote_ahead,
        remote_tip: remote_oid.to_string(),
        local_only_entries: local_only,
        modified_entries: modified,
        other_changed_files: other,
    })
}

/// Adopt the remote tip exactly as reviewed (`expected_remote_oid`): re-fetch,
/// refuse if the remote has moved past it, then under the configured
/// authenticity policy verify the remote-only commits and hard-advance the
/// branch to the remote tip. Mirrors `pull_verified` minus the fast-forward
/// guard (we are *resolving* divergence, so we adopt regardless).
///
/// # Errors
///
/// Returns [`ErrorCode::PullFfFailed`] if the remote advanced since the user
/// reviewed the divergence, or a git/signing error otherwise.
pub fn adopt_remote(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
    expected_remote_oid: &str,
) -> Result<SyncResult, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot pull"))?
        .to_string();

    let (_branch, temp_ref, fetched_oid) = fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // Stale-confirmation guard: adopt exactly the tip the user reviewed.
    let expected = git2::Oid::from_str(expected_remote_oid)?;
    if fetched_oid != expected {
        cleanup();
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Remote changed since you reviewed the divergence; pull again.",
        ));
    }

    let pre_oid = repo.head().ok().and_then(|r| r.target());
    let mode = policy.mode;

    if mode == VerifyMode::Off {
        advance_branch(&repo, &branch_name, fetched_oid)?;
        cleanup();
        return Ok(SyncResult {
            changed: pre_oid != Some(fetched_oid),
            head: short_hash(&fetched_oid),
            authenticity: empty_authenticity(VerifyMode::Off),
        });
    }

    // Audit/Enforce: verify the remote-only range (merge_base, fetched] — NOT
    // (pre_oid, fetched], which would violate `verify_range`'s descendant
    // contract on a divergence.
    let pre = pre_oid.unwrap_or(fetched_oid);
    let base = repo.merge_base(pre, fetched_oid).unwrap_or(pre);
    let trusted = signing::trusted_fingerprints(policy);
    let new_commits = signing::verify_range(&repo, base, fetched_oid, &trusted, &policy.ignored)?;
    let open_issues: Vec<_> = new_commits
        .iter()
        .filter(|c| !c.ignored && c.status.is_issue())
        .cloned()
        .collect();

    let blocked = mode == VerifyMode::Enforce && !open_issues.is_empty();
    if blocked {
        cleanup();
        return Ok(SyncResult {
            changed: false,
            head: short_hash(&pre),
            authenticity: AuthenticityResult {
                mode,
                new_commits,
                open_issues,
                blocked: true,
            },
        });
    }

    advance_branch(&repo, &branch_name, fetched_oid)?;
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

    // ── create-store primitives ──────────────────────────────────────────

    #[test]
    fn init_repo_and_commit_initial_create_first_commit_no_parent() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        init_repo(dir.path()).expect("init_repo");

        // Write a recipients file, then make the no-parent initial commit.
        std::fs::write(dir.path().join(".age-recipients"), "age1abc\n").unwrap();
        let message = "Initialized Store for age1abc";
        let head = commit_initial(dir.path(), &[".age-recipients".to_string()], message)
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
        assert!(tree.get_path(Path::new(".age-recipients")).is_ok());

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
    fn pull_repo_noops_without_origin() {
        let dir = tempfile::tempdir().expect("failed to create temp dir");
        init_repo(dir.path()).unwrap();
        let repo = Repository::open(dir.path()).unwrap();
        let oid = create_empty_commit(&repo, &test_signature());

        let policy = AuthenticityConfig::default();
        let outcome = pull_repo(dir.path(), &GitAuth::None, &policy)
            .expect("pull must no-op, not error, without origin");
        match outcome {
            SyncOutcome::FastForwarded(r) => {
                assert!(!r.changed, "no-op pull reports no change");
                assert_eq!(r.head, short_hash(&oid));
            }
            SyncOutcome::Diverged(d) => panic!("expected FastForwarded no-op, got Diverged: {d:?}"),
        }
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

    #[test]
    fn embedded_ca_bundle_is_valid_pem() {
        // Guards against a truncated/corrupt bundle shipping (e.g. a botched
        // refresh-ca). The full Mozilla root set carries well over 100 roots.
        let count = EMBEDDED_CA_BUNDLE
            .matches("-----BEGIN CERTIFICATE-----")
            .count();
        assert!(
            count >= 100,
            "embedded CA bundle has only {count} certs, expected a full Mozilla root set (~120+)"
        );
    }
}
