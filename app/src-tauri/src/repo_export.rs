// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Repository export (R078): writes the active repository as a portable,
//! self-describing archive — the minimal `v1` instance of the whole-application
//! export format (R088). The archive is a zip containing:
//!
//! - `manifest.json` — `{ type: "gpm.export", version: 1, repositories: [one] }`
//!   (the R088 schema; tolerant-reader, additive-forward-compatible),
//! - `repo.bundle` — the repository's full encrypted history as a Git bundle
//!   (the actual payload; a pure git op that never decrypts), and
//! - `README.md` (en) + `README.zh-cn.md` (中文) — a human note in each supported
//!   locale, one file each (GitHub-style multi-README).
//!
//! Threat model (see R078): bundling packs git objects without decrypting, so
//! export runs under App Lock and never places a secret in memory. The leak
//! surface equals the git remote's (entry paths, structure, commit messages);
//! the manifest adds only non-secret descriptors (`backend`/`crypto`, no URL).
//! The bundle bytes never enter the `WebView`: the archive is staged to a file and
//! the file-save plugin streams it to the user-chosen destination. Recoverability
//! of a default (unencrypted) export hinges only on the existing age identity —
//! the envelope carries no new secret to lose. (Optional recipient-encryption is
//! R089; symmetric restore is R087.)

use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use rustpass::{Error, ErrorCode, RepoConfig};
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_file_save::FileSaveExt;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use crate::AppState;
use crate::read::StageGuard;

/// Suggested name for the saved archive (and the staged temp file).
const ARCHIVE_FILENAME: &str = "gpm-export.zip";
/// The staged full-history Git bundle (assembled into the archive, then wiped).
const BUNDLE_STAGE_FILENAME: &str = "gpm-repo.bundle";

// ── Manifest (R088 v1 minimal instance) ─────────────────────────────────────

/// One repository entry in the export manifest. The R088 schema reserves more
/// fields (settings, an optional `secrets` slot owned by R089); v1 emits this
/// minimal descriptor, and a tolerant reader ignores unknown future fields.
#[derive(serde::Serialize)]
struct ManifestRepo {
    /// Inner payload kind — a Git bundle today (`saf-snapshot` for a future
    /// non-git backend, per R088).
    format: &'static str,
    /// Storage backend (`git` for the built-in; `ext:<name>` for a future
    /// pluggable backend). Derived from `RepoConfig::backend`.
    backend: String,
    /// Crypto backend (`age` or `gpg`). Derived from `RepoConfig::crypto`.
    crypto: String,
    /// Filename of the payload inside the archive.
    payload: &'static str,
}

/// The export manifest. `generated`/`gpm_version` are additive top-level extras
/// a tolerant reader (R088) ignores — forward-compatible additions to `v1`.
#[derive(serde::Serialize)]
struct Manifest {
    #[serde(rename = "type")]
    kind: &'static str,
    version: u32,
    /// Unix-second generation timestamp (UTC).
    generated: u64,
    gpm_version: &'static str,
    repositories: Vec<ManifestRepo>,
}

/// Build the manifest JSON. `cfg` is read unlock-free (`repo.json` is sealed
/// under the auth-free key); `None` only if the config read unexpectedly failed,
/// in which case the descriptors default to `git`/`age` (the bundle — which is
/// git/age-encrypted by construction — already built successfully). No URL or
/// repository identifier is recorded: a bare bundle does not reveal the remote,
/// so neither does the manifest (R078: no new leak surface).
fn build_manifest(cfg: Option<&RepoConfig>) -> String {
    let (backend, crypto) = cfg.map_or_else(
        || ("git".to_string(), "age".to_string()),
        |c| {
            let backend = match c.backend.as_deref() {
                Some(b) if !b.is_empty() => b.to_string(),
                _ => "git".to_string(),
            };
            let crypto = match c.crypto.as_deref() {
                Some("gpg") => "gpg",
                _ => "age",
            }
            .to_string();
            (backend, crypto)
        },
    );
    let m = Manifest {
        kind: "gpm.export",
        version: 1,
        generated: SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
        gpm_version: env!("CARGO_PKG_VERSION"),
        repositories: vec![ManifestRepo {
            format: "git-bundle",
            backend,
            crypto,
            payload: "repo.bundle",
        }],
    };
    serde_json::to_string_pretty(&m).unwrap_or_else(|_| "{}".to_string())
}

/// A README file in the archive: its zip-entry name + localized body. The frontend
/// builds one entry per supported locale (driven by its `SUPPORTED_LOCALES`), so
/// adding a locale is a frontend-only change — the backend never names a locale.
#[derive(serde::Deserialize)]
struct ReadmeEntry {
    /// Zip-entry name, e.g. `README.md` or `README.zh-cn.md`.
    name: String,
    /// The full locale-owned markdown (heading + prose).
    body: String,
}

/// A README zip-entry name must be `README.md` or `README.<x>.md` — no path
/// separators, no `..` traversal, and distinct from `manifest.json` /
/// `repo.bundle` (a collision would let a bad entry shadow the real payload in a
/// naive extractor). Defense-in-depth: the frontend only sends these, but the IPC
/// boundary is untrusted.
fn is_readme_name(name: &str) -> bool {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    let Some((stem, ext)) = name.rsplit_once('.') else {
        return false;
    };
    ext == "md" && (stem == "README" || stem.starts_with("README."))
}

/// Assemble the archive into `zip_path`: `manifest.json` + one README entry per
/// supported locale + `repo.bundle`. The text entries are deflated; the bundle is
/// stored (a git packfile is already deflate-compressed, so re-deflating burns CPU
/// for ~no gain) and streamed in chunk-by-chunk via `io::copy`. File-backed (not
/// the in-memory `Vec` the diagnostics exporter uses) so a large bundle never sits
/// in RAM. Pure (no `State`/`AppHandle`) so the assembly is unit-testable.
fn build_export_zip(
    zip_path: &Path,
    manifest: &str,
    readmes: &[ReadmeEntry],
    bundle_path: &Path,
) -> Result<(), Error> {
    let file = std::fs::File::create(zip_path)
        .map_err(|e| Error::new(ErrorCode::IoError, format!("create export zip: {e}")))?;
    // The archive carries plaintext metadata (entry paths, commit messages) from
    // the bundle; 0600 keeps a stage stranded by a hard kill unreadable by others.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(zip_path, std::fs::Permissions::from_mode(0o600));
    }
    let mut zw = ZipWriter::new(file);
    let text = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let raw = SimpleFileOptions::default().compression_method(CompressionMethod::Stored);

    zw.start_file("manifest.json", text)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip start manifest: {e}")))?;
    zw.write_all(manifest.as_bytes())
        .map_err(|e| Error::new(ErrorCode::IoError, format!("zip write manifest: {e}")))?;
    // One README per supported locale (README.md, README.zh-cn.md, …), passed in
    // by the frontend — the backend is locale-blind and writes each body verbatim.
    for r in readmes {
        zw.start_file(&r.name, text)
            .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip start {}: {e}", r.name)))?;
        zw.write_all(r.body.as_bytes())
            .map_err(|e| Error::new(ErrorCode::IoError, format!("zip write {}: {e}", r.name)))?;
    }
    zw.start_file("repo.bundle", raw)
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip start bundle: {e}")))?;
    let mut bf = std::fs::File::open(bundle_path)
        .map_err(|e| Error::new(ErrorCode::IoError, format!("open staged bundle: {e}")))?;
    // Stream the bundle into the entry (8 KB copy buffer) — never holds it in RAM.
    std::io::copy(&mut bf, &mut zw)
        .map_err(|e| Error::new(ErrorCode::IoError, format!("stream bundle into zip: {e}")))?;
    zw.finish()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("zip finish: {e}")))?;
    Ok(())
}

/// Export the active repository as a `gpm-export.zip` archive to a user-chosen
/// location via the system save dialog (`SAF` `ACTION_CREATE_DOCUMENT` on Android,
/// a native dialog on desktop). The archive bytes never enter the `WebView`.
/// Returns [`ErrorCode::Cancelled`] if the user dismisses the save dialog.
///
/// `readmes` is a JSON array of `{ name, body }` README entries — one per
/// supported locale, built by the frontend. Passed as a string because the Android
/// IPC bridge deserializes nested structs unreliably (a single string param is
/// safe); the locale set lives entirely in the frontend.
///
/// Runs under App Lock: `create_bundle` touches storage only (never the
/// identity), and `Store::config` reads the auth-free-sealed `repo.json`. See the
/// module docs for the threat model.
#[tauri::command]
#[allow(clippy::needless_pass_by_value)] // Tauri IPC needs the owned State param shape (house style)
pub(crate) async fn export_repository(
    app: AppHandle,
    state: State<'_, AppState>,
    readmes: String,
) -> Result<(), Error> {
    export_repository_core(&state, &app, &readmes).await
}

/// Runtime-generic core of [`export_repository`], so in-crate tests can drive it
/// against a mock runtime. See [`export_repository`] for the contract.
pub(crate) async fn export_repository_core<R: Runtime>(
    state: &State<'_, AppState>,
    app: &AppHandle<R>,
    readmes: &str,
) -> Result<(), Error> {
    // Single-flight: shares the file-save plugin's one SAF picker slot with the
    // attachment + diagnostics exports.
    let _guard = crate::export_guard::FileSaveGuard::acquire()?;

    // Parse + validate the README entries up front (fail fast — don't build the
    // bundle if the payload is bad). One per supported locale, built by the
    // frontend; passed as a JSON string for the Android IPC bridge (a single
    // string param deserializes reliably; nested structs do not).
    let readmes: Vec<ReadmeEntry> = serde_json::from_str(readmes).map_err(|e| {
        Error::new(
            ErrorCode::StoreError,
            format!("invalid readmes payload: {e}"),
        )
    })?;
    for r in &readmes {
        if !is_readme_name(&r.name) {
            return Err(Error::new(
                ErrorCode::StoreError,
                format!("refused README entry name: {:?}", r.name),
            ));
        }
    }

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| Error::new(ErrorCode::StoreError, format!("cache dir unavailable: {e}")))?;
    let bundle_path = cache_dir.join(BUNDLE_STAGE_FILENAME);
    let zip_path = cache_dir.join(ARCHIVE_FILENAME);
    // RAII-wipe both stages on every return path / panic (the bundle carries
    // plaintext metadata; diagnostics-style manual cleanup is not panic-safe).
    let _bundle_stage = StageGuard::new(&bundle_path);
    let _zip_stage = StageGuard::new(&zip_path);

    // 1. Build the bundle to its own stage (rustpass; repo_lock inside; under
    //    App Lock — create_bundle touches storage only, never the identity).
    state.store.create_bundle(&bundle_path).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&bundle_path, std::fs::Permissions::from_mode(0o600));
    }

    // 2. Assemble the envelope: manifest + one README per locale + the bundle,
    //    streamed into a File-backed zip on a blocking thread.
    let cfg = state.store.config().await.ok();
    let manifest = build_manifest(cfg.as_ref());
    let zp = zip_path.clone();
    let bp = bundle_path.clone();
    tauri::async_runtime::spawn_blocking(move || build_export_zip(&zp, &manifest, &readmes, &bp))
        .await
        .map_err(|e| {
            Error::new(
                ErrorCode::StoreError,
                format!("export build task failed: {e}"),
            )
        })??;

    // 3. Free the bundle stage before the (slow) SAF copy so peak disk during the
    //    save is the zip only (the bundle is already inside it).
    let _ = tokio::fs::remove_file(&bundle_path).await;

    // 4. Save the archive (plugin streams the staged zip to the destination).
    let save_result = app
        .file_save()
        .save(
            ARCHIVE_FILENAME.to_string(),
            zip_path.clone(),
            "application/zip".to_string(),
        )
        .await;

    match save_result {
        Ok(()) => Ok(()),
        Err(e) if e.code == "CANCELLED" => Err(Error::new(ErrorCode::Cancelled, "Save cancelled")),
        Err(e) => Err(Error::new(
            ErrorCode::StoreError,
            format!("repository export failed: {}", e.message),
        )),
    }
}

/// Best-effort removal of stranded repo-export stages (`gpm-repo.bundle` +
/// `gpm-export.zip`) from a prior run killed mid-export (`StageGuard`'s Drop runs
/// on panic/cancel but not on SIGKILL). Called once at app startup.
pub(crate) async fn sweep_repo_export_stage<R: Runtime>(app: &AppHandle<R>) {
    let Ok(cache_dir) = app.path().app_cache_dir() else {
        return;
    };
    let _ = tokio::fs::remove_file(cache_dir.join(BUNDLE_STAGE_FILENAME)).await;
    let _ = tokio::fs::remove_file(cache_dir.join(ARCHIVE_FILENAME)).await;
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)] // manifest-structure assertions index a known schema
mod tests {
    use super::*;
    use std::io::Read;

    /// Read a single entry back out of the archive (for round-trip assertions).
    fn read_entry(zip_path: &Path, name: &str) -> Vec<u8> {
        let f = std::fs::File::open(zip_path).unwrap();
        let mut za = zip::ZipArchive::new(f).expect("valid zip");
        let mut e = za
            .by_name(name)
            .unwrap_or_else(|_| panic!("entry {name} missing"));
        let mut buf = Vec::new();
        e.read_to_end(&mut buf).unwrap();
        buf
    }

    /// The manifest is the R088 v1 minimal instance: a fixed type marker,
    /// version 1, and one repository entry with the git-bundle descriptor. No
    /// URL/identifier (no leak surface beyond the bundle). Covers the age
    /// default, a GPG store, and a failed config read (→ git/age defaults).
    #[test]
    fn build_manifest_emits_r088_v1_minimal_instance() {
        let age_cfg = RepoConfig {
            backend: None,
            crypto: None,
            ..Default::default()
        };
        let m: serde_json::Value = serde_json::from_str(&build_manifest(Some(&age_cfg))).unwrap();
        assert_eq!(m["type"], "gpm.export");
        assert_eq!(m["version"], 1);
        assert_eq!(m["repositories"][0]["format"], "git-bundle");
        assert_eq!(m["repositories"][0]["backend"], "git");
        assert_eq!(m["repositories"][0]["crypto"], "age");
        assert_eq!(m["repositories"][0]["payload"], "repo.bundle");
        assert!(
            m["repositories"][0].get("url").is_none(),
            "no URL recorded (no leak surface beyond the bundle)"
        );

        // GPG store: crypto gpg.
        let gpg_cfg = RepoConfig {
            crypto: Some("gpg".to_string()),
            ..Default::default()
        };
        let m: serde_json::Value = serde_json::from_str(&build_manifest(Some(&gpg_cfg))).unwrap();
        assert_eq!(m["repositories"][0]["crypto"], "gpg");

        // Config read failed (None): degrades to git/age defaults, still valid v1.
        let m: serde_json::Value = serde_json::from_str(&build_manifest(None)).unwrap();
        assert_eq!(m["repositories"][0]["backend"], "git");
        assert_eq!(m["repositories"][0]["crypto"], "age");
    }

    /// `is_readme_name` accepts `README.md` / `README.<x>.md` and rejects path
    /// traversal, non-markdown names, and collisions with `manifest.json` /
    /// `repo.bundle` — so an untrusted IPC payload can't shadow the payload or
    /// plant a zip-slip entry.
    #[test]
    fn is_readme_name_accepts_readmes_rejects_paths_and_collisions() {
        assert!(is_readme_name("README.md"));
        assert!(is_readme_name("README.zh-cn.md"));
        assert!(is_readme_name("README.pt-br.md"));
        // Path separators / traversal.
        assert!(!is_readme_name("../README.md"));
        assert!(!is_readme_name("a/README.md"));
        assert!(!is_readme_name("README..md"));
        // Not a README, or not markdown.
        assert!(!is_readme_name("manifest.json"));
        assert!(!is_readme_name("repo.bundle"));
        assert!(!is_readme_name("NOTES.md"));
        assert!(!is_readme_name("README"));
        assert!(!is_readme_name("README.txt"));
    }

    /// `build_export_zip` assembles exactly `manifest.json` + `README.md` (en) +
    /// `README.zh-cn.md` (中文) + `repo.bundle` (in that order) and streams the
    /// bundle in byte-for-byte (the `Stored`/`io::copy` path doesn't corrupt it).
    /// A real bundle's git round-trip is proven in the rustpass integration test;
    /// here a stand-in bundle suffices to pin the envelope structure.
    #[test]
    fn build_export_zip_streams_bundle_into_archive() {
        let dir = tempfile::tempdir().unwrap();
        let bundle_path = dir.path().join("repo.bundle");
        let bundle_bytes = b"# v2 git bundle\n0000 HEAD\n\nPACK\x00\x01\x02fake";
        std::fs::write(&bundle_path, bundle_bytes).unwrap();

        let zip_path = dir.path().join("gpm-export.zip");
        let manifest = build_manifest(None);
        build_export_zip(
            &zip_path,
            &manifest,
            &[
                ReadmeEntry {
                    name: "README.md".to_string(),
                    body: "English body.".to_string(),
                },
                ReadmeEntry {
                    name: "README.zh-cn.md".to_string(),
                    body: "中文正文。".to_string(),
                },
            ],
            &bundle_path,
        )
        .unwrap();

        let names: Vec<String> = {
            let f = std::fs::File::open(&zip_path).unwrap();
            let mut za = zip::ZipArchive::new(f).expect("valid zip");
            let n = za.len();
            (0..n)
                .filter_map(|i| za.by_index(i).ok().map(|e| e.name().to_string()))
                .collect()
        };
        assert_eq!(
            names,
            vec![
                "manifest.json",
                "README.md",
                "README.zh-cn.md",
                "repo.bundle"
            ],
            "exactly four entries, in order"
        );

        // The bundle entry is preserved byte-for-byte (streaming didn't corrupt it).
        assert_eq!(read_entry(&zip_path, "repo.bundle"), bundle_bytes);

        // manifest.json parses as the R088 v1 schema and points at repo.bundle.
        let m: serde_json::Value =
            serde_json::from_slice(&read_entry(&zip_path, "manifest.json")).unwrap();
        assert_eq!(m["type"], "gpm.export");
        assert_eq!(m["repositories"][0]["payload"], "repo.bundle");

        // One README per locale, each written verbatim from its body.
        assert_eq!(
            String::from_utf8(read_entry(&zip_path, "README.md")).unwrap(),
            "English body."
        );
        assert_eq!(
            String::from_utf8(read_entry(&zip_path, "README.zh-cn.md")).unwrap(),
            "中文正文。"
        );
    }
}
