// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repository / app configuration commands — repo config display, the commit
//! author identity, and a full reset. When this grows further (import/export,
//! per-repo settings), it can graduate to a `config/` directory of submodules.

use rustpass::{CommitIdentity, Error, LockMode, RepoConfig, Store};
use tauri::{AppHandle, State};

use crate::AppState;
use crate::app_config::{AppConfig, GateIdle};
use crate::identity::{
    LockEventReason, emit_lock_state, refresh_security_cache, reset_gate_idle_timer,
    reset_lock_timer,
};

/// IPC-safe projection of [`RepoConfig`] with credential fields masked, so the
/// full PAT / SSH private key / passphrase never reach the `WebView`. The mask is
/// applied in the only constructor ([`From<RepoConfig>`]); every repo-config
/// command returns this type, so forgetting to mask is a compile error rather
/// than a silent leak. `#[serde(transparent)]` serializes the inner `RepoConfig`
/// verbatim, so the on-wire shape (and the frontend `RepoConfig` type) is
/// unchanged — only the credential *values* differ (masked here, full on disk).
#[derive(Debug, Clone, serde::Serialize)]
#[serde(transparent)]
pub(crate) struct RepoConfigPublic(RepoConfig);

/// Fixed presence marker for credentials that are never displayed positionally
/// (an SSH private key is viewed via its public half; a passphrase is short and
/// human-chosen, so first/last chars would leak too much). Non-empty ⇒ set.
const PRESENCE_MASK: &str = "••••";

/// Mask a PAT for display: keep the first and last 4 chars (enough to tell two
/// tokens apart, and the `ghp_`-style provider prefix is public anyway) and hide
/// the middle behind a fixed run of bullets. Tokens of 8 chars or fewer collapse
/// to the presence marker so no positional leak occurs on a short token.
fn mask_pat(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 8 {
        return PRESENCE_MASK.to_string();
    }
    let n = chars.len();
    // `take`/`skip` avoid panicking slices; safe because we returned above when
    // `len <= 8`, so `n >= 9` here.
    let head: String = chars.iter().take(4).collect();
    let tail: String = chars.iter().skip(n - 4).collect();
    format!("{head}{PRESENCE_MASK}{tail}")
}

impl From<RepoConfig> for RepoConfigPublic {
    fn from(mut rc: RepoConfig) -> Self {
        rc.pat = rc.pat.take().map(|p| mask_pat(&p));
        rc.ssh_key = rc.ssh_key.take().map(|_| PRESENCE_MASK.to_string());
        rc.ssh_passphrase = rc.ssh_passphrase.take().map(|_| PRESENCE_MASK.to_string());
        Self(rc)
    }
}

/// Get the current repo config (for display in settings).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn get_config(state: State<'_, AppState>) -> Result<RepoConfigPublic, Error> {
    // repo.json is repo-scoped — read it off the active repo facade (rooted at
    // `config_dir/repositories/<id>/` post-m0010, NOT the device facade).
    state
        .active_repo()?
        .config()
        .await
        .map(RepoConfigPublic::from)
}

/// Reset all configuration and local data.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn reset_config(state: State<'_, AppState>, app: AppHandle) -> Result<(), Error> {
    log::info!("config: reset");
    // Cancel timer
    state.lock_timer.disarm();
    // Wipe the entry-view cache too — a decrypted entry must not outlive the
    // identity/config that's being reset. (The detail page's leave-wipe usually
    // clears it first; the backend owns the invariant.)
    crate::entry_cache::soft_wipe_entry_cache(
        &state,
        &app,
        crate::entry_cache::EntryCacheReason::Lock,
    );
    reset_config_core(&state).await?;
    // After a reset there is no identity, so the app is no longer locked — emit
    // the real state so any open unlock overlay closes. Emit against the device
    // facade (the registry is now empty); its identity was wiped by the reset.
    emit_lock_state(&app, &state.store, false, LockEventReason::Reset).await;
    Ok(())
}

/// The data-wipe core of Emergency Reset: every registered repository's files,
/// any repo files stranded at the config root, orphaned `repositories/<id>/`
/// dirs, and the persisted registry fields. Device prefs in `app.json`
/// (locale/theme/lock-mode/…) survive. Extracted from the command so the wipe
/// contract is testable without a Wry handle.
pub(crate) async fn reset_config_core(state: &AppState) -> Result<(), Error> {
    // Wipe every registered repository's files (each repo's repo.json/identity/
    // app_id_pass/repo clone). Each registered facade is rooted at its own
    // per-repo dir (post-m0010: `config_dir/repositories/<id>/`), so iterating
    // the registry wipes each one wherever it lives — NOT the device facade
    // (`state.store` at `config_dir`, which owns only `app.json`). `reset()`
    // wipes the repo dir + sealed repo files and leaves the device-scoped
    // `app.json` (and its prefs) intact.
    //
    // A per-wipe fs failure (EBUSY/EACCES on an in-flight sync's file, …) logs
    // and CONTINUES: the registry bookkeeping below must be cleared no matter
    // what — a reset that aborted early with `repositories` still persisted
    // would re-register dead ids on the next launch and wedge re-setup behind
    // `register_first_repo`'s non-empty no-op. The first collected failure is
    // surfaced to the caller after the bookkeeping lands.
    let mut first_err: Option<Error> = None;
    for id in state.registry.list_ids() {
        if let Some(facade) = state.registry.facade(&id)
            && let Err(e) = facade.reset().await
        {
            log::warn!("config: reset could not wipe repository {id}: {e}");
            first_err.get_or_insert(e);
        }
    }
    // Residue the registry loop cannot reach — "erase ALL local data" must not
    // leave secrets behind in either stranded shape:
    // (a) repo files still at the config root (a half-registered or
    //     mid-migration state — exactly when a user reaches for reset). The
    //     device facade's own `reset()` removes exactly those root files
    //     (identity/repo.json/app_id_pass + the clone at repo.json's
    //     local_path) while leaving the device-scoped `app.json` intact.
    if let Err(e) = state.store.reset().await {
        log::warn!("config: reset could not wipe root-stranded files: {e}");
        first_err.get_or_insert(e);
    }
    // (b) every `repositories/<id>/` dir — the (now file-less) registered
    //     roots and any orphan whose id never made it into `app.json`.
    //     Best-effort: a removal failure logs and continues — the registered
    //     file wipes above already succeeded.
    let repos_dir = crate::repositories_dir(state.store.config_dir());
    if let Ok(mut entries) = tokio::fs::read_dir(&repos_dir).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            if let Err(e) = tokio::fs::remove_dir_all(entry.path()).await {
                log::warn!("config: reset could not remove repo dir: {e}");
            }
        }
    }
    // Drop the in-memory index and the persisted registry fields
    // (repositories/last_active) — the repos they named are gone. Runs even
    // after a partial wipe (see above).
    state.registry.clear();
    state.app_config.clear_repositories().await?;
    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(())
}

/// Set the git commit author identity. A `null` field clears it, reverting to
/// the app default. Returns the updated repo config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_commit_identity(
    state: State<'_, AppState>,
    name: Option<String>,
    email: Option<String>,
) -> Result<RepoConfigPublic, Error> {
    log::info!("config: set-commit-identity");
    state
        .active_repo()?
        .set_commit_identity(name, email)
        .await
        .map(RepoConfigPublic::from)
}

/// Set the HTTPS personal access token. `null` (or blank/whitespace) clears it.
/// Returns the updated repo config (PAT masked for display).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_pat(
    state: State<'_, AppState>,
    pat: Option<String>,
) -> Result<RepoConfigPublic, Error> {
    log::info!("config: set-pat");
    state
        .active_repo()?
        .set_pat(pat)
        .await
        .map(RepoConfigPublic::from)
}

/// Remove the stored SSH key + passphrase. A stored PAT, if any, becomes the
/// active auth method. Returns the updated repo config (masked).
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn clear_ssh_key(state: State<'_, AppState>) -> Result<RepoConfigPublic, Error> {
    log::info!("config: clear-ssh-key");
    state
        .active_repo()?
        .clear_ssh_key()
        .await
        .map(RepoConfigPublic::from)
}

/// Validate a PAT against the remote before saving it: a read-only `git fetch`
/// into a throwaway ref (HEAD untouched). Throws on auth/network failure so the
/// UI can refuse to save a bad token. Runs cancellably (R034): a user cancel
/// during the probe reaches the fetch's credentials/transfer callbacks.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn verify_git_auth(
    state: State<'_, AppState>,
    app: AppHandle,
    pat: String,
) -> Result<(), Error> {
    log::info!("config: verify-git-auth");
    let store = state.active_repo()?;
    crate::git::run_cancellable(&state, app, move |cancel, _tx, slot| async move {
        // Setup-time op (no `write_mu`): arm up-front so the probe is
        // cancellable. The guard disarms when the future drops.
        let _guard = crate::git::SlotGuard::arm(slot, cancel.clone());
        store.verify_pat(pat, Some(cancel)).await
    })
    .await
}

/// Set the app auto-lock mode (`immediate` / `{ idle: secs }` / `never`).
/// Refreshes the `AppState` cache and re-applies the timer so the new mode takes
/// effect immediately (Immediate/Never disarm; Idle re-arms). Returns the
/// updated app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_lock_mode(
    state: State<'_, AppState>,
    app: AppHandle,
    mode: LockMode,
) -> Result<AppConfig, Error> {
    log::info!("config: set-lock-mode: {mode:?}");
    let cfg = state.app_config.set_lock_mode(mode).await?;
    // Re-apply the mode to the active repo's identity cache + timer.
    let active = state.active_repo()?;
    refresh_security_cache(&state, &active).await;
    // Apply the new mode to the live timer (reads the just-refreshed cache).
    reset_lock_timer(&state, &app, &active);
    Ok(cfg)
}

/// Set the app-launch-gate in-app idle timeout (`"off"` / `{ "after": secs }`).
/// Returns the updated app config. Applies the new setting to the live gate
/// idle timer (reads the just-updated `AppConfigStore` cache), so a mid-session
/// change takes effect immediately — no unlock cycle needed.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_gate_idle(
    state: State<'_, AppState>,
    app: AppHandle,
    mode: GateIdle,
) -> Result<AppConfig, Error> {
    log::info!("config: set-gate-idle: {mode:?}");
    let cfg = state.app_config.set_gate_idle(mode).await?;
    // Apply to the active repo's gate-idle timer; reads the just-updated
    // app_config cache.
    let active = state.active_repo()?;
    reset_gate_idle_timer(&state, &app, &active);
    Ok(cfg)
}

/// Set the password-view auto-clear override (`null` = default, `0` = never).
/// Returns the updated app config; the UI reads the new value via `get_app_config`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_view_clear_secs(
    state: State<'_, AppState>,
    secs: Option<u64>,
) -> Result<AppConfig, Error> {
    log::info!("config: set-view-clear-secs: {secs:?}");
    state.app_config.set_view_clear_secs(secs).await
}

/// Set the clipboard auto-clear override (`null` = default, `0` = never).
/// Refreshes the `AppState` cache so the next copy honors it. Returns the updated
/// app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_clipboard_clear_secs(
    state: State<'_, AppState>,
    secs: Option<u64>,
) -> Result<AppConfig, Error> {
    log::info!("config: set-clipboard-clear-secs: {secs:?}");
    let cfg = state.app_config.set_clipboard_clear_secs(secs).await?;
    // Refresh the active repo's cache (== `state.store` under the single-repo
    // invariant) so the next copy honors the new clear window.
    let active = state.active_repo()?;
    refresh_security_cache(&state, &active).await;
    Ok(cfg)
}

/// Set the per-device autosync flag — whether each save wraps in a pull → write
/// → push (`true`, the default) or stays local until a manual Sync. Also pushes
/// the value into the `Store`'s injected cache (`autosync_write` reads it).
/// Returns the updated app config.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_autosync(
    state: State<'_, AppState>,
    app: AppHandle,
    enabled: bool,
) -> Result<AppConfig, Error> {
    log::info!("config: set-autosync: {enabled}");
    let cfg = state.app_config.set_autosync(enabled).await?;
    // Seed the active repo facade's injected autosync cache;
    // `autosync_write` reads it.
    state.active_repo()?.set_autosync(enabled);
    // Background sync is linked to AutoSync — re-apply the schedule
    // when AutoSync goes on, cancel it when off (else the Worker wakes every
    // interval just to no-op-skip on the autosync gate, wasting battery).
    if enabled {
        crate::reschedule_background_sync(&app, state.app_config.background_sync()).await;
    } else {
        crate::cancel_background_sync(&app).await;
    }
    Ok(cfg)
}

/// Take-once: whether a background sync left a divergence / authenticity-block
/// attention marker, removing it. The foreground calls this on cold-start to
/// decide whether to trigger a sync + surface the badge.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn consume_sync_attention(state: State<'_, AppState>) -> Result<bool, Error> {
    Ok(state.app_config.consume_sync_attention_marker().await)
}

/// Set the periodic background-sync cadence (`Off` opts out). Persists to
/// `pref.json` and returns the merged config. The Android `WorkManager`
/// schedule is re-applied by the scheduling wiring on the `AppHandle`.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)]
pub(crate) async fn set_background_sync(
    state: State<'_, AppState>,
    app: AppHandle,
    cadence: crate::app_config::BackgroundSyncCadence,
) -> Result<AppConfig, Error> {
    log::info!("config: set-background-sync: {cadence:?}");
    let cfg = state.app_config.set_background_sync(cadence).await?;
    // Re-apply the WorkManager schedule from the new cadence.
    crate::reschedule_background_sync(&app, cadence).await;
    Ok(cfg)
}

/// The default commit author identity (for UI display).
#[tauri::command]
pub(crate) async fn get_commit_identity_default() -> CommitIdentity {
    Store::commit_identity_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pat: Option<&str>, ssh_key: Option<&str>, passphrase: Option<&str>) -> RepoConfig {
        RepoConfig {
            url: "https://example.com/repo.git".to_string(),
            pat: pat.map(str::to_string),
            ssh_key: ssh_key.map(str::to_string),
            ssh_passphrase: passphrase.map(str::to_string),
            local_path: "/tmp/repo".to_string(),
            commit_user_name: Some("Alice".to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn mask_pat_long_keeps_ends() {
        // 40-char token: first 4 + mask + last 4.
        assert_eq!(
            mask_pat("ghp_0123456789abcdefghijklmnopqrstuvwxyz"),
            "ghp_••••wxyz"
        );
    }

    #[test]
    fn mask_pat_short_collapses_to_presence() {
        assert_eq!(mask_pat("short"), PRESENCE_MASK);
        assert_eq!(mask_pat("12345678"), PRESENCE_MASK); // exactly 8 → presence
        assert_eq!(mask_pat(""), PRESENCE_MASK);
    }

    #[test]
    fn mask_pat_boundary_9_chars_keeps_ends() {
        // 9 chars: just past the threshold → first 4 + mask + last 4.
        assert_eq!(mask_pat("abcdefghi"), "abcd••••fghi");
    }

    #[test]
    fn repo_config_public_masks_credentials_not_other_fields() {
        let rc = cfg(
            Some("ghp_0123456789abcdefghijklmnopqrstuvwxyz"),
            Some("-----BEGIN KEY-----"),
            Some("secret"),
        );
        let public = RepoConfigPublic::from(rc).0; // inner RepoConfig for the assertion
        assert_eq!(public.pat.as_deref(), Some("ghp_••••wxyz"));
        assert_eq!(public.ssh_key.as_deref(), Some(PRESENCE_MASK));
        assert_eq!(public.ssh_passphrase.as_deref(), Some(PRESENCE_MASK));
        // Non-secret fields pass through verbatim.
        assert_eq!(public.url, "https://example.com/repo.git");
        assert_eq!(public.local_path, "/tmp/repo");
        assert_eq!(public.commit_user_name.as_deref(), Some("Alice"));
    }

    #[test]
    fn repo_config_public_none_credentials_stay_none() {
        let rc = cfg(None, None, None);
        let public = RepoConfigPublic::from(rc).0;
        assert!(public.pat.is_none());
        assert!(public.ssh_key.is_none());
        assert!(public.ssh_passphrase.is_none());
    }

    #[test]
    fn repo_config_public_serializes_masked_same_shape() {
        // transparent ⇒ the JSON key set matches RepoConfig, so the frontend
        // RepoConfig type still parses it — but the full PAT never serializes.
        let rc = cfg(Some("ghp_token123456789012345678901234567890"), None, None);
        let json = serde_json::to_string(&RepoConfigPublic::from(rc)).unwrap();
        assert!(json.contains("\"pat\""), "pat key present: {json}");
        assert!(
            !json.contains("ghp_token"),
            "full PAT must not serialize: {json}"
        );
        assert!(json.contains("ghp_••••"), "masked PAT present: {json}");
    }
}
