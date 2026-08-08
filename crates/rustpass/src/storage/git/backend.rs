// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The async [`StorageBackend`] shell for the git backend.
//!
//! [`GitStorage`] owns the working-tree root it operates against (set at
//! construction by the registry). Each method adapts a blocking free function
//! in `commit`/`pull`/`divergence` to async via `spawn_blocking`; file ops
//! delegate to `worktree`. Auth / policy stay per-call (user-mutable in
//! `repo.json`), carried by [`StorageCtx`].

use std::io;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::crypto::SecretExt;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::storage::{
    CancelToken, CommitKind, FilePresence, GitAuth, KeepLocalOutcome, ProgressSender,
    StorageBackend, StorageCtx, SyncDivergence, SyncOutcome, SyncResult,
};
use crate::template;

use super::worktree::{
    assert_within_repo, ensure_within_repo, list_entries, resolve_entry_path, write_atomic,
};
use super::{commit, divergence, history, pull};

/// The git storage backend (owns its working-tree root).
///
/// Load-bearing invariant: `root` is the working-tree root; methods
/// re-`discover`/operate per call and do NOT cache a `Repository` handle. This
/// matters for `Store::reset` tearing down the dir while an in-flight op holds
/// an `Arc` to this backend — no held handle means no stale handle on a
/// removed dir.
#[derive(Debug, Clone)]
pub struct GitStorage {
    /// The working-tree root the backend operates against, pinned at
    /// construction by the registry from the root token in `repo.json`.
    root: PathBuf,
}

impl GitStorage {
    /// Construct a git storage backend rooted at `root`.
    ///
    /// The registry is the sanctioned constructor — it threads the root token
    /// from `repo.json` (the post-unlock resolve) or a setup path (which knows
    /// the `repo_dir` it just created).
    pub fn new(root: impl Into<PathBuf>) -> Self {
        GitStorage { root: root.into() }
    }
}

#[async_trait]
impl StorageBackend for GitStorage {
    async fn list(&self, ext: SecretExt) -> Result<Vec<Entry>, Error> {
        let root = self.root.clone();
        // WalkDir is synchronous (blocking I/O) — offload it. SecretExt is Copy.
        spawn_blocking(move || list_entries(&root, ext)).await?
    }

    async fn get(&self, passfile: &str) -> Result<Vec<u8>, Error> {
        ensure_within_repo(passfile)?;
        let file_path = resolve_entry_path(&self.root, passfile)?;
        fs::read(&file_path).await.map_err(|e| {
            Error::new(
                ErrorCode::IoError,
                format!("Failed to read entry file: {e}"),
            )
        })
    }

    async fn set(&self, passfile: &str, ciphertext: &[u8]) -> Result<(), Error> {
        // Reject `..` / absolute names BEFORE any fs op — the trait is `pub`, so
        // a caller that skips `Store::validate_secret_name` still can't mkdir or
        // write outside the repo. (`assert_within_repo` below is the 2nd layer.)
        ensure_within_repo(passfile)?;
        let file_path = self.root.join(passfile);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(&self.root, file_path.parent().unwrap_or(Path::new("")))?;
        write_atomic(&file_path, ciphertext).await
    }

    async fn delete(&self, passfile: &str) -> Result<(), Error> {
        ensure_within_repo(passfile)?;
        // Existence + within-repo guard before any mutation.
        resolve_entry_path(&self.root, passfile)?;
        let file_path = self.root.join(passfile);
        assert_within_repo(&self.root, file_path.parent().unwrap_or(Path::new("")))?;
        match fs::remove_file(&file_path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Err(Error::new(
                ErrorCode::EntryNotFound,
                format!("Entry not found: {passfile}"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    async fn read_file(&self, rel_path: &str) -> Result<Vec<u8>, Error> {
        ensure_within_repo(rel_path)?;
        // resolve_entry_path checks existence + within-repo (canonicalize) in one
        // step — no caller-level exists-then-read, so no TOCTOU that could shrink
        // the recipient set.
        let file_path = resolve_entry_path(&self.root, rel_path)?;
        fs::read(&file_path).await.map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::new(
                ErrorCode::EntryNotFound,
                format!("File not found: {rel_path}"),
            ),
            _ => Error::new(ErrorCode::IoError, format!("Failed to read file: {e}")),
        })
    }

    async fn file_liveness(&self, rel_path: &str) -> Result<FilePresence, Error> {
        ensure_within_repo(rel_path)?;
        // The liveness guard must NOT follow symlinks, so it uses `symlink_metadata`
        // (the `lstat` analogue) against the file path. Sourced from `self.root`
        // (the backend's owned root, pinned at resolve) so the guard and the
        // actual recipients read share ONE root — no two-sources-of-truth between
        // a per-op `local_path` and the resolve-time root.
        let file_path = self.root.join(rel_path);
        match fs::symlink_metadata(&file_path).await {
            // The file is absent. Distinguish a genuine uninitialized store
            // (root exists, just no recipients index yet → Absent) from a
            // configured-but-missing checkout (root itself gone → hard error):
            // the latter must NOT read as empty, or `save_identity` would
            // accept any identity against a store whose checkout it can't see.
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if fs::symlink_metadata(&self.root).await.is_err() {
                    return Err(Error::new(
                        ErrorCode::StoreError,
                        "configured repository checkout is missing",
                    ));
                }
                Ok(FilePresence::Absent)
            }
            Err(e) => Err(Error::new(
                ErrorCode::IoError,
                format!("Failed to read repo-relative file: {e}"),
            )),
            Ok(meta) => {
                if !meta.is_file() {
                    // A symlink (dangling or escaping), directory, or other
                    // non-regular file is not safe to read — reject loudly.
                    // Treating it as empty would `ensureOurKeyID` to only our
                    // key on the next encrypt (silently shrinking the recipient
                    // set if planted at the recipients index).
                    return Err(Error::new(
                        ErrorCode::StoreError,
                        "repo-relative file is not a regular file — possible tampering",
                    ));
                }
                Ok(FilePresence::Present)
            }
        }
    }

    async fn write_file_atomic(&self, rel_path: &str, bytes: &[u8]) -> Result<(), Error> {
        ensure_within_repo(rel_path)?;
        let file_path = self.root.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(&self.root, file_path.parent().unwrap_or(Path::new("")))?;
        write_atomic(&file_path, bytes).await
    }

    async fn list_dir(&self, rel_prefix: &str) -> Result<Vec<String>, Error> {
        ensure_within_repo(rel_prefix)?;
        let dir = resolve_entry_path(&self.root, rel_prefix)?;
        let mut out: Vec<String> = Vec::new();
        let mut entries = fs::read_dir(&dir).await.map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => {
                Error::new(ErrorCode::EntryNotFound, format!("Not found: {rel_prefix}"))
            }
            _ => Error::new(ErrorCode::IoError, format!("Failed to list dir: {e}")),
        })?;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_file() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                // Return repo-relative paths (prefix + "/" + filename) — the
                // form callers re-use in `read_file`.
                out.push(format!("{rel_prefix}/{name}"));
            }
        }
        Ok(out)
    }

    async fn lookup_template(&self, name: &str) -> Result<Option<String>, Error> {
        let root = self.root.clone();
        let name_owned = name.to_string();
        // Filesystem walk; cheap enough to run on a blocking thread.
        Ok(spawn_blocking(move || template::lookup_template_in_repo(&root, &name_owned)).await?)
    }

    // ── RCS ops ─────────────────────────────────────────────────────────────
    //
    // Each method adapts a blocking free function in `commit`/`pull`/`divergence`
    // to the async trait: move owned args into a `spawn_blocking` closure and
    // pass the `&StorageCtx` fields by value (cloning the cheap ones —
    // `GitAuth`/`AuthenticityConfig` — since the closure must be `'static`).

    async fn clone_repo(
        &self,
        auth: &GitAuth,
        url: &str,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let auth = auth.clone();
        let url = url.to_string();
        let dest = self.root.clone();
        spawn_blocking(move || {
            commit::clone_repo(&url, &dest, &auth, cancel.as_ref(), progress.as_ref())
        })
        .await?
    }

    async fn init_repo(&self) -> Result<(), Error> {
        let root = self.root.clone();
        spawn_blocking(move || commit::init_repo(&root)).await?
    }

    async fn remote_add(&self, name: &str, url: &str) -> Result<(), Error> {
        let root = self.root.clone();
        let name = name.to_string();
        let url = url.to_string();
        spawn_blocking(move || commit::remote_add(&root, &name, &url)).await?
    }

    async fn set_config(&self, repo_path: &Path, key: &str, value: &str) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        let key = key.to_string();
        let value = value.to_string();
        spawn_blocking(move || commit::set_config(&repo_path, &key, &value)).await?
    }

    async fn commit(
        &self,
        ctx: &StorageCtx<'_>,
        kind: CommitKind,
        paths: &[String],
        message: &str,
    ) -> Result<String, Error> {
        let root = self.root.clone();
        let name = ctx.commit_name.map(str::to_string);
        let email = ctx.commit_email.map(str::to_string);
        let paths = paths.to_vec();
        let message = message.to_string();
        spawn_blocking(move || match kind {
            CommitKind::Add => {
                commit::commit(&root, &paths, &message, name.as_deref(), email.as_deref())
            }
            CommitKind::Remove => {
                commit::commit_removal(&root, &paths, &message, name.as_deref(), email.as_deref())
            }
        })
        .await?
    }

    async fn commit_initial(&self, paths: &[String], message: &str) -> Result<String, Error> {
        let root = self.root.clone();
        let paths = paths.to_vec();
        let message = message.to_string();
        spawn_blocking(move || commit::commit_initial(&root, &paths, &message)).await?
    }

    async fn push(
        &self,
        ctx: &StorageCtx<'_>,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        spawn_blocking(move || commit::push(&root, &auth, cancel.as_ref(), progress.as_ref()))
            .await?
    }

    async fn pull(
        &self,
        ctx: &StorageCtx<'_>,
        ext: SecretExt,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        spawn_blocking(move || {
            pull::pull_repo(
                &root,
                &auth,
                &policy,
                cancel.as_ref(),
                progress.as_ref(),
                ext,
            )
        })
        .await?
    }

    async fn adopt_remote(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
        cancel: Option<CancelToken>,
    ) -> Result<SyncResult, Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        let expected = expected_remote_oid.to_string();
        spawn_blocking(move || {
            pull::adopt_remote(&root, &auth, &policy, &expected, cancel.as_ref())
        })
        .await?
    }

    async fn preview_divergence(
        &self,
        ctx: &StorageCtx<'_>,
        ext: SecretExt,
        cancel: Option<CancelToken>,
    ) -> Result<SyncDivergence, Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        spawn_blocking(move || divergence::preview_divergence(&root, &auth, cancel.as_ref(), ext))
            .await?
    }

    async fn keep_local_plan(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
        ext: SecretExt,
        cancel: Option<CancelToken>,
    ) -> Result<KeepLocalOutcome, Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        let expected = expected_remote_oid.to_string();
        spawn_blocking(move || {
            divergence::keep_local_plan(&root, &auth, &policy, &expected, cancel.as_ref(), ext)
        })
        .await?
    }

    async fn keep_local_advance(&self, fetched_oid: &str) -> Result<(), Error> {
        let root = self.root.clone();
        let fetched = fetched_oid.to_string();
        spawn_blocking(move || divergence::keep_local_advance(&root, &fetched)).await?
    }

    async fn keep_local_finalize(
        &self,
        ctx: &StorageCtx<'_>,
        ciphertexts: &[(String, Vec<u8>)],
        deletes: &[String],
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<String, Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        let name = ctx.commit_name.map(str::to_string);
        let email = ctx.commit_email.map(str::to_string);
        let entries = ciphertexts.to_vec();
        let deletes = deletes.to_vec();
        spawn_blocking(move || {
            divergence::keep_local_finalize(
                &root,
                &auth,
                &entries,
                &deletes,
                name.as_deref(),
                email.as_deref(),
                cancel.as_ref(),
                progress.as_ref(),
            )
        })
        .await?
    }

    async fn current_head(&self) -> Result<String, Error> {
        let root = self.root.clone();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&root)
                .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
            let head = repo
                .head()
                .map_err(|e| {
                    Error::new(ErrorCode::StoreError, format!("Failed to read HEAD: {e}"))
                })?
                .target()
                .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit"))?;
            Ok(head.to_string())
        })
        .await?
    }

    async fn verify_auth(
        &self,
        ctx: &StorageCtx<'_>,
        cancel: Option<CancelToken>,
    ) -> Result<(), Error> {
        let root = self.root.clone();
        let auth = ctx.auth.clone();
        spawn_blocking(move || pull::verify_remote_auth(&root, &auth, cancel.as_ref())).await?
    }

    async fn blob_at_revision(
        &self,
        passfile: &str,
        commit_oid: &str,
    ) -> Result<Option<Vec<u8>>, Error> {
        ensure_within_repo(passfile)?;
        let root = self.root.clone();
        let passfile = passfile.to_string();
        let commit_oid = commit_oid.to_string();
        spawn_blocking(move || history::blob_at_commit_at(&root, &commit_oid, &passfile)).await?
    }

    async fn entry_oid(&self, rel_path: &str) -> Result<Option<String>, Error> {
        ensure_within_repo(rel_path)?;
        let root = self.root.clone();
        let rel = rel_path.to_string();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&root)
                .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
            let tree = head_tree(&repo)?;
            // None when the entry is absent at HEAD (a teammate deleted it) OR is
            // not a blob — a subtree/gitlink planted at <name>.age must NOT be
            // returned as a blob base-version, or the orchestrator's oid compare
            // would span object types and subvert the conflict check (R026).
            Ok(tree
                .get_path(Path::new(&rel))
                .ok()
                .filter(|e| e.kind() == Some(git2::ObjectType::Blob))
                .map(|e| e.id().to_string()))
        })
        .await?
    }

    async fn get_with_oid(&self, rel_path: &str) -> Result<(Vec<u8>, String), Error> {
        ensure_within_repo(rel_path)?;
        let root = self.root.clone();
        let rel = rel_path.to_string();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&root)
                .map_err(|_| Error::new(ErrorCode::NoRepo, "No git repository found at path"))?;
            let tree = head_tree(&repo)?;
            let entry = tree.get_path(Path::new(&rel)).map_err(|_| {
                // Fail-closed: never pair bytes with a missing/stale oid. The read
                // path surfaces this as a clear error rather than downgrading to
                // an unprotected base version (RFC R026, codex #1+#2).
                Error::new(
                    ErrorCode::EntryNotFound,
                    format!("Entry not found at HEAD: {rel}"),
                )
            })?;
            // Only a Blob is a secret entry. A subtree/gitlink at <name>.age is
            // not — surface EntryNotFound (the documented fail-closed contract)
            // rather than letting find_blob fail as a generic StoreError.
            if entry.kind() != Some(git2::ObjectType::Blob) {
                return Err(Error::new(
                    ErrorCode::EntryNotFound,
                    format!("Not a secret blob at HEAD: {rel}"),
                ));
            }
            let oid = entry.id();
            // Read the blob content from the SAME tree snapshot (not the worktree)
            // so the bytes and the oid cannot diverge under a concurrent pull.
            let bytes = repo
                .find_blob(oid)
                .map_err(|e| {
                    Error::new(ErrorCode::StoreError, format!("Failed to read blob: {e}"))
                })?
                .content()
                .to_vec();
            Ok((bytes, oid.to_string()))
        })
        .await?
    }
}

/// Resolve HEAD to its commit tree — the shared prelude of [`GitStorage::entry_oid`]
/// and [`GitStorage::get_with_oid`]. Both read from the committed state (not the
/// worktree) so the base-version they capture/compare is measured against HEAD.
/// Uses `find_tree(commit.tree_id())` rather than `commit.tree()` so the returned
/// tree borrows the repository directly (not the local commit) and can outlive it.
fn head_tree(repo: &git2::Repository) -> Result<git2::Tree<'_>, Error> {
    let head = repo
        .head()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("Failed to read HEAD: {e}")))?
        .target()
        .ok_or_else(|| Error::new(ErrorCode::PullFfFailed, "No HEAD commit"))?;
    let tree_id = repo
        .find_commit(head)
        .map_err(|e| {
            Error::new(
                ErrorCode::StoreError,
                format!("Failed to read HEAD commit: {e}"),
            )
        })?
        .tree_id();
    repo.find_tree(tree_id).map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("Failed to read HEAD tree: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage
            .set("cloud/aws.age", b"ciphertext-bytes")
            .await
            .unwrap();
        let got = storage.get("cloud/aws.age").await.unwrap();
        assert_eq!(got, b"ciphertext-bytes");
    }

    #[tokio::test]
    async fn set_rejects_dotdot_name_before_any_fs_op() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        // A `..` name must be rejected by the lexical guard BEFORE create_dir_all
        // runs — so no directory is created outside the repo, and the error is
        // the within-repo rejection (ENTRY_NOT_FOUND), not an I/O error.
        let err = storage.set("../escape.age", b"x").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        let err = storage.set("legit/../escape.age", b"x").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_and_delete_reject_dotdot_name() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        assert_eq!(
            storage.get("../escape.age").await.unwrap_err().code,
            "ENTRY_NOT_FOUND"
        );
        assert_eq!(
            storage.delete("../escape.age").await.unwrap_err().code,
            "ENTRY_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn delete_missing_returns_entry_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        let err = storage.delete("nope.age").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    /// The recipients-index read/write path now goes through the generic file
    /// ops (storage owns the bytes; crypto owns the format). Round-trips through
    /// `write_file_atomic` + `read_file`, not the dropped
    /// `list_recipients`/`write_recipients` pair.
    #[tokio::test]
    async fn write_file_atomic_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage
            .write_file_atomic(".age-recipients", b"age1abc\n")
            .await
            .unwrap();
        let got = storage.read_file(".age-recipients").await.unwrap();
        assert_eq!(got, b"age1abc\n");
    }

    /// `read_file` returns `EntryNotFound` for a missing file — the no-TOCTOU
    /// contract (no separate `exists` step the caller could race).
    #[tokio::test]
    async fn read_file_missing_returns_entry_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        let err = storage.read_file(".age-recipients").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    /// Plumbing proof (learning: behavior-preserving-refactor-plumbing-test):
    /// `list(ext)` actually filters on the extension — a `.gpg` file is NOT
    /// returned when `ext` is `.age`. An all-`.age` fixture set would pass even
    /// if `ext` were silently ignored, so this negative case is what proves the
    /// plumbing carries the extension through.
    #[tokio::test]
    #[allow(clippy::indexing_slicing)]
    async fn list_extension_filter_excludes_other_extensions() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage.set("age-entry.age", b"x").await.unwrap();
        storage
            .write_file_atomic("gpg-entry.gpg", b"x")
            .await
            .unwrap();
        let entries = storage.list(SecretExt::AGE).await.unwrap();
        assert_eq!(
            entries.len(),
            1,
            ".gpg must be excluded when listing with ext=.age"
        );
        assert_eq!(entries[0].name, "age-entry");
    }

    /// `list_dir` returns repo-relative paths (`prefix/<name>`) for files under
    /// the prefix, non-recursive — subdirectories are skipped, not descended.
    #[tokio::test]
    async fn list_dir_returns_repo_relative_files() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage.write_file_atomic("pk/a", b"x").await.unwrap();
        storage.write_file_atomic("pk/b", b"x").await.unwrap();
        std::fs::create_dir(dir.path().join("pk/sub")).unwrap();
        let mut got = storage.list_dir("pk").await.unwrap();
        got.sort();
        assert_eq!(
            got,
            vec!["pk/a", "pk/b"],
            "files only, repo-relative; subdirs skipped"
        );
    }

    /// `list_dir` rejects a `..` prefix lexically before any fs op, matching the
    /// within-repo guard the other storage methods apply.
    #[tokio::test]
    async fn list_dir_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        assert_eq!(
            storage.list_dir("../escape").await.unwrap_err().code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `read_file`'s generic surface rejects a `..` path — the recipients-index
    /// and auxiliary-file read path, not just the `get`/`set`/`delete` secret
    /// paths already covered above.
    #[tokio::test]
    async fn read_file_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        assert_eq!(
            storage.read_file("../escape").await.unwrap_err().code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `write_file_atomic`'s generic surface rejects a `..` path — the
    /// recipients write path (and `.public-keys/` in Phase 3), not just `set`.
    #[tokio::test]
    async fn write_file_atomic_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        assert_eq!(
            storage
                .write_file_atomic("../escape", b"x")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `RepoFiles<'a>` adapts `&dyn StorageBackend` into the [`RepoFileView`]
    /// the crypto backend consumes. Pins the wiring + the borrow-lifetime
    /// invariant (no `repo_path` arg post-R051 — the backend owns the root).
    #[tokio::test]
    async fn repo_files_view_round_trips_through_storage() {
        use crate::storage::{RepoFileView, RepoFiles};
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage
            .write_file_atomic(".age-recipients", b"age1abc\n")
            .await
            .unwrap();
        let view = RepoFiles::new(&storage);
        let v: &dyn RepoFileView = &view;
        assert_eq!(v.read(".age-recipients").await.unwrap(), b"age1abc\n");
    }

    /// R026 read primitive: `get_with_oid` on an absent path returns
    /// `EntryNotFound` (fail-closed) — never `Ok` with empty bytes that the
    /// orchestrator would pair with a stale oid and silently downgrade an
    /// edit/delete to an unprotected base version.
    #[tokio::test]
    async fn get_with_oid_returns_entry_not_found_for_absent_path() {
        use crate::storage::git::test_support::{create_empty_commit, test_signature};
        let dir = tempfile::tempdir().unwrap();
        // `get_with_oid` reads from the HEAD tree, so the repo needs an initial
        // commit (an empty tree suffices — the absent path is what we test).
        let repo = git2::Repository::init(dir.path()).unwrap();
        let _oid = create_empty_commit(&repo, &test_signature());
        drop(repo);

        let storage = GitStorage::new(dir.path());
        let err = storage.get_with_oid("missing.age").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    /// R026 read primitive: `get_with_oid` returns the SAME oid as `entry_oid`
    /// for a present blob (the orchestrator compares them byte-for-byte), plus
    /// the blob bytes from the SAME HEAD-tree snapshot (no worktree read).
    #[tokio::test]
    async fn get_with_oid_matches_entry_oid_for_present_blob() {
        use crate::storage::git::test_support::test_signature;
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        storage.set("entry.age", b"ciphertext-bytes").await.unwrap();
        // Commit so HEAD's tree carries `entry.age` at a real blob oid.
        {
            let repo = git2::Repository::init(dir.path()).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("entry.age")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = test_signature();
            repo.commit(Some("HEAD"), &sig, &sig, "seed", &tree, &[])
                .unwrap();
            drop(tree);
            drop(index);
            drop(repo);
        }

        let oid = storage
            .entry_oid("entry.age")
            .await
            .unwrap()
            .expect("present at HEAD");
        let (bytes, oid_from_get) = storage
            .get_with_oid("entry.age")
            .await
            .expect("get_with_oid");
        assert_eq!(oid_from_get, oid, "oids must match");
        assert_eq!(bytes, b"ciphertext-bytes");
    }

    /// R026 read primitive: a subtree planted at `<name>.age` is NOT a secret
    /// blob. `entry_oid` returns `None` (the base-version guard skips it) and
    /// `get_with_oid` returns `EntryNotFound` (the fail-closed blob-kind guard)
    /// — rather than pairing bytes with a tree oid and subverting the conflict
    /// check.
    #[tokio::test]
    async fn entry_oid_none_and_get_with_oid_not_found_for_subtree() {
        use crate::storage::git::test_support::test_signature;
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage::new(dir.path());
        // Plant "entry.age" as a directory with a file inside, so the HEAD-tree
        // entry at "entry.age" is a Tree, not a Blob.
        std::fs::create_dir(dir.path().join("entry.age")).unwrap();
        std::fs::write(dir.path().join("entry.age").join("child"), b"x").unwrap();
        {
            let repo = git2::Repository::init(dir.path()).unwrap();
            let mut index = repo.index().unwrap();
            index.add_path(Path::new("entry.age/child")).unwrap();
            index.write().unwrap();
            let tree_id = index.write_tree().unwrap();
            let tree = repo.find_tree(tree_id).unwrap();
            let sig = test_signature();
            repo.commit(Some("HEAD"), &sig, &sig, "seed subtree", &tree, &[])
                .unwrap();
            drop(tree);
            drop(index);
            drop(repo);
        }

        let oid = storage.entry_oid("entry.age").await.unwrap();
        assert!(oid.is_none(), "subtree at <name>.age → None (fail-closed)");
        let err = storage.get_with_oid("entry.age").await.unwrap_err();
        assert_eq!(
            err.code, "ENTRY_NOT_FOUND",
            "subtree at <name>.age → EntryNotFound (fail-closed blob-kind guard)"
        );
    }
}
