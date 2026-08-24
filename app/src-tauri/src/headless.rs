// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! The shared headless bootstrap gate chain — the prefix every OS-started,
//! no-`AppHandle` entry (the `WorkManager` `SyncWorker`, the Autofill fill
//! surface) runs before touching any repository. Single-sourced here so the
//! gate order and skip reasons cannot drift between the two callers.

use std::path::Path;
use std::sync::Arc;

use rustpass::Store;

use crate::app_config::{AppConfigStore, PeekOutcome};
use crate::migrations::APP_CONFIG_SCHEMA_VERSION;

/// Outcome of the shared gate chain: the decoded key plus the bound app
/// config, or the reason the entry must skip / fail before any repo work.
pub(crate) enum HeadlessGates {
    /// Skipped before touching any repository. Not an error.
    Skipped(&'static str),
    /// A config load failure worth surfacing (not retrying past).
    Error { message: String },
    /// Gates passed; the device facade is bound to `app_cfg` and reloaded.
    Ready {
        master_key: [u8; 32],
        app_cfg: Arc<AppConfigStore>,
    },
}

/// Decode the auth-free master key and bind the device facade to the merged
/// app config, gated on the app-config schema.
///
/// R074: the auth-free key must be decoded FIRST — the registry and behavior
/// live in the sealed merged `app.json`, unreadable without it. The key
/// arrives base64-encoded (the Rust↔Keystore IPC shape). Missing ⇒ the store
/// isn't set up yet (R064: the auth-free master is permanent, so its absence
/// is never "App Lock on") — skip, don't error.
///
/// R064: headless callers hold the auth-free master key only — the vault
/// bridge is wiped here so no identity key sits in headless memory by
/// default. (The fill decrypt path re-keys the vault seal per call from a key
/// the Kotlin side unsealed — see `jni_fill`.)
///
/// Schema gate (D2): a separate-process fire can land mid-m0010, when the
/// repo files are half-moved and the registry is already non-empty. Skip
/// (never error) until the schema settles to current. Absent/Corrupt fall
/// through — an unconfigured app skips at the caller's own gates.
pub(crate) async fn app_context(config_dir: &Path, master_key_b64: &str) -> HeadlessGates {
    let Some(master_key) = crate::decode_master_key(master_key_b64) else {
        return HeadlessGates::Skipped("no_key");
    };

    let app_cfg = AppConfigStore::new(config_dir).await;
    // The device facade (rooted at config_dir) reads the sealed merged
    // `app.json` (the registry + behavior). The per-repo facades are built by
    // the caller at `repositories/<id>/`.
    let device = Arc::new(Store::new(config_dir.to_path_buf(), Some(master_key)));
    device.set_vault_key(None);
    app_cfg.set_store(Arc::clone(&device));
    if matches!(
        app_cfg.peek_schema_version().await,
        PeekOutcome::Version(v) if v < APP_CONFIG_SCHEMA_VERSION
    ) {
        return HeadlessGates::Skipped("mid_migrate");
    }
    // Load the sealed merged config so the caller reads the persisted state
    // (not cold-start defaults).
    if let Err(e) = app_cfg.reload().await {
        return HeadlessGates::Error {
            message: e.to_string(),
        };
    }
    HeadlessGates::Ready {
        master_key,
        app_cfg: Arc::new(app_cfg),
    }
}
