// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::str;

use tokio::fs;
use zeroize::Zeroizing;

use crate::crypto::{CryptoBackend, GPG_PUBLIC_KEYS_DIR, GPG_RECIPIENTS_FILE, GpgBackend};
use crate::error::{Error, ErrorCode};
use crate::identity::{self, validate_identity_format};
use crate::recipient::serialize_recipients;
use crate::signing::AuthenticityConfig;
use crate::storage::{CancelToken, CommitKind, GitAuth, ProgressSender, StorageCtx};

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

    /// Derive the gopass seed material — the recipient token (`0x` + last 16 hex
    /// of the primary fingerprint, gopass's `Key.ID()`) and the armored public
    /// key — from an imported GPG secret key. Both are PUBLIC-packet data, so no
    /// passphrase is needed (S2K only guards secret-key material). rpgp parses
    /// attacker-controllable bytes, so the parse runs on a blocking thread inside
    /// `catch_unwind` (mirroring `gpg_identity_preview`): a panic on malformed
    /// armor surfaces as `InvalidIdentity`, never a runtime-unwinding `JoinError`.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidIdentity`] if the armor is unparseable; [`ErrorCode::StoreError`]
    /// if the blocking task fails to join.
    async fn derive_gpg_seed_material(identity: &str) -> Result<(String, String), Error> {
        // Zeroizing: the imported armor is secret-key material — keep it
        // wipe-on-drop, matching the `PendingIdentity`/`save_identity` discipline.
        let identity_owned = Zeroizing::new(identity.to_string());
        tokio::task::spawn_blocking(move || {
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let sk =
                    crate::crypto::openpgp::parse_armored_secret_key(identity_owned.as_bytes())?;
                let pubkey = crate::crypto::openpgp::armor_public_key(&sk.to_public_key())?;
                let token = GpgBackend.identity_recipient(identity_owned.as_str(), None)?;
                Ok::<_, Error>((token, pubkey))
            }));
            match parsed {
                Ok(Ok(vals)) => Ok(vals),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "GPG key armor could not be parsed (malformed)",
                )),
            }
        })
        .await
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("blocking task join: {e}")))?
    }

    /// Create a brand-new gopass-compatible GPG/OpenPGP store on device — the
    /// GPG counterpart to [`create_store`](Store::create_store). The user imports
    /// a single existing GPG secret key (`identity`); its recipient token seeds
    /// `.gpg-id` and its armored public key seeds `.public-keys/<token>`.
    ///
    /// Mirrors gopass `init` (GPG/gitfs): `git init`, set the `diff.gpg`
    /// diff-driver config, commit `.gitattributes`, seed `.gpg-id` +
    /// `.public-keys/<token>`, commit "Add current content of password store",
    /// and — when `repo_url` is given — record an `origin` remote. Provisioning
    /// derives only PUBLIC seed material (token + armored pubkey) from the
    /// imported secret; the secret itself is NOT persisted here — that is
    /// [`save_identity`](Store::save_identity)'s job (the create flow calls
    /// `complete_setup_from_file` afterwards).
    ///
    /// **No push.** The first push is a separate step ([`Store::push`]),
    /// performed only after both the repo config and the identity are durable —
    /// so the remote can never receive a store whose recipient's identity has
    /// been lost locally (the orphan-recipient hole). Mirrors [`create_store`].
    ///
    /// Auth is ignored when no `repo_url` is given, so a stray credential can
    /// never be persisted against an empty URL.
    ///
    /// On any failure after `git init`, the partial repo directory and any
    /// config are removed so the next attempt starts clean.
    ///
    /// **Known limitation (shared with the open-existing flow):** a hardware /
    /// OpenPGP-card key with no usable secret material is expected to be rejected
    /// by the preceding verify step (`validate_identity_passphrase`); this method
    /// does not re-check for it, since the public seed material it derives does
    /// not need the secret.
    ///
    /// # Errors
    ///
    /// Returns `InvalidIdentity` if `identity` is empty or unparseable, or a
    /// git/IO error if initialization, the config writes, the recipients writes,
    /// the commits, the remote add, or config persistence fails.
    pub async fn create_gpg_store(
        &self,
        repo_url: Option<&str>,
        auth: &GitAuth,
        identity: &str,
    ) -> Result<(), Error> {
        if identity.trim().is_empty() {
            return Err(Error::new(
                ErrorCode::InvalidIdentity,
                "Identity must not be empty",
            ));
        }

        // Derive the public seed material (recipient token + armored pubkey) from
        // the imported secret — passphrase-free, panic-isolated on a blocking
        // thread. See [`Self::derive_gpg_seed_material`].
        let (recipient_token, pubkey_armor) = Self::derive_gpg_seed_material(identity).await?;

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
        // failure-cleanup order below.
        if repo_dir.exists() {
            fs::remove_dir_all(&repo_dir).await?;
        }
        self.config.clear_all().await?;

        let bootstrap = async {
            self.resolve_and_set(Some("git"), &repo_dir.to_string_lossy())?;
            self.resolve_and_set_crypto(Some("gpg"))?;
            self.storage()?.init_repo(&repo_dir).await?;

            // gopass's `fixConfig`: record the diff-driver config that the
            // committed `.gitattributes` (`*.gpg diff=gpg`) references. This is
            // per-working-copy (`.git/config`); a desktop gopass re-creates it on
            // clone via its own `fixConfig`, so it only affects gpm's own
            // checkout (which never runs `git diff`). gpm sets it for
            // gopass-faithfulness with the `.gitattributes` it pairs with.
            self.storage()?
                .set_config(&repo_dir, "diff.gpg.binary", "true")
                .await?;
            self.storage()?
                .set_config(&repo_dir, "diff.gpg.textconv", "gpg --no-tty --decrypt")
                .await?;

            // Commit 1 (no parent): `.gitattributes` (gopass's
            // "Configure git repository for gpg file diff.").
            self.storage()?
                .write_file_atomic(&repo_dir, ".gitattributes", b"*.gpg diff=gpg\n")
                .await?;
            self.storage()?
                .commit_initial(
                    &repo_dir,
                    &[".gitattributes".to_string()],
                    "Configure git repository for gpg file diff.",
                )
                .await?;

            // Seed `.gpg-id` (the recipient token) + `.public-keys/<token>` (the
            // armored public key, raw — NOT `serialize_recipients`, which would
            // mangle the multi-line armor block). `write_file_atomic` creates the
            // `.public-keys/` subdir implicitly.
            let id_bytes = serialize_recipients(std::slice::from_ref(&recipient_token));
            self.storage()?
                .write_file_atomic(&repo_dir, GPG_RECIPIENTS_FILE, &id_bytes)
                .await?;
            let pubkey_rel = format!("{GPG_PUBLIC_KEYS_DIR}/{recipient_token}");
            self.storage()?
                .write_file_atomic(&repo_dir, &pubkey_rel, pubkey_armor.as_bytes())
                .await?;

            // Commit 2 (with parent): `.gpg-id` + `.public-keys/<token>` (gopass's
            // "Add current content of password store"). The parented `commit`
            // takes a `StorageCtx`; build a default one (no RepoConfig saved yet)
            // — `commit_name: None` → the app-default identity, matching commit
            // 1. Do NOT call `commit_initial` here: it hardcodes no-parent and
            // would orphan commit 1.
            let default_policy = AuthenticityConfig::default();
            let ctx = StorageCtx {
                repo_path: &repo_dir,
                auth,
                policy: &default_policy,
                commit_name: None,
                commit_email: None,
            };
            self.storage()?
                .commit(
                    &ctx,
                    CommitKind::Add,
                    &[GPG_RECIPIENTS_FILE.to_string(), pubkey_rel.clone()],
                    "Add current content of password store",
                )
                .await?;

            if has_url {
                self.storage()?.remote_add(&repo_dir, "origin", url).await?;
            }

            let local_path = repo_dir.to_string_lossy().to_string();
            self.config
                .save_repo_config_with_crypto(url, auth, &local_path, Some("gpg"))
                .await?;
            Ok::<(), Error>(())
        };

        if let Err(e) = bootstrap.await {
            // Best-effort cleanup: a partial repo dir or half-written config must
            // not leave the store looking initialized. Cleanup failures are
            // swallowed (the bootstrap error `e` is what we return) — log them.
            if let Err(cleanup) = fs::remove_dir_all(&repo_dir).await {
                log::warn!("create-gpg-store: partial-repo cleanup failed: {cleanup}");
            }
            if let Err(cleanup) = self.config.clear_all().await {
                log::warn!("create-gpg-store: config clear-all cleanup failed: {cleanup}");
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

    /// Preview a GPG/OpenPGP secret key's public metadata for the setup pick
    /// step: primary user id, full fingerprint, the gopass recipient id
    /// (`0x`+16hex), and a membership probe against the cloned store. All
    /// public-packet data — no passphrase needed — so the UI can show
    /// "<uid> (<fingerprint>)" plus a membership badge before the passphrase
    /// prompt. `is_recipient` is `None` when there is no store or no recipients
    /// to match against. Does NOT mutate the store or resolve a backend; the
    /// authoritative membership gate + crypto-kind persist run in
    /// [`Store::save_identity`].
    ///
    /// # Errors
    ///
    /// [`ErrorCode::InvalidIdentity`] if the armor is unparseable;
    /// [`ErrorCode::StoreError`]/I/O if the recipient pool can't be read for the
    /// membership probe.
    pub async fn gpg_identity_preview(&self, identity: &str) -> Result<GpgIdentityPreview, Error> {
        // uid + fingerprint are public-packet data — parse off the async thread.
        // rpgp parses attacker-controllable picked bytes, so `catch_unwind` the
        // parse (the codebase convention — see `identity_recipient`): a panic on
        // malformed armor surfaces as `InvalidIdentity` (the contract the docstring
        // promises + the UI's "invalid identity" branch keys on), not a silent
        // `StoreError` from the `JoinError`.
        let identity_bytes = identity.as_bytes().to_vec();
        let (user_id, fingerprint) = tokio::task::spawn_blocking(move || {
            let parsed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let sk = crate::crypto::openpgp::parse_armored_secret_key(&identity_bytes)?;
                let fp = crate::crypto::openpgp::primary_fingerprint(&sk.to_public_key());
                let uid = crate::crypto::openpgp::primary_user_id(&sk);
                Ok::<_, Error>((uid, fp))
            }));
            match parsed {
                Ok(Ok(vals)) => Ok(vals),
                Ok(Err(e)) => Err(e),
                Err(_) => Err(Error::new(
                    ErrorCode::InvalidIdentity,
                    "GPG key armor could not be parsed (malformed)",
                )),
            }
        })
        .await??;

        let recipient = GpgBackend.identity_recipient(identity, None)?;

        let is_recipient = match self.config.load_repo_config().await {
            Ok(rc) => {
                self.probe_membership(&rc, identity, Some("gpg"), None)
                    .await?
            }
            Err(e) if e.code == "NO_REPO" => None,
            Err(e) => return Err(e),
        };

        Ok(GpgIdentityPreview {
            user_id,
            fingerprint,
            recipient,
            is_recipient,
        })
    }
}

/// Public metadata for a picked GPG/OpenPGP secret key, returned by
/// [`Store::gpg_identity_preview`] so the setup pick panel can display the key
/// (uid + fingerprint + membership) before asking for its S2K passphrase.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GpgIdentityPreview {
    /// The key's primary user id (e.g. `Jordan <jordan@example.com>`), if any.
    pub user_id: Option<String>,
    /// The key's full primary fingerprint.
    pub fingerprint: String,
    /// The gopass recipient id (`0x` + last 16 hex of the fingerprint).
    pub recipient: String,
    /// `Some(true)`/`Some(false)` if membership against the cloned store's
    /// recipients was definitively determined; `None` if there is no store or no
    /// recipients to match against. The authoritative gate is `save_identity`.
    pub is_recipient: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `gpg_identity_preview` parses a generated GPG key's public metadata:
    /// primary user id, the full fingerprint, and the gopass recipient id
    /// (`0x` + last 16 hex of the fingerprint). With no store configured, the
    /// membership probe is `None` (nothing to match against).
    #[tokio::test]
    async fn gpg_identity_preview_parses_public_metadata() {
        let uid = "preview user <preview@gpm.local>";
        let (sk, _pk) = crate::crypto::openpgp::generate_keypair(uid, None).expect("keygen");
        let armor = crate::crypto::openpgp::armor_secret_key(&sk).expect("armor");
        // No repo configured — preview still parses; is_recipient is None.
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        let preview = store
            .gpg_identity_preview(&armor)
            .await
            .expect("parse a generated GPG key");
        assert_eq!(preview.user_id.as_deref(), Some(uid));
        assert!(
            !preview.fingerprint.is_empty(),
            "fingerprint must be derived"
        );
        assert!(
            preview.recipient.starts_with("0x") && preview.recipient.len() == 2 + 16,
            "recipient is gopass Key.ID() (0x + 16 hex), got {}",
            preview.recipient
        );
        assert_eq!(
            preview.recipient,
            format!("0x{}", &preview.fingerprint[24..]),
            "recipient is the last 16 hex of the fingerprint"
        );
        assert_eq!(
            preview.is_recipient, None,
            "no store → membership undetermined"
        );
    }

    /// The parse-isolation contract on `gpg_identity_preview`: malformed armor (a
    /// truncated PGP block) surfaces as `InvalidIdentity`, never a panic crossing
    /// `spawn_blocking` (rpgp panics on crafted packets). The wrap returns
    /// `InvalidIdentity` for BOTH a parse error (`Ok(Err)`) and a panic (`Err`) —
    /// so this asserts the user-facing contract regardless of which rpgp takes.
    #[tokio::test]
    async fn gpg_identity_preview_malformed_armor_is_invalid_identity() {
        let config_dir = tempfile::tempdir().unwrap();
        let store = Store::new(config_dir.path().to_path_buf(), None);
        let malformed = "-----BEGIN PGP PRIVATE KEY BLOCK-----\n\ntruncated garbage";
        let err = store
            .gpg_identity_preview(malformed)
            .await
            .expect_err("malformed armor must error, not panic");
        assert_eq!(
            err.code, "INVALID_IDENTITY",
            "malformed armor surfaces as InvalidIdentity, not a JoinError/StoreError"
        );
    }
}
