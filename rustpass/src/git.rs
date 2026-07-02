// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
#[cfg(target_os = "android")]
use std::ffi::c_int;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "android")]
use foreign_types::ForeignType;
use git2::{FetchOptions, RemoteCallbacks, Repository};

use crate::config::{DEFAULT_COMMIT_EMAIL, DEFAULT_COMMIT_NAME};
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

/// Before an HTTPS git op on Android, load the embedded Mozilla roots into
/// libgit2's OpenSSL trust store. No-op on non-Android targets and on non-HTTPS
/// remotes (SSH / local-only stores), so a failed load never blocks those flows.
///
/// `rustpass` owns the `git2`/libgit2 dependency, so it owns this
/// platform-specific transport workaround. The vendored OpenSSL is built
/// `no-stdio` on Android (`openssl-src`), which compiles out `BIO_new_file` —
/// the usual approach (`git2::opts::set_ssl_cert_file`) is impossible there,
/// since libgit2 eagerly calls `SSL_CTX_load_verify_locations` → `BIO_new_file`
/// → NULL → `x509 certificate routines::BIO lib`. Instead the bundle is parsed
/// from memory (`BIO_new_mem_buf` survives `no-stdio`) and each root is added
/// via libgit2's `GIT_OPT_ADD_SSL_X509_CERT` → `X509_STORE_add_cert` (no file
/// BIO). Runs at most once process-wide; see [`ensure_https_ca_loaded`].
#[allow(clippy::unnecessary_wraps)] // returns Err only on the android branch below
fn ensure_https_ca_for_origin(repo: &Repository) -> Result<(), Error> {
    #[cfg(target_os = "android")]
    {
        let origin_is_https = repo
            .find_remote("origin")
            .ok()
            .and_then(|r| r.url().map(|u| u.starts_with("https://")))
            .unwrap_or(false);
        if origin_is_https {
            ensure_https_ca_loaded()?;
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = repo;
    }
    Ok(())
}

/// libgit2 option to push a raw X509 root into its OpenSSL trust store. Not
/// exported by name in `libgit2-sys` 0.18.x; the value is the last member of
/// `git_libgit2_opt_t` in the vendored libgit2 1.9.4. [`ensure_https_ca_loaded`]
/// asserts the version, so a `git2` bump that shifts this fails loudly instead
/// of silently corrupting the trust store.
#[cfg(target_os = "android")]
const GIT_OPT_ADD_SSL_X509_CERT: c_int = 45;

/// Cache the one-time in-memory CA load. A second call would make
/// `X509_STORE_add_cert` reject every root as a duplicate, so the `OnceLock`
/// runs the load at most once process-wide and makes repeats a safe no-op
/// returning the cached outcome.
#[cfg(target_os = "android")]
static CA_LOAD_RESULT: std::sync::OnceLock<Result<(), Error>> = std::sync::OnceLock::new();

/// Load [`EMBEDDED_CA_BUNDLE`] into libgit2's trust store, once. Android only.
///
/// # Errors
///
/// Returns [`ErrorCode::StoreError`] if the bundle parses zero roots or a
/// mid-bundle parse failure would leave a partial trust store.
#[cfg(target_os = "android")]
fn ensure_https_ca_loaded() -> Result<(), Error> {
    CA_LOAD_RESULT
        .get_or_init(|| {
            // `add_certs_from_pem` mutates libgit2's process-global trust store,
            // so the `OnceLock` is load-bearing for thread-safety (not just the
            // duplicate-cert correctness noted on `CA_LOAD_RESULT`): it runs the
            // load exactly once process-wide; a repeat returns the cached
            // outcome.
            match add_certs_from_pem(EMBEDDED_CA_BUNDLE.as_bytes()) {
                Ok(n) if n > 0 => Ok(()),
                Ok(_) => Err(Error::new(
                    ErrorCode::StoreError,
                    "embedded CA bundle parsed 0 certificates; HTTPS git cannot verify servers",
                )),
                Err(e) => Err(e),
            }
        })
        .clone()
}

/// Initialize libgit2 (`git2::init` is crate-private) and assert the vendored
/// libgit2 is 1.9.x — the version the [`GIT_OPT_ADD_SSL_X509_CERT`] value is
/// pinned to.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn init_libgit2_for_ca_opts() -> Result<(), Error> {
    unsafe {
        libgit2_sys::git_libgit2_init();
        let (mut major, mut minor, mut rev): (c_int, c_int, c_int) = (0, 0, 0);
        let rc = libgit2_sys::git_libgit2_version(&mut major, &mut minor, &mut rev);
        if rc != 0 || !(major == 1 && minor == 9) {
            return Err(Error::new(
                ErrorCode::StoreError,
                format!(
                    "GIT_OPT_ADD_SSL_X509_CERT=45 is pinned to libgit2 1.9.x; got \
                     {major}.{minor}.{rev}"
                ),
            ));
        }
    }
    Ok(())
}

/// Parse every `BEGIN CERTIFICATE` block from a concatenated PEM bundle and call
/// `on_cert` for each. Returns the count parsed.
///
/// `openssl::x509::X509::stack_from_pem` owns the EOF-vs-parse-failure
/// distinction (clean end-of-data → a shorter/empty `Vec`; a corrupt block →
/// `Err`), so a truncated/corrupt bundle surfaces as an error instead of a
/// silent partial trust store. Desktop-runnable (only the `openssl` crate, no
/// libgit2 mutation) so the bundle-validity unit test exercises the real
/// OpenSSL parser instead of a string match.
#[cfg(any(target_os = "android", test))]
fn for_each_cert_in_pem(
    pem: &[u8],
    mut on_cert: impl FnMut(&openssl::x509::X509) -> Result<(), Error>,
) -> Result<usize, Error> {
    let certs = openssl::x509::X509::stack_from_pem(pem).map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("CA bundle failed to parse: {e}"),
        )
    })?;
    for cert in &certs {
        on_cert(cert)?;
    }
    Ok(certs.len())
}

/// Parse `pem` and add every root to libgit2's OpenSSL trust store via
/// `GIT_OPT_ADD_SSL_X509_CERT` (Android only — desktop finds the system store
/// itself). Returns the count added. Reuses [`for_each_cert_in_pem`] so the
/// parse path is shared with the desktop unit test.
#[cfg(target_os = "android")]
#[allow(unsafe_code)]
fn add_certs_from_pem(pem: &[u8]) -> Result<usize, Error> {
    init_libgit2_for_ca_opts()?;
    for_each_cert_in_pem(pem, |cert| {
        // SAFETY: `cert` is a valid X509 parsed from the embedded bundle, and
        // `git_libgit2_opts` is variadic with the X509* as the option arg.
        let rc = unsafe { libgit2_sys::git_libgit2_opts(GIT_OPT_ADD_SSL_X509_CERT, cert.as_ptr()) };
        if rc != 0 {
            return Err(Error::new(
                ErrorCode::StoreError,
                "libgit2 rejected an embedded CA root (GIT_OPT_ADD_SSL_X509_CERT)",
            ));
        }
        Ok(())
    })
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

/// Shared cancellation token. Set to `true` to abort an in-progress git
/// operation (clone/pull): the `transfer_progress` callback returns `false`,
/// libgit2 aborts the transfer, and the caller maps the result to
/// [`ErrorCode::Cancelled`].
pub type CancelToken = Arc<AtomicBool>;

/// Progress data reported by git2 during a transfer. Sent over a synchronous
/// [`ProgressSender`] from inside git2's C callbacks (which run on the blocking
/// thread), so the channel is `std::sync::mpsc` — not async — keeping the
/// library runtime-free.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GitProgress {
    /// Total objects the remote advertised.
    pub total_objects: usize,
    /// Objects received so far.
    pub received_objects: usize,
    /// Objects indexed so far.
    pub indexed_objects: usize,
    /// Raw bytes received so far.
    pub received_bytes: usize,
    /// Total deltas the remote advertised.
    pub total_deltas: usize,
    /// Deltas indexed so far.
    pub indexed_deltas: usize,
    /// Textual sideband message (e.g. "Counting objects"). `None` for pure
    /// transfer-stat updates.
    pub message: Option<String>,
}

/// Synchronous sender for [`GitProgress`], safe to call from git2's C callbacks
/// running on the blocking thread.
pub type ProgressSender = std::sync::mpsc::Sender<GitProgress>;

/// `true` if `cancel` is set, signalling the running git operation to abort.
fn cancelled(cancel: Option<&CancelToken>) -> bool {
    cancel.is_some_and(|c| c.load(Ordering::Relaxed))
}

/// Map a git2 transfer-progress snapshot onto the serialisable [`GitProgress`].
fn progress_from_transfer(p: &git2::Progress<'_>) -> GitProgress {
    GitProgress {
        total_objects: p.total_objects(),
        received_objects: p.received_objects(),
        indexed_objects: p.indexed_objects(),
        received_bytes: p.received_bytes(),
        total_deltas: p.total_deltas(),
        indexed_deltas: p.indexed_deltas(),
        message: None,
    }
}

/// If `result` failed and the cancel token is set, re-map it to
/// [`ErrorCode::Cancelled`]; otherwise pass it through. Wraps the fetch paths
/// (`pull_off`/`pull_verified`) whose `remote.fetch` `?`-propagates a libgit2
/// abort when `transfer_progress` returned `false`.
fn cancelled_or<T>(result: Result<T, Error>, cancel: Option<&CancelToken>) -> Result<T, Error> {
    match result {
        Err(_) if cancelled(cancel) => Err(Error::new(ErrorCode::Cancelled, "Pull cancelled")),
        other => other,
    }
}

/// Build credential callbacks based on the authentication method.
///
/// When `cancel`/`progress` are `Some`, also register `transfer_progress`
/// (reports object/byte stats and aborts the transfer by returning `false` once
/// the token is set) and `sideband_progress` (forwards textual messages).
fn build_remote_callbacks<'a>(
    auth: &'a GitAuth,
    cancel: Option<&CancelToken>,
    progress: Option<&ProgressSender>,
) -> RemoteCallbacks<'a> {
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

    // Report object/byte transfer stats; abort the transfer (return `false`)
    // once the cancel token is set — libgit2 then errors the in-flight fetch.
    let cancel_tok = cancel.cloned();
    let progress_tx = progress.cloned();
    callbacks.transfer_progress(move |p| {
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(progress_from_transfer(&p));
        }
        !cancelled(cancel_tok.as_ref())
    });

    // Forward textual sideband messages (e.g. "Counting objects..."). Also
    // abort the transfer once the cancel token is set: sideband fires more
    // often than `transfer_progress` during the counting/resolving phases, so
    // honouring it keeps cancel latency tight there too.
    let progress_tx = progress.cloned();
    let cancel_tok = cancel.cloned();
    callbacks.sideband_progress(move |msg| {
        if let Some(tx) = progress_tx.as_ref() {
            let _ = tx.send(GitProgress {
                message: Some(String::from_utf8_lossy(msg).into_owned()),
                ..Default::default()
            });
        }
        !cancelled(cancel_tok.as_ref())
    });

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
pub fn clone_repo(
    url: &str,
    dest: &Path,
    auth: &GitAuth,
    cancel: Option<&CancelToken>,
    progress: Option<&ProgressSender>,
) -> Result<(), Error> {
    #[cfg(target_os = "android")]
    if url.starts_with("https://") {
        ensure_https_ca_loaded()?;
    }
    // Remove existing directory if present (re-clone)
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }

    let callbacks = build_remote_callbacks(auth, cancel, progress);

    let mut fetch_opts = FetchOptions::new();
    fetch_opts.remote_callbacks(callbacks);

    let mut builder = git2::build::RepoBuilder::new();
    builder.fetch_options(fetch_opts);

    if let Err(e) = builder.clone(url, dest) {
        // A failed or cancelled clone leaves a partial `dest` on disk (notably
        // `config_dir/repo` after a user cancel). Remove it so the next attempt
        // starts clean, mirroring `Store::create_store`'s failure cleanup.
        let _ = std::fs::remove_dir_all(dest);
        return Err(if cancelled(cancel) {
            Error::new(ErrorCode::Cancelled, "Clone cancelled")
        } else {
            classify_git_error(e.message())
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
    cancel: Option<&CancelToken>,
    progress: Option<&ProgressSender>,
) -> Result<SyncOutcome, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    ensure_https_ca_for_origin(&repo)?;

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

    let callbacks = build_remote_callbacks(auth, cancel, progress);

    // Off mode: temp-ref fetch + fast-forward (divergence surfaced, not errored).
    if policy.mode == VerifyMode::Off {
        return cancelled_or(pull_off(&repo, &mut remote, callbacks), cancel);
    }

    // Audit / Enforce: verify-before-checkout.
    cancelled_or(pull_verified(&repo, &mut remote, callbacks, policy), cancel)
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
    opts.remote_callbacks(build_remote_callbacks(auth, None, None));
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
    ensure_https_ca_for_origin(&repo)?;
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
    fetch_opts.remote_callbacks(build_remote_callbacks(auth, None, None));
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
    ensure_https_ca_for_origin(&repo)?;
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
    ensure_https_ca_for_origin(&repo)?;
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

/// How the fetched remote tip relates to the local HEAD. Used by the pull paths
/// (and the sync-divergence preview) to tell the three benign cases apart from a
/// true split, so a strictly-local-ahead repo (unpushed commit, remote unchanged)
/// is a no-op pull rather than a spurious divergence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchClass {
    /// `fetched == local HEAD` — nothing changed either way.
    Equal,
    /// Remote is strictly ahead of local — a fast-forward pull applies it.
    RemoteAhead,
    /// Local is strictly ahead of remote — a pull is a no-op; a push publishes.
    LocalAhead,
    /// Neither is an ancestor of the other — true divergence, needs resolution.
    Diverged,
}

/// Classify the local-HEAD vs fetched-tip relationship via `graph_descendant_of`
/// in both directions. `Equal` is checked first (it is not a descendant relation).
fn classify_relation(
    repo: &Repository,
    pre: git2::Oid,
    fetched: git2::Oid,
) -> Result<FetchClass, Error> {
    if pre == fetched {
        return Ok(FetchClass::Equal);
    }
    if repo.graph_descendant_of(fetched, pre)? {
        return Ok(FetchClass::RemoteAhead);
    }
    if repo.graph_descendant_of(pre, fetched)? {
        return Ok(FetchClass::LocalAhead);
    }
    Ok(FetchClass::Diverged)
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

    // Classify the local-vs-remote relationship so a strictly-local-ahead repo
    // (an unpushed commit, remote unchanged) is a no-op pull — not a spurious
    // divergence. Only a true both-sides split surfaces for resolution.
    match classify_relation(repo, pre_oid, fetched_oid)? {
        FetchClass::Equal => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: short_hash(&fetched_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
        // Remote is behind us: nothing to fetch. The caller pushes to publish.
        FetchClass::LocalAhead => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: short_hash(&pre_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
        FetchClass::Diverged => {
            let div = divergence_info(repo, pre_oid, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::Diverged(div))
        }
        FetchClass::RemoteAhead => {
            advance_branch(repo, &branch_name, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: true,
                head: short_hash(&fetched_oid),
                authenticity: empty_authenticity(VerifyMode::Off),
            }))
        }
    }
}

/// Verify the `(pre_oid, fetched_oid]` range under `policy`, then — unless
/// Enforce blocks — fast-forward to `fetched_oid`. Returns the [`SyncResult`].
/// The caller owns temp-ref cleanup. (The `RemoteAhead` arm of [`pull_verified`].)
fn verify_and_advance(
    repo: &Repository,
    branch_name: &str,
    pre_oid: git2::Oid,
    fetched_oid: git2::Oid,
    policy: &AuthenticityConfig,
) -> Result<SyncResult, Error> {
    let mode = policy.mode;
    let trusted = signing::trusted_fingerprints(policy);
    let new_commits = signing::verify_range(repo, pre_oid, fetched_oid, &trusted, &policy.ignored)?;
    let open_issues: Vec<_> = new_commits
        .iter()
        .filter(|c| !c.ignored && c.status.is_issue())
        .cloned()
        .collect();

    // Enforce: refuse to advance when a non-ignored blocking issue remains —
    // HEAD and the working tree stay put.
    if mode == VerifyMode::Enforce && !open_issues.is_empty() {
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
    advance_branch(repo, branch_name, fetched_oid)?;
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

    // Classify before verifying: only a genuine remote-ahead fetch has a range
    // to verify. A local-ahead repo is a no-op pull (the caller pushes); a true
    // split surfaces for resolution. This mirrors `pull_off`.
    match classify_relation(repo, pre_oid, fetched_oid)? {
        FetchClass::Equal => {
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: short_hash(&fetched_oid),
                authenticity: AuthenticityResult {
                    mode,
                    new_commits: Vec::new(),
                    open_issues: Vec::new(),
                    blocked: false,
                },
            }))
        }
        FetchClass::LocalAhead => {
            // Remote is behind us: nothing to fetch or verify. Caller pushes.
            cleanup();
            Ok(SyncOutcome::FastForwarded(SyncResult {
                changed: false,
                head: short_hash(&pre_oid),
                authenticity: AuthenticityResult {
                    mode,
                    new_commits: Vec::new(),
                    open_issues: Vec::new(),
                    blocked: false,
                },
            }))
        }
        FetchClass::Diverged => {
            let div = divergence_info(repo, pre_oid, fetched_oid)?;
            cleanup();
            Ok(SyncOutcome::Diverged(div))
        }
        FetchClass::RemoteAhead => {
            let result = verify_and_advance(repo, &branch_name, pre_oid, fetched_oid, policy)?;
            cleanup();
            Ok(SyncOutcome::FastForwarded(result))
        }
    }
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
    let s = path.to_string_lossy().into_owned();
    if is_age_entry(path) {
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
    ensure_https_ca_for_origin(&repo)?;

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

/// A local-side `.age` entry to replay onto the remote tip during a "keep mine"
/// divergence resolution: its worktree-relative path plus its ciphertext blob at
/// the local HEAD. The caller decrypts + re-encrypts the blob — git has no
/// identity, so the crypto stays in `Store` (matching `encrypt_and_write`).
#[derive(Debug, Clone)]
pub(crate) struct KeepLocalReplay {
    /// Worktree-relative path, e.g. `servers/db.age`.
    pub rel_path: String,
    /// The entry's ciphertext at the local HEAD, to decrypt + re-encrypt.
    pub blob: Vec<u8>,
}

/// What a "keep mine" resolution must replay onto the reviewed remote tip.
#[derive(Debug, Clone)]
pub(crate) struct KeepLocalPlan {
    /// Full hash of the fetched remote tip the plan was computed against. Passed
    /// to [`keep_local_advance`] so the adopt reuses the SAME tip (no second
    /// fetch — a second fetch could race past the reviewed tip and bypass the
    /// authenticity check under Enforce).
    pub fetched_oid: String,
    /// Local-side `.age` entries to re-encrypt + write onto the tip.
    pub replays: Vec<KeepLocalReplay>,
    /// Local-side `.age` entries to re-delete on the tip (local deletions that
    /// "keep mine" preserves).
    pub deletes: Vec<String>,
    /// Authenticity outcome for the returned [`SyncResult`] (the remote-only
    /// range's verification). `blocked` is false here — a block is returned as
    /// [`KeepLocalOutcome::Blocked`].
    pub authenticity: AuthenticityResult,
}

/// Outcome of [`keep_local_plan`]: proceed with a plan, or stop because Enforce
/// refused the remote-only range (HEAD left unchanged).
#[derive(Debug, Clone)]
pub(crate) enum KeepLocalOutcome {
    /// Enforce blocked the adopt — HEAD unchanged; surface this result.
    Blocked(SyncResult),
    /// Proceed: replay the plan onto the reviewed remote tip.
    Plan(KeepLocalPlan),
}

/// Whether `path` is an `.age` secret (case-insensitive suffix).
fn is_age_entry(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("age"))
}

/// Defense-in-depth: ensure a worktree-relative path from a git tree diff resolves
/// inside the repo — only `Normal`/`CurDir` components (rejects `..`, leading `/`,
/// Windows drive prefixes). Git rejects `..` in tree entries and gpm validates
/// secret names on write, so this is a backstop; [`keep_local_finalize`] replays
/// paths sourced from a (possibly remote) tree diff, so it asserts containment
/// before any filesystem write/delete, mirroring `Store::assert_within_repo`.
fn rel_within_repo(rel: &str) -> Result<(), Error> {
    use std::path::Component;
    let outside = Path::new(rel)
        .components()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir));
    if outside {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }
    Ok(())
}

/// `.age`-entry changes on one side of a diff vs the base tree: paths the side
/// added/modified (with the side's blob, for replay) and paths it deleted. A
/// rename counts as delete(old) + add(new). Used for BOTH sides of a "keep mine"
/// plan — the local side yields what to replay; the remote side yields the
/// touched-path set for conflict detection (its blobs are unused).
struct AgeDiff {
    /// `(rel_path, blob_bytes)` the side has at `side_oid`.
    changed: Vec<(String, Vec<u8>)>,
    /// Worktree-relative paths the side deleted.
    deleted: Vec<String>,
}

/// Diff `base_tree` → `side_tree` and collect the `.age` changes on the side.
fn age_diff_side(
    repo: &Repository,
    base_tree: &git2::Tree<'_>,
    side_tree: &git2::Tree<'_>,
    side_oid: git2::Oid,
) -> Result<AgeDiff, Error> {
    let mut changed: Vec<(String, Vec<u8>)> = Vec::new();
    let mut deleted: Vec<String> = Vec::new();
    for delta in repo
        .diff_tree_to_tree(Some(base_tree), Some(side_tree), None)?
        .deltas()
    {
        match delta.status() {
            git2::Delta::Added | git2::Delta::Modified | git2::Delta::Copied => {
                if let Some(p) = delta.new_file().path()
                    && is_age_entry(p)
                {
                    let rel = p.to_string_lossy().into_owned();
                    let blob = blob_at_commit(repo, side_oid, &rel).unwrap_or_default();
                    changed.push((rel, blob));
                }
            }
            git2::Delta::Deleted => {
                if let Some(p) = delta.old_file().path()
                    && is_age_entry(p)
                {
                    deleted.push(p.to_string_lossy().into_owned());
                }
            }
            // A rename is delete(old) + add(new).
            git2::Delta::Renamed => {
                if let Some(old) = delta.old_file().path()
                    && is_age_entry(old)
                {
                    deleted.push(old.to_string_lossy().into_owned());
                }
                if let Some(new) = delta.new_file().path()
                    && is_age_entry(new)
                {
                    let rel = new.to_string_lossy().into_owned();
                    let blob = blob_at_commit(repo, side_oid, &rel).unwrap_or_default();
                    changed.push((rel, blob));
                }
            }
            _ => {}
        }
    }
    Ok(AgeDiff { changed, deleted })
}

/// If a `.age` entry was changed on BOTH sides (an irreconcilable same-secret
/// conflict), return the `PushRejected` error. A local replay collides with ANY
/// remote touch; a local delete collides only with a non-delete remote change
/// (both-deleted is agreement, not a conflict). The caller cleans up before
/// propagating the error.
fn keep_local_conflict(
    replays: &[KeepLocalReplay],
    deletes: &[String],
    remote_touched: &HashMap<String, bool>,
) -> Result<(), Error> {
    for r in replays {
        if remote_touched.contains_key(&r.rel_path) {
            return Err(Error::new(
                ErrorCode::PushRejected,
                format!(
                    "Can't keep mine: \"{}\" changed on both sides. Adopt the remote or cancel.",
                    r.rel_path.trim_end_matches(".age")
                ),
            ));
        }
    }
    for d in deletes {
        if matches!(remote_touched.get(d), Some(false)) {
            return Err(Error::new(
                ErrorCode::PushRejected,
                format!(
                    "Can't keep mine: \"{}\" was deleted locally but changed remotely. \
                     Adopt the remote or cancel.",
                    d.trim_end_matches(".age")
                ),
            ));
        }
    }
    Ok(())
}

/// Compute the "keep mine" plan: fetch the remote tip, refuse if it moved past
/// the reviewed `expected_remote_oid`, verify the remote-only range under the
/// authenticity policy (mirroring [`adopt_remote`]), then compute which local
/// `.age` entries to replay (re-encrypt) and which to re-delete on the tip. Does
/// NOT move HEAD — the caller decrypts/re-encrypts, then
/// [`keep_local_advance`] + [`keep_local_finalize`] apply it.
///
/// Refuses ([`ErrorCode::PushRejected`]) when a `.age` entry was changed on BOTH
/// sides (an irreconcilable same-secret conflict) — the user must adopt the
/// remote or cancel; gpm never merges `.age` blobs.
///
/// Non-secret local changes (`.gopass-recipients`, templates) are NOT replayed:
/// "keep mine" adopts the remote's non-secret files verbatim and re-encrypts only
/// secrets onto them. gpm is single-identity today, so local recipient edits do
/// not arise; multi-recipient overwrite-safety is deferred (TODO).
pub(crate) fn keep_local_plan(
    repo_path: &Path,
    auth: &GitAuth,
    policy: &AuthenticityConfig,
    expected_remote_oid: &str,
) -> Result<KeepLocalOutcome, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let pre_oid = repo.head().ok().and_then(|r| r.target()).ok_or_else(|| {
        Error::new(
            ErrorCode::PullFfFailed,
            "No HEAD to compute a keep-mine plan",
        )
    })?;

    let (_branch, temp_ref, fetched_oid) = fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };

    // Stale-confirmation guard: keep exactly the tip the user reviewed.
    let expected = git2::Oid::from_str(expected_remote_oid)?;
    if fetched_oid != expected {
        cleanup();
        return Err(Error::new(
            ErrorCode::PullFfFailed,
            "Remote changed since you reviewed the divergence; pull again.",
        ));
    }

    let base = repo.merge_base(pre_oid, fetched_oid)?;
    let mode = policy.mode;

    // Authenticity: verify the remote-only range (base, fetched] — identical to
    // adopt_remote. A block under Enforce stops here with HEAD untouched.
    let (new_commits, open_issues, blocked) = if mode == VerifyMode::Off {
        (Vec::new(), Vec::new(), false)
    } else {
        let trusted = signing::trusted_fingerprints(policy);
        let nc = signing::verify_range(&repo, base, fetched_oid, &trusted, &policy.ignored)?;
        let oi: Vec<_> = nc
            .iter()
            .filter(|c| !c.ignored && c.status.is_issue())
            .cloned()
            .collect();
        let bl = mode == VerifyMode::Enforce && !oi.is_empty();
        (nc, oi, bl)
    };
    if blocked {
        cleanup();
        return Ok(KeepLocalOutcome::Blocked(SyncResult {
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

    let base_tree = repo.find_commit(base)?.tree()?;
    let local_tree = repo.find_commit(pre_oid)?.tree()?;
    let remote_tree = repo.find_commit(fetched_oid)?.tree()?;

    // Local changes vs base: entries to replay (added/modified) or re-delete.
    let local_diff = age_diff_side(&repo, &base_tree, &local_tree, pre_oid)?;
    let replays: Vec<KeepLocalReplay> = local_diff
        .changed
        .into_iter()
        .map(|(rel_path, blob)| KeepLocalReplay { rel_path, blob })
        .collect();
    let deletes = local_diff.deleted;

    // Remote changes vs base: every `.age` path the remote touched (value = was
    // it a deletion?), for same-secret conflict detection.
    let remote_diff = age_diff_side(&repo, &base_tree, &remote_tree, fetched_oid)?;
    let mut remote_touched: HashMap<String, bool> = HashMap::new();
    for (p, _) in remote_diff.changed {
        remote_touched.insert(p, false);
    }
    for p in remote_diff.deleted {
        remote_touched.insert(p, true);
    }

    // Refuse irreconcilable same-secret conflicts (both sides touched the same
    // `.age` entry). See [`keep_local_conflict`].
    if let Err(e) = keep_local_conflict(&replays, &deletes, &remote_touched) {
        cleanup();
        return Err(e);
    }

    cleanup();
    Ok(KeepLocalOutcome::Plan(KeepLocalPlan {
        fetched_oid: fetched_oid.to_string(),
        replays,
        deletes,
        authenticity: AuthenticityResult {
            mode,
            new_commits,
            open_issues,
            blocked: false,
        },
    }))
}

/// Advance the branch + worktree to the reviewed remote tip (`fetched_oid`),
/// WITHOUT refetching. The fetched commit is still in the object DB (the plan
/// only deleted its temp ref), so this reuses the exact tip the authenticity
/// check ran against — no TOCTOU under Enforce.
pub(crate) fn keep_local_advance(repo_path: &Path, fetched_oid: &str) -> Result<(), Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let branch_name = repo
        .head()?
        .shorthand()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "Detached HEAD; cannot advance"))?
        .to_string();
    let target = git2::Oid::from_str(fetched_oid)?;
    advance_branch(&repo, &branch_name, target)
}

/// Apply a "keep mine" plan onto the (already-advanced) remote tip: write the
/// re-encrypted `entries`, apply the local `deletes`, commit on HEAD, and push
/// (now a fast-forward — our commit sits on the reviewed remote tip). Returns the
/// new HEAD short hash. Crypto is done by the caller; this is pure git + IO.
pub(crate) fn keep_local_finalize(
    repo_path: &Path,
    auth: &GitAuth,
    entries: &[(String, Vec<u8>)],
    deletes: &[String],
    name: Option<&str>,
    email: Option<&str>,
) -> Result<String, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;

    let mut index = repo.index()?;
    for (rel, ciphertext) in entries {
        rel_within_repo(rel)?;
        let file_path = repo_path.join(rel);
        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&file_path, ciphertext)?;
        index.add_path(Path::new(rel)).map_err(|e| {
            Error::new(ErrorCode::StoreError, format!("Failed to stage {rel}: {e}"))
        })?;
    }
    for rel in deletes {
        rel_within_repo(rel)?;
        let file_path = repo_path.join(rel);
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
        // Tolerate an already-gone index entry: the remote may have deleted it
        // too (both-deleted agreement). remove_path errors on an untracked path.
        let _ = index.remove_path(Path::new(rel));
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
    repo.commit(
        Some("HEAD"),
        &sig,
        &sig,
        "Keep local changes (re-encrypted onto remote)",
        &tree,
        &[&parent],
    )?;

    push_current_branch(&repo, auth)?;

    let head = repo
        .head()?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD after keep-mine commit"))?;
    Ok(short_hash(&head))
}

/// Fetch the remote tip and compute the local-vs-remote divergence preview,
/// WITHOUT moving the working branch. Called after a push rejection (the write
/// path knows divergence is real) so the app can surface the resolution modal on
/// demand.
pub(crate) fn preview_divergence(
    repo_path: &Path,
    auth: &GitAuth,
) -> Result<SyncDivergence, Error> {
    let repo = Repository::discover(repo_path)
        .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
    let pre_oid = repo
        .head()
        .ok()
        .and_then(|r| r.target())
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD to compute divergence"))?;
    let (_branch, temp_ref, fetched_oid) = fetch_remote_into_temp(&repo, auth)?;
    let cleanup = || {
        drop(repo.find_reference(&temp_ref).and_then(|mut r| r.delete()));
    };
    let div = divergence_info(&repo, pre_oid, fetched_oid)?;
    cleanup();
    Ok(div)
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

    // ── cancel token + progress ──────────────────────────────────────────

    #[test]
    fn cancelled_is_false_without_token() {
        assert!(!cancelled(None));
    }

    #[test]
    fn cancelled_reads_token_state() {
        let token: CancelToken = Arc::new(AtomicBool::new(false));
        assert!(!cancelled(Some(&token)));
        token.store(true, Ordering::Relaxed);
        assert!(cancelled(Some(&token)));
    }

    #[test]
    fn cancelled_or_passes_through_success() {
        let token = Arc::new(AtomicBool::new(true));
        let ok: Result<(), Error> = Ok(());
        let out: Result<(), Error> = cancelled_or(ok, Some(&token));
        assert!(out.is_ok(), "an Ok result is never re-mapped to Cancelled");
    }

    #[test]
    fn cancelled_or_maps_failure_to_cancelled_when_set() {
        let token = Arc::new(AtomicBool::new(true));
        let err = Error::new(ErrorCode::NetworkError, "boom");
        let out: Result<(), Error> = cancelled_or(Err(err), Some(&token));
        let e = out.expect_err("set token must remap the error to Cancelled");
        assert_eq!(e.code, "CANCELLED");
    }

    #[test]
    fn cancelled_or_keeps_original_error_when_not_set() {
        let token = Arc::new(AtomicBool::new(false));
        let err = Error::new(ErrorCode::NetworkError, "boom");
        let out: Result<(), Error> = cancelled_or(Err(err), Some(&token));
        let e = out.expect_err("unset token must keep the original error");
        assert_eq!(e.code, "NETWORK_ERROR");
    }

    // ── transfer_progress callback wiring ────────────────────────────────
    //
    // Local clones copy objects directly and bypass the fetch callbacks, so we
    // force the REMOTE transport via `CloneLocal::None` — the same
    // `transfer_progress` path a real https/ssh clone drives, where our
    // cancel/progress hooks ride.

    /// Build a bare "remote" seeded with one committed file.
    fn bare_repo_with_file() -> tempfile::TempDir {
        let work = tempfile::tempdir().expect("work dir");
        let repo = Repository::init(work.path()).expect("init work");
        std::fs::write(work.path().join("f.age"), b"secret").expect("write file");
        let mut index = repo.index().expect("index");
        index.add_path(Path::new("f.age")).expect("add path");
        index.write().expect("index write");
        let tree_id = index.write_tree().expect("write tree");
        drop(index);
        let tree = repo.find_tree(tree_id).expect("tree");
        let sig = test_signature();
        repo.commit(Some("HEAD"), &sig, &sig, "init", &tree, &[])
            .expect("commit");
        drop(tree);
        drop(repo);

        let bare = tempfile::tempdir().expect("bare dir");
        git2::build::RepoBuilder::new()
            .bare(true)
            .clone(work.path().to_str().expect("work path utf-8"), bare.path())
            .expect("bare clone");
        bare
    }

    /// Clone `src` → `dest` over the fetch transport (`CloneLocal::None`), using
    /// [`build_remote_callbacks`] with the given cancel/progress hooks.
    fn clone_via_fetch(
        src: &Path,
        dest: &Path,
        cancel: Option<&CancelToken>,
        progress: Option<&ProgressSender>,
    ) -> Result<(), git2::Error> {
        let auth = GitAuth::None;
        let callbacks = build_remote_callbacks(&auth, cancel, progress);
        let mut fetch_opts = FetchOptions::new();
        fetch_opts.remote_callbacks(callbacks);
        let mut builder = git2::build::RepoBuilder::new();
        builder.clone_local(git2::build::CloneLocal::None);
        builder.fetch_options(fetch_opts);
        builder.clone(src.to_str().expect("src path utf-8"), dest)?;
        Ok(())
    }

    #[test]
    fn transfer_progress_reports_objects_over_fetch_transport() {
        let bare = bare_repo_with_file();
        let dest = tempfile::tempdir().expect("dest dir");
        let (tx, rx) = std::sync::mpsc::channel::<GitProgress>();
        clone_via_fetch(bare.path(), dest.path(), None, Some(&tx))
            .expect("clone via fetch transport should succeed");
        drop(tx); // close the channel so the drain terminates

        let messages: Vec<GitProgress> = rx.iter().collect();
        assert!(
            messages.iter().any(|m| m.total_objects > 0),
            "expected a transfer-progress update reporting objects, got: {messages:?}"
        );
    }

    #[test]
    fn transfer_progress_aborts_clone_when_cancel_pre_armed() {
        let bare = bare_repo_with_file();
        let dest = tempfile::tempdir().expect("dest dir");
        let cancel: CancelToken = Arc::new(AtomicBool::new(true));
        let result = clone_via_fetch(bare.path(), dest.path(), Some(&cancel), None);
        assert!(
            result.is_err(),
            "a pre-armed cancel token must make transfer_progress return false and abort the clone"
        );
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
        let outcome = pull_repo(dir.path(), &GitAuth::None, &policy, None, None)
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
    fn embedded_ca_bundle_parses_cleanly() {
        // Regression guard: parse the shipped bundle through the real OpenSSL
        // PEM reader — the same path Android uses to load roots — not just a
        // string match. A botched `refresh-ca` (truncated/corrupt bundle) must
        // either fail to parse or yield the full ~120+ root set, never a silent
        // partial trust store.
        let count = for_each_cert_in_pem(EMBEDDED_CA_BUNDLE.as_bytes(), |_| Ok(()))
            .expect("embedded CA bundle must parse cleanly under the real OpenSSL PEM reader");
        assert!(
            count >= 100,
            "embedded CA bundle parsed only {count} certs, expected a full Mozilla root set (~120+)"
        );
    }

    #[test]
    fn for_each_cert_in_pem_rejects_corrupt_pem() {
        // A `BEGIN CERTIFICATE` line followed by non-base64 must surface as a
        // parse failure, not silently parse zero certs (which would otherwise
        // load an empty trust store).
        let corrupt =
            b"-----BEGIN CERTIFICATE-----\n@@@ not valid base64 @@@\n-----END CERTIFICATE-----\n";
        let result = for_each_cert_in_pem(corrupt, |_| Ok(()));
        assert!(
            result.is_err(),
            "corrupt PEM must error, not silently parse zero certs"
        );
    }

    #[test]
    fn for_each_cert_in_pem_empty_is_zero() {
        // Empty / non-PEM input parses to zero certs with no error (clean EOF).
        // The Android loader's zero-count guard turns `Ok(0)` into a hard error
        // so HTTPS never proceeds with an empty trust store.
        let count = for_each_cert_in_pem(b"", |_| Ok(()))
            .expect("empty input is clean EOF, not a parse failure");
        assert_eq!(count, 0, "empty PEM should parse zero certs without error");
    }
}
