// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The async [`StorageBackend`] shell for the git backend.
//!
//! [`GitStorage`] is stateless — `repo_path` / auth / policy are passed per call
//! (the real durable state is git's on-disk index, re-attached each op via
//! `Repository::discover`). Each method adapts a blocking free function in
//! `commit`/`pull`/`divergence` to async via `spawn_blocking`; file ops delegate
//! to `worktree`.

use std::path::Path;

use async_trait::async_trait;
use tokio::fs;
use tokio::task::spawn_blocking;

use crate::crypto::SecretExt;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::storage::{
    CancelToken, CommitKind, GitAuth, KeepLocalOutcome, ProgressSender, StorageBackend, StorageCtx,
    SyncDivergence, SyncOutcome, SyncResult,
};
use crate::template;

use super::worktree::{
    assert_within_repo, ensure_within_repo, list_entries, resolve_entry_path, write_atomic,
};
use super::{commit, divergence, pull};

/// The git storage backend (stateless — `repo_path` passed per call).
#[derive(Debug, Default, Clone, Copy)]
pub struct GitStorage;

#[async_trait]
impl StorageBackend for GitStorage {
    async fn list(&self, repo_path: &Path, ext: SecretExt) -> Result<Vec<Entry>, Error> {
        let repo_path = repo_path.to_path_buf();
        // WalkDir is synchronous (blocking I/O) — offload it. SecretExt is Copy.
        spawn_blocking(move || list_entries(&repo_path, ext)).await?
    }

    async fn get(&self, repo_path: &Path, passfile: &str) -> Result<Vec<u8>, Error> {
        ensure_within_repo(passfile)?;
        let file_path = resolve_entry_path(repo_path, passfile)?;
        fs::read(&file_path).await.map_err(|e| {
            Error::new(
                ErrorCode::IoError,
                format!("Failed to read entry file: {e}"),
            )
        })
    }

    async fn set(&self, repo_path: &Path, passfile: &str, ciphertext: &[u8]) -> Result<(), Error> {
        // Reject `..` / absolute names BEFORE any fs op — the trait is `pub`, so
        // a caller that skips `Store::validate_secret_name` still can't mkdir or
        // write outside the repo. (`assert_within_repo` below is the 2nd layer.)
        ensure_within_repo(passfile)?;
        let file_path = repo_path.join(passfile);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        write_atomic(&file_path, ciphertext).await
    }

    async fn delete(&self, repo_path: &Path, passfile: &str) -> Result<(), Error> {
        ensure_within_repo(passfile)?;
        // Existence + within-repo guard before any mutation.
        resolve_entry_path(repo_path, passfile)?;
        let file_path = repo_path.join(passfile);
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        match fs::remove_file(&file_path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::new(
                ErrorCode::EntryNotFound,
                format!("Entry not found: {passfile}"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    async fn read_file(&self, repo_path: &Path, rel_path: &str) -> Result<Vec<u8>, Error> {
        ensure_within_repo(rel_path)?;
        // resolve_entry_path checks existence + within-repo (canonicalize) in one
        // step — no caller-level exists-then-read, so no TOCTOU that could shrink
        // the recipient set.
        let file_path = resolve_entry_path(repo_path, rel_path)?;
        fs::read(&file_path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Error::new(
                ErrorCode::EntryNotFound,
                format!("File not found: {rel_path}"),
            ),
            _ => Error::new(ErrorCode::IoError, format!("Failed to read file: {e}")),
        })
    }

    async fn write_file_atomic(
        &self,
        repo_path: &Path,
        rel_path: &str,
        bytes: &[u8],
    ) -> Result<(), Error> {
        ensure_within_repo(rel_path)?;
        let file_path = repo_path.join(rel_path);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        write_atomic(&file_path, bytes).await
    }

    async fn list_dir(&self, repo_path: &Path, rel_prefix: &str) -> Result<Vec<String>, Error> {
        ensure_within_repo(rel_prefix)?;
        let dir = resolve_entry_path(repo_path, rel_prefix)?;
        let mut out: Vec<String> = Vec::new();
        let mut entries = fs::read_dir(&dir).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => {
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

    async fn lookup_template(&self, repo_path: &Path, name: &str) -> Result<Option<String>, Error> {
        let repo_path = repo_path.to_path_buf();
        let name_owned = name.to_string();
        // Filesystem walk; cheap enough to run on a blocking thread.
        Ok(
            spawn_blocking(move || template::lookup_template_in_repo(&repo_path, &name_owned))
                .await?,
        )
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
        dest: &Path,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let auth = auth.clone();
        let url = url.to_string();
        let dest = dest.to_path_buf();
        spawn_blocking(move || {
            commit::clone_repo(&url, &dest, &auth, cancel.as_ref(), progress.as_ref())
        })
        .await?
    }

    async fn init_repo(&self, repo_path: &Path) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        spawn_blocking(move || commit::init_repo(&repo_path)).await?
    }

    async fn remote_add(&self, repo_path: &Path, name: &str, url: &str) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        let name = name.to_string();
        let url = url.to_string();
        spawn_blocking(move || commit::remote_add(&repo_path, &name, &url)).await?
    }

    async fn commit(
        &self,
        ctx: &StorageCtx<'_>,
        kind: CommitKind,
        paths: &[String],
        message: &str,
    ) -> Result<String, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let name = ctx.commit_name.map(str::to_string);
        let email = ctx.commit_email.map(str::to_string);
        let paths = paths.to_vec();
        let message = message.to_string();
        spawn_blocking(move || match kind {
            CommitKind::Add => commit::commit(
                &repo_path,
                &paths,
                &message,
                name.as_deref(),
                email.as_deref(),
            ),
            CommitKind::Remove => commit::commit_removal(
                &repo_path,
                &paths,
                &message,
                name.as_deref(),
                email.as_deref(),
            ),
        })
        .await?
    }

    async fn commit_initial(
        &self,
        repo_path: &Path,
        paths: &[String],
        message: &str,
    ) -> Result<String, Error> {
        let repo_path = repo_path.to_path_buf();
        let paths = paths.to_vec();
        let message = message.to_string();
        spawn_blocking(move || commit::commit_initial(&repo_path, &paths, &message)).await?
    }

    async fn push(&self, ctx: &StorageCtx<'_>) -> Result<(), Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        spawn_blocking(move || commit::push(&repo_path, &auth)).await?
    }

    async fn pull(
        &self,
        ctx: &StorageCtx<'_>,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<SyncOutcome, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        spawn_blocking(move || {
            pull::pull_repo(
                &repo_path,
                &auth,
                &policy,
                cancel.as_ref(),
                progress.as_ref(),
            )
        })
        .await?
    }

    async fn adopt_remote(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
    ) -> Result<SyncResult, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        let expected = expected_remote_oid.to_string();
        spawn_blocking(move || pull::adopt_remote(&repo_path, &auth, &policy, &expected)).await?
    }

    async fn preview_divergence(&self, ctx: &StorageCtx<'_>) -> Result<SyncDivergence, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        spawn_blocking(move || divergence::preview_divergence(&repo_path, &auth)).await?
    }

    async fn keep_local_plan(
        &self,
        ctx: &StorageCtx<'_>,
        expected_remote_oid: &str,
    ) -> Result<KeepLocalOutcome, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        let policy = ctx.policy.clone();
        let expected = expected_remote_oid.to_string();
        spawn_blocking(move || divergence::keep_local_plan(&repo_path, &auth, &policy, &expected))
            .await?
    }

    async fn keep_local_advance(&self, repo_path: &Path, fetched_oid: &str) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        let fetched = fetched_oid.to_string();
        spawn_blocking(move || divergence::keep_local_advance(&repo_path, &fetched)).await?
    }

    async fn keep_local_finalize(
        &self,
        ctx: &StorageCtx<'_>,
        ciphertexts: &[(String, Vec<u8>)],
        deletes: &[String],
    ) -> Result<String, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        let name = ctx.commit_name.map(str::to_string);
        let email = ctx.commit_email.map(str::to_string);
        let entries = ciphertexts.to_vec();
        let deletes = deletes.to_vec();
        spawn_blocking(move || {
            divergence::keep_local_finalize(
                &repo_path,
                &auth,
                &entries,
                &deletes,
                name.as_deref(),
                email.as_deref(),
            )
        })
        .await?
    }

    async fn current_head(&self, repo_path: &Path) -> Result<String, Error> {
        let repo_path = repo_path.to_path_buf();
        spawn_blocking(move || {
            let repo = git2::Repository::discover(&repo_path)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        storage
            .set(dir.path(), "cloud/aws.age", b"ciphertext-bytes")
            .await
            .unwrap();
        let got = storage.get(dir.path(), "cloud/aws.age").await.unwrap();
        assert_eq!(got, b"ciphertext-bytes");
    }

    #[tokio::test]
    async fn set_rejects_dotdot_name_before_any_fs_op() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        // A `..` name must be rejected by the lexical guard BEFORE create_dir_all
        // runs — so no directory is created outside the repo, and the error is
        // the within-repo rejection (ENTRY_NOT_FOUND), not an I/O error.
        let err = storage
            .set(dir.path(), "../escape.age", b"x")
            .await
            .unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        let err = storage
            .set(dir.path(), "legit/../escape.age", b"x")
            .await
            .unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_and_delete_reject_dotdot_name() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        assert_eq!(
            storage
                .get(dir.path(), "../escape.age")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
        assert_eq!(
            storage
                .delete(dir.path(), "../escape.age")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
    }

    #[tokio::test]
    async fn delete_missing_returns_entry_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        let err = storage.delete(dir.path(), "nope.age").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    /// The recipients-index read/write path now goes through the generic file
    /// ops (storage owns the bytes; crypto owns the format). Round-trips through
    /// `write_file_atomic` + `read_file`, not the dropped
    /// `list_recipients`/`write_recipients` pair.
    #[tokio::test]
    async fn write_file_atomic_then_read_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        storage
            .write_file_atomic(dir.path(), ".age-recipients", b"age1abc\n")
            .await
            .unwrap();
        let got = storage
            .read_file(dir.path(), ".age-recipients")
            .await
            .unwrap();
        assert_eq!(got, b"age1abc\n");
    }

    /// `read_file` returns `EntryNotFound` for a missing file — the no-TOCTOU
    /// contract (no separate `exists` step the caller could race).
    #[tokio::test]
    async fn read_file_missing_returns_entry_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        let err = storage
            .read_file(dir.path(), ".age-recipients")
            .await
            .unwrap_err();
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
        let storage = GitStorage;
        storage
            .set(dir.path(), "age-entry.age", b"x")
            .await
            .unwrap();
        storage
            .write_file_atomic(dir.path(), "gpg-entry.gpg", b"x")
            .await
            .unwrap();
        let entries = storage.list(dir.path(), SecretExt::AGE).await.unwrap();
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
        let storage = GitStorage;
        storage
            .write_file_atomic(dir.path(), "pk/a", b"x")
            .await
            .unwrap();
        storage
            .write_file_atomic(dir.path(), "pk/b", b"x")
            .await
            .unwrap();
        std::fs::create_dir(dir.path().join("pk/sub")).unwrap();
        let mut got = storage.list_dir(dir.path(), "pk").await.unwrap();
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
        let storage = GitStorage;
        assert_eq!(
            storage
                .list_dir(dir.path(), "../escape")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `read_file`'s generic surface rejects a `..` path — the recipients-index
    /// and auxiliary-file read path, not just the `get`/`set`/`delete` secret
    /// paths already covered above.
    #[tokio::test]
    async fn read_file_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        assert_eq!(
            storage
                .read_file(dir.path(), "../escape")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `write_file_atomic`'s generic surface rejects a `..` path — the
    /// recipients write path (and `.public-keys/` in Phase 3), not just `set`.
    #[tokio::test]
    async fn write_file_atomic_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        assert_eq!(
            storage
                .write_file_atomic(dir.path(), "../escape", b"x")
                .await
                .unwrap_err()
                .code,
            "ENTRY_NOT_FOUND"
        );
    }

    /// `RepoFiles<'a>` adapts `&dyn StorageBackend` + `repo_path` into the
    /// [`RepoFileView`] the crypto backend consumes (Phase 1.2+). Pins the wiring
    /// + the borrow-lifetime invariant while the seam has no production caller.
    #[tokio::test]
    async fn repo_files_view_round_trips_through_storage() {
        use crate::storage::{RepoFileView, RepoFiles};
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        storage
            .write_file_atomic(dir.path(), ".age-recipients", b"age1abc\n")
            .await
            .unwrap();
        let view = RepoFiles::new(&storage, dir.path());
        let v: &dyn RepoFileView = &view;
        assert_eq!(v.read(".age-recipients").await.unwrap(), b"age1abc\n");
    }
}
