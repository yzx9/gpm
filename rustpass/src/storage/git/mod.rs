// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The git storage backend — sole [`StorageBackend`] implementation.
//!
//! Owns the repo working-tree file ops: list/get/set/delete `.age` entries,
//! read/write the recipients file, and look up templates. The within-repo
//! path-traversal guard (`resolve_entry_path`/`assert_within_repo`) lives here,
//! so `Store` hands the backend an entry *name* and the backend maps it to
//! `<repo>/<name>.age` and validates the resolved path.
//!
//! RCS ops (clone/pull/push/keep-mine) live in [`rcs`] as blocking free
//! functions, adapted to this trait via `spawn_blocking`. `GitStorage` is
//! stateless — auth/policy are passed per-op, not held at construction (the real
//! durable state is git's on-disk index, re-attached each op via
//! `Repository::discover`).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;
use tokio::task::spawn_blocking;
use walkdir::WalkDir;

use crate::entry::Entry;
use crate::error::{Error, ErrorCode};
use crate::recipient;
use crate::storage::{
    CancelToken, CommitKind, GitAuth, KeepLocalOutcome, ProgressSender, StorageBackend, StorageCtx,
    SyncDivergence, SyncOutcome, SyncResult,
};
use crate::template;

/// Blocking RCS + transport free functions (`clone_repo`, `pull_repo`, keep-mine,
/// CA-bundle, …). Adapted to the async trait below via `spawn_blocking`.
mod rcs;

/// The git storage backend (stateless — `repo_path` passed per call).
#[derive(Debug, Default, Clone, Copy)]
pub struct GitStorage;

#[async_trait]
impl StorageBackend for GitStorage {
    async fn list(&self, repo_path: &Path) -> Result<Vec<Entry>, Error> {
        let repo_path = repo_path.to_path_buf();
        // WalkDir is synchronous (blocking I/O) — offload it.
        spawn_blocking(move || list_entries(&repo_path)).await?
    }

    async fn get(&self, repo_path: &Path, name: &str) -> Result<Vec<u8>, Error> {
        let passfile = passfile_rel(name);
        ensure_within_repo(&passfile)?;
        let file_path = resolve_entry_path(repo_path, &passfile)?;
        fs::read(&file_path).await.map_err(|e| {
            Error::new(
                ErrorCode::IoError,
                format!("Failed to read entry file: {e}"),
            )
        })
    }

    async fn set(&self, repo_path: &Path, name: &str, ciphertext: &[u8]) -> Result<(), Error> {
        let passfile = passfile_rel(name);
        // Reject `..` / absolute names BEFORE any fs op — the trait is `pub`, so
        // a caller that skips `Store::validate_secret_name` still can't mkdir or
        // write outside the repo. (`assert_within_repo` below is the 2nd layer.)
        ensure_within_repo(&passfile)?;
        let file_path = repo_path.join(&passfile);
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        write_atomic(&file_path, ciphertext).await
    }

    async fn delete(&self, repo_path: &Path, name: &str) -> Result<(), Error> {
        let passfile = passfile_rel(name);
        ensure_within_repo(&passfile)?;
        // Existence + within-repo guard before any mutation.
        resolve_entry_path(repo_path, &passfile)?;
        let file_path = repo_path.join(&passfile);
        assert_within_repo(repo_path, file_path.parent().unwrap_or(Path::new("")))?;
        match fs::remove_file(&file_path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(Error::new(
                ErrorCode::EntryNotFound,
                format!("Entry not found: {name}"),
            )),
            Err(e) => Err(e.into()),
        }
    }

    async fn list_recipients(&self, repo_path: &Path) -> Result<Vec<recipient::Recipient>, Error> {
        recipient::list_recipients(repo_path).await
    }

    async fn write_recipients(&self, repo_path: &Path, recipients: &[String]) -> Result<(), Error> {
        recipient::write_recipients(repo_path, recipients).await
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
    // Each method adapts a blocking free function in `rcs` to the async trait:
    // move owned args into a `spawn_blocking` closure and pass the `&StorageCtx`
    // fields by value (cloning the cheap ones — `GitAuth`/`AuthenticityConfig` —
    // since the closure must be `'static`).

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
            rcs::clone_repo(&url, &dest, &auth, cancel.as_ref(), progress.as_ref())
        })
        .await?
    }

    async fn init_repo(&self, repo_path: &Path) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        spawn_blocking(move || rcs::init_repo(&repo_path)).await?
    }

    async fn remote_add(&self, repo_path: &Path, name: &str, url: &str) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        let name = name.to_string();
        let url = url.to_string();
        spawn_blocking(move || rcs::remote_add(&repo_path, &name, &url)).await?
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
            CommitKind::Add => rcs::commit(
                &repo_path,
                &paths,
                &message,
                name.as_deref(),
                email.as_deref(),
            ),
            CommitKind::Remove => rcs::commit_removal(
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
        spawn_blocking(move || rcs::commit_initial(&repo_path, &paths, &message)).await?
    }

    async fn push(&self, ctx: &StorageCtx<'_>) -> Result<(), Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        spawn_blocking(move || rcs::push(&repo_path, &auth)).await?
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
            rcs::pull_repo(
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
        spawn_blocking(move || rcs::adopt_remote(&repo_path, &auth, &policy, &expected)).await?
    }

    async fn preview_divergence(&self, ctx: &StorageCtx<'_>) -> Result<SyncDivergence, Error> {
        let repo_path = ctx.repo_path.to_path_buf();
        let auth = ctx.auth.clone();
        spawn_blocking(move || rcs::preview_divergence(&repo_path, &auth)).await?
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
        spawn_blocking(move || rcs::keep_local_plan(&repo_path, &auth, &policy, &expected)).await?
    }

    async fn keep_local_advance(&self, repo_path: &Path, fetched_oid: &str) -> Result<(), Error> {
        let repo_path = repo_path.to_path_buf();
        let fetched = fetched_oid.to_string();
        spawn_blocking(move || rcs::keep_local_advance(&repo_path, &fetched)).await?
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
            rcs::keep_local_finalize(
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

// ── Relocated working-tree helpers (were free fns in `store.rs`) ────────────
//
// These moved here so `storage::git` doesn't depend on `store` (which would
// reopen the `store` ↔ `storage` module cycle the relocation avoided).
// `list_entries` and
// `resolve_entry_path` stay `pub` and are re-exported from `store` so existing
// integration-test call sites (`store::list_entries`, `store::resolve_entry_path`)
// keep compiling unchanged.

/// Walk a gopass store directory and return all `.age` entries.
///
/// Skips the `.git` directory. Only returns files with a `.age` extension.
///
/// # Errors
///
/// Returns an error if the repository path does not exist.
pub fn list_entries(repo_path: &Path) -> Result<Vec<Entry>, Error> {
    if !repo_path.exists() {
        return Err(Error::new(
            ErrorCode::NoRepo,
            "Repository path does not exist",
        ));
    }

    let mut entries: Vec<Entry> = WalkDir::new(repo_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.file_name().to_str().is_some_and(|name| {
                Path::new(name)
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
            })
        })
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == ".git"))
        .filter_map(|e| {
            let rel = e.path().strip_prefix(repo_path).ok()?;
            let rel_str = rel.to_str()?.to_string();
            let name = rel_str.trim_end_matches(".age").to_string();
            Some(Entry {
                path: rel_str,
                name,
            })
        })
        .collect();

    entries.sort_by_key(|a| a.name.to_lowercase());
    Ok(entries)
}

/// Verify an entry file exists within the repo and return its full path.
///
/// # Errors
///
/// Returns an error if the entry does not exist or if the resolved path
/// escapes the repository directory (path traversal guard).
pub fn resolve_entry_path(repo_path: &Path, entry_path: &str) -> Result<PathBuf, Error> {
    let full_path = repo_path.join(entry_path);

    if !full_path.exists() {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            format!("Entry not found: {entry_path}"),
        ));
    }

    let canonical_repo = repo_path.canonicalize()?;
    let canonical_entry = full_path.canonicalize()?;
    if !canonical_entry.starts_with(&canonical_repo) {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }

    Ok(full_path)
}

/// Defense-in-depth check that `dir` resolves inside `repo_path`.
///
/// Used after creating a secret's parent directory: the directory exists, so it
/// can be canonicalized, and we assert it is contained by the canonical repo
/// root. Catches any traversal a name-validation gap would otherwise allow.
fn assert_within_repo(repo_path: &Path, dir: &Path) -> Result<(), Error> {
    let canonical_repo = repo_path.canonicalize()?;
    let canonical_dir = dir.canonicalize()?;
    if !canonical_dir.starts_with(&canonical_repo) {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }
    Ok(())
}

/// Lexically reject a `passfile` whose path could escape the repo, BEFORE any
/// filesystem op.
///
/// This is the backend's own front-line guard: `StorageBackend` is `pub`, so a
/// caller that skips `Store::validate_secret_name` (e.g. a future in-tree
/// caller, a second backend impl, or a test) still can't reach
/// `create_dir_all`/`remove_file` with a `..` or absolute name. It runs before
/// the post-op `assert_within_repo` canonicalize check — two layers, since
/// neither alone is a sandbox (canonicalize needs the path to exist; lexical
/// check can't see symlinks).
fn ensure_within_repo(passfile: &str) -> Result<(), Error> {
    let escaped = Path::new(passfile).components().any(|c| {
        matches!(
            c,
            std::path::Component::ParentDir
                | std::path::Component::RootDir
                | std::path::Component::Prefix(_)
        )
    });
    if escaped {
        return Err(Error::new(
            ErrorCode::EntryNotFound,
            "Entry path is outside repository",
        ));
    }
    Ok(())
}

/// Atomic write: write to a temp file beside the target, then rename over it.
///
/// Mirrors [`Config`'s](crate::config::Config) atomic write so a failed write
/// can never leave a half-written ciphertext behind.
async fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, data).await?;
    fs::rename(&temp_path, path).await?;
    Ok(())
}

/// The on-disk relative path for a secret named `name` (gopass `passfile`).
///
/// A leading `/` is stripped; if the name already ends in `.age` it is kept
/// as-is, otherwise `.age` is appended. Matches the resolution `get` uses.
pub(crate) fn passfile_rel(name: &str) -> String {
    let name = name.trim_start_matches('/');
    if Path::new(name)
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("age"))
    {
        name.to_string()
    } else {
        format!("{name}.age")
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
            .set(dir.path(), "cloud/aws", b"ciphertext-bytes")
            .await
            .unwrap();
        let got = storage.get(dir.path(), "cloud/aws").await.unwrap();
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
            .set(dir.path(), "../escape", b"x")
            .await
            .unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        let err = storage
            .set(dir.path(), "legit/../escape", b"x")
            .await
            .unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }

    #[tokio::test]
    async fn get_and_delete_reject_dotdot_name() {
        let dir = tempfile::tempdir().unwrap();
        let storage = GitStorage;
        assert_eq!(
            storage.get(dir.path(), "../escape").await.unwrap_err().code,
            "ENTRY_NOT_FOUND"
        );
        assert_eq!(
            storage
                .delete(dir.path(), "../escape")
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
        let err = storage.delete(dir.path(), "nope").await.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
    }
}
