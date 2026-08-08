// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::collections::HashMap;
use std::hash::BuildHasher;
use std::str;

use tokio::task::spawn_blocking;

use crate::error::{Error, ErrorCode};
use crate::secret::Secret;
use crate::signing;
use crate::storage::git::passfile_rel;
use crate::storage::{CommitKind, RepoFiles};
use crate::template;

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

/// The outcome of reading one past revision of a secret (R027). Ciphertext
/// never leaves [`Store::get_at_revision`]: a revision the current identity
/// can't decrypt is reported as [`RevisionContent::Undecryptable`], not
/// surfaced.
#[derive(Debug)]
pub enum RevisionContent {
    /// Decrypted past value.
    Decrypted(Secret),
    /// The revision's ciphertext can't be decrypted with the current identity
    /// (recipient-set rotation, an identity change, or a teammate's revision).
    Undecryptable,
    /// The commit deleted the entry — no blob at that commit.
    Deleted,
}

impl Store {
    /// Fuzzy-search the configured repository's entries by `query`, ranked by
    /// relevance: best match first, ties broken by `path`. Returns one page of
    /// up to `limit` entries starting at `offset`, plus the **total** match
    /// count (independent of the slice). An empty query matches every entry
    /// (alpha-sorted) — equivalent to [`list`](Store::list).
    ///
    /// Ranking is a stable strict total order — score descending, then unique
    /// `path` ascending — so paging a fixed entry set by offset never splits a
    /// tie or reorders between requests.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn search(
        &self,
        query: &str,
        offset: usize,
        limit: usize,
    ) -> Result<RankedPage, Error> {
        let entries = self.storage()?.list(self.secret_ext()?).await?;
        let q = query.to_string();
        Ok(spawn_blocking(move || slice_page(rank_entries(entries, &q), offset, limit)).await?)
    }

    /// One page of the configured repository's entries, alpha-sorted —
    /// [`search`](Store::search) with an empty query, since an empty query ranks
    /// to the alpha-sorted full set.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured or the repo path
    /// does not exist.
    pub async fn list(&self, offset: usize, limit: usize) -> Result<RankedPage, Error> {
        self.search("", offset, limit).await
    }

    /// Decrypt and return a secret by entry name.
    ///
    /// If the identity is encrypted, uses the cached (unlocked) identity.
    /// If the identity is plaintext, loads directly from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry does not exist, the identity is missing,
    /// the identity is encrypted but not unlocked, or decryption fails.
    pub async fn get(&self, name: &str) -> Result<Secret, Error> {
        let encrypted = self
            .storage()?
            .get(&passfile_rel(name, self.secret_ext()?))
            .await?;
        let identity_bytes = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let decrypted = crypto.decrypt(&encrypted, &identity_bytes).await?;
        Secret::parse(&decrypted)
    }

    /// Decrypt entry `name` AND capture its blob oid at HEAD, both from the SAME
    /// HEAD commit-tree snapshot (atomic) — the read-time base version for
    /// base-version-aware edit (RFC R026). Errors propagate (fail-closed): if the
    /// oid cannot be captured the read fails with a clear error rather than
    /// silently downgrading to an unprotected base. Use [`Store::get`] when the
    /// oid is not needed.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::EntryNotFound`] if the entry is absent at HEAD (fail-closed),
    /// [`ErrorCode::NoRepo`] if no repo is found, or a git/crypto error.
    pub async fn get_with_oid(&self, name: &str) -> Result<(Secret, String), Error> {
        let passfile = passfile_rel(name, self.secret_ext()?);
        let (encrypted, oid) = self.storage()?.get_with_oid(&passfile).await?;
        let identity_bytes = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let decrypted = crypto.decrypt(&encrypted, &identity_bytes).await?;
        let secret = Secret::parse(&decrypted)?;
        Ok((secret, oid))
    }

    /// Blob oid of entry `name` at HEAD, or `None` if absent. Cheap, non-secret,
    /// needs no identity/decrypt — used by the delete base-version capture on the
    /// detail page (decoupled from reveal, so delete-without-reveal is still
    /// protected) and available for the orchestrator's pre-write check.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::NoRepo`] if no repo is found, or a git error. Returns
    /// `Ok(None)` (not an error) when the entry is absent at HEAD.
    pub async fn entry_oid(&self, name: &str) -> Result<Option<String>, Error> {
        let passfile = passfile_rel(name, self.secret_ext()?);
        self.storage()?.entry_oid(&passfile).await
    }

    /// Encrypt and write a secret to the store, then commit **locally** (no
    /// sync, no push).
    ///
    /// This is gopass's `set` (write) command, local-only. The plaintext is
    /// encrypted to every recipient in the store's `.age-recipients`, with our
    /// own key guaranteed to be among the encryption
    /// targets (mirroring gopass's `ensureOurKeyID`, so we can always read back
    /// what we wrote), written to `<name>.age`, and committed on the current
    /// branch. It does **not** pull or push — publishing is the caller's job.
    /// Production callers go through [`Store::autosync_write`], which wraps this
    /// in a pull → write → push and routes a rejected push to the sync-time
    /// divergence surface; calling `set` directly skips that serialization, so
    /// it is for tests and the orchestrator only.
    ///
    /// The base-version silent-clobber this once risked is guarded at the
    /// orchestrator: [`Store::autosync_write`] refuses a stale edit/delete via a
    /// base-oid check (RFC R026). This primitive stays local-only (no guard) —
    /// call it directly only in tests or inside the orchestrator.
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a malformed name, `InvalidIdentity` if no
    /// usable recipient (and our own key) can be derived, or a git error if
    /// staging or committing fails.
    pub async fn set(&self, name: &str, plaintext: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let rcs = self.rcs_ctx().await?;
        let passfile = self.encrypt_and_write(name, plaintext).await?;
        let head = self
            .commit_local(
                &rcs,
                CommitKind::Add,
                passfile,
                format!("Save secret: {name}"),
            )
            .await?;
        Ok(WriteResult { commit: head })
    }

    /// Delete a secret: remove `<name>.age` and commit the removal **locally**
    /// (no sync, no push). The delete sibling of [`set`].
    ///
    /// Local-only, like [`set`]: no pre-sync, no push, no rollback. Publishing is
    /// the caller's job — production callers go through [`Store::autosync_write`],
    /// which wraps this in pull → delete → push and routes a rejected push to the
    /// sync-time divergence surface. Calling `delete` directly is for tests and
    /// the orchestrator only.
    ///
    /// Like [`set`], the base-version silent-clobber this once risked is guarded
    /// at the orchestrator ([`Store::autosync_write`], RFC R026); this primitive
    /// stays local-only (no guard).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidEntryName`] for a malformed name,
    /// [`ErrorCode::EntryNotFound`] if the entry doesn't exist, or a git error
    /// from the underlying remove/commit.
    pub async fn delete(&self, name: &str) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let passfile = passfile_rel(name, self.secret_ext()?);
        let rcs = self.rcs_ctx().await?;

        // Existence + within-repo guard + remove the worktree file. The index
        // removal is staged in the commit below.
        self.storage()?.delete(&passfile).await?;

        let head = self
            .commit_local(
                &rcs,
                CommitKind::Remove,
                passfile,
                format!("Delete secret: {name}"),
            )
            .await?;
        Ok(WriteResult { commit: head })
    }

    /// Edit a secret in place: overwrite an **existing** entry's body via the
    /// local-only [`Store::set`]. The edit sibling of [`create`] — but gated on
    /// existence and with no template applied, so a typo'd name can't silently
    /// create a stray entry and the user's raw edited body is stored verbatim
    /// (templates shape new secrets, not mutations).
    ///
    /// The existence gate is a **local typo guard**; it is not a remote-state
    /// invariant. Edit inherits [`set`]'s base-version story (see its docs): the
    /// silent-clobber is guarded at the orchestrator ([`Store::autosync_write`],
    /// RFC R026), not in this local-only primitive.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::InvalidEntryName`] for a malformed name,
    /// [`ErrorCode::EntryNotFound`] if the entry doesn't exist, or whatever
    /// [`Store::set`] returns.
    pub async fn update(&self, name: &str, plaintext: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let repo_path = self.repo_path().await?;
        // Existence gate: a local typo guard so edit can't create a stray entry.
        // resolve_entry_path also guards path traversal (used identically by `get`
        // and `delete`). NOT a remote-state check.
        resolve_entry_path(&repo_path, &passfile_rel(name, self.secret_ext()?))?;
        // Raw write primitive (no template), local-only via `set`.
        self.set(name, plaintext).await
    }

    /// Look up the content template (`.pass-template`) that applies to `name`,
    /// walking up the directory tree (gopass `LookupTemplate`).
    ///
    /// Returns `Ok(None)` when no template applies. Templates are stored as
    /// plaintext, so this reads straight from the worktree.
    ///
    /// # Errors
    ///
    /// Returns an error if the store is not configured.
    pub async fn lookup_template(&self, name: &str) -> Result<Option<String>, Error> {
        self.storage()?.lookup_template(name).await
    }

    /// Create a secret, applying a matching `.pass-template` if one exists
    /// (gopass `renderTemplate`).
    ///
    /// `content` becomes the template's `.Content` (usually the password); the
    /// rendered template is what gets stored. When no template applies, the
    /// content is stored verbatim. Either way the result is written and
    /// committed locally via [`Store::set`] (no sync/push from `create` itself).
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a bad name, `TemplateError` if a template
    /// references an unknown variable, or whatever [`Store::set`] returns.
    pub async fn create(&self, name: &str, content: &[u8]) -> Result<WriteResult, Error> {
        validate_secret_name(name)?;
        let rendered = self.resolve_template(name, content).await?;
        let final_bytes = rendered.map_or_else(|| content.to_vec(), String::into_bytes);
        self.set(name, &final_bytes).await
    }

    /// Resolve a `.pass-template` for `name` against `content` and return the
    /// rendered body, or `None` when no (non-empty) template applies or the
    /// payload isn't UTF-8. Shared by [`Store::create`] and
    /// [`Store::preview_create`].
    async fn resolve_template(&self, name: &str, content: &[u8]) -> Result<Option<String>, Error> {
        // Templates render against text; secrets are text, so a non-UTF-8
        // payload just skips templating.
        Ok(
            match (
                str::from_utf8(content).ok(),
                self.lookup_template(name).await?,
            ) {
                (Some(text), Some(tpl)) if !tpl.trim().is_empty() => {
                    Some(template::render(&tpl, &template_vars(name, text))?)
                }
                _ => None,
            },
        )
    }

    /// Preview what [`Store::create`] would store for `name` + `content`: the
    /// rendered template body when a `.pass-template` applies, or `None` when no
    /// template applies (in which case `content` is stored verbatim). Writes
    /// nothing — used by the UI to show what a template will produce before save.
    ///
    /// `content` becomes the template's `.Content`, exactly as in [`create`].
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` for a bad name, or `TemplateError` if a
    /// template references an unknown variable.
    pub async fn preview_create(
        &self,
        name: &str,
        content: &[u8],
    ) -> Result<Option<String>, Error> {
        validate_secret_name(name)?;
        self.resolve_template(name, content).await
    }

    /// Create a secret from one of the built-in presets (gopass `gopass create`
    /// wizard). `fields` maps each preset field key to its value; the `password`
    /// field becomes the secret's first line and the rest become `key: value`
    /// body lines. The secret is generated at `<prefix>/<name-from-fields>`.
    ///
    /// # Errors
    ///
    /// Returns `InvalidEntryName` if the preset is unknown or a required field
    /// is missing, or whatever [`Store::create`] returns.
    pub async fn create_from_preset<S: BuildHasher>(
        &self,
        preset_id: &str,
        fields: &HashMap<&str, String, S>,
    ) -> Result<WriteResult, Error> {
        let preset = template::find_preset(preset_id).ok_or_else(|| {
            Error::new(
                ErrorCode::InvalidEntryName,
                format!("unknown create preset: {preset_id:?}"),
            )
        })?;
        let name = template::preset_name(preset, fields)?;
        let body = template::preset_body(preset, fields)?;
        self.create(&name, &body).await
    }

    /// Commit `passfile` (the caller has already mutated the worktree) locally,
    /// with **no push**. `kind` is `Add` for a save or `Remove` for a delete.
    /// This is the local-only commit half shared by the local-only write
    /// primitives ([`Store::set`] / [`Store::delete`]).
    async fn commit_local(
        &self,
        rcs: &RcsCtx,
        kind: CommitKind,
        passfile: String,
        message: String,
    ) -> Result<String, Error> {
        self.storage()?
            .commit(&rcs.ctx(), kind, &[passfile], &message)
            .await
    }

    /// Encrypt `plaintext` to the store recipients (ensuring our own key is
    /// included) and write it to `<name>.age` atomically. Returns the passfile
    /// path relative to the repo root.
    async fn encrypt_and_write(&self, name: &str, plaintext: &[u8]) -> Result<String, Error> {
        let passfile = passfile_rel(name, self.secret_ext()?);

        // Encrypt to the store's recipients plus our own key (ensureOurKeyID),
        // reading the index through a view bound to the storage backend — the
        // backend owns recipient resolution + the encrypt step, and the
        // recipients liveness guard now runs behind the same backend (no
        // per-op `local_path` vs owned-root gap on the guard).
        let identity_bytes = self.get_identity_bytes().await?;
        let storage = self.storage()?;
        let view = RepoFiles::new(&*storage);
        let ciphertext = self
            .crypto()?
            .encrypt(plaintext, &identity_bytes, &view)
            .await?;

        storage.set(&passfile, &ciphertext).await?;
        Ok(passfile)
    }

    /// One page of revisions for `name` — the commits (newest first) that
    /// touched it, each with verification status. The per-secret history view
    /// (R027). `base_oid` anchors pagination: page 0 passes `None` (captures
    /// HEAD); later pages pass the prior page's `base_oid` so a background
    /// fast-forward can't drift the window.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn list_revisions(
        &self,
        name: &str,
        offset: usize,
        limit: usize,
        base_oid: Option<&str>,
    ) -> Result<signing::RevisionPage, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let passfile = passfile_rel(name, self.secret_ext()?);
        let base_owned = base_oid.map(str::to_string);
        spawn_blocking(move || {
            signing::list_path_signatures_at(
                &repo_path,
                &passfile,
                offset,
                limit,
                base_owned.as_deref(),
                &trusted,
                &ignored,
            )
        })
        .await?
    }

    /// Read one past revision of `name` at `commit_oid` (a full oid from a
    /// revision listing) and decrypt it with the current identity (R027). The
    /// outcome distinguishes a decryptable past value from a revision the current
    /// identity can't decrypt (recipient rotation / a teammate's revision) and
    /// from a revision that deleted the entry. Ciphertext never leaves this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo can't be opened, the entry path is invalid,
    /// or the identity is encrypted but not unlocked (so the caller can prompt);
    /// a decrypt failure is `Ok(RevisionContent::Undecryptable)`, not an error.
    pub async fn get_at_revision(
        &self,
        name: &str,
        commit_oid: &str,
    ) -> Result<RevisionContent, Error> {
        let passfile = passfile_rel(name, self.secret_ext()?);

        let encrypted = self
            .storage()?
            .blob_at_revision(&passfile, commit_oid)
            .await?;
        let Some(encrypted) = encrypted else {
            return Ok(RevisionContent::Deleted);
        };

        let identity_bytes = self.get_identity_bytes().await?;
        let crypto = self.crypto()?;
        let Ok(decrypted) = crypto.decrypt(&encrypted, &identity_bytes).await else {
            return Ok(RevisionContent::Undecryptable);
        };
        Ok(RevisionContent::Decrypted(Secret::parse(&decrypted)?))
    }
}

/// Build the [`template::TemplateVars`] for an entry named `name` with the
/// given content text. All name-derived slices borrow `name`.
fn template_vars<'a>(name: &'a str, content: &'a str) -> template::TemplateVars<'a> {
    let base = name.rfind('/').map_or(name, |i| &name[i + 1..]);
    let dir = name.rfind('/').map_or("", |i| &name[..i]);
    let dirname = dir.rfind('/').map_or(dir, |i| &dir[i + 1..]);
    template::TemplateVars {
        content,
        name: base,
        path: name,
        dir,
        dirname,
    }
}

/// Validate a secret name before writing (gopass `ValidateSecretName`).
///
/// Rejects empty/whitespace names, leading or trailing `/`, empty segments
/// (`//`), backslashes, NUL and other control characters, and `.`/`..` path
/// segments. This is the front-line path-traversal guard; [`assert_within_repo`]
/// is the defense-in-depth backstop.
fn validate_secret_name(name: &str) -> Result<(), Error> {
    if name.trim().is_empty() {
        return Err(invalid_name("Secret name must not be empty"));
    }
    if name.starts_with('/') || name.ends_with('/') {
        return Err(invalid_name("Secret name must not start or end with '/'"));
    }
    if name.contains("//") {
        return Err(invalid_name(
            "Secret name must not contain empty path segments",
        ));
    }
    if name.contains('\\') || name.contains('\0') {
        return Err(invalid_name(
            "Secret name must not contain backslashes or NUL bytes",
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid_name(
            "Secret name must not contain control characters",
        ));
    }
    if name.split('/').any(|seg| seg == ".." || seg == ".") {
        return Err(invalid_name(
            "Secret name must not contain '.' or '..' segments",
        ));
    }
    Ok(())
}

/// Build an `InvalidEntryName` error (keeps call sites terse).
fn invalid_name(message: &str) -> Error {
    Error::new(ErrorCode::InvalidEntryName, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::SecretExt;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn resolve_entry_path_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("cloud");
        std::fs::create_dir_all(&file_path).unwrap();
        std::fs::write(file_path.join("aws.age"), b"encrypted").unwrap();

        let result = resolve_entry_path(dir.path(), "cloud/aws.age");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), dir.path().join("cloud/aws.age"));
    }

    #[test]
    fn resolve_entry_path_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "nonexistent.age");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "../../../etc/passwd");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
    }

    #[test]
    fn resolve_entry_path_traversal_deep() {
        let dir = tempfile::tempdir().unwrap();
        let result = resolve_entry_path(dir.path(), "foo/../../bar/../../../etc");
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "ENTRY_NOT_FOUND");
    }

    #[test]
    #[cfg(unix)]
    fn resolve_entry_path_symlink_escape() {
        let external_dir = tempfile::tempdir().unwrap();
        let external_file = external_dir.path().join("target.txt");
        std::fs::write(&external_file, b"external-secret").unwrap();

        let repo_dir = tempfile::tempdir().unwrap();
        let link_path = repo_dir.path().join("escape.age");
        symlink(&external_file, &link_path).unwrap();

        let result = resolve_entry_path(repo_dir.path(), "escape.age");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code, "ENTRY_NOT_FOUND");
        assert!(err.message.contains("outside repository"));
    }

    #[test]
    fn list_entries_nonexistent_dir() {
        let missing = PathBuf::from("/tmp/gpm_no_such_dir_12345");
        assert!(!missing.exists());
        let result = list_entries(&missing, SecretExt::AGE);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, "NO_REPO");
    }

    // validate_secret_name is the name-side sanitizer (resolve_entry_path in
    // storage::git is the separate filesystem-side guard). Covers every branch.
    #[test]
    fn validate_secret_name_accepts_valid_names() {
        assert!(validate_secret_name("cloud/aws/root").is_ok());
        assert!(validate_secret_name("a").is_ok());
        // a '.' inside a segment (not a standalone '.'/..' segment) is allowed
        assert!(validate_secret_name("websites/github.com/alice").is_ok());
    }

    #[test]
    fn validate_secret_name_rejects_empty() {
        assert!(validate_secret_name("").is_err());
        assert!(validate_secret_name("   ").is_err());
    }

    #[test]
    fn validate_secret_name_rejects_leading_trailing_slash() {
        assert!(validate_secret_name("/foo").is_err());
        assert!(validate_secret_name("foo/").is_err());
    }

    #[test]
    fn validate_secret_name_rejects_empty_segments() {
        assert!(validate_secret_name("foo//bar").is_err());
    }

    #[test]
    fn validate_secret_name_rejects_backslash_and_nul() {
        assert!(validate_secret_name("foo\\bar").is_err());
        assert!(validate_secret_name("foo\0bar").is_err());
    }

    #[test]
    fn validate_secret_name_rejects_control_chars() {
        assert!(validate_secret_name("foo\x01bar").is_err());
        assert!(validate_secret_name("foo\nbar").is_err());
    }

    #[test]
    fn validate_secret_name_rejects_dot_segments() {
        assert!(validate_secret_name(".").is_err());
        assert!(validate_secret_name("..").is_err());
        assert!(validate_secret_name("foo/..").is_err());
        assert!(validate_secret_name("foo/.").is_err());
        assert!(validate_secret_name("../foo").is_err());
    }
}
