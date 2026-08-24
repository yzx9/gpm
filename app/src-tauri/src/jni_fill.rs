// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Headless autofill entries for the Android Autofill fill activity (R056).
//!
//! [`run_fill_list`] and [`run_fill_decrypt`] are the pure, host-testable
//! cores; the `#[no_mangle]` JNI shim at the bottom is
//! `#[cfg(target_os = "android")]`, so on other targets this module still
//! compiles and the cores unit-test without a device. The symbols land in
//! `libgpm_lib.so` and are called by `xyz.yzx9.gpm.AutofillBridge` over JNI.
//!
//! Both cores are stateless per call — facades are rebuilt, nothing crosses
//! calls. The Kotlin side owns the process-lifetime vault-key cache (the
//! accepted R056 v0 trade-off); only the base64 vault key crosses JNI, and
//! the identity file itself is unsealed here in Rust, never in Kotlin.

use std::path::PathBuf;

use rustpass::Store;
use serde::Serialize;

/// One fillable entry: the repo it lives under plus the extension-stripped
/// name that is both the `Store::get` key and the display string.
#[allow(dead_code)] // serialized to Kotlin; constructed by the core + host tests.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct FillEntry {
    pub(crate) repo_id: String,
    pub(crate) name: String,
}

/// The list result crossed to Kotlin as JSON (`org.json`, exact serde keys).
/// `Ok` carries the full unsorted list — no `total`: the MVP never pages.
#[allow(dead_code)] // used by the Android-only JNI shim + the host tests.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum FillListResult {
    Ok {
        entries: Vec<FillEntry>,
    },
    /// Precondition not met — each reason maps to an empty state in the
    /// activity. `no_key` | `mid_migrate` | `no_repositories` | `not_ready`.
    Skipped {
        reason: &'static str,
    },
    Error {
        message: String,
    },
}

/// The decrypt result crossed to Kotlin as JSON. The username is always
/// present (the entry-path fallback never yields empty for a real entry).
#[allow(dead_code)] // used by the Android-only JNI shim + the host tests.
#[derive(Debug, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum FillDecryptResult {
    Ok {
        password: String,
        username: String,
    },
    /// `no_key` | `not_ready` | `app_locked` | `passphrase_locked` |
    /// `bad_vault_key`.
    Skipped {
        reason: &'static str,
    },
    Error {
        message: String,
    },
}

/// List every entry of every ready registered repository, unsorted (registry
/// order, alpha within a repo). Identity-free: the vault bridge stays wiped
/// (the shared `headless` chain), so listing never prompts and never touches
/// the identity. Repos are error-tolerant (`jni_sync` D2 style): a repo that
/// fails to resolve is skipped, not an error — `not_ready` only when none
/// produced a list at all.
#[allow(dead_code)] // called by the Android-only JNI shim + the host tests.
pub(crate) async fn run_fill_list(config_dir: PathBuf, master_key_b64: String) -> FillListResult {
    let (master_key, app_cfg) =
        match crate::headless::app_context(&config_dir, &master_key_b64).await {
            crate::headless::HeadlessGates::Skipped(reason) => {
                return FillListResult::Skipped { reason };
            }
            crate::headless::HeadlessGates::Error { message } => {
                return FillListResult::Error { message };
            }
            crate::headless::HeadlessGates::Ready {
                master_key,
                app_cfg,
            } => (master_key, app_cfg),
        };

    let ids = app_cfg.get().repositories.clone();
    if ids.is_empty() {
        return FillListResult::Skipped {
            reason: "no_repositories",
        };
    }
    let mut entries = Vec::new();
    let mut any_listed = false;
    for id_str in &ids {
        let id = crate::registry::RepoId::from(id_str.clone());
        let store = crate::build_repo_facade(&config_dir, Some(master_key), &id);
        store.set_vault_key(None);
        // Listing needs both backends: `list` walks the storage with the
        // crypto backend's secret extension (`secret_ext()` → `crypto()`).
        if store.resolve_storage().await.is_err() || store.resolve_crypto().await.is_err() {
            continue;
        }
        if !store.is_repo_ready() {
            continue;
        }
        if let Ok(page) = store.list(0, usize::MAX).await {
            any_listed = true;
            entries.extend(page.entries.into_iter().map(|e| FillEntry {
                repo_id: id_str.clone(),
                name: e.name,
            }));
        }
    }
    if !any_listed {
        return FillListResult::Skipped {
            reason: "not_ready",
        };
    }
    FillListResult::Ok { entries }
}

/// Decrypt one entry for the fill. `vault_key_b64` empty ⇒ App Lock off (or
/// the Kotlin side found no sealed vault slot): the identity stays readable
/// through the master-keyed seal. Non-empty ⇒ the key the Kotlin side just
/// unsealed via `BiometricPrompt`.
///
/// The identity residence is probed unconditionally before `get`
/// (`is_identity_under_master`): under master ⇒ the vault key is never
/// injected (a stale cached key after an in-app App-Lock toggle is harmlessly
/// ignored); under vault ⇒ the key is required — absent ⇒ `app_locked`
/// (never inferred from `SEAL_*` codes: a wrong key fails the AEAD tag as
/// `SEAL_TAMPERED`, which here means genuine corruption ⇒ `Error`).
#[allow(dead_code)] // called by the Android-only JNI shim + the host tests.
pub(crate) async fn run_fill_decrypt(
    config_dir: PathBuf,
    master_key_b64: String,
    vault_key_b64: String,
    repo_id: String,
    entry_name: String,
) -> FillDecryptResult {
    let Some(master_key) = crate::decode_master_key(&master_key_b64) else {
        return FillDecryptResult::Skipped { reason: "no_key" };
    };
    if !crate::registry::RepoId::is_valid_form(&repo_id) {
        return FillDecryptResult::Error {
            message: "invalid repository id".to_string(),
        };
    }
    let id = crate::registry::RepoId::from(repo_id);
    let store = crate::build_repo_facade(&config_dir, Some(master_key), &id);
    if store.resolve_storage().await.is_err() || store.resolve_crypto().await.is_err() {
        return FillDecryptResult::Skipped {
            reason: "not_ready",
        };
    }
    if !store.is_repo_ready() {
        return FillDecryptResult::Skipped {
            reason: "not_ready",
        };
    }
    if !store.is_identity_under_master().await {
        let Some(vault_key) = crate::decode_master_key(&vault_key_b64) else {
            return FillDecryptResult::Skipped {
                reason: if vault_key_b64.is_empty() {
                    "app_locked"
                } else {
                    "bad_vault_key"
                },
            };
        };
        store.set_vault_key(Some(vault_key));
    }
    match store.get(&entry_name).await {
        Ok(secret) => FillDecryptResult::Ok {
            password: secret.password().to_string(),
            username: extract_username(&secret, &entry_name),
        },
        Err(e) if e.code == "IDENTITY_ENCRYPTED" => {
            if try_headless_auto_unlock(&store).await
                && let Ok(secret) = store.get(&entry_name).await
            {
                FillDecryptResult::Ok {
                    password: secret.password().to_string(),
                    username: extract_username(&secret, &entry_name),
                }
            } else {
                FillDecryptResult::Skipped {
                    reason: "passphrase_locked",
                }
            }
        }
        Err(e) => FillDecryptResult::Error {
            message: e.to_string(),
        },
    }
}

/// Headless mirror of `applock::try_identity_auto_unlock`, minus the
/// self-heal (a headless fill never mutates config state): with the
/// identity-auto-unlock opt-in on, read the sealed `app_id_pass` and unlock
/// the identity session so the retried `get` can decrypt.
async fn try_headless_auto_unlock(store: &Store) -> bool {
    let Ok(rc) = store.config().await else {
        return false;
    };
    if !rc.unlock_identity_with_app {
        return false;
    }
    if !store.is_identity_encrypted().await {
        return false;
    }
    let Ok(pass) = store.load_app_identity_pass().await else {
        return false; // slot absent — per-op auth, which the fill surface can't offer
    };
    // age passphrases are UTF-8; an invalid sequence means a corrupt slot.
    let Ok(s) = std::str::from_utf8(pass.as_slice()) else {
        return false;
    };
    store.unlock(s).await.is_ok()
}

/// The gopass autofill value rule: the body `login:` attribute
/// (case-insensitive), then `username:`, then the entry path's last segment.
fn extract_username(secret: &rustpass::Secret, entry_name: &str) -> String {
    for key in ["login", "username"] {
        if let Some(value) = secret.get_ci(key)
            && let Ok(s) = std::str::from_utf8(value)
            && !s.is_empty()
        {
            return s.to_string();
        }
    }
    entry_name
        .rsplit('/')
        .next()
        .unwrap_or(entry_name)
        .to_string()
}

// ---------------------------------------------------------------------------
// Android JNI shim — not compiled on other targets (verified via
// `cargo check --target aarch64-linux-android`, not on the host).
// ---------------------------------------------------------------------------
#[cfg(target_os = "android")]
mod jni {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    use jni::EnvUnowned;
    use jni::errors::LogErrorAndDefault;
    use jni::objects::{JClass, JString};
    use serde::Serialize;
    use tokio::runtime::Runtime;

    /// A runtime owned by the fill entries, separate from jni_sync's: a fill
    /// dispatch is time-boxed by the OS and must not queue behind a long
    /// background sync `block_on` sharing one worker thread.
    fn runtime() -> &'static Runtime {
        static RT: OnceLock<Runtime> = OnceLock::new();
        RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("gpm autofill runtime")
        })
    }

    /// `catch_unwind` + JSON-serialize a core result, preserving the
    /// error-JSON contract (`with_env`'s `LogErrorAndDefault` below would
    /// otherwise turn a panic into a null).
    fn guarded_json(build: impl FnOnce() -> String) -> String {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(build)) {
            Ok(json) => json,
            Err(_) => r#"{"status":"error","message":"internal panic"}"#.to_string(),
        }
    }

    fn serialize<T: Serialize>(result: &T) -> String {
        serde_json::to_string(result)
            .unwrap_or_else(|_| r#"{"status":"error","message":"serialize_failed"}"#.to_string())
    }

    /// Entry point for `AutofillBridge.nativeListEntries(configDir, masterKeyB64)`.
    ///
    /// # Safety
    /// JNI surface — called from Kotlin with valid string arguments.
    // JNI FFI entry: the `unsafe(no_mangle)` attribute is intrinsic to exporting
    // a JVM-callable symbol (edition 2024 requires the `unsafe(...)` wrapper).
    // The function body itself is safe Rust.
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_xyz_yzx9_gpm_AutofillBridge_nativeListEntries<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JClass<'local>,
        config_dir: JString<'local>,
        master_key_b64: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| -> jni::errors::Result<JString<'local>> {
                let config_dir: String = config_dir.try_to_string(env).unwrap_or_default();
                let master_key_b64: String = master_key_b64.try_to_string(env).unwrap_or_default();
                let json = guarded_json(|| {
                    let result = runtime().block_on(super::run_fill_list(
                        PathBuf::from(config_dir),
                        master_key_b64,
                    ));
                    serialize(&result)
                });
                env.new_string(json)
            })
            .resolve::<LogErrorAndDefault>()
    }

    /// Entry point for `AutofillBridge.nativeDecryptEntry(configDir,
    /// masterKeyB64, vaultKeyB64, repoId, entryName)`.
    ///
    /// # Safety
    /// JNI surface — called from Kotlin with valid string arguments.
    #[allow(unsafe_code)]
    #[unsafe(no_mangle)]
    pub extern "system" fn Java_xyz_yzx9_gpm_AutofillBridge_nativeDecryptEntry<'local>(
        mut unowned_env: EnvUnowned<'local>,
        _class: JClass<'local>,
        config_dir: JString<'local>,
        master_key_b64: JString<'local>,
        vault_key_b64: JString<'local>,
        repo_id: JString<'local>,
        entry_name: JString<'local>,
    ) -> JString<'local> {
        unowned_env
            .with_env(|env| -> jni::errors::Result<JString<'local>> {
                let config_dir: String = config_dir.try_to_string(env).unwrap_or_default();
                let master_key_b64: String = master_key_b64.try_to_string(env).unwrap_or_default();
                let vault_key_b64: String = vault_key_b64.try_to_string(env).unwrap_or_default();
                let repo_id: String = repo_id.try_to_string(env).unwrap_or_default();
                let entry_name: String = entry_name.try_to_string(env).unwrap_or_default();
                let json = guarded_json(|| {
                    let result = runtime().block_on(super::run_fill_decrypt(
                        PathBuf::from(config_dir),
                        master_key_b64,
                        vault_key_b64,
                        repo_id,
                        entry_name,
                    ));
                    serialize(&result)
                });
                env.new_string(json)
            })
            .resolve::<LogErrorAndDefault>()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use base64::Engine;
    use rustpass::{GitAuth, Store};

    use super::*;

    const ID_A: &str = "0123456789abcdef0123456789abcdef";
    const ID_B: &str = "fedcba9876543210fedcba9876543210";

    /// Seed one configured repo under `<config>/repositories/<id>/` backed by
    /// a fresh bare remote. Holds the crypto permit for the configure round
    /// trip; released on return (callers needing more crypto work re-acquire).
    async fn seed_repo(
        config_dir: &std::path::Path,
        id: &str,
        entries: &[(&str, &[u8])],
        master: [u8; 32],
    ) -> (tempfile::TempDir, Arc<Store>) {
        let _crypto = crate::tests::crypto_permit().await;
        let (identity, recipient) = crate::tests::generate_test_keypair();
        let bare = crate::tests::create_bare_repo(entries, &recipient);
        let repo_dir = config_dir.join("repositories").join(id);
        std::fs::create_dir_all(&repo_dir).expect("create repo dir");
        let store = Arc::new(Store::new(repo_dir, Some(master)));
        store
            .configure(
                bare.path().to_str().expect("utf-8 tempdir path"),
                &GitAuth::None,
                &identity,
                None,
            )
            .await
            .expect("configure should succeed");
        (bare, store)
    }

    /// Register `ids` in the sealed merged app config at the current schema
    /// (the `jni_sync` `skips_when_schema_below_current` seeding idiom, but at
    /// the current version so the fill list proceeds past the gates).
    async fn seed_registry(config_dir: &std::path::Path, master: [u8; 32], ids: &[&str]) {
        let device = Arc::new(Store::new(config_dir.to_path_buf(), Some(master)));
        let cfg = crate::app_config::AppConfig {
            schema_version: crate::migrations::APP_CONFIG_SCHEMA_VERSION,
            repositories: ids.iter().map(|s| (*s).to_string()).collect(),
            ..Default::default()
        };
        let json = serde_json::to_vec(&cfg).expect("serialize app config");
        device
            .save_app_config(&json)
            .await
            .expect("save app config");
    }

    fn keys() -> ([u8; 32], [u8; 32], String, String) {
        let master = rustpass::seal::generate_master_key().unwrap();
        let vault = rustpass::seal::generate_master_key().unwrap();
        (
            master,
            vault,
            crate::B64.encode(master),
            crate::B64.encode(vault),
        )
    }

    // ------------------------------------------------------------- list ----

    #[tokio::test]
    async fn list_skips_on_bad_key() {
        let dir = tempfile::tempdir().unwrap();
        let res = run_fill_list(dir.path().to_path_buf(), String::from("not-a-key")).await;
        assert!(matches!(res, FillListResult::Skipped { reason: "no_key" }));
    }

    #[tokio::test]
    async fn list_skips_when_schema_below_current() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        let store = Arc::new(Store::new(dir.path().to_path_buf(), Some(master)));
        let cfg = crate::app_config::AppConfig {
            schema_version: crate::migrations::APP_CONFIG_SCHEMA_VERSION - 1,
            repositories: vec![ID_A.to_string()],
            ..Default::default()
        };
        let json = serde_json::to_vec(&cfg).unwrap();
        store.save_app_config(&json).await.unwrap();
        drop(store);

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        assert!(matches!(
            res,
            FillListResult::Skipped {
                reason: "mid_migrate"
            }
        ));
    }

    #[tokio::test]
    async fn list_skips_when_no_repositories() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_registry(dir.path(), master, &[]).await;

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        assert!(matches!(
            res,
            FillListResult::Skipped {
                reason: "no_repositories"
            }
        ));
    }

    #[tokio::test]
    async fn list_skips_when_no_repo_ready() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        // Registered id with no repositories/<id>/ on disk — nothing resolves.
        seed_registry(dir.path(), master, &[ID_A]).await;

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        assert!(matches!(
            res,
            FillListResult::Skipped {
                reason: "not_ready"
            }
        ));
    }

    #[tokio::test]
    async fn list_collects_entries_across_repos() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_repo(
            dir.path(),
            ID_A,
            &[("cloud/aws/root.age", b"pw-a\n"), ("alpha.age", b"pw-a2\n")],
            master,
        )
        .await;
        seed_repo(
            dir.path(),
            ID_B,
            &[("web/example/alice.age", b"pw-b\n")],
            master,
        )
        .await;
        seed_registry(dir.path(), master, &[ID_A, ID_B]).await;

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        let FillListResult::Ok { entries } = res else {
            panic!("expected Ok, got {res:?}");
        };
        // Registry order, alpha within a repo, extension stripped.
        assert_eq!(
            entries,
            vec![
                FillEntry {
                    repo_id: ID_A.to_string(),
                    name: "alpha".to_string()
                },
                FillEntry {
                    repo_id: ID_A.to_string(),
                    name: "cloud/aws/root".to_string()
                },
                FillEntry {
                    repo_id: ID_B.to_string(),
                    name: "web/example/alice".to_string()
                },
            ]
        );
    }

    #[tokio::test]
    async fn list_tolerates_one_broken_repo() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_repo(dir.path(), ID_A, &[("only.age", b"pw\n")], master).await;
        // ID_B registered but absent — skipped, not an error.
        seed_registry(dir.path(), master, &[ID_A, ID_B]).await;

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        let FillListResult::Ok { entries } = res else {
            panic!("expected Ok, got {res:?}");
        };
        assert_eq!(entries.len(), 1);
        assert_eq!(entries.first().expect("one entry").name, "only");
    }

    #[tokio::test]
    async fn list_ready_repo_with_no_entries_is_ok_empty() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_repo(dir.path(), ID_A, &[], master).await;
        seed_registry(dir.path(), master, &[ID_A]).await;

        let res = run_fill_list(dir.path().to_path_buf(), master_b64).await;
        assert!(matches!(res, FillListResult::Ok { entries } if entries.is_empty()));
    }

    // --------------------------------------------------- extract_username ----

    fn parse(content: &[u8]) -> rustpass::Secret {
        rustpass::Secret::parse(content).expect("parse secret")
    }

    #[test]
    fn username_prefers_login_attr() {
        let secret = parse(b"pw\nlogin: alice\nusername: bob\n");
        assert_eq!(extract_username(&secret, "web/x/user"), "alice");
    }

    #[test]
    fn username_falls_back_to_username_attr() {
        let secret = parse(b"pw\nusername: bob\n");
        assert_eq!(extract_username(&secret, "web/x/user"), "bob");
    }

    #[test]
    fn username_attr_match_is_case_insensitive() {
        let secret = parse(b"pw\nLogin: alice\n");
        assert_eq!(extract_username(&secret, "web/x/user"), "alice");
    }

    #[test]
    fn username_falls_back_to_path_last_segment() {
        let secret = parse(b"pw\n");
        assert_eq!(extract_username(&secret, "cloud/aws/root"), "root");
    }

    #[test]
    fn username_skips_empty_and_non_utf8_attr_values() {
        let secret = parse(b"pw\nlogin: \n");
        assert_eq!(extract_username(&secret, "cloud/aws/root"), "root");
    }

    // ----------------------------------------------------------- decrypt ----

    #[tokio::test]
    async fn decrypt_app_lock_off_uses_master_only() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_repo(
            dir.path(),
            ID_A,
            &[("web/example/alice.age", b"s3cret\nlogin: alice\n")],
            master,
        )
        .await;

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            ID_A.to_string(),
            "web/example/alice".to_string(),
        )
        .await;
        assert!(
            matches!(res, FillDecryptResult::Ok { ref password, ref username } if password == "s3cret" && username == "alice"),
            "expected Ok with values, got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_app_lock_on_injects_vault_key() {
        let dir = tempfile::tempdir().unwrap();
        let (master, vault, master_b64, vault_b64) = keys();
        let (_bare, store) = seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_vault_key(Some(vault));
        store.rekey_identity_to_vault().await.unwrap();
        drop(store);

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            vault_b64,
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(matches!(res, FillDecryptResult::Ok { .. }), "got {res:?}");
    }

    #[tokio::test]
    async fn decrypt_app_lock_on_without_vault_key_is_app_locked() {
        let dir = tempfile::tempdir().unwrap();
        let (master, vault, master_b64, _) = keys();
        let (_bare, store) = seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_vault_key(Some(vault));
        store.rekey_identity_to_vault().await.unwrap();
        drop(store);

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(
                res,
                FillDecryptResult::Skipped {
                    reason: "app_locked"
                }
            ),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_stale_vault_key_falls_back_to_master() {
        // App Lock off on disk (identity under master) but the Kotlin cache
        // still hands over the old vault key — the probe ignores it.
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, stale_vault_b64) = keys();
        seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            stale_vault_b64,
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(matches!(res, FillDecryptResult::Ok { .. }), "got {res:?}");
    }

    #[tokio::test]
    async fn decrypt_bad_vault_key_b64_is_bad_vault_key() {
        let dir = tempfile::tempdir().unwrap();
        let (master, vault, master_b64, _) = keys();
        let (_bare, store) = seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_vault_key(Some(vault));
        store.rekey_identity_to_vault().await.unwrap();
        drop(store);

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::from("not-a-key"),
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(
                res,
                FillDecryptResult::Skipped {
                    reason: "bad_vault_key"
                }
            ),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_identity_corruption_is_error_not_app_locked() {
        let dir = tempfile::tempdir().unwrap();
        let (master, vault, master_b64, vault_b64) = keys();
        let (_bare, store) = seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_vault_key(Some(vault));
        store.rekey_identity_to_vault().await.unwrap();
        drop(store);
        // Corrupt the envelope's tail (the GCM tag): the residence probe still
        // says "under vault" (master unseal fails), and the real unseal with
        // the correct key fails the tag ⇒ Error, never app_locked.
        let identity_path = dir.path().join("repositories").join(ID_A).join("identity");
        let mut bytes = std::fs::read(&identity_path).unwrap();
        let byte = bytes.last_mut().expect("non-empty identity");
        *byte ^= 0xff;
        std::fs::write(&identity_path, &bytes).unwrap();

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            vault_b64,
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(res, FillDecryptResult::Error { .. }),
            "corruption must surface as Error, got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_passphrase_locked_without_auto_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        let (_bare, store) = seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_passphrase("correct-horse").await.unwrap();
        drop(store);

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(
                res,
                FillDecryptResult::Skipped {
                    reason: "passphrase_locked"
                }
            ),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_auto_unlock_retries_with_sealed_passphrase() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        let (_bare, store) = seed_repo(
            dir.path(),
            ID_A,
            &[("a.age", b"pw\nlogin: alice\n")],
            master,
        )
        .await;
        let _crypto = crate::tests::crypto_permit().await;
        store.set_passphrase("correct-horse").await.unwrap();
        store.set_unlock_identity_with_app(true).await.unwrap();
        store.save_app_identity_pass("correct-horse").await.unwrap();
        drop(store);

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            ID_A.to_string(),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(res, FillDecryptResult::Ok { ref username, .. } if username == "alice"),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_entry_not_found_is_error() {
        let dir = tempfile::tempdir().unwrap();
        let (master, _, master_b64, _) = keys();
        seed_repo(dir.path(), ID_A, &[("a.age", b"pw\n")], master).await;

        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            ID_A.to_string(),
            "does-not-exist".to_string(),
        )
        .await;
        assert!(
            matches!(res, FillDecryptResult::Error { .. }),
            "got {res:?}"
        );
    }

    #[tokio::test]
    async fn decrypt_rejects_malformed_repo_id() {
        let dir = tempfile::tempdir().unwrap();
        let (_, _, master_b64, _) = keys();
        let res = run_fill_decrypt(
            dir.path().to_path_buf(),
            master_b64,
            String::new(),
            String::from("../escape"),
            "a".to_string(),
        )
        .await;
        assert!(
            matches!(res, FillDecryptResult::Error { .. }),
            "got {res:?}"
        );
    }

    // ------------------------------------------------------- wire pinning ----

    /// Pin the exact JSON keys Kotlin's `AutofillJson` parses — the repo's
    /// IPC-drift class (kebab-acronym, `@InvokeArg` flat) defended at the
    /// source.
    #[test]
    fn wire_pins_exact_json_keys() {
        let ok = FillListResult::Ok {
            entries: vec![FillEntry {
                repo_id: "ab".to_string(),
                name: "x/y".to_string(),
            }],
        };
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"status":"ok","entries":[{"repo_id":"ab","name":"x/y"}]}"#
        );
        assert_eq!(
            serde_json::to_string(&FillListResult::Skipped { reason: "no_key" }).unwrap(),
            r#"{"status":"skipped","reason":"no_key"}"#
        );
        assert_eq!(
            serde_json::to_string(&FillDecryptResult::Ok {
                password: "p".to_string(),
                username: "u".to_string()
            })
            .unwrap(),
            r#"{"status":"ok","password":"p","username":"u"}"#
        );
        assert_eq!(
            serde_json::to_string(&FillDecryptResult::Skipped {
                reason: "app_locked"
            })
            .unwrap(),
            r#"{"status":"skipped","reason":"app_locked"}"#
        );
    }
}
