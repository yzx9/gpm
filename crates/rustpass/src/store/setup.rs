// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::str;

use tokio::fs;

use crate::error::{Error, ErrorCode};
use crate::identity::{self, validate_identity_format};
use crate::recipient::serialize_recipients;
use crate::storage::{CancelToken, GitAuth, ProgressSender};

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// The crypto backend's recipients-index filename (`.age-recipients` today).
    fn recipients_file(&self) -> Result<&'static str, Error> {
        Ok(self.crypto()?.profile().recipients_filename)
    }

    /// Clone the repository and save repo config.
    ///
    /// Does **not** save the age identity — that is done via
    /// [`save_identity`](Store::save_identity). Clears any existing
    /// configuration before cloning.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone fails or the config cannot be persisted.
    pub async fn clone_only(&self, repo_url: &str, auth: &GitAuth) -> Result<(), Error> {
        self.clone_only_with(repo_url, auth, None, None).await
    }

    /// Cancellable, progress-reporting variant of [`clone_only`](Store::clone_only).
    ///
    /// `cancel` aborts the in-progress clone (mapped to [`ErrorCode::Cancelled`]
    /// by the storage backend); `progress` receives transfer stats. Both are `None`
    /// on the plain [`clone_only`](Store::clone_only) path, which is used outside
    /// the user-initiated UI clone.
    ///
    /// # Errors
    ///
    /// Returns an error if the clone fails or the config cannot be persisted.
    pub async fn clone_only_with(
        &self,
        repo_url: &str,
        auth: &GitAuth,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
        self.resolve_and_set_crypto(None)?;
        self.storage()?
            .clone_repo(auth, repo_url, &repo_dir, cancel, progress)
            .await?;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, auth, &local_path)
            .await?;

        Ok(())
    }

    /// Create a brand-new gopass-compatible age store on device.
    ///
    /// Mirrors gopass `setup`/`init`: `git init`, seed `.age-recipients` with the
    /// single `recipient`, make the no-parent "Initialized Store" commit, and —
    /// when `repo_url` is given — record an `origin` remote. This is
    /// identity-agnostic: it takes only the public `recipient`, never identity
    /// bytes, so the generated identity is persisted separately via
    /// [`save_identity`](Store::save_identity) (the create flow calls
    /// `complete_setup` afterwards).
    ///
    /// **No push.** The first push is a separate step (`Store::push`), performed
    /// only after both the repo config and the identity are durable — so the
    /// remote can never receive a store whose recipient's identity has been lost
    /// locally (the orphan-recipient hole). If no `repo_url` is given the store
    /// is local-only and never pushed.
    ///
    /// Auth is ignored when no `repo_url` is given, so a stray credential can
    /// never be persisted against an empty URL.
    ///
    /// On any failure after `git init`, the partial repo directory and any
    /// config are removed so the next attempt starts clean.
    ///
    /// # Errors
    ///
    /// Returns `InvalidIdentity` if `recipient` is empty, or a git/IO error if
    /// initialization, the recipients write, the commit, the remote add, or
    /// config persistence fails.
    pub async fn create_store(
        &self,
        repo_url: Option<&str>,
        auth: &GitAuth,
        recipient: &str,
    ) -> Result<(), Error> {
        if recipient.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Recipient must not be empty",
            ));
        }

        // No URL → local-only store: ignore any stray auth (defensive; the
        // frontend also validates), persisting no credentials against an empty
        // URL. `none_auth` is named so the borrow outlives the awaited
        // `bootstrap` block below (a throwaway `&GitAuth::None` would not).
        let has_url = repo_url.is_some_and(|u| !u.trim().is_empty());
        let url = repo_url.unwrap_or("");
        let none_auth = GitAuth::None;
        let auth = if has_url { auth } else { &none_auth };

        let repo_dir = self.config.config_dir().join("repo");
        // Remove the repo dir first, then clear the config — mirroring the
        // failure-cleanup order below. If remove_dir_all fails we leave the
        // prior identity + config intact, rather than deleting the identity
        // while the old repo still sits on disk.
        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }
        self.config.clear_all().await?;

        let bootstrap = async {
            self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
            self.resolve_and_set_crypto(None)?;
            self.storage()?.init_repo(&repo_dir).await?;

            let recipients_bytes = serialize_recipients(&[recipient.to_string()]);
            self.storage()?
                .write_file_atomic(&repo_dir, self.recipients_file()?, &recipients_bytes)
                .await?;

            let message = format!("Initialized Store for {recipient}");
            let rel_paths = vec![self.recipients_file()?.to_string()];
            self.storage()?
                .commit_initial(&repo_dir, &rel_paths, &message)
                .await?;

            if has_url {
                self.storage()?.remote_add(&repo_dir, "origin", url).await?;
            }

            let local_path = repo_dir.to_string_lossy().to_string();
            self.config.save_repo_config(url, auth, &local_path).await?;
            // TODO(0016-recipients-pinning): TOFU-pin the seeded recipient on first write.
            Ok::<(), Error>(())
        };

        if let Err(e) = bootstrap.await {
            // Best-effort cleanup: a partial repo dir or half-written config must
            // not leave the store looking initialized. Cleanup failures are
            // swallowed (the bootstrap error `e` is what we return) — log them.
            if let Err(cleanup) = fs::remove_dir_all(&repo_dir).await {
                log::warn!("create-store: partial-repo cleanup failed: {cleanup}");
            }
            if let Err(cleanup) = self.config.clear_all().await {
                log::warn!("create-store: config clear-all cleanup failed: {cleanup}");
            }
            return Err(e);
        }
        Ok(())
    }

    /// Configure the store: validate identity, clone repo, save config.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the clone fails,
    /// or the config cannot be persisted.
    pub async fn configure(
        &self,
        repo_url: &str,
        auth: &GitAuth,
        identity: &str,
        identity_passphrase: Option<&str>,
    ) -> Result<(), Error> {
        self.configure_with(repo_url, auth, identity, identity_passphrase, None, None)
            .await
    }

    /// Cancellable, progress-reporting variant of [`configure`](Store::configure).
    ///
    /// `cancel` aborts the in-progress clone (mapped to [`ErrorCode::Cancelled`]);
    /// `progress` receives transfer stats. Both are `None` on the plain
    /// [`configure`](Store::configure) path.
    ///
    /// # Errors
    ///
    /// Returns an error if the identity format is invalid, the clone fails,
    /// or the config cannot be persisted.
    pub async fn configure_with(
        &self,
        repo_url: &str,
        auth: &GitAuth,
        identity: &str,
        identity_passphrase: Option<&str>,
        cancel: Option<CancelToken>,
        progress: Option<ProgressSender>,
    ) -> Result<(), Error> {
        // age-keygen writes # comment lines before the key; keep only the key
        // so it is parsed and stored consistently with the paste path.
        let identity = identity::normalize_identity_text(identity);
        let identity_bytes = identity.as_bytes();
        validate_identity_format(identity_bytes)?;

        // A fresh/cloned store uses the age built-in; pin it before the identity
        // validation below touches the crypto backend. (A GPG store has its own
        // setup path; the post-unlock resolve corrects this default.)
        self.resolve_and_set_crypto(None)?;

        // Validate identity can derive a recipient (verifies key is usable)
        let _ = self
            .crypto()?
            .identity_recipient(identity, identity_passphrase)?;

        let repo_dir = self.config.config_dir().join("repo");
        self.config.clear_all().await?;

        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }

        self.config.save_identity(identity_bytes, None).await?;

        self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
        self.storage()?
            .clone_repo(auth, repo_url, &repo_dir, cancel, progress)
            .await?;

        let local_path = repo_dir.to_string_lossy().to_string();
        self.config
            .save_repo_config(repo_url, auth, &local_path)
            .await?;

        Ok(())
    }
}
