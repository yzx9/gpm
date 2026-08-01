// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Working-tree file ops + within-repo path guards for the git backend.
//!
//! The content half of [`StorageBackend`](crate::storage::StorageBackend): list,
//! resolve, atomic-write, and delete `.age` entries, plus the multi-layer
//! path-traversal defense (`ensure_within_repo` lexical check before any fs op,
//! `assert_within_repo` canonicalize check after). `Store` hands the backend an
//! entry *name*; these helpers map it to `<repo>/<name>.age` and validate the
//! resolved path.
//!
//! `list_entries` / `resolve_entry_path` / `passfile_rel` are re-exported from
//! [`super`] so existing call sites (`store::list_entries`,
//! `store::resolve_entry_path`, `store::passfile_rel`) keep compiling.

use std::path::{Path, PathBuf};

use tokio::fs;
use walkdir::WalkDir;

use crate::crypto::SecretExt;
use crate::entry::Entry;
use crate::error::{Error, ErrorCode};

/// Walk a gopass store directory and return all entries with the `ext`
/// extension (`.age` / `.gpg` — the crypto backend's secret extension).
///
/// Skips the `.git` directory. Only returns files whose extension matches `ext`.
///
/// # Errors
///
/// Returns an error if the repository path does not exist.
pub fn list_entries(repo_path: &Path, ext: SecretExt) -> Result<Vec<Entry>, Error> {
    if !repo_path.exists() {
        return Err(Error::new(
            ErrorCode::NoRepo,
            "Repository path does not exist",
        ));
    }
    // `ext` is the backend's typed SecretExt; the file-extension comparison is
    // dotless + ASCII-case.
    let s = ext.as_str();
    let want = s.trim_start_matches('.').to_ascii_lowercase();

    let mut entries: Vec<Entry> = WalkDir::new(repo_path)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            e.file_name().to_str().is_some_and(|name| {
                Path::new(name)
                    .extension()
                    .is_some_and(|x| x.eq_ignore_ascii_case(&want))
            })
        })
        .filter(|e| !e.path().components().any(|c| c.as_os_str() == ".git"))
        .filter_map(|e| {
            let rel = e.path().strip_prefix(repo_path).ok()?;
            let rel_str = rel.to_str()?.to_string();
            let name = rel_str.trim_end_matches(s).to_string();
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
pub(super) fn assert_within_repo(repo_path: &Path, dir: &Path) -> Result<(), Error> {
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
pub(super) fn ensure_within_repo(passfile: &str) -> Result<(), Error> {
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

/// Build a temp-file path beside `target` with an unpredictable suffix, in the
/// same parent so the rename onto `target` stays atomic (same filesystem). The
/// random suffix means a malicious clone can't pre-plant this exact path as a
/// symlink — a path-derived `.tmp` name is predictable, and `fs::write` follows
/// a symlink planted there, writing outside the repo — and two concurrent
/// writes to the same target can't collide on one temp name.
pub(super) fn random_temp_sibling(target: &Path) -> PathBuf {
    let suffix = rand::random::<u64>();
    let name = match target.file_name() {
        Some(n) => format!("{}.tmp.{suffix}", n.to_string_lossy()),
        None => format!(".tmp.{suffix}"),
    };
    match target.parent() {
        Some(parent) => parent.join(name),
        None => PathBuf::from(name),
    }
}

/// Atomic write: write to a randomly-named temp file beside the target, then
/// rename over it. Mirrors [`Config`'s](crate::config::Config) atomic write so a
/// failed write can never leave a half-written ciphertext behind; the temp name
/// is randomized (see [`random_temp_sibling`]) so it can't be pre-planted as a
/// symlink or collide with a concurrent write.
pub(super) async fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let temp_path = random_temp_sibling(path);
    fs::write(&temp_path, data).await?;
    fs::rename(&temp_path, path).await?;
    Ok(())
}

/// The on-disk relative path for a secret named `name` (gopass `passfile`).
///
/// A leading `/` is stripped; if the name already ends in `ext`
/// (ASCII-case-insensitive, matching the legacy `eq_ignore_ascii_case("age")`
/// behavior so `set("foo.AGE")` round-trips instead of producing a stray
/// `foo.AGE.age`) it is kept as-is, otherwise `ext` is appended. `ext` is the
/// crypto backend's secret extension (`.age` / `.gpg`). Matches the resolution
/// `get` uses.
pub(crate) fn passfile_rel(name: &str, ext: SecretExt) -> String {
    let name = name.trim_start_matches('/');
    let s = ext.as_str();
    if name.to_ascii_lowercase().ends_with(&s.to_ascii_lowercase()) {
        name.to_string()
    } else {
        format!("{name}{s}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression: `passfile_rel` extension match stays ASCII-case-insensitive
    /// (the legacy `eq_ignore_ascii_case("age")` behavior), so a name already
    /// ending in `.AGE` is kept as-is rather than producing a stray `.age`
    /// append that would break `get` round-trips.
    #[test]
    fn passfile_rel_case_insensitive_extension_match() {
        assert_eq!(passfile_rel("foo.AGE", SecretExt::AGE), "foo.AGE");
        assert_eq!(passfile_rel("foo.age", SecretExt::AGE), "foo.age");
        assert_eq!(passfile_rel("foo", SecretExt::AGE), "foo.age");
        assert_eq!(passfile_rel("dir/up.AGE", SecretExt::AGE), "dir/up.AGE");
    }
}
