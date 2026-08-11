// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Diagnostics export bundle: assembles the full rotated log, the non-secret
//! user settings, a redacted view of the repository configuration, a device-info
//! summary, and a manifest into an in-memory tarball (gzip), then saves it to a
//! user-chosen location via the [`tauri_plugin_file_save`] plugin.
//!
//! Threat model (see SECURITY.md § Diagnostics logging): the on-device log is
//! plaintext by construction, but a bundle that **leaves the device** is a
//! more-sensitive artifact. The defense here is **redaction by construction**
//! (the bundling path never reads a secret: the log never holds one, the prefs
//! are plaintext-by-design, and the repo config is rendered through
//! [`rustpass::RepoConfig::redacted`] which reduces credentials to `[REDACTED]`
//! presence) plus the mandatory pre-export confirmation the frontend shows.
//! The bundle bytes never enter the `WebView`: the tarball is staged to a temp
//! file and the file-save plugin streams that file to the destination the user
//! picks.
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

use flate2::Compression;
use flate2::write::GzEncoder;
use rustpass::{Error, ErrorCode};
use tauri::{AppHandle, Manager, State};
use tauri_plugin_device_info::DeviceInfoExt;
use tauri_plugin_file_save::FileSaveExt;

use crate::app_config::now_unix;
use crate::archive::{append_entry, finish_tar_gz};
use crate::{AppState, logging};

/// Suggested name for the saved bundle (and the staged temp file).
const BUNDLE_FILENAME: &str = "gpm-diagnostics.tar.gz";

/// One entry in the bundle tarball.
struct BundleEntry {
    name: &'static str,
    bytes: Vec<u8>,
}

/// Build the bundle tarball in memory from the given entries (gzip, level 6).
/// Pure (no `State`/`AppHandle`) so the assembly is unit-testable with
/// constructed entries.
fn build_bundle(entries: &[BundleEntry]) -> Result<Vec<u8>, Error> {
    // `GzEncoder::new` pins the gzip-header mtime to 0 (the default), so
    // build_bundle is byte-deterministic given fixed entries (a real export also
    // varies by the manifest timestamp, which is an input here, not generated).
    // Compression::default() = level 6.
    let encoder = GzEncoder::new(std::io::Cursor::new(Vec::new()), Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for e in entries {
        let mut data = std::io::Cursor::new(&e.bytes[..]);
        append_entry(&mut builder, e.name, &mut data, e.bytes.len() as u64)?;
    }
    let buf = finish_tar_gz(builder)?;
    Ok(buf.into_inner())
}

/// Build the human-readable `MANIFEST.txt`: app version, generation timestamp
/// (Unix seconds, UTC), app-lock state, the entry list, the repo-config status,
/// and the redaction note.
fn build_manifest(app_locked: bool, repo_status: &str, entry_names: &[&str]) -> String {
    let secs = now_unix();
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

/// Export a diagnostics bundle (a gzip tarball) to a user-chosen location via
/// the system save dialog (SAF `ACTION_CREATE_DOCUMENT` on Android, a native
/// dialog on desktop). The bundle bytes never enter the `WebView`. Returns
/// [`ErrorCode::Cancelled`] if the user dismisses the save dialog (the frontend
/// treats that as a silent cancel, not an error toast).
///
/// A single source failing never fails the whole export: the log, prefs, and
/// device info degrade to empty/default, and a locked/unreadable repo config is
/// omitted with a manifest note. Only the tarball assembly or the final save can
/// hard-fail the command.
#[tauri::command]
#[allow(clippy::too_many_lines)] // linear gather → tar → stage → save pipeline; clearest as one fn
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
        // The active repo's facade (the bundle describes the vault the user is
        // in). After C2 relocate, `state.store` is the device facade (no
        // repo.json), so this must resolve the registry — not `state.store`.
        match state.registry.active_facade() {
            Some(store) => match store.config().await {
                Ok(cfg) => (
                    serde_json::to_string_pretty(&cfg.redacted())
                        .unwrap_or_else(|_| "{}".to_string()),
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
            },
            None => (
                String::new(),
                "omitted: no repository configured".to_string(),
            ),
        }
    };

    // ── 2. Assemble the manifest + entries, then tar+gzip ────────────────────

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

    // gzip is CPU work over a few MB (up to ~4 MB of verbose log); keep it off
    // the async worker the way the codebase wraps git/scrypt.
    let tar_bytes = tauri::async_runtime::spawn_blocking(move || build_bundle(&entries))
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
    tokio::fs::write(&temp_path, &tar_bytes)
        .await
        .map_err(|e| Error::new(ErrorCode::IoError, format!("failed to stage bundle: {e}")))?;

    // ── 4. Save (picker + write), then wipe the stage regardless of outcome ─
    let save_result = app
        .file_save()
        .save(
            BUNDLE_FILENAME.to_string(),
            temp_path.clone(),
            "application/gzip".to_string(),
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
    use flate2::read::GzDecoder;
    use std::io::{self, Read};

    /// Read a single entry back out of the tarball (for round-trip assertions).
    fn read_entry(tar_gz: &[u8], name: &str) -> Vec<u8> {
        let mut ar = tar::Archive::new(GzDecoder::new(io::Cursor::new(tar_gz)));
        for entry in ar.entries().expect("valid gzip+tar") {
            let mut e = entry.expect("entry");
            if e.path().unwrap().to_str().unwrap() == name {
                let mut buf = Vec::new();
                e.read_to_end(&mut buf).expect("read entry");
                return buf;
            }
        }
        panic!("entry {name} missing");
    }

    #[test]
    fn build_bundle_produces_gzip_magic_bytes() {
        // The output is genuinely gzip-wrapped, not a raw tar: round-tripping
        // with the same flate2+tar libs is mildly circular, so pin the magic.
        let tar_gz = build_bundle(&[BundleEntry {
            name: "MANIFEST.txt",
            bytes: b"x".to_vec(),
        }])
        .expect("build");
        let magic = tar_gz.get(..2).expect("non-empty gzip output");
        assert_eq!(magic, &[0x1f, 0x8b], "gzip magic header");
    }

    #[test]
    fn build_bundle_is_byte_deterministic() {
        // Same entries -> same bytes: the gzip/tar headers carry no timestamp,
        // so two builds of identical input are byte-identical. (A real export
        // varies by the manifest `generated` timestamp, which is an input here.)
        let entries = [BundleEntry {
            name: "a.txt",
            bytes: b"hello".to_vec(),
        }];
        let a = build_bundle(&entries).expect("build");
        let b = build_bundle(&entries).expect("build");
        assert_eq!(a, b, "identical inputs must produce byte-identical output");
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
        let tar_gz = build_bundle(&entries).expect("build");
        assert_eq!(read_entry(&tar_gz, "a.txt"), b"hello");
        assert_eq!(read_entry(&tar_gz, "b.bin"), vec![0, 1, 2, 255]);
    }

    #[test]
    fn build_bundle_empty_entries_is_a_valid_empty_tarball() {
        let tar_gz = build_bundle(&[]).expect("build");
        let mut ar = tar::Archive::new(GzDecoder::new(io::Cursor::new(tar_gz)));
        assert_eq!(
            ar.entries().expect("valid gzip+tar").count(),
            0,
            "no entries"
        );
    }

    #[test]
    fn build_bundle_compresses_a_large_entry() {
        // Highly-repetitive input compresses far smaller than its raw size.
        let big: Vec<u8> = vec![b'g'; 100_000];
        let tar_gz = build_bundle(&[BundleEntry {
            name: "gpm.log",
            bytes: big.clone(),
        }])
        .expect("build");
        assert!(
            tar_gz.len() < 10_000,
            "deflate should shrink it: {} bytes",
            tar_gz.len()
        );
        assert_eq!(
            read_entry(&tar_gz, "gpm.log"),
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
        let tar_gz = build_bundle(&[BundleEntry {
            name: "repo_config.json",
            bytes: repo_json.into_bytes(),
        }])
        .expect("build");
        let out = String::from_utf8(read_entry(&tar_gz, "repo_config.json")).expect("utf8");
        assert!(out.contains("[REDACTED]"), "presence marker missing: {out}");
        assert!(
            !out.contains("ghp_LEAK_ME"),
            "PAT leaked into bundle: {out}"
        );
        assert!(
            !out.contains("BEGIN OPENSSH PRIVATE KEY"),
            "ssh key leaked into bundle: {out}"
        );
        assert!(
            !out.contains("ssh-secret"),
            "passphrase leaked into bundle: {out}"
        );
        assert!(
            !out.contains("alice"),
            "url userinfo leaked into bundle: {out}"
        );
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
