// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Storage / RCS backend abstraction.
//!
//! Home of the [`StorageBackend`] trait (mirroring gopass's `Storage` interface,
//! which embeds RCS). The sole implementation today is [`crate::storage::git::GitStorage`];
//! `Store` holds a `Box<dyn StorageBackend>` and routes every working-tree file
//! op through it. RCS methods (clone/pull/push/keep-mine) join the trait as the
//! git module is consolidated into `storage/git`.
//!
//! This module also owns the shared sync / write / commit-identity *result*
//! types that the trait surfaces, relocated here from `store` so the
//! `Store` → `StorageBackend` edge doesn't form a `store ↔ storage` module cycle:
//! the trait (defined here) must return `SyncOutcome` without importing `store`,
//! and the git storage impl must construct it without depending on `store`.
//!
//! `GitAuth`, the progress/cancellation types, and the keep-mine plan/outcome
//! types also live here (relocated from `git.rs`) so the RCS trait methods can
//! name them without a `storage → git` dependency.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::crypto::SecretExt;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::signing::{AuthenticityConfig, CommitSigInfo, VerifyMode};

/// The git storage backend (the sole `StorageBackend` implementation today).
pub mod git;

/// Re-entrant access to [`git::GitStorage`].
pub use git::GitStorage;

// ── Auth / progress / keep-mine types (relocated from `git.rs`) ─────────────
//
// These are the storage layer's transport + resolution types. They live here
// (not in `git.rs`) so the `StorageBackend` trait can name them in its method
// signatures without a `storage → git` dependency. `git.rs` re-exports them
// until its RCS bodies fold into `storage/git` and the `git` module disappears.

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
/// [`ErrorCode::Cancelled`](crate::ErrorCode::Cancelled).
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

/// A local-side `.age` entry to replay onto the remote tip during a "keep mine"
/// divergence resolution: its worktree-relative path plus its ciphertext blob at
/// the local HEAD. The caller decrypts + re-encrypts the blob — git has no
/// identity, so the crypto stays in `Store`.
///
/// Plaintext **never** enters the storage layer: this is the type-system half of
/// the no-rebase keep-mine contract (plaintext never enters the storage
/// layer). `Store` decrypts the
/// `blob`, re-encrypts with the current recipient set, and hands the new
/// ciphertext back to `keep_local_finalize`.
#[derive(Debug, Clone)]
pub struct KeepLocalReplay {
    /// Worktree-relative path, e.g. `servers/db.age`.
    pub rel_path: String,
    /// The entry's ciphertext at the local HEAD, to decrypt + re-encrypt.
    pub blob: Vec<u8>,
}

/// What a "keep mine" resolution must replay onto the reviewed remote tip.
#[derive(Debug, Clone)]
pub struct KeepLocalPlan {
    /// Full hash of the fetched remote tip the plan was computed against. Passed
    /// to `keep_local_advance` so the adopt reuses the SAME tip (no second fetch
    /// — a second fetch could race past the reviewed tip and bypass the
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

/// Outcome of `keep_local_plan`: proceed with a plan, or stop because Enforce
/// refused the remote-only range (HEAD left unchanged).
#[derive(Debug, Clone)]
pub enum KeepLocalOutcome {
    /// Enforce blocked the adopt — HEAD unchanged; surface this result.
    Blocked(SyncResult),
    /// Proceed: replay the plan onto the reviewed remote tip.
    Plan(KeepLocalPlan),
}

/// Configuration an RCS operation needs from `Store`, built fresh from
/// `RepoConfig` per op — the backend is stateless (config is user-mutable and
/// unknown at construction time).
///
/// `GitStorage` is stateless — the real durable state is git's on-disk index,
/// re-attached each op via `Repository::discover` — so auth/policy/commit-
/// identity are passed in here rather than held at backend construction.
/// `RepoConfig` is user-mutable within the Store's lifetime (Settings edits),
/// so holding these at construction would go stale on the next edit;
/// `Store::new` also runs before any repo is configured, so there'd be nothing
/// to hold. File-op methods (`get`/`set`/…) keep their per-call `repo_path` and
/// don't take a `StorageCtx` — they need no auth/policy.
#[derive(Debug, Clone, Copy)]
pub struct StorageCtx<'a> {
    /// The repo working-tree root (`RepoConfig::local_path`).
    pub repo_path: &'a Path,
    /// Git remote credentials (`RepoConfig::to_git_auth`).
    pub auth: &'a GitAuth,
    /// Repository authenticity policy (`RepoConfig::authenticity`).
    pub policy: &'a AuthenticityConfig,
    /// Commit author name; `None` ⇒ the backend's app default.
    pub commit_name: Option<&'a str>,
    /// Commit author email; `None` ⇒ the backend's app default.
    pub commit_email: Option<&'a str>,
}

/// How [`StorageBackend::commit`] stages `paths` — `Add` (stage content) or
/// `Remove` (stage deletion). Carried as data because trait methods can't be
/// passed as fn pointers, so `Store::commit_local` picks the kind instead of
/// selecting `commit` vs `commit_removal` by function pointer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    /// Stage `paths` (add/modify) — `git add`.
    Add,
    /// Stage the removal of `paths` (the worktree files are already gone) —
    /// `git rm`.
    Remove,
}

/// Swappable storage backend (gopass `internal/backend/storage.go` analogue).
///
/// Owns the repo working tree: file ops for secrets, the recipients file, and
/// template lookups. RCS ops (clone/pull/push/keep-mine) land on this trait as
/// the git module is consolidated; today only the file-op surface exists. The
/// trait is `Send + Sync` so
/// `Box<dyn StorageBackend>` stays `Send + Sync` for `Store`'s `AppState`.
///
/// File ops own their within-repo path-traversal guard (`get`/`set`/`delete`
/// validate the resolved path), so `Store` passes an entry *name* and the
/// backend maps it to `<repo>/<name>.age`. Lifecycle dir management (creating /
/// wiping the whole repo dir in `configure`/`reset`) stays in `Store` — it's
/// setup/teardown, not content access.
#[async_trait]
pub trait StorageBackend: Send + Sync {
    /// List every secret entry under `repo_path` whose extension is `ext`
    /// (alpha-sorted, `.git` skipped). `ext` is the crypto backend's typed
    /// [`SecretExt`](crate::crypto::SecretExt) — storage is extension-agnostic
    /// and matches whatever the caller passes.
    ///
    /// # Errors
    ///
    /// Returns [`crate::error::ErrorCode::NoRepo`] if `repo_path` doesn't exist.
    async fn list(&self, repo_path: &Path, ext: SecretExt) -> Result<Vec<Entry>, Error>;

    /// Read the bytes of a secret's `passfile` (a repo-relative path like
    /// `cloud/aws.age`). The caller builds `passfile` from the crypto backend's
    /// extension; storage never names an extension itself, so it can't typo one.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if the entry is missing or the
    /// resolved path escapes the repo; [`crate::error::ErrorCode::IoError`] on a
    /// read failure.
    async fn get(&self, repo_path: &Path, passfile: &str) -> Result<Vec<u8>, Error>;

    /// Atomically write `ciphertext` to `<repo>/<passfile>`, creating parent
    /// directories. Temp-file + rename so a failure can't leave a half-written
    /// secret behind.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if the resolved path escapes
    /// the repo; otherwise an I/O error.
    async fn set(&self, repo_path: &Path, passfile: &str, ciphertext: &[u8]) -> Result<(), Error>;

    /// Remove entry `passfile`.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if missing or the path escapes
    /// the repo; otherwise an I/O error.
    async fn delete(&self, repo_path: &Path, passfile: &str) -> Result<(), Error>;

    /// Read an arbitrary repo-relative file (`.age-recipients`, `.gpg-id`,
    /// `.public-keys/<fpr>`, …) — the recipients index and any crypto-owned
    /// auxiliary files. **No `exists` step** — returns
    /// [`crate::error::ErrorCode::EntryNotFound`] for a missing file, avoiding
    /// the exists-then-read TOCTOU that could silently shrink the recipient set.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if missing or the path escapes
    /// the repo; [`crate::error::ErrorCode::IoError`] on a read failure.
    async fn read_file(&self, repo_path: &Path, rel_path: &str) -> Result<Vec<u8>, Error>;

    /// Atomically write `bytes` to an arbitrary repo-relative file, creating
    /// parent directories. The recipients-index write path (setup / add
    /// recipient) and `.public-keys/<fpr>` writes go through here.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if the path escapes the repo;
    /// otherwise an I/O error.
    async fn write_file_atomic(
        &self,
        repo_path: &Path,
        rel_path: &str,
        bytes: &[u8],
    ) -> Result<(), Error>;

    /// List repo-relative file paths under `rel_prefix` (non-recursive), for
    /// enumerating a crypto-owned directory such as GPG's `.public-keys/`.
    /// Returns relative paths including the prefix.
    ///
    /// # Errors
    ///
    /// [`crate::error::ErrorCode::EntryNotFound`] if the prefix is missing or
    /// escapes the repo; otherwise an I/O error.
    async fn list_dir(&self, repo_path: &Path, rel_prefix: &str) -> Result<Vec<String>, Error>;

    /// Look up the `.pass-template` that applies to `name`, walking up the tree
    /// (gopass `LookupTemplate`). `Ok(None)` when no template applies.
    ///
    /// # Errors
    ///
    /// Returns an error only if `repo_path` can't be resolved; a missing
    /// template is `Ok(None)`.
    async fn lookup_template(&self, repo_path: &Path, name: &str) -> Result<Option<String>, Error>;

    /// Clone `url` to `dest` over HTTPS/SSH with `auth`. Setup-time op — no
    /// [`StorageCtx`] (there is no `RepoConfig` to build one from yet). An
    /// existing `dest` is removed first.
    ///
    /// # Errors
    ///
    /// `CloneFailed`/`NetworkError` on auth/network/filesystem failure;
    /// `Cancelled` if the cancel token is set mid-clone.
    async fn clone_repo(
        &self,
        auth: &GitAuth,
        url: &str,
        dest: &Path,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error>;

    /// Initialize a new git repo at `repo_path` (no commits, no remote).
    ///
    /// # Errors
    ///
    /// Returns an error if `Repository::init` fails.
    async fn init_repo(&self, repo_path: &Path) -> Result<(), Error>;

    /// Add a remote `name` → `url` locally (no network contact).
    ///
    /// # Errors
    ///
    /// Returns an error if the repo can't be opened or the remote already exists.
    async fn remote_add(&self, repo_path: &Path, name: &str, url: &str) -> Result<(), Error>;

    /// Stage `paths` and commit on HEAD. `kind` selects `git add` vs `git rm`;
    /// the commit identity comes from `ctx`. Returns the new HEAD short hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo can't be opened or staging/committing fails.
    async fn commit(
        &self,
        ctx: &StorageCtx<'_>,
        kind: CommitKind,
        paths: &[String],
        message: &str,
    ) -> Result<String, Error>;

    /// Create the initial (no-parent) commit — gopass's "Initialized Store".
    ///
    /// # Errors
    ///
    /// Returns an error if the repo can't be opened or the commit fails.
    async fn commit_initial(
        &self,
        repo_path: &Path,
        paths: &[String],
        message: &str,
    ) -> Result<String, Error>;

    /// Push the current branch to `origin`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PushRejected`] when the remote diverged; otherwise a
    /// network/auth error.
    async fn push(&self, ctx: &StorageCtx<'_>) -> Result<(), Error>;

    /// Pull (fetch + fast-forward) from `origin` under `ctx`'s authenticity
    /// policy. Returns [`SyncOutcome::FastForwarded`] for a normal pull or
    /// [`SyncOutcome::Diverged`] when the branches have split.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo/remote can't be opened or the fetch fails.
    async fn pull(
        &self,
        ctx: &StorageCtx<'_>,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error>;

    /// Adopt the reviewed remote tip exactly (`expected_remote_oid`): re-fetch,
    /// refuse if the remote moved past it, verify the remote-only range under
    /// `ctx`'s policy, then hard-advance the branch. Divergence resolution.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PullFfFailed`] if the remote advanced since review;
    /// otherwise a git/signing error.
    async fn adopt_remote(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
    ) -> Result<SyncResult, Error>;

    /// Fetch the remote tip and compute the local-vs-remote divergence preview
    /// without moving HEAD. Called after a push rejection.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo/remote can't be opened or the fetch fails.
    async fn preview_divergence(&self, ctx: &StorageCtx<'_>) -> Result<SyncDivergence, Error>;

    /// Compute the "keep mine" replay plan: which local `.age` entries to
    /// re-encrypt onto the reviewed remote tip. Returns CIPHERTEXT blobs —
    /// plaintext never enters the storage layer — or [`KeepLocalOutcome::Blocked`]
    /// if Enforce refused the remote-only range.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PushRejected`] for an irreconcilable same-secret conflict;
    /// otherwise a git/signing error.
    async fn keep_local_plan(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
    ) -> Result<KeepLocalOutcome, Error>;

    /// Advance HEAD + worktree to the already-fetched `fetched_oid` (no re-fetch
    /// — the plan captured the reviewed tip).
    ///
    /// # Errors
    ///
    /// Returns an error if the repo can't be opened or the ref move fails.
    async fn keep_local_advance(&self, repo_path: &Path, fetched_oid: &str) -> Result<(), Error>;

    /// Apply a keep-mine plan onto the (already-advanced) tip: write
    /// `ciphertexts`, apply `deletes`, commit, and push. Receives CIPHERTEXT —
    /// `Store` did the decrypt + re-encrypt to the current recipients, so
    /// plaintext never enters the storage layer. Returns the new HEAD short hash.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::PushRejected`] if the push races a newer remote; otherwise a
    /// git/IO error.
    async fn keep_local_finalize(
        &self,
        ctx: &StorageCtx<'_>,
        ciphertexts: &[(String, Vec<u8>)],
        deletes: &[String],
    ) -> Result<String, Error>;

    /// Full hash of the current HEAD commit.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NoRepo`] if no repo at `repo_path`;
    /// [`ErrorCode::PullFfFailed`] if HEAD is unborn.
    async fn current_head(&self, repo_path: &Path) -> Result<String, Error>;
}

/// Read-only view of repo working-tree files, bound to a specific `repo_path`.
///
/// Built by `Store` per-op and passed to the crypto backend (which has no other
/// way to name repo files — it knows nothing of `repo_path`). The crypto
/// backend's recipient resolution (age reads the recipients index; GPG reads
/// `.gpg-id` + enumerates `.public-keys/`) goes through this. Writes do NOT —
/// those stay `Store` → [`StorageBackend::write_file_atomic`] so storage keeps
/// its exclusive-write invariant.
#[async_trait]
pub trait RepoFileView: Send + Sync {
    /// The absolute working-tree root this view is bound to. Crypto backends use
    /// it for the recipients-index liveness guard (a direct `lstat`), since
    /// [`read`](Self::read) / [`list_dir`](Self::list_dir) are repo-relative and
    /// can't name an absolute guard path.
    fn repo_path(&self) -> &Path;
    /// Read a repo-relative file. Returns
    /// [`ErrorCode::EntryNotFound`](crate::error::ErrorCode) for a missing file
    /// — no `exists`-then-`read` race.
    async fn read(&self, rel_path: &str) -> Result<Vec<u8>, Error>;
    /// List repo-relative paths under `rel_prefix` (non-recursive).
    async fn list_dir(&self, rel_prefix: &str) -> Result<Vec<String>, Error>;
}

/// A [`RepoFileView`] bound to `storage` at `repo_path`.
///
/// `Store` constructs one per-op (cheap — two borrows) and hands
/// `&RepoFiles as &dyn RepoFileView` to the crypto backend. Borrowed over `'a`,
/// so the caller must hold `storage` + `repo_path` across the await (Store
/// does — it owns both for the op's lifetime).
#[allow(missing_debug_implementations)]
pub struct RepoFiles<'a> {
    storage: &'a dyn StorageBackend,
    repo_path: &'a Path,
}

impl<'a> RepoFiles<'a> {
    /// Bind a read-only file view to `storage` at `repo_path`.
    pub fn new(storage: &'a dyn StorageBackend, repo_path: &'a Path) -> Self {
        RepoFiles { storage, repo_path }
    }
}

#[async_trait]
impl RepoFileView for RepoFiles<'_> {
    fn repo_path(&self) -> &Path {
        self.repo_path
    }
    async fn read(&self, rel_path: &str) -> Result<Vec<u8>, Error> {
        self.storage.read_file(self.repo_path, rel_path).await
    }
    async fn list_dir(&self, rel_prefix: &str) -> Result<Vec<String>, Error> {
        self.storage.list_dir(self.repo_path, rel_prefix).await
    }
}

/// Outcome of [`validate_recipients_index_liveness`] — whether the recipients
/// index is safe to read, or genuinely absent (an uninitialized store).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientsIndexPresence {
    /// The index doesn't exist but the checkout does — an uninitialized store.
    /// The caller treats this as an empty recipient set (first-time setup).
    Absent,
    /// The index is a regular file, safe to read through
    /// [`StorageBackend::read_file`].
    Present,
}

/// Liveness/safety guard for the recipients index, run before any read.
///
/// A malicious clone can plant a symlink (dangling, or escaping the checkout) at
/// the recipients path. Reading such a plant as "uninitialized → empty" would
/// make the encrypt path `ensureOurKeyID` and silently re-encrypt to only the
/// local key, shrinking the recipient set and pushing the result. So the index
/// is `lstat`-checked (without following symlinks) before it is read.
///
/// Returns [`RecipientsIndexPresence::Absent`] for a genuinely-missing index
/// (the repo dir exists, just no index yet — an uninitialized store), and
/// [`RecipientsIndexPresence::Present`] when the index is a regular file safe to
/// read. Every other case is a hard error: a configured-but-missing checkout
/// (`repo_path` itself gone), a non-regular index (symlink, directory — possible
/// tampering), or an I/O error.
///
/// # Errors
///
/// [`ErrorCode::StoreError`] for a missing configured checkout (`repo_path`
/// itself gone) or a non-regular index (symlink, directory — possible
/// tampering); [`ErrorCode::IoError`] on a metadata failure. A genuinely-missing
/// index on an *existing* checkout is NOT an error — it returns
/// [`RecipientsIndexPresence::Absent`].
///
/// This is a direct filesystem `lstat`, not a [`StorageBackend`] call — the
/// guard must NOT follow symlinks, and `read_file` is repo-relative. Storage's
/// own within-repo path guard still applies as defense-in-depth on the read.
pub async fn validate_recipients_index_liveness(
    repo_path: &Path,
    recipients_rel: &str,
) -> Result<RecipientsIndexPresence, Error> {
    let recipients_path = repo_path.join(recipients_rel);
    match fs::symlink_metadata(&recipients_path).await {
        // The index is absent. Distinguish a genuine uninitialized store (repo
        // dir exists, just no recipients index yet → absent) from a configured-
        // but-missing checkout (repo_path itself gone → hard error): the latter
        // must NOT read as empty, or `save_identity` would accept any identity
        // against a store whose checkout it can't even see.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if fs::symlink_metadata(repo_path).await.is_err() {
                return Err(Error::new(
                    ErrorCode::StoreError,
                    "configured repository checkout is missing",
                ));
            }
            Ok(RecipientsIndexPresence::Absent)
        }
        Err(e) => Err(Error::new(
            ErrorCode::IoError,
            format!("Failed to read recipients index: {e}"),
        )),
        Ok(meta) => {
            if !meta.is_file() {
                // A symlink (dangling or escaping), directory, or other
                // non-regular file is not a valid recipients index — reject
                // loudly. Treating it as empty would `ensureOurKeyID` to only
                // our key on the next encrypt.
                return Err(Error::new(
                    ErrorCode::StoreError,
                    "recipients index is not a regular file — possible tampering",
                ));
            }
            Ok(RecipientsIndexPresence::Present)
        }
    }
}

/// Result of a sync (pull) operation — aligned with gopass `Store.Sync`.
#[derive(Debug, Clone, Serialize)]
pub struct SyncResult {
    /// Whether any new commits were pulled (HEAD advanced).
    pub changed: bool,
    /// Short hash (7 chars) of the (current) HEAD commit.
    pub head: String,
    /// Repository-authenticity outcome of this pull.
    pub authenticity: AuthenticityResult,
}

/// Authenticity outcome of a sync (pull) — surfaced to the frontend so it can
/// pop the Audit mismatch modal / Enforce-block modal without re-verifying.
#[derive(Debug, Clone, Serialize)]
pub struct AuthenticityResult {
    /// The mode in force during this pull.
    pub mode: VerifyMode,
    /// Commits in the pulled range `(old HEAD, new HEAD]`, newest first.
    /// Empty for `VerifyMode::Off` (no verification is done).
    pub new_commits: Vec<CommitSigInfo>,
    /// Subset of `new_commits` that are non-Verified and not ignored — the
    /// actionable issues.
    pub open_issues: Vec<CommitSigInfo>,
    /// `true` only when Enforce refused checkout (HEAD did not advance).
    pub blocked: bool,
}

/// Outcome of a sync (pull): a normal pull, or a divergence the caller must
/// resolve. Replaces the bare [`SyncResult`] return of [`crate::store::Store::sync`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SyncOutcome {
    /// Normal fast-forward pull (changed or not).
    FastForwarded(SyncResult),
    /// Local and remote have diverged; the working branch is unchanged. The
    /// caller must resolve via [`crate::store::Store::resolve_sync_divergence`].
    Diverged(SyncDivergence),
}

/// Local-vs-remote divergence, surfaced so the user can decide whether to
/// discard local-only changes and adopt the remote. Carries no secrets — entry
/// *names* / file paths only.
#[derive(Debug, Clone, Serialize)]
pub struct SyncDivergence {
    /// Commits reachable from local HEAD but not the merge base.
    pub local_ahead: usize,
    /// Commits reachable from the remote tip but not the merge base.
    pub remote_ahead: usize,
    /// Full hash of the remote tip this preview was computed against. Passed
    /// back to [`crate::store::Store::resolve_sync_divergence`] so we adopt
    /// exactly what was reviewed (no stale-confirmation TOCTOU).
    pub remote_tip: String,
    /// Secret entries (`.age` stripped) present locally, absent remotely —
    /// **deleted** by "adopt remote".
    pub local_only_entries: Vec<String>,
    /// Secret entries present on both sides whose `.age` bytes differ —
    /// **overwritten** by "adopt remote". (May over-report identical-plaintext
    /// re-encryptions until the decrypt-and-compare enhancement lands.)
    pub modified_entries: Vec<String>,
    /// Non-secret tracked files changed on the local side (templates,
    /// recipients, …) — also discarded/overwritten by a hard reset.
    pub other_changed_files: Vec<String>,
}

/// Result of a successful write (`Store::set`) — the new HEAD commit hash.
#[derive(Debug, Clone, Serialize)]
pub struct WriteResult {
    /// Short hash (7 chars) of the commit that recorded the write.
    pub commit: String,
}

/// The default commit author identity, surfaced to the UI so it can display
/// what's in effect without hardcoding the value.
#[derive(Debug, Clone, Serialize)]
pub struct CommitIdentity {
    /// Commit author name.
    pub name: String,
    /// Commit author email.
    pub email: String,
}

/// Outcome of a write attempt.
///
/// Writes run through [`crate::store::Store::autosync_write`] (pull → write →
/// push). A normal save returns [`WriteOutcome::Written`]. Two non-terminal
/// outcomes surface a modal instead of a generic error:
/// - [`WriteOutcome::NeedsDivergenceResolve`] — the push was rejected because
///   the remote moved during the write (a race); the carried [`SyncDivergence`]
///   lets the UI show the resolve modal without a second round-trip.
/// - [`WriteOutcome::AuthenticityBlocked`] — the pre-write pull was refused
///   under Enforce signature verification (HEAD did not advance); the carried
///   [`AuthenticityResult`] reuses the pull path's block-issue UI.
///
/// **Limitation:**
/// this only catches the push-rejection *race*. A write built on a stale read
/// can still fast-forward over a newer remote change and push cleanly — no
/// modal — silently overwriting it (recoverable in git history). That limitation
/// is surfaced to the user in Settings, not per-write.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WriteOutcome {
    /// The secret was written and committed — and pushed when autosync is on.
    /// Carries the new HEAD.
    Written(WriteResult),
    /// The push was rejected — the remote moved during the write (a race). The
    /// local commit was made; the caller resolves via
    /// [`crate::store::Store::resolve_sync_divergence`] using the carried
    /// preview. Carries no plaintext.
    NeedsDivergenceResolve(SyncDivergence),
    /// The pre-write pull was refused under Enforce authenticity mode (HEAD did
    /// not advance). No local write was made. The caller reuses the pull path's
    /// block-issue UI with the carried result.
    AuthenticityBlocked(AuthenticityResult),
}

/// How to resolve a [`SyncOutcome::Diverged`] (the user's choice). "Cancel" is
/// client-side — the frontend simply doesn't call
/// [`crate::store::Store::resolve_sync_divergence`] — so it is absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceChoice {
    /// Discard local-only changes and adopt the reviewed remote tip exactly.
    AdoptRemote,
    /// Keep local changes: re-encrypt the local-only `.age` entries onto the
    /// reviewed remote tip (with the current recipient set) and push. Refused
    /// ([`crate::error::ErrorCode::PushRejected`]) for an irreconcilable
    /// same-secret conflict or an undecryptable local entry — the user must
    /// adopt or cancel.
    KeepMine,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A regular recipients index is `Present` — safe to read through storage.
    #[tokio::test]
    async fn liveness_regular_file_is_present() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join(".age-recipients"), b"age1abc\n")
            .await
            .unwrap();
        let presence = validate_recipients_index_liveness(dir.path(), ".age-recipients")
            .await
            .unwrap();
        assert_eq!(presence, RecipientsIndexPresence::Present);
    }

    /// A genuinely-missing index on an existing checkout is `Absent` — an
    /// uninitialized store, not an error (so first-time setup proceeds).
    #[tokio::test]
    async fn liveness_missing_index_is_absent() {
        let dir = tempfile::tempdir().unwrap();
        let presence = validate_recipients_index_liveness(dir.path(), ".age-recipients")
            .await
            .unwrap();
        assert_eq!(presence, RecipientsIndexPresence::Absent);
    }

    /// A configured-but-missing checkout (repo dir gone) is a hard `StoreError`,
    /// NOT `Absent` — `save_identity` must not accept any identity against a
    /// store whose checkout it can't see.
    #[tokio::test]
    async fn liveness_missing_checkout_errors() {
        let missing =
            std::path::PathBuf::from("/tmp/gpm_no_such_checkout_liveness_test_recipients");
        assert!(!missing.exists());
        let err = validate_recipients_index_liveness(&missing, ".age-recipients")
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "a missing configured checkout is an anomaly, not an empty store"
        );
    }

    /// A dangling symlink at the index path is tampering — `lstat` sees the
    /// symlink itself (not its missing target) → not a regular file → hard error.
    #[cfg(unix)]
    #[tokio::test]
    async fn liveness_dangling_symlink_errors() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        symlink(
            "/nonexistent/gpm-dangling-liveness",
            dir.path().join(".age-recipients"),
        )
        .unwrap();
        let err = validate_recipients_index_liveness(dir.path(), ".age-recipients")
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "dangling symlink must be tampering, not an empty set"
        );
    }

    /// An escaping symlink (points outside the repo) is tampering — `lstat` does
    /// not follow it, so the regular-file check rejects before any read could
    /// resolve + read the victim.
    #[cfg(unix)]
    #[tokio::test]
    async fn liveness_escaping_symlink_errors() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        let victim = external.path().join("victim");
        std::fs::write(&victim, b"age1stolen\n").unwrap();
        symlink(&victim, dir.path().join(".age-recipients")).unwrap();
        let err = validate_recipients_index_liveness(dir.path(), ".age-recipients")
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "escaping symlink must be tampering, not adopted"
        );
    }

    /// A directory at the index path is not a regular file — rejected as
    /// tampering, the same as a symlink (the docstring enumerates both).
    #[tokio::test]
    async fn liveness_directory_at_index_errors() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join(".age-recipients"))
            .await
            .unwrap();
        let err = validate_recipients_index_liveness(dir.path(), ".age-recipients")
            .await
            .unwrap_err();
        assert_eq!(
            err.code, "STORE_ERROR",
            "a directory at the index path is tampering, not an empty set"
        );
    }
}
