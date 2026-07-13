// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

use std::path::{Path, PathBuf};

use serde_json;
use tokio::fs;

use crate::crypto;
use crate::error::{Error, ErrorCode};
use crate::identity::{IdentityType, classify_identity};
use crate::seal::Seal;
use crate::signing::AuthenticityConfig;
use crate::storage::GitAuth;

/// Default commit author name used when none is configured. Single source of
/// the value — read by the commit fallback and surfaced to the UI for display.
pub(crate) const DEFAULT_COMMIT_NAME: &str = "gpm";
/// Default commit author email used when none is configured.
pub(crate) const DEFAULT_COMMIT_EMAIL: &str = "gpm@local";

/// Default seconds a revealed password stays in the DOM before auto-clear.
/// Used when `view_clear_secs` is `None` (the field is absent, e.g. an older
/// config predating the setting).
pub const DEFAULT_VIEW_CLEAR_SECS: u64 = 45;
/// Default seconds the clipboard holds a copied password before auto-clear.
/// Used when `clipboard_clear_secs` is `None`.
pub const DEFAULT_CLIPBOARD_CLEAR_SECS: u64 = 45;

/// How the app auto-locks the identity cache.
///
/// `Immediate` (the default) is the no-cache, per-operation mode: the identity
/// is wiped right after each secret access rather than held for a session. The
/// UI splits this from the hard "lock overlay" transition so a just-revealed
/// password can stay on screen until its own view-clear timer. `Idle(n)` is the
/// classic session model (wipe after `n` seconds of inactivity); `Never` keeps
/// the identity cached until a manual lock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LockMode {
    /// Per-operation: wipe the identity immediately after each secret access.
    /// No idle timer is armed.
    #[default]
    Immediate,
    /// Session: keep the identity cached, wipe after `n` seconds of inactivity.
    Idle(u64),
    /// Never auto-lock; the identity stays cached until a manual lock.
    Never,
}

impl LockMode {
    /// Whether this is the default variant ([`LockMode::Immediate`]). Used to
    /// skip the field from `repo.json` when unset, so a config that never
    /// touched the setting is byte-identical to one written before the field
    /// existed.
    #[must_use]
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Immediate)
    }
}

/// Atomic write: write data to a temp file then rename over the target.
///
/// Prevents file corruption if the write fails mid-operation. Used for both
/// the identity file and `signing.json`.
async fn save_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let temp_path = path.with_extension("tmp");
    fs::write(&temp_path, data).await?;
    fs::rename(&temp_path, path).await?;
    Ok(())
}

/// Configuration and identity persistence for a password store.
///
/// This is the **sealed-files tier** of gpm's three persistence tiers (RFC
/// 0038): (1) Git — the age-encrypted repository of secrets; (2) sealed files —
/// `repo.json` + `identity`, owned here; (3) plaintext files — `app.json`
/// (owned by the app shell, `src-tauri`). The secrets themselves live in tier
/// 1 (the on-disk clone this config points at); tiers 2 and 3 are local
/// metadata that never leave the device.
///
/// Manages storage of the age identity and repository-scoped configuration in
/// an app-private directory. On Android, this is app-private storage; on
/// desktop, it's the standard config directory. `repo.json` and `identity` are
/// sealed at rest with AEAD where the platform supports it; on desktop the
/// master key is `None` so the [`Seal`] is a plaintext passthrough.
#[derive(Debug)]
pub struct Config {
    config_dir: PathBuf,
    /// At-rest AEAD layer; `None` master key ⇒ plaintext passthrough.
    seal: Seal,
}

impl Config {
    /// Create a new config instance rooted at the given directory.
    ///
    /// `master_key` seals `repo.json`/`identity` at rest (AES-256-GCM); pass
    /// `None` for plaintext passthrough (desktop / tests).
    #[must_use]
    pub fn new(config_dir: PathBuf, master_key: Option<[u8; 32]>) -> Self {
        Self {
            config_dir,
            seal: Seal::new(master_key),
        }
    }

    /// Replace the seal master key at runtime. Used by the app-launch
    /// biometric lock to inject the key after the unlock prompt (retrieved from
    /// the biometric-gated Keystore) and to wipe it (`None`) when the process is
    /// backgrounded, so a locked store's envelopes fail `SealKeyUnavailable`
    /// until the next unlock. On desktop the key stays `None` (passthrough).
    pub fn set_master_key(&self, master_key: Option<[u8; 32]>) {
        self.seal.set_key(master_key);
    }

    /// Get the config directory used by this instance.
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    fn identity_path(&self) -> PathBuf {
        self.config_dir.join("identity")
    }

    fn repo_config_path(&self) -> PathBuf {
        self.config_dir.join("repo.json")
    }

    /// Save the age identity to local storage.
    ///
    /// If `passphrase` is `Some`, the identity is encrypted with age scrypt
    /// before writing. If `None`, the identity is stored as plaintext.
    /// Uses atomic write (temp file + rename) to prevent corruption.
    ///
    /// The caller is responsible for zeroizing the identity bytes after this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created, encryption
    /// fails, or the file cannot be written.
    pub async fn save_identity(
        &self,
        identity: &[u8],
        passphrase: Option<&str>,
    ) -> Result<(), Error> {
        fs::create_dir_all(&self.config_dir).await?;

        let inner = match passphrase {
            Some(pw) if !pw.is_empty() => crypto::encrypt_identity(pw, identity)?,
            _ => identity.to_vec(),
        };
        let sealed = self.seal.seal("identity", &inner)?;

        save_atomic(&self.identity_path(), &sealed).await
    }

    /// Check if the stored identity file is passphrase-encrypted.
    ///
    /// Returns `false` if no identity file exists.
    pub async fn is_identity_encrypted(&self) -> bool {
        match self.load_identity().await {
            Ok(bytes) => classify_identity(&bytes) == IdentityType::AgeEncrypted,
            Err(_) => false,
        }
    }

    /// Load the age identity from local storage.
    ///
    /// The caller **must** zeroize the returned bytes after use.
    ///
    /// # Errors
    ///
    /// Returns an error if no identity has been configured or the file cannot
    /// be read.
    pub async fn load_identity(&self) -> Result<Vec<u8>, Error> {
        let path = self.identity_path();
        if !path.exists() {
            return Err(Error::new(
                ErrorCode::NoIdentity,
                "No identity configured. Run setup first.",
            ));
        }
        let raw = fs::read(&path).await?;
        self.seal.unseal("identity", &raw)
    }

    /// Delete the stored identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub async fn delete_identity(&self) -> Result<(), Error> {
        let path = self.identity_path();
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Path of the optional identity-passphrase slot used by the app-launch
    /// biometric gate's identity-auto-unlock opt-in (RFC 0028). When that opt-in
    /// is on, the identity passphrase is AEAD-sealed under the seal master
    /// key here — so a successful app-unlock (which retrieves the master key via
    /// one biometric prompt) can unlock the identity with NO second prompt. The
    /// master key (biometric-gated when app-lock is on) gates this slot, so the
    /// passphrase is effectively behind the single app-unlock biometric.
    fn app_identity_pass_path(&self) -> PathBuf {
        self.config_dir.join("app_id_pass")
    }

    /// Seal `passphrase` under the seal master key into the identity-pass
    /// slot. No-op-equivalent on desktop (the key is `None` ⇒ passthrough, so
    /// the slot stores plaintext — acceptable since desktop has no app-lock).
    ///
    /// The caller is responsible for zeroizing `passphrase` after this call.
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created, the AEAD
    /// seal fails, or the file cannot be written.
    pub async fn save_app_identity_pass(&self, passphrase: &[u8]) -> Result<(), Error> {
        fs::create_dir_all(&self.config_dir).await?;
        let sealed = self.seal.seal("app_identity_pass", passphrase)?;
        save_atomic(&self.app_identity_pass_path(), &sealed).await
    }

    /// Load the sealed identity passphrase. The caller **must** zeroize the
    /// returned bytes after use. Returns [`ErrorCode::NoIdentity`] if the slot
    /// is absent (the opt-in was never enabled, or cleared).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or the AEAD unseal fails
    /// (e.g. `SealKeyUnavailable` while the master key is wiped).
    pub async fn load_app_identity_pass(&self) -> Result<Vec<u8>, Error> {
        let path = self.app_identity_pass_path();
        if !path.exists() {
            return Err(Error::new(
                ErrorCode::NoIdentity,
                "No app identity passphrase stored",
            ));
        }
        let raw = fs::read(&path).await?;
        self.seal.unseal("app_identity_pass", &raw)
    }

    /// Clear the identity-passphrase slot (best-effort). Used when the opt-in is
    /// disabled or self-healing a stale sealed passphrase.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be removed.
    pub async fn clear_app_identity_pass(&self) -> Result<(), Error> {
        let path = self.app_identity_pass_path();
        if path.exists() {
            fs::remove_file(path).await?;
        }
        Ok(())
    }

    /// Save repository configuration (URL + local path).
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub async fn save_repo_config(
        &self,
        url: &str,
        pat: Option<&str>,
        ssh_key: Option<&str>,
        ssh_passphrase: Option<&str>,
        local_path: &str,
    ) -> Result<(), Error> {
        let config = RepoConfig {
            url: url.to_string(),
            pat: pat.map(String::from),
            ssh_key: ssh_key.map(String::from),
            ssh_passphrase: ssh_passphrase.map(String::from),
            local_path: local_path.to_string(),
            // Setup never pins the default — left `None` so an uncustomized
            // identity auto-tracks the shipped default across versions.
            commit_user_name: None,
            commit_user_email: None,
            // The identity-auto-unlock opt-in is off at setup; the user enables
            // it from Settings.
            unlock_identity_with_app: false,
            authenticity: AuthenticityConfig::default(),
            // Setup always configures the git built-in; an `ext:` backend is
            // chosen only by its own (0046) setup path via `save_repo_config_full`.
            backend: None,
        };
        // Delegate to the atomic variant so `repo.json` is never observed
        // half-written (temp file + rename), matching `save_identity`. Matters
        // for `create_store`'s bootstrap, which saves config after git init.
        self.save_repo_config_full(&config).await
    }

    /// Persist a full [`RepoConfig`] atomically (used by the
    /// authenticity-mutation paths, which load → mutate a field → save).
    ///
    /// # Errors
    ///
    /// Returns an error if the config directory cannot be created or the file
    /// cannot be written.
    pub async fn save_repo_config_full(&self, config: &RepoConfig) -> Result<(), Error> {
        fs::create_dir_all(&self.config_dir).await?;
        let json = serde_json::to_string_pretty(config)?;
        let sealed = self.seal.seal("repo_config", json.as_bytes())?;
        save_atomic(&self.repo_config_path(), &sealed).await
    }

    /// Read + unseal `repo.json` and deserialize into `T`. The default view is
    /// [`RepoConfig`] (the slimmed repo-scoped shape); the config-scope migration
    /// reads the legacy shape via a `LegacyRepoConfig` view to recover fields the
    /// slimmed `RepoConfig` drops on deserialize (serde ignores unknown fields).
    ///
    /// # Errors
    ///
    /// Returns an error if no config exists, the file cannot be read, the AEAD
    /// unseal fails (key unavailable / tag mismatch), or the JSON is malformed.
    pub async fn load_repo_config_as<T: serde::de::DeserializeOwned>(&self) -> Result<T, Error> {
        let path = self.repo_config_path();
        if !path.exists() {
            return Err(Error::new(
                ErrorCode::NoRepo,
                "No repository configured. Run setup first.",
            ));
        }
        let raw = fs::read(&path).await?;
        let json = self.seal.unseal("repo_config", &raw)?;
        Ok(serde_json::from_slice(&json)?)
    }

    /// Load the repo-scoped config. Thin wrapper over
    /// [`load_repo_config_as`](Self::load_repo_config_as) for the common case.
    ///
    /// # Errors
    ///
    /// See [`load_repo_config_as`](Self::load_repo_config_as).
    pub async fn load_repo_config(&self) -> Result<RepoConfig, Error> {
        self.load_repo_config_as().await
    }

    /// Check if setup is complete (both identity and repo config exist).
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.identity_path().exists() && self.repo_config_path().exists()
    }

    /// Check if repo config exists (identity may or may not be present).
    #[must_use]
    pub fn repo_config_exists(&self) -> bool {
        self.repo_config_path().exists()
    }

    /// Clear all stored configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the files cannot be removed.
    pub async fn clear_all(&self) -> Result<(), Error> {
        if self.identity_path().exists() {
            fs::remove_file(self.identity_path()).await?;
        }
        if self.repo_config_path().exists() {
            fs::remove_file(self.repo_config_path()).await?;
        }
        // The sealed identity-passphrase slot is repo-scoped (paired with the
        // identity above), so a repo reset clears it too — leaving the at-rest
        // master key (app-scoped, Keystore) untouched.
        if self.app_identity_pass_path().exists() {
            fs::remove_file(self.app_identity_pass_path()).await?;
        }
        Ok(())
    }

    /// Migrate private files onto the current seal envelope.
    ///
    /// For each file: seal plaintext, and re-wrap any legacy-magic (`GPMATR1`)
    /// envelope as `GPMSEL1`. No-op when no master key is configured (desktop /
    /// tests), for current-magic envelopes, and for missing files. Each change
    /// is verified by roundtrip then committed atomically, so a crash leaves the
    /// prior bytes intact. Covers `repo_config`, `identity`, and the optional
    /// identity-auto-unlock passphrase slot (file `app_id_pass`, AAD `app_identity_pass`).
    ///
    /// When the master key is absent (App Lock deferred at cold start), legacy
    /// re-wraps soft-skip rather than error — see [`wrap_if_needed`].
    ///
    /// # Errors
    ///
    /// Returns an error if a file cannot be read, sealed/unsealed, or written.
    pub async fn migrate_seal(&self) -> Result<(), Error> {
        self.wrap_if_needed(&self.repo_config_path(), "repo_config")
            .await?;
        self.wrap_if_needed(&self.identity_path(), "identity")
            .await?;
        self.wrap_if_needed(&self.app_identity_pass_path(), "app_identity_pass")
            .await?;
        Ok(())
    }

    /// If `path` holds plaintext, seal it; if it holds a legacy-magic envelope,
    /// re-wrap it under the current magic. No-op for current-magic envelopes and
    /// missing files.
    ///
    /// A legacy re-wrap whose master key is absent (App Lock deferred at cold
    /// start) **soft-skips** — returns `Ok(())` with the file untouched — so the
    /// startup migrate stays a clean no-op under App Lock; conversion then runs
    /// via the one-shot post-unlock migrate in `applock::app_unlock`. A tampered
    /// envelope (wrong key) still propagates as [`ErrorCode::SealTampered`].
    async fn wrap_if_needed(&self, path: &Path, name: &str) -> Result<(), Error> {
        if !path.exists() {
            return Ok(());
        }
        let raw = fs::read(path).await?;
        if crate::seal::is_envelope(&raw) {
            // TODO: v1.0.x — remove this legacy re-wrap branch with LEGACY_MAGIC.
            if crate::seal::is_legacy_envelope(&raw) {
                // Soft-skip SealKeyUnavailable: key absent = App Lock deferred
                // at cold start, expected, not an error. Tampered propagates.
                let plain = match self.seal.unseal(name, &raw) {
                    Ok(p) => p,
                    Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => return Ok(()),
                    Err(e) => return Err(e),
                };
                let resealed = self.seal.seal(name, &plain)?;
                // Verify the roundtrip before committing, so a broken re-wrap
                // never overwrites a readable envelope.
                if self.seal.unseal(name, &resealed)? != plain {
                    return Err(Error::new(
                        ErrorCode::StoreError,
                        "seal migration roundtrip check failed",
                    ));
                }
                save_atomic(path, &resealed).await?;
            }
            return Ok(());
        }
        let sealed = self.seal.seal(name, &raw)?;
        // Verify the roundtrip before committing, so a broken seal never
        // overwrites readable plaintext.
        if self.seal.unseal(name, &sealed)? != raw {
            return Err(Error::new(
                ErrorCode::StoreError,
                "seal migration roundtrip check failed",
            ));
        }
        save_atomic(path, &sealed).await
    }
}

/// Skip-helper for the default-`false` bool flags on [`RepoConfig`]: keeps
/// `repo.json` free of those keys when the flag was never toggled, so an older
/// config (written before the field existed) is byte-identical to a fresh one.
/// Takes `&bool` (not by value) because serde's `skip_serializing_if` requires a
/// `fn(&T) -> bool`.
#[must_use]
#[allow(clippy::trivially_copy_pass_by_ref)] // serde skip_serializing_if needs &T
fn is_false(b: &bool) -> bool {
    !*b
}

/// Repository configuration persisted to disk.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RepoConfig {
    /// Remote repository URL.
    pub url: String,
    /// Optional personal access token for HTTPS authentication.
    pub pat: Option<String>,
    /// Optional SSH private key for SSH authentication.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ssh_key: Option<String>,
    /// Optional passphrase for encrypted SSH key.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub ssh_passphrase: Option<String>,
    /// Local filesystem path where the repo is cloned.
    pub local_path: String,
    /// Optional git commit author name; `None` uses the app default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub commit_user_name: Option<String>,
    /// Optional git commit author email; `None` uses the app default.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub commit_user_email: Option<String>,
    /// Whether a successful app-unlock should also unlock the identity session
    /// (no separate identity prompt on the next copy/show). Independent of the
    /// auto-lock timing presets and only meaningful when the app-launch biometric
    /// gate is enabled; defaults off. Read after the app-unlock injects the master key.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unlock_identity_with_app: bool,
    /// Repository authenticity config (verification mode + trusted signing
    /// keys + ignored issues). Skipped from serialization when default so
    /// users who never enable authenticity see no change to `repo.json`'s
    /// shape.
    #[serde(default, skip_serializing_if = "AuthenticityConfig::is_default")]
    pub authenticity: AuthenticityConfig,
    /// Which storage backend this store uses. `None` (the default) means the
    /// built-in git backend; `"ext:<name>"` selects an extension backend the
    /// app registered at startup. Lives in sealed `repo.json`, so it is
    /// unreadable until app unlock — resolved post-unlock, not at `Store::new`.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub backend: Option<String>,
}

impl RepoConfig {
    /// Build a [`GitAuth`](crate::storage::GitAuth) from stored credentials.
    ///
    /// SSH key takes priority if both PAT and SSH key are present.
    #[must_use]
    pub fn to_git_auth(&self) -> GitAuth {
        if let Some(key) = &self.ssh_key {
            GitAuth::Ssh {
                username: "git".to_string(),
                private_key: key.clone(),
                passphrase: self.ssh_passphrase.clone(),
            }
        } else if let Some(token) = &self.pat {
            GitAuth::Pat(token.clone())
        } else {
            GitAuth::None
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn create_config() -> (Config, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::new(dir.path().to_path_buf(), None);
        (config, dir)
    }

    #[tokio::test]
    async fn save_load_identity_roundtrip() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config.save_identity(identity, None).await.unwrap();
        let loaded = config.load_identity().await.unwrap();

        assert_eq!(loaded, identity);
    }

    #[tokio::test]
    async fn save_load_encrypted_identity_roundtrip() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config
            .save_identity(identity, Some("test-passphrase"))
            .await
            .unwrap();

        assert!(
            config.is_identity_encrypted().await,
            "identity should be encrypted"
        );

        let loaded = config.load_identity().await.unwrap();
        assert!(loaded.starts_with(b"-----BEGIN AGE ENCRYPTED FILE-----"));

        let decrypted = crypto::decrypt_identity("test-passphrase", &loaded).unwrap();
        assert_eq!(decrypted, identity);
    }

    #[tokio::test]
    async fn save_identity_empty_passphrase_stores_plaintext() {
        let (config, _dir) = create_config();
        let identity = b"AGE-SECRET-KEY-1TEST1234567890";

        config.save_identity(identity, Some("")).await.unwrap();
        assert!(
            !config.is_identity_encrypted().await,
            "empty passphrase should store plaintext"
        );

        let loaded = config.load_identity().await.unwrap();
        assert_eq!(loaded, identity);
    }

    #[tokio::test]
    async fn is_identity_encrypted_false_when_no_identity() {
        let (config, _dir) = create_config();
        assert!(!config.is_identity_encrypted().await);
    }

    #[tokio::test]
    async fn load_identity_missing() {
        let (config, _dir) = create_config();

        let err = config.load_identity().await.unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[tokio::test]
    async fn delete_identity_removes_file() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        assert!(config.identity_path().exists());

        config.delete_identity().await.unwrap();
        assert!(!config.identity_path().exists());

        let err = config.load_identity().await.unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[tokio::test]
    async fn save_load_repo_config_roundtrip() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat-token"),
                None,
                None,
                "/local/repo",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "https://example.com/repo.git");
        assert_eq!(cfg.pat, Some(String::from("pat-token")));
        assert_eq!(cfg.local_path, "/local/repo");
    }

    #[tokio::test]
    async fn repo_config_with_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("my-secret-pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.pat, Some(String::from("my-secret-pat")));
    }

    #[tokio::test]
    async fn repo_config_without_pat() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.pat, None);
    }

    #[test]
    fn is_configured_false_initially() {
        let (config, _dir) = create_config();

        assert!(!config.is_configured());
    }

    #[tokio::test]
    async fn is_configured_true_after_setup() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        assert!(config.is_configured());
    }

    #[tokio::test]
    async fn clear_all_removes_everything() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();
        assert!(config.is_configured());

        config.clear_all().await.unwrap();

        assert!(!config.is_configured());
        let identity_err = config.load_identity().await.unwrap_err();
        assert_eq!(identity_err.code, "NO_IDENTITY");
        let repo_err = config.load_repo_config().await.unwrap_err();
        assert_eq!(repo_err.code, "NO_REPO");
    }

    #[tokio::test]
    async fn overwrite_identity() {
        let (config, _dir) = create_config();

        config.save_identity(b"first-identity", None).await.unwrap();
        config
            .save_identity(b"second-identity", None)
            .await
            .unwrap();

        let loaded = config.load_identity().await.unwrap();
        assert_eq!(loaded, b"second-identity");
    }

    #[tokio::test]
    async fn partial_setup_identity_only() {
        let (config, _dir) = create_config();

        config.save_identity(b"test-identity", None).await.unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without repo config"
        );
    }

    #[tokio::test]
    async fn partial_setup_repo_only() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                None,
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();
        assert!(
            !config.is_configured(),
            "should not be configured without identity"
        );
    }

    #[tokio::test]
    async fn repo_config_with_ssh_key() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "git@github.com:user/repo.git",
                None,
                Some("-----BEGIN OPENSSH PRIVATE KEY-----\ntest-key\n-----END OPENSSH PRIVATE KEY-----"),
                Some("passphrase123"),
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "git@github.com:user/repo.git");
        assert_eq!(cfg.pat, None);
        assert!(cfg.ssh_key.is_some(), "ssh_key should be set");
        assert!(
            cfg.ssh_key.as_ref().unwrap().contains("BEGIN OPENSSH"),
            "ssh_key should contain key data"
        );
        assert_eq!(cfg.ssh_passphrase, Some(String::from("passphrase123")));
    }

    #[tokio::test]
    async fn repo_config_backward_compat_no_ssh_fields() {
        let (config, _dir) = create_config();

        // Simulate old config JSON without ssh_key/ssh_passphrase fields
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let old_json =
            r#"{"url":"https://example.com/repo.git","pat":"my-token","local_path":"/local/path"}"#;
        std::fs::write(config.repo_config_path(), old_json).unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.url, "https://example.com/repo.git");
        assert_eq!(cfg.pat, Some(String::from("my-token")));
        assert_eq!(
            cfg.ssh_key, None,
            "ssh_key should default to None for old config"
        );
        assert_eq!(
            cfg.ssh_passphrase, None,
            "ssh_passphrase should default to None for old config"
        );
    }

    #[test]
    fn to_git_auth_returns_ssh_when_key_present() {
        let cfg = RepoConfig {
            url: "git@github.com:user/repo.git".to_string(),
            pat: Some("some-token".to_string()),
            ssh_key: Some("test-key".to_string()),
            ssh_passphrase: Some("test-pass".to_string()),
            local_path: "/local".to_string(),
            ..Default::default()
        };

        let auth = cfg.to_git_auth();
        match auth {
            GitAuth::Ssh {
                username,
                private_key,
                passphrase,
            } => {
                assert_eq!(username, "git");
                assert_eq!(private_key, "test-key");
                assert_eq!(passphrase, Some("test-pass".to_string()));
            }
            _ => panic!("expected GitAuth::Ssh, got {auth:?}"),
        }
    }

    #[test]
    fn to_git_auth_returns_pat_when_no_ssh_key() {
        let cfg = RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            pat: Some("my-token".to_string()),
            ssh_key: None,
            ssh_passphrase: None,
            local_path: "/local".to_string(),
            ..Default::default()
        };

        let auth = cfg.to_git_auth();
        match auth {
            GitAuth::Pat(token) => assert_eq!(token, "my-token"),
            _ => panic!("expected GitAuth::Pat, got {auth:?}"),
        }
    }

    #[test]
    fn to_git_auth_returns_none_when_no_credentials() {
        let cfg = RepoConfig {
            url: "https://example.com/public-repo.git".to_string(),
            pat: None,
            ssh_key: None,
            ssh_passphrase: None,
            local_path: "/local".to_string(),
            ..Default::default()
        };

        let auth = cfg.to_git_auth();
        assert!(
            matches!(auth, GitAuth::None),
            "expected GitAuth::None, got {auth:?}"
        );
    }

    #[test]
    fn to_git_auth_ssh_overrides_pat() {
        // When both PAT and SSH key are present, SSH takes priority
        let cfg = RepoConfig {
            url: "git@github.com:user/repo.git".to_string(),
            pat: Some("ignored-token".to_string()),
            ssh_key: Some("ssh-key".to_string()),
            ssh_passphrase: None,
            local_path: "/local".to_string(),
            ..Default::default()
        };

        let auth = cfg.to_git_auth();
        assert!(
            matches!(auth, GitAuth::Ssh { .. }),
            "SSH should take priority over PAT"
        );
    }

    #[tokio::test]
    async fn repo_config_ssh_key_without_passphrase() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "git@github.com:user/repo.git",
                None,
                Some("test-key"),
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.ssh_key, Some(String::from("test-key")));
        assert_eq!(cfg.ssh_passphrase, None);
    }

    #[tokio::test]
    async fn repo_config_ssh_fields_not_serialized_when_none() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        // Read raw JSON to verify ssh fields are omitted
        let json = std::fs::read_to_string(config.repo_config_path()).unwrap();
        assert!(
            !json.contains("ssh_key"),
            "ssh_key should not appear in JSON when None"
        );
        assert!(
            !json.contains("ssh_passphrase"),
            "ssh_passphrase should not appear in JSON when None"
        );
    }

    #[tokio::test]
    async fn repo_config_commit_identity_roundtrip() {
        let (config, _dir) = create_config();

        std::fs::create_dir_all(&config.config_dir).unwrap();
        let rc = RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            pat: None,
            ssh_key: None,
            ssh_passphrase: None,
            local_path: "/local/path".to_string(),
            commit_user_name: Some("Alice".to_string()),
            commit_user_email: Some("alice@example.com".to_string()),
            ..Default::default()
        };
        config.save_repo_config_full(&rc).await.unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.commit_user_name.as_deref(), Some("Alice"));
        assert_eq!(cfg.commit_user_email.as_deref(), Some("alice@example.com"));
    }

    #[tokio::test]
    async fn repo_config_backward_compat_no_commit_identity() {
        let (config, _dir) = create_config();

        // Old config JSON written before commit identity existed.
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let old_json =
            r#"{"url":"https://example.com/repo.git","pat":"my-token","local_path":"/local/path"}"#;
        std::fs::write(config.repo_config_path(), old_json).unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.commit_user_name, None);
        assert_eq!(cfg.commit_user_email, None);
    }

    #[tokio::test]
    async fn repo_config_commit_identity_omitted_when_none() {
        let (config, _dir) = create_config();

        config
            .save_repo_config(
                "https://example.com/repo.git",
                Some("pat"),
                None,
                None,
                "/local/path",
            )
            .await
            .unwrap();

        let json = std::fs::read_to_string(config.repo_config_path()).unwrap();
        assert!(!json.contains("commit_user_name"));
        assert!(!json.contains("commit_user_email"));
    }

    #[tokio::test]
    async fn repo_config_backend_round_trip() {
        let (config, _dir) = create_config();
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let rc = RepoConfig {
            url: "https://x/repo".to_string(),
            local_path: "/p".to_string(),
            backend: Some("ext:cloud-folder".to_string()),
            ..Default::default()
        };
        config.save_repo_config_full(&rc).await.unwrap();
        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.backend.as_deref(), Some("ext:cloud-folder"));
    }

    #[tokio::test]
    async fn repo_config_backend_omitted_when_none() {
        let (config, _dir) = create_config();
        config
            .save_repo_config("https://x/repo", None, None, None, "/p")
            .await
            .unwrap();
        // No master key ⇒ passthrough, so the JSON is readable plaintext.
        let json = std::fs::read_to_string(config.repo_config_path()).unwrap();
        assert!(
            !json.contains("backend"),
            "backend must not be serialized when None (git default)"
        );
        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(cfg.backend, None);
    }

    #[tokio::test]
    async fn repo_config_backend_defaults_none_for_old_config() {
        // A config written before the `backend` field existed deserializes as
        // None (git) — backward compat.
        let (config, _dir) = create_config();
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let old_json = r#"{"url":"https://x/repo","local_path":"/p"}"#;
        std::fs::write(config.repo_config_path(), old_json).unwrap();
        let cfg = config.load_repo_config().await.unwrap();
        assert_eq!(
            cfg.backend, None,
            "old config without backend deserializes as None (git)"
        );
    }

    #[tokio::test]
    async fn creates_config_dir_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a/b/c");
        let config = Config::new(nested.clone(), None);

        assert!(!nested.exists(), "precondition: dir does not exist");
        config.save_identity(b"test", None).await.unwrap();
        assert!(
            nested.exists(),
            "save_identity should create the config dir"
        );
    }

    #[tokio::test]
    async fn migrate_seal_wraps_plaintext_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let key = crate::seal::generate_master_key().unwrap();

        // Simulate a pre-migration plaintext repo.json on disk.
        std::fs::create_dir_all(dir.path()).unwrap();
        let plaintext = r#"{"url":"https://x/repo","pat":"secret","local_path":"/p"}"#;
        std::fs::write(dir.path().join("repo.json"), plaintext).unwrap();
        assert!(
            !crate::seal::is_envelope(&std::fs::read(dir.path().join("repo.json")).unwrap()),
            "precondition: plaintext file"
        );

        let cfg = Config::new(dir.path().to_path_buf(), Some(key));
        cfg.migrate_seal().await.unwrap();

        // The file is now an seal envelope, and still loads correctly.
        let raw = std::fs::read(dir.path().join("repo.json")).unwrap();
        assert!(
            crate::seal::is_envelope(&raw),
            "repo.json should be wrapped"
        );
        let rc = cfg.load_repo_config().await.unwrap();
        assert_eq!(rc.url, "https://x/repo");
        assert_eq!(rc.pat.as_deref(), Some("secret"));

        // Idempotent: a second migration is a no-op (already wrapped).
        cfg.migrate_seal().await.unwrap();
    }

    #[tokio::test]
    async fn migrate_seal_rewraps_legacy_envelope() {
        // A pre-rename on-disk envelope carries the GPMATR1 magic. Build one by
        // sealing with the current key, then patching the 7-byte magic back —
        // the magic is a plaintext prefix (not AEAD-authenticated), so the tag
        // still verifies and this faithfully reproduces an old file.
        let dir = tempfile::tempdir().unwrap();
        let key = crate::seal::generate_master_key().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();

        let plaintext = br#"{"url":"https://x/repo","pat":"secret","local_path":"/p"}"#;
        let sealer = Seal::new(Some(key));
        let mut legacy = sealer.seal("repo_config", plaintext).unwrap();
        assert!(legacy.starts_with(b"GPMSEL1"));
        legacy.get_mut(..7).unwrap().copy_from_slice(b"GPMATR1");
        std::fs::write(dir.path().join("repo.json"), &legacy).unwrap();
        assert!(crate::seal::is_legacy_envelope(&legacy));

        let cfg = Config::new(dir.path().to_path_buf(), Some(key));
        cfg.migrate_seal().await.unwrap();

        // Re-wrapped to the current magic, still decryptable to the same bytes.
        let raw = std::fs::read(dir.path().join("repo.json")).unwrap();
        assert!(
            raw.starts_with(b"GPMSEL1"),
            "should be re-wrapped to GPMSEL1"
        );
        assert!(!crate::seal::is_legacy_envelope(&raw));
        let rc = cfg.load_repo_config().await.unwrap();
        assert_eq!(rc.url, "https://x/repo");
        assert_eq!(rc.pat.as_deref(), Some("secret"));

        // Idempotent: a second migration is a no-op (now current magic).
        cfg.migrate_seal().await.unwrap();
    }

    #[tokio::test]
    async fn migrate_seal_soft_skips_legacy_when_key_absent() {
        // App-Lock cold start: the master key is NOT injected yet, but a legacy
        // envelope already sits on disk. The migrate must soft-skip (Ok, file
        // untouched), NOT error — the post-unlock migrate converts it once the
        // key arrives. Regression for the App-Lock cold-start timing.
        let dir = tempfile::tempdir().unwrap();
        let key = crate::seal::generate_master_key().unwrap();
        std::fs::create_dir_all(dir.path()).unwrap();

        let plaintext = br#"{"url":"https://x/repo","pat":"secret","local_path":"/p"}"#;
        let sealer = Seal::new(Some(key));
        let mut legacy = sealer.seal("repo_config", plaintext).unwrap();
        legacy.get_mut(..7).unwrap().copy_from_slice(b"GPMATR1");
        std::fs::write(dir.path().join("repo.json"), &legacy).unwrap();

        // Config built WITHOUT the key (App Lock deferred at cold start).
        let cfg = Config::new(dir.path().to_path_buf(), None);
        // Must not error — the soft-skip returns Ok.
        cfg.migrate_seal().await.unwrap();

        // File untouched — still the legacy envelope, readable later via dual-read.
        let raw = std::fs::read(dir.path().join("repo.json")).unwrap();
        assert!(
            crate::seal::is_legacy_envelope(&raw),
            "soft-skip leaves the file untouched"
        );
        assert_eq!(raw, legacy);
    }

    #[tokio::test]
    async fn passthrough_migrate_is_noop_on_plaintext() {
        // No master key ⇒ migration must leave plaintext files untouched.
        let (cfg, dir) = create_config();
        cfg.save_repo_config("https://x/repo", Some("pat"), None, None, "/p")
            .await
            .unwrap();
        cfg.migrate_seal().await.unwrap();
        let raw = std::fs::read(dir.path().join("repo.json")).unwrap();
        assert!(
            !crate::seal::is_envelope(&raw),
            "passthrough must not wrap files"
        );
    }

    #[tokio::test]
    async fn app_identity_pass_slot_roundtrip_under_master_key() {
        // The identity-auto-unlock slot seals the passphrase under the master
        // key (None here ⇒ passthrough, mirroring desktop; the seal/open path is
        // what matters). With a real key the AAD binding protects it.
        let (config, _dir) = create_config();
        let pass = b"correct horse battery staple";

        config.save_app_identity_pass(pass).await.unwrap();
        assert_eq!(config.load_app_identity_pass().await.unwrap(), pass);

        // Clearing removes the slot.
        config.clear_app_identity_pass().await.unwrap();
        let err = config.load_app_identity_pass().await.unwrap_err();
        assert_eq!(err.code, "NO_IDENTITY");
    }

    #[tokio::test]
    async fn app_identity_pass_slot_bound_to_master_key() {
        // A passphrase sealed under one master key cannot be opened under
        // another (or with the key absent once it has been used) — the AAD +
        // AEAD tag enforce that the slot stays under its sealing key.
        let dir = tempfile::tempdir().unwrap();
        let key = crate::seal::generate_master_key().unwrap();
        let sealed_cfg = Config::new(dir.path().to_path_buf(), Some(key));
        sealed_cfg
            .save_app_identity_pass(b"secret-pass")
            .await
            .unwrap();

        // Opens under the same key.
        assert_eq!(
            sealed_cfg.load_app_identity_pass().await.unwrap(),
            b"secret-pass"
        );

        // A different key (a fresh store pointing at the same dir) cannot open
        // the slot the first key sealed.
        let other_key = crate::seal::generate_master_key().unwrap();
        let other_cfg = Config::new(dir.path().to_path_buf(), Some(other_key));
        let err = other_cfg.load_app_identity_pass().await.unwrap_err();
        assert_eq!(err.code, "SEAL_TAMPERED");
    }

    #[tokio::test]
    async fn repo_config_unlock_identity_with_app_roundtrip() {
        let (config, _dir) = create_config();
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let rc = RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            local_path: "/local/path".to_string(),
            unlock_identity_with_app: true,
            ..Default::default()
        };
        config.save_repo_config_full(&rc).await.unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert!(cfg.unlock_identity_with_app);
    }

    #[tokio::test]
    async fn repo_config_unlock_identity_with_app_omitted_when_false() {
        let (config, _dir) = create_config();
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let rc = RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            local_path: "/local/path".to_string(),
            // The flag is left at its default (false).
            ..Default::default()
        };
        config.save_repo_config_full(&rc).await.unwrap();

        let json = std::fs::read_to_string(config.repo_config_path()).unwrap();
        assert!(
            !json.contains("unlock_identity_with_app"),
            "false flag must not be serialized"
        );
    }

    #[tokio::test]
    async fn repo_config_unlock_identity_with_app_default_false_for_old_config() {
        let (config, _dir) = create_config();
        // A config written before the flag existed.
        std::fs::create_dir_all(&config.config_dir).unwrap();
        let old_json = r#"{"url":"https://example.com/repo.git","pat":"t","local_path":"/p"}"#;
        std::fs::write(config.repo_config_path(), old_json).unwrap();

        let cfg = config.load_repo_config().await.unwrap();
        assert!(!cfg.unlock_identity_with_app);
    }
}
