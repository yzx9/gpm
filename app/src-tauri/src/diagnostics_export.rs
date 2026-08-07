// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Diagnostics export bundle: assembles the full rotated log, the non-secret
//! user settings, a redacted view of the repository configuration, a device-info
//! summary, and a manifest into an in-memory zip, then saves it to a
//! user-chosen location via the [`tauri_plugin_file_save`] plugin.
//!
//! Threat model (see SECURITY.md § Diagnostics logging): the on-device log is
//! plaintext by construction, but a bundle that **leaves the device** is a
//! more-sensitive artifact. The defense here is **redaction by construction**
//! (the bundling path never reads a secret: the log never holds one, the prefs
//! are plaintext-by-design, and the repo config is rendered through
//! [`rustpass::RepoConfig::redacted`] which reduces credentials to `[REDACTED]`
//! presence) plus the mandatory pre-export confirmation the frontend shows.
//! The bundle bytes never enter the `WebView`: the zip is staged to a temp file
//! and the file-save plugin streams that file to the destination the user picks.
//!
//! Graceful degrade: the export is not unlock-gated. When the app is locked, the
//! redacted repo config is omitted (it carries git credentials) and the single
//! `app_config.json` entry carries only the display prefs — the behavior prefs
//! are omitted — and the manifest says why; the log, device info, and display
//! prefs always ship, so the bundle works even when the bug is an unlock/startup
//! failure. (R074: there is one sealed merged `app.json`, readable under lock
//! via the auth-free key — the behavior omission is a privacy choice, not a
//! seal-forced one.)

use std::sync::atomic::Ordering;
use std::time;
use std::{io::Write, time::SystemTime};

use rustpass::{Error, ErrorCode};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_device_info::DeviceInfoExt;
use tauri_plugin_file_save::FileSaveExt;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::{AppState, logging};

/// Suggested name for the saved bundle (and the staged temp file).
const BUNDLE_FILENAME: &str = "gpm-diagnostics.zip";

/// One entry in the bundle zip.
struct BundleEntry {
    name: &'static str,
    bytes: Vec<u8>,
}

/// Build the bundle zip in memory from the given entries (Deflated). Pure (no
/// `State`/`AppHandle`) so the assembly is unit-testable with constructed
/// entries.
fn build_bundle(entries: &[BundleEntry]) -> Result<Vec<u8>, Error> {
    let mut zw = ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for e in entries {
        zw.start_file(e.name, opts)
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip start_file: {e}")))?;
        zw.write_all(&e.bytes)
            .map_err(|e| Error::new(ErrorCode::IoError, format!("zip write: {e}")))?;
    }
    let cursor = zw
        .finish()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip finish: {e}")))?;
    Ok(cursor.into_inner())
}

/// Build the human-readable `MANIFEST.txt`: app version, generation timestamp
/// (Unix seconds, UTC), app-lock state, the entry list, the repo-config status,
/// and the redaction note.
fn build_manifest(app_locked: bool, repo_status: &str, entry_names: &[&str]) -> String {
    let secs = SystemTime::now()
        .duration_since(time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let list: String = entry_names.iter().fold(String::new(), |mut acc, n| {
        acc.push_str("  - ");
        acc.push_str(n);
        acc.push('\n');
        acc
    });
    format!(
        "gpm diagnostics bundle\n\
         ======================\n\
         App version: {ver}\n\
         Generated: {secs} (Unix seconds, UTC)\n\
         App locked at export: {locked}\n\n\
         Contents:\n\
         {list}\n\
         Repository config: {repo_status}\n\n\
         Secrets (access tokens, SSH keys, passphrases) are replaced with [REDACTED].\n\
         When the app is locked, repo_config.json is omitted and app_config.json\n\
         carries only the display prefs (behavior prefs are omitted).\n",
        ver = env!("CARGO_PKG_VERSION"),
        locked = app_locked,
    )
}

/// Export a diagnostics bundle (zip) to a user-chosen location via the system
/// save dialog (SAF `ACTION_CREATE_DOCUMENT` on Android, a native dialog on
/// desktop). The bundle bytes never enter the `WebView`. Returns [`ErrorCode::Cancelled`]
/// if the user dismisses the save dialog (the frontend treats that as a silent
/// cancel, not an error toast).
///
/// A single source failing never fails the whole export: the log, prefs, and
/// device info degrade to empty/default, and a locked/unreadable repo config is
/// omitted with a manifest note. Only the zip assembly or the final save can
/// hard-fail the command.
#[tauri::command]
#[allow(clippy::too_many_lines)] // linear gather → zip → stage → save pipeline; clearest as one fn
#[allow(clippy::needless_pass_by_value)] // Tauri IPC needs the owned State param shape (house style)
pub(crate) async fn export_diagnostics(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), Error> {
    // Single-flight: reject a second concurrent export. The frontend disables
    // the button mid-export, but a server-side guard is cheap defense-in-depth
    // against a non-UI caller — and it now shares the single slot with
    // `export_attachment`, since both drive the file-save plugin's one pending
    // SAF picker (a concurrent pair would otherwise clobber its pendingTempPath).
    let _export_guard = crate::export_guard::FileSaveGuard::acquire()?;

    let app_locked = state.app_locked.load(Ordering::SeqCst);

    // ── 1. Gather (best-effort; one failure never fails the export) ──────────

    // Full rotated log, untruncated (the viewer caps at 256 KiB; the export
    // carries every segment). A missing log dir => empty, not an error.
    let log_bytes = match logging::log_dir(&app) {
        Ok(dir) => logging::read_log_bytes(&dir, &logging::log_base(&app)).await,
        Err(_) => Vec::new(),
    };

    // App config — a single `app_config.json` entry (R074 collapsed pref.json +
    // the sealed behavior slot into one sealed merged app.json). When unlocked,
    // ship the full merged config; when locked, ship a DISPLAY-ONLY projection
    // (locale/theme/verbose/cadence) — the behavior prefs (lock_mode, autosync,
    // …) are omitted while locked, matching the prior "behavior omitted when
    // locked" posture. (The merged file is readable under lock via the auth-free
    // key — decision D — so the omission is a privacy choice, not a seal-forced
    // one. None of these fields are secrets.)
    let app_config_json = if app_locked {
        serde_json::to_string_pretty(&state.app_config.get_pref())
            .unwrap_or_else(|_| "{}".to_string())
    } else {
        serde_json::to_string_pretty(&state.app_config.get()).unwrap_or_else(|_| "{}".to_string())
    };

    // Device info (Android build fields + UA + display; OS/arch/version
    // fallback). Best-effort.
    let device_json = match app.device_info().read().await {
        Ok(d) => serde_json::to_string_pretty(&d).unwrap_or_else(|_| "{}".to_string()),
        Err(_) => "{}".to_string(),
    };

    // Redacted repo config: only when unlocked (it carries git credentials,
    // reduced to [REDACTED] presence via `redacted()`). (The Logs screen is only
    // reachable unlocked, so this branch is defense for a non-UI caller.)
    let (repo_json, repo_status) = if app_locked {
        (String::new(), "omitted: app locked".to_string())
    } else {
        match state.store.config().await {
            Ok(cfg) => (
                serde_json::to_string_pretty(&cfg.redacted()).unwrap_or_else(|_| "{}".to_string()),
                "included".to_string(),
            ),
            Err(e) if e.code == "NO_REPO" => (
                String::new(),
                "omitted: no repository configured".to_string(),
            ),
            Err(e) if e.code == "SEAL_KEY_UNAVAILABLE" => {
                (String::new(), "omitted: sealed key unavailable".to_string())
            }
            Err(e) if e.code == "SEAL_TAMPERED" => {
                (String::new(), "omitted: seal tampered".to_string())
            }
            Err(e) => (String::new(), format!("omitted: {e}")),
        }
    };

    // ── 2. Assemble the manifest + entries, then zip ─────────────────────────

    let mut names: Vec<&str> = vec![
        "MANIFEST.txt",
        "gpm.log",
        "app_config.json",
        "device_info.json",
    ];
    if !app_locked {
        names.push("repo_config.json");
    }
    let manifest = build_manifest(app_locked, &repo_status, &names);

    let mut entries: Vec<BundleEntry> = vec![
        BundleEntry {
            name: "MANIFEST.txt",
            bytes: manifest.into_bytes(),
        },
        BundleEntry {
            name: "gpm.log",
            bytes: log_bytes,
        },
        BundleEntry {
            name: "app_config.json",
            bytes: app_config_json.into_bytes(),
        },
        BundleEntry {
            name: "device_info.json",
            bytes: device_json.into_bytes(),
        },
    ];
    if !app_locked {
        entries.push(BundleEntry {
            name: "repo_config.json",
            bytes: repo_json.into_bytes(),
        });
    }

    // Zip deflate is CPU work over a few MB (up to ~4 MB of verbose log); keep
    // it off the async worker the way the codebase wraps git/scrypt.
    let zip_bytes = tauri::async_runtime::spawn_blocking(move || build_bundle(&entries))
        .await
        .map_err(|e| {
            Error::new(
                ErrorCode::StoreError,
                format!("bundle build task failed: {e}"),
            )
        })??;
    // The redacted repo-config view holds no secret (credentials are
    // `[REDACTED]`), so there is no secret-derived buffer to wipe here; the
    // original `RepoConfig` stays in `Store` and is never cloned out.

    // ── 3. Stage to a temp file, then hand the path to the save plugin ───────
    // Staging (not base64-over-IPC) keeps the save streaming and peak RAM low.
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("cache dir unavailable: {e}")))?;
    let temp_path = cache_dir.join(BUNDLE_FILENAME);
    let _ = tokio::fs::remove_file(&temp_path).await; // best-effort wipe of any prior stage
    tokio::fs::write(&temp_path, &zip_bytes)
        .await
        .map_err(|e| Error::new(ErrorCode::IoError, format!("failed to stage bundle: {e}")))?;

    // ── 4. Save (picker + write), then wipe the stage regardless of outcome ─
    let save_result = app
        .file_save()
        .save(
            BUNDLE_FILENAME.to_string(),
            temp_path.clone(),
            "application/zip".to_string(),
        )
        .await;
    let _ = tokio::fs::remove_file(&temp_path).await;

    match save_result {
        Ok(()) => Ok(()),
        Err(e) if e.code == "CANCELLED" => Err(Error::new(ErrorCode::Cancelled, "Save cancelled")),
        Err(e) => Err(Error::new(
            ErrorCode::StoreError,
            format!("diagnostics export failed: {}", e.message),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Read};

    /// Read a single entry back out of a zip (for round-trip assertions).
    fn read_entry(zip_bytes: &[u8], name: &str) -> Vec<u8> {
        let mut za = zip::ZipArchive::new(io::Cursor::new(zip_bytes)).expect("valid zip");
        let mut f = za.by_name(name).expect("entry exists");
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).expect("read entry");
        buf
    }

    #[test]
    fn build_bundle_round_trips_entries() {
        let entries = vec![
            BundleEntry {
                name: "a.txt",
                bytes: b"hello".to_vec(),
            },
            BundleEntry {
                name: "b.bin",
                bytes: vec![0, 1, 2, 255],
            },
        ];
        let zip = build_bundle(&entries).expect("build");
        assert_eq!(read_entry(&zip, "a.txt"), b"hello");
        assert_eq!(read_entry(&zip, "b.bin"), vec![0, 1, 2, 255]);
    }

    #[test]
    fn build_bundle_empty_entries_is_a_valid_empty_zip() {
        let zip = build_bundle(&[]).expect("build");
        let za = zip::ZipArchive::new(io::Cursor::new(zip)).expect("valid zip");
        assert_eq!(za.len(), 0, "no entries");
    }

    #[test]
    fn build_bundle_compresses_a_large_entry() {
        // Highly-repetitive input compresses far smaller than its raw size.
        let big: Vec<u8> = vec![b'g'; 100_000];
        let zip = build_bundle(&[BundleEntry {
            name: "gpm.log",
            bytes: big.clone(),
        }])
        .expect("build");
        assert!(
            zip.len() < 10_000,
            "deflate should shrink it: {} bytes",
            zip.len()
        );
        assert_eq!(
            read_entry(&zip, "gpm.log"),
            big,
            "round-trip decompresses exactly"
        );
    }

    #[test]
    fn bundle_carries_redacted_not_raw_repo_config() {
        // A RepoConfig carrying real secrets must reach the bundle only in its
        // redacted form: the token is absent, the [REDACTED] presence marker is
        // present. Pins the redaction guarantee at the bundle layer.
        let cfg = rustpass::RepoConfig {
            url: "https://alice:hunter2@git.example.com/o/r.git".to_string(),
            pat: Some("ghp_LEAK_ME".to_string()),
            ssh_key: Some("-----BEGIN OPENSSH PRIVATE KEY-----".to_string()),
            ssh_passphrase: Some("ssh-secret".to_string()),
            local_path: "/tmp/repo".to_string(),
            ..Default::default()
        };
        let repo_json = serde_json::to_string(&cfg.redacted()).expect("redacted serializes");
        let zip = build_bundle(&[BundleEntry {
            name: "repo_config.json",
            bytes: repo_json.into_bytes(),
        }])
        .expect("build");
        let out = String::from_utf8(read_entry(&zip, "repo_config.json")).expect("utf8");
        assert!(out.contains("[REDACTED]"), "presence marker missing: {out}");
        assert!(
            !out.contains("ghp_LEAK_ME"),
            "PAT leaked into bundle: {out}"
        );
        assert!(
            !out.contains("BEGIN OPENSSH PRIVATE KEY"),
            "ssh key leaked into bundle: {out}"
        );
        assert!(!out.contains("ssh-secret"), "passphrase leaked: {out}");
        assert!(!out.contains("alice"), "url userinfo leaked: {out}");
    }

    #[test]
    fn manifest_notes_omitted_when_locked() {
        let names = [
            "MANIFEST.txt",
            "gpm.log",
            "app_config.json",
            "device_info.json",
        ];
        let m = build_manifest(true, "omitted: app locked", &names);
        assert!(m.contains("App locked at export: true"), "{m}");
        assert!(m.contains("Repository config: omitted: app locked"), "{m}");
        // No repo_config entry in the Contents list when locked. The privacy note
        // names the file, so check the list-bullet form, not the bare name.
        assert!(!m.contains("- repo_config.json"), "{m}");
    }

    #[test]
    fn manifest_notes_included_when_unlocked() {
        let names = [
            "MANIFEST.txt",
            "gpm.log",
            "app_config.json",
            "device_info.json",
            "repo_config.json",
        ];
        let m = build_manifest(false, "included", &names);
        assert!(m.contains("App locked at export: false"), "{m}");
        assert!(m.contains("Repository config: included"), "{m}");
        assert!(m.contains("repo_config.json"), "{m}");
    }
}
