// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Backend-only file-save plugin: saves a staged file to a destination the user
//! picks — Android Storage Access Framework (`ACTION_CREATE_DOCUMENT`) on
//! Android, the official `tauri-plugin-dialog` save on desktop.
//!
//! We own the write rather than using `tauri-plugin-fs`'s `Fs::open` for a
//! content URI: the official path is synchronous (it parks the tokio worker on
//! the mobile-plugin round-trip) and panics `unimplemented!()` when the chosen
//! destination cannot be opened for writing. Owning the Kotlin write gives a
//! real error path (`SAVE_FAILED`) and an async round-trip.
//!
//! This is a **backend-only** plugin: the frontend never calls it directly. The
//! diagnostics-export command calls [`FileSaveExt::file_save`] and then
//! [`FileSaveHandle::save`]; only the staged file path crosses to Kotlin, never
//! the bundle bytes through the WebView (Kotlin streams the staged file to the
//! chosen destination).

use std::path::{Path, PathBuf};

#[cfg(target_os = "android")]
use tauri::plugin::mobile::PluginInvokeError;
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};
#[cfg(not(target_os = "android"))]
use tauri_plugin_dialog::{DialogExt, FilePath};

/// Android package hosting the `FileSavePlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.filesave";

/// Error returned by file-save operations. Carries a machine-readable `code`
/// (`CANCELLED`, `SAVE_FAILED`, `IO_ERROR`) and a safe (no-secret) message. The
/// app layer maps `CANCELLED` to its own soft-cancel state and the rest to a
/// real error.
#[derive(Debug, Clone)]
pub struct FileSaveError {
    /// Machine-readable code, e.g. `CANCELLED`, `SAVE_FAILED`, `IO_ERROR`.
    pub code: String,
    /// Safe (no-secret) human-readable message.
    pub message: String,
}

/// Map a Tauri mobile-plugin invoke error into a [`FileSaveError`], preserving
/// the Kotlin-supplied code (e.g. `CANCELLED`) when present.
#[cfg(target_os = "android")]
fn map_invoke_err(err: PluginInvokeError) -> FileSaveError {
    match err {
        PluginInvokeError::InvokeRejected(resp) => FileSaveError {
            code: resp.code.unwrap_or_else(|| "SAVE_FAILED".to_string()),
            message: resp.message.unwrap_or_else(|| "Save failed".to_string()),
        },
        other => FileSaveError {
            code: "SAVE_FAILED".to_string(),
            message: other.to_string(),
        },
    }
}

/// Lowercase filename extension without the dot, or `None` when the name has
/// none. Labels the desktop save-dialog filter; the Android path uses the
/// caller-supplied MIME type instead.
fn extension_of(filename: &str) -> Option<String> {
    Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
}

// ---------------------------------------------------------------------------
// FileSaveHandle (cfg-gated: mobile plugin handle on Android, AppHandle
// elsewhere so it can drive tauri-plugin-dialog)
// ---------------------------------------------------------------------------

/// Handle to the file save. On Android it wraps the mobile plugin handle; on
/// other targets it wraps the [`tauri::AppHandle`] used to drive
/// `tauri-plugin-dialog`.
#[cfg(target_os = "android")]
pub struct FileSaveHandle<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the file save — wraps the [`tauri::AppHandle`] on non-Android
/// targets so the desktop `save` can drive `tauri-plugin-dialog`.
#[cfg(not(target_os = "android"))]
pub struct FileSaveHandle<R: Runtime>(tauri::AppHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> FileSaveHandle<R> {
    /// Pop the SAF save dialog (`ACTION_CREATE_DOCUMENT`) and stream the staged
    /// file at `temp_path` into the chosen destination. `filename` is the
    /// suggested name; `mime_type` is the picker's MIME filter (e.g.
    /// `application/zip`, `application/octet-stream`). Returns `CANCELLED` if
    /// the user dismisses the picker.
    pub async fn save(
        &self,
        filename: String,
        temp_path: PathBuf,
        mime_type: String,
    ) -> Result<(), FileSaveError> {
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            filename: &'a str,
            temp_path: &'a str,
            mime_type: &'a str,
        }
        #[derive(serde::Deserialize)]
        struct SaveResp {
            #[allow(dead_code)]
            ok: bool,
        }

        let temp = temp_path.to_string_lossy();
        let _resp: SaveResp = self
            .0
            .run_mobile_plugin_async(
                "save",
                Payload {
                    filename: &filename,
                    temp_path: temp.as_ref(),
                    mime_type: &mime_type,
                },
            )
            .await
            .map_err(map_invoke_err)?;
        Ok(())
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> FileSaveHandle<R> {
    /// Pop the native save dialog and copy the staged file at `temp_path` into
    /// the chosen destination. The filter is derived from `filename`'s extension
    /// (more specific than a MIME on a native dialog); `mime_type` is accepted
    /// for signature parity with Android but unused here. Returns `CANCELLED` if
    /// the user dismisses it.
    pub async fn save(
        &self,
        filename: String,
        temp_path: PathBuf,
        _mime_type: String,
    ) -> Result<(), FileSaveError> {
        let handle = self.0.clone();
        // `blocking_save_file` drives the dialog on the main thread and blocks
        // the caller — run it on a blocking task so the async runtime is spared.
        let dest = tauri::async_runtime::spawn_blocking(move || {
            let mut builder = handle.dialog().file().set_file_name(filename.clone());
            if let Some(ext) = extension_of(&filename) {
                let label = ext.to_uppercase();
                builder = builder.add_filter(&label, &[ext.as_str()]);
            }
            builder.blocking_save_file()
        })
        .await
        .map_err(|e| FileSaveError {
            code: "SAVE_FAILED".to_string(),
            message: format!("Save task failed: {e}"),
        })?;

        let Some(FilePath::Path(path)) = dest else {
            return Err(FileSaveError {
                code: "CANCELLED".to_string(),
                message: "Save dialog cancelled".to_string(),
            });
        };

        tokio::fs::copy(&temp_path, &path)
            .await
            .map_err(|e| FileSaveError {
                code: "IO_ERROR".to_string(),
                message: format!("Failed to write bundle: {e}"),
            })?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the file-save handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait FileSaveExt<R: Runtime> {
    /// Obtain the file-save handle. Always present (registered on every
    /// target).
    fn file_save(&self) -> &FileSaveHandle<R>;
}

impl<R: Runtime, T: Manager<R>> FileSaveExt<R> for T {
    fn file_save(&self) -> &FileSaveHandle<R> {
        self.state::<FileSaveHandle<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the file-save plugin.
///
/// On Android, registers the Kotlin `FileSavePlugin` and manages the handle. On
/// other targets, manages a handle that drives `tauri-plugin-dialog`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("file-save")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "FileSavePlugin")?;
                app.manage(FileSaveHandle(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(FileSaveHandle::<R>(app.clone()));
            }
            Ok(())
        })
        .build()
}
