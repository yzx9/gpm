// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
// SPDX-License-Identifier: MIT OR Apache-2.0

use std::str;

use tokio::task::spawn_blocking;

use crate::config::{self, RepoConfig, UpdateOutcome};
use crate::crypto::openpgp;
use crate::error::{Error, ErrorCode};
use crate::signing::{
    self, AuthenticityConfig, CommitSigInfo, CommitSigStatus, TrustedGpgKey, TrustedKey, VerifyMode,
};

// Impl-split submodule: mod.rs is the shared scope for Store's split impl, so a
// super-glob is the idiomatic import (pedantic flags it; scoped allow).
#[allow(clippy::wildcard_imports)]
use super::*;

impl Store {
    /// Load the persisted authenticity config (the `authenticity` field of
    /// `repo.json`). Defaults to Off / empty when the repo isn't configured
    /// yet — pre-setup there is nothing to verify.
    ///
    /// # Errors
    ///
    /// Returns an error if `repo.json` exists but cannot be read or parsed.
    pub async fn authenticity_config(&self) -> Result<AuthenticityConfig, Error> {
        match self.config.load_repo_config().await {
            Ok(rc) => Ok(rc.authenticity),
            // No repo configured yet → authenticity is trivially Off.
            Err(e) if e.code == "NO_REPO" => Ok(AuthenticityConfig::default()),
            Err(e) => Err(e),
        }
    }

    /// Set the verification mode. Refuses [`VerifyMode::Enforce`] when no
    /// trusted key (SSH **or** GPG) is recorded yet (Enforce with zero keys
    /// would block every pull). Returns the effective stored mode.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigError`] if Enforce is requested with no
    /// trusted keys, or the config cannot be persisted.
    pub async fn set_verification_mode(&self, mode: VerifyMode) -> Result<VerifyMode, Error> {
        self.config
            .update_repo_config(|rc| {
                if mode == VerifyMode::Enforce && !rc.authenticity.has_any_trusted_key() {
                    return Err(Error::new(
                        ErrorCode::ConfigError,
                        "Add a trusted signing key before enabling Enforce.",
                    ));
                }
                rc.authenticity.mode = mode;
                Ok(UpdateOutcome::Changed(rc.authenticity.mode))
            })
            .await
    }

    /// Set the git commit author identity. `None` (or blank) for a field clears
    /// it, reverting to the app default so the value keeps tracking future
    /// shipped defaults. Values are trimmed; characters that would corrupt a
    /// commit (`<`, `>`, control bytes) are rejected. Returns the persisted
    /// [`RepoConfig`].
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ConfigError`] if a value contains an invalid
    /// character, or if the config cannot be loaded or persisted.
    pub async fn set_commit_identity(
        &self,
        name: Option<String>,
        email: Option<String>,
    ) -> Result<RepoConfig, Error> {
        let normalize = |v: Option<String>| -> Result<Option<String>, Error> {
            let Some(s) = v else {
                return Ok(None);
            };
            let t = s.trim();
            if t.is_empty() {
                return Ok(None);
            }
            // Reject characters that corrupt the commit's `Name <email>` line:
            // control bytes (newline, NUL, …) and the envelope delimiters. The
            // `git` CLI rejects these for user.name/user.email; libgit2's
            // `Signature::now` validates nothing, so gpm must.
            if let Some(c) = t.chars().find(|&c| c.is_control() || c == '<' || c == '>') {
                return Err(Error::new(
                    ErrorCode::ConfigError,
                    format!(
                        "Commit identity contains an invalid character ({c:?}). Newlines, \
                         angle brackets, and control characters corrupt git commits."
                    ),
                ));
            }
            Ok(Some(t.to_string()))
        };
        let name = normalize(name)?;
        let email = normalize(email)?;
        self.config
            .update_repo_config(|rc| {
                rc.commit_user_name.clone_from(&name);
                rc.commit_user_email.clone_from(&email);
                Ok(UpdateOutcome::Changed(rc.clone()))
            })
            .await
    }

    /// The default commit author identity, for UI display. Reads the shipped
    /// default so the frontend never hardcodes it.
    #[must_use]
    pub fn commit_identity_default() -> CommitIdentity {
        CommitIdentity {
            name: config::DEFAULT_COMMIT_NAME.to_string(),
            email: config::DEFAULT_COMMIT_EMAIL.to_string(),
        }
    }

    /// Add a trusted signing public key. Validates the key, derives its
    /// fingerprint, and dedupes — if a key with the same fingerprint is already
    /// trusted, the existing entry is returned unchanged (idempotent).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the public key is not a
    /// parseable OpenSSH key, or the config cannot be persisted.
    pub async fn add_trusted_key(
        &self,
        public_key: &str,
        label: &str,
    ) -> Result<TrustedKey, Error> {
        let fingerprint = signing::fingerprint_of_public_key(public_key)?;
        let head = self.current_head_hash().await.unwrap_or_default();

        self.config
            .update_repo_config(|rc| {
                // Idempotent: an already-trusted fingerprint returns the
                // existing entry WITHOUT a config write.
                if let Some(existing) = rc
                    .authenticity
                    .trusted_keys
                    .iter()
                    .find(|k| k.fingerprint == fingerprint)
                    .cloned()
                {
                    return Ok(UpdateOutcome::Unchanged(existing));
                }
                let key = TrustedKey {
                    public_key: public_key.trim().to_string(),
                    fingerprint,
                    label: label.to_string(),
                    added_at_commit: head,
                };
                rc.authenticity.trusted_keys.push(key.clone());
                Ok(UpdateOutcome::Changed(key))
            })
            .await
    }

    /// Remove a trusted signing key by fingerprint. Removing the last trusted
    /// key of either kind (SSH or GPG) while in Enforce downgrades to Audit
    /// (Enforce with zero keys would block everything).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be persisted.
    pub async fn remove_trusted_key(&self, fingerprint: &str) -> Result<(), Error> {
        self.config
            .update_repo_config(|rc| {
                rc.authenticity
                    .trusted_keys
                    .retain(|k| k.fingerprint != fingerprint);
                if !rc.authenticity.has_any_trusted_key()
                    && rc.authenticity.mode == VerifyMode::Enforce
                {
                    rc.authenticity.mode = VerifyMode::Audit;
                }
                Ok(UpdateOutcome::Changed(()))
            })
            .await
    }

    /// Add a trusted GPG/OpenPGP public key (RFC 0009). Parses the armored
    /// block, derives the primary-key fingerprint, and dedupes — if a key with
    /// the same primary fingerprint is already trusted, the existing entry is
    /// returned unchanged (idempotent).
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the armor is unparseable or its
    /// self-signatures do not validate, or an error if the config cannot be
    /// persisted.
    pub async fn add_trusted_gpg_key(
        &self,
        armored_public_key: &str,
        label: &str,
    ) -> Result<TrustedGpgKey, Error> {
        // Bound the input before rpgp parses it — a mis-pasted/mis-picked
        // multi-MB blob is rejected with the same "not a usable GPG key" error
        // whether it arrives via paste or file import.
        if armored_public_key.len() > crate::MAX_GPG_KEY_FILE_BYTES {
            return Err(Error::new(
                ErrorCode::SshKeyInvalid,
                format!(
                    "GPG public key too large ({} bytes; limit {} bytes) — not an armored public key.",
                    armored_public_key.len(),
                    crate::MAX_GPG_KEY_FILE_BYTES
                ),
            ));
        }
        let key = openpgp::parse_armored_public_key(armored_public_key)?;
        let fingerprint = openpgp::primary_fingerprint(&key);
        let head = self.current_head_hash().await.unwrap_or_default();

        self.config
            .update_repo_config(|rc| {
                // Idempotent: an already-trusted fingerprint returns the
                // existing entry WITHOUT a config write.
                if let Some(existing) = rc
                    .authenticity
                    .trusted_gpg_keys
                    .iter()
                    .find(|k| k.fingerprint == fingerprint)
                    .cloned()
                {
                    return Ok(UpdateOutcome::Unchanged(existing));
                }
                let entry = TrustedGpgKey {
                    armored_public_key: armored_public_key.trim().to_string(),
                    fingerprint,
                    label: label.to_string(),
                    added_at_commit: head,
                };
                rc.authenticity.trusted_gpg_keys.push(entry.clone());
                Ok(UpdateOutcome::Changed(entry))
            })
            .await
    }

    /// Remove a trusted GPG key by primary fingerprint. Removing the last
    /// trusted key of either kind (SSH or GPG) while in Enforce downgrades to
    /// Audit (Enforce with zero keys would block everything).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be persisted.
    pub async fn remove_trusted_gpg_key(&self, fingerprint: &str) -> Result<(), Error> {
        self.config
            .update_repo_config(|rc| {
                rc.authenticity
                    .trusted_gpg_keys
                    .retain(|k| k.fingerprint != fingerprint);
                if !rc.authenticity.has_any_trusted_key()
                    && rc.authenticity.mode == VerifyMode::Enforce
                {
                    rc.authenticity.mode = VerifyMode::Audit;
                }
                Ok(UpdateOutcome::Changed(()))
            })
            .await
    }

    /// The per-key parse warnings for the persisted trusted GPG keys — one
    /// human-readable string per entry that failed to re-parse. A trusted key
    /// that later breaks must not silently downgrade commits to
    /// `UnverifiedSignature`; the Settings card surfaces these so the user can
    /// re-add or remove the broken entry. Settings-load frequency only — the
    /// per-commit verifier path uses `TrustSet::from_config` (separate).
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be read.
    pub async fn gpg_key_parse_warnings(&self) -> Result<Vec<String>, Error> {
        let rc = self.config.load_repo_config().await?;
        let armored = rc
            .authenticity
            .trusted_gpg_keys
            .iter()
            .map(|k| k.armored_public_key.as_str());
        let (_keys, warnings) = openpgp::parse_trusted_keys(armored);
        Ok(warnings)
    }

    /// Record a per-commit ignore, scoped to this commit + its **current**
    /// status. The status is recomputed server-side (the caller passes only the
    /// hash), so the recorded `IgnoredIssue.status` always matches what
    /// `verify_range` will later compute — keeping the per-status ignore match
    /// stable. Idempotent.
    ///
    /// No-op (still Ok) for a commit whose status is not an issue (e.g.
    /// `Verified`) — there is nothing to ignore.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit hash is invalid, the repo cannot be
    /// opened, or the config cannot be persisted.
    pub async fn ignore_commit_issue(&self, commit: &str) -> Result<CommitSigInfo, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();

        // Derive the full CommitSigInfo once (a single signature verify). Its
        // status drives the is-issue check, and its metadata is returned to the
        // caller so the UI can refresh the row in place without a second IPC
        // (no write-then-re-read window).
        let commit_owned = commit.to_string();
        let repo_path_for_info = repo_path.clone();
        let info = spawn_blocking(move || {
            signing::commit_sig_info_at(&repo_path_for_info, &commit_owned, &trusted, &ignored)
        })
        .await??;

        // Record the ignore for a real issue (idempotent). A newly-written entry
        // means this commit is now ignored, so flip the returned flag.
        if info.status.is_issue() {
            let already = rc
                .authenticity
                .ignored
                .iter()
                .any(|i| i.commit == info.hash && i.status == info.status);
            if !already {
                let head = self.current_head_hash().await.unwrap_or_default();
                // Store the full resolved hash (`info.hash`), not the raw caller
                // input — `is_ignored` matches on the full OID, so a short hash or
                // revspec input would otherwise persist an entry that never matches
                // future verification.
                let hash = info.hash.clone();
                let status = info.status.clone();
                self.config
                    .update_repo_config(move |rc| {
                        // Re-check idempotence on the FRESH in-lock snapshot —
                        // a concurrent ignore of the same commit may have landed
                        // between the outer read and here.
                        let already = rc
                            .authenticity
                            .ignored
                            .iter()
                            .any(|i| i.commit == hash && i.status == status);
                        if already {
                            return Ok(UpdateOutcome::Unchanged(()));
                        }
                        rc.authenticity.ignored.push(signing::IgnoredIssue {
                            commit: hash,
                            status,
                            ignored_at_commit: head,
                        });
                        Ok(UpdateOutcome::Changed(()))
                    })
                    .await?;
                return Ok(CommitSigInfo {
                    ignored: true,
                    ..info
                });
            }
        }
        Ok(info)
    }

    /// The verification status of the current HEAD commit (cheap; cached
    /// config, single commit verify). Used by the indicator badge.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn head_signature_status(&self) -> Result<CommitSigStatus, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        spawn_blocking(move || signing::head_status_at(&repo_path, &trusted)).await?
    }

    /// The OpenSSH public key of HEAD's SSH-signature signer (for the
    /// "trust this signer" TOFU flow), or `None` if HEAD is unsigned or not
    /// SSH-signed.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn head_signer_public_key(&self) -> Result<Option<String>, Error> {
        let repo_path = self.repo_path().await?;
        spawn_blocking(move || signing::head_signer_public_key_at(&repo_path)).await?
    }

    /// Trust the SSH-signature signer of a specific commit ("trust this
    /// signer" TOFU from the history detail view). Errors if the commit is
    /// unsigned or not SSH-signed.
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::SshKeyInvalid`] if the commit has no SSH signer,
    /// or [`ErrorCode::SshKeyInvalid`] if the public key is invalid.
    pub async fn trust_commit_signer(
        &self,
        commit_hash: &str,
        label: &str,
    ) -> Result<TrustedKey, Error> {
        let repo_path = self.repo_path().await?;
        let hash_owned = commit_hash.to_string();
        let public_key =
            spawn_blocking(move || signing::signer_public_key_at(&repo_path, &hash_owned))
                .await??;
        let public_key = public_key.ok_or_else(|| {
            Error::new(
                ErrorCode::SshKeyInvalid,
                "This commit is not signed by an SSH key — nothing to trust.",
            )
        })?;
        self.add_trusted_key(&public_key, label).await
    }

    /// Verify every commit in the half-open range `(from, to]` (newest first)
    /// against the trusted set + ignore list.
    ///
    /// # Errors
    ///
    /// Returns an error if the hashes are invalid, the repo cannot be opened,
    /// or the walk fails.
    pub async fn verify_range(&self, from: &str, to: &str) -> Result<Vec<CommitSigInfo>, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let from_owned = from.to_string();
        let to_owned = to.to_string();
        spawn_blocking(move || {
            signing::verify_range_at(&repo_path, &from_owned, &to_owned, &trusted, &ignored)
        })
        .await?
    }

    /// The `limit` most recent commits (HEAD and ancestors, newest first) with
    /// per-commit verification status. Used by the `/history` screen.
    ///
    /// # Errors
    ///
    /// Returns an error if the repo cannot be opened or HEAD cannot be read.
    pub async fn list_commit_signatures(
        &self,
        offset: usize,
        limit: usize,
    ) -> Result<signing::CommitSigPage, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        spawn_blocking(move || {
            signing::list_commit_signatures_at(&repo_path, offset, limit, &trusted, &ignored)
        })
        .await?
    }

    /// A single commit's metadata + verification status (the `/history` detail
    /// sheet). `commit_hash` may be a full or short hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the hash is invalid, the commit cannot be found,
    /// or its signature cannot be read.
    pub async fn commit_signature(&self, commit_hash: &str) -> Result<CommitSigInfo, Error> {
        let repo_path = self.repo_path().await?;
        let rc = self.config.load_repo_config().await?;
        let trusted = signing::TrustSet::from_config(&rc.authenticity);
        let ignored = rc.authenticity.ignored.clone();
        let hash_owned = commit_hash.to_string();
        spawn_blocking(move || {
            signing::commit_sig_info_at(&repo_path, &hash_owned, &trusted, &ignored)
        })
        .await?
    }
}
