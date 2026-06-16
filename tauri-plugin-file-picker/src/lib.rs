// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Tauri plugin that opens a native file picker and reads the picked file's
//! bytes into Rust — the Storage Access Framework (`ACTION_OPEN_DOCUMENT`) on
//! Android, the official `tauri-plugin-dialog` on desktop.
//!
//! This is a **backend-only** plugin: the frontend never calls it directly.
//! App-layer commands call [`FilePickerExt::file_picker`] to obtain the handle
//! and then `pick` — the file contents flow Kotlin → Rust (or dialog → Rust on
//! desktop) and never reach the WebView.

#[cfg(target_os = "android")]
use base64::{Engine as _, engine::general_purpose::STANDARD};
use tauri::plugin::{Builder, TauriPlugin};
use tauri::{Manager, Runtime};

/// Android package hosting the `FilePickerPlugin` Kotlin class.
#[cfg(target_os = "android")]
const PLUGIN_IDENTIFIER: &str = "xyz.yzx9.gpm.filepicker";

/// A picked file's raw contents plus a best-effort display name.
#[derive(Debug, Clone)]
pub struct PickedFile {
    /// Raw file contents. The caller is responsible for zeroizing secrets.
    pub bytes: Vec<u8>,
    /// `OpenableColumns.DISPLAY_NAME` (Android) / file name (desktop).
    pub filename: Option<String>,
}

/// Error returned by file-picker operations.
///
/// Carries a machine-readable `code` (`CANCELLED`, `PICK_FAILED`,
/// `DECODE_FAILED`, `IO_ERROR`, ...) and a safe (no-secret) message. The app
/// layer maps this into its own error type before anything reaches the frontend.
#[derive(Debug, Clone)]
pub struct FilePickerError {
    /// Machine-readable code, e.g. `CANCELLED`, `PICK_FAILED`, `DECODE_FAILED`.
    pub code: String,
    /// Safe (no-secret) human-readable message.
    pub message: String,
}

/// Map a Tauri mobile-plugin invoke error into a [`FilePickerError`],
/// preserving the Kotlin-supplied code (e.g. `CANCELLED`) when present.
#[cfg(target_os = "android")]
fn map_invoke_err(err: tauri::plugin::mobile::PluginInvokeError) -> FilePickerError {
    use tauri::plugin::mobile::PluginInvokeError;
    match err {
        PluginInvokeError::InvokeRejected(resp) => FilePickerError {
            code: resp.code.unwrap_or_else(|| "PICK_FAILED".to_string()),
            message: resp
                .message
                .unwrap_or_else(|| "File picker failed".to_string()),
        },
        other => FilePickerError {
            code: "PICK_FAILED".to_string(),
            message: other.to_string(),
        },
    }
}

// ---------------------------------------------------------------------------
// FilePicker handle (cfg-gated: mobile plugin handle on Android, AppHandle
// elsewhere so it can drive tauri-plugin-dialog)
// ---------------------------------------------------------------------------

/// Handle to the file picker. On Android it wraps the mobile plugin handle; on
/// other targets it wraps the [`tauri::AppHandle`] used to drive
/// `tauri-plugin-dialog`.
#[cfg(target_os = "android")]
pub struct FilePicker<R: Runtime>(tauri::plugin::PluginHandle<R>);

/// Handle to the file picker — wraps the [`tauri::AppHandle`] on non-Android
/// targets so the desktop `pick` can drive `tauri-plugin-dialog`.
#[cfg(not(target_os = "android"))]
pub struct FilePicker<R: Runtime>(tauri::AppHandle<R>);

#[cfg(target_os = "android")]
impl<R: Runtime> FilePicker<R> {
    /// Open the SAF picker, read the picked file via `ContentResolver`, and
    /// return its bytes (base64 on the Kotlin → Rust hop, decoded here).
    pub async fn pick(&self) -> Result<PickedFile, FilePickerError> {
        #[derive(serde::Deserialize)]
        struct Resp {
            bytes_b64: String,
            filename: Option<String>,
        }

        let resp = self
            .0
            .run_mobile_plugin_async::<Resp>("pick", ())
            .await
            .map_err(map_invoke_err)?;

        let bytes = STANDARD
            .decode(&resp.bytes_b64)
            .map_err(|e| FilePickerError {
                code: "DECODE_FAILED".to_string(),
                message: format!("Failed to decode picked file: {e}"),
            })?;

        Ok(PickedFile {
            bytes,
            filename: resp.filename,
        })
    }
}

#[cfg(not(target_os = "android"))]
impl<R: Runtime> FilePicker<R> {
    /// Open the native file dialog, read the picked file, and return its bytes.
    pub async fn pick(&self) -> Result<PickedFile, FilePickerError> {
        use tauri_plugin_dialog::{DialogExt, FilePath};

        let handle = self.0.clone();
        // `blocking_pick_file` drives the dialog on the main thread and blocks
        // the caller — run it on a blocking task so the async runtime is spared.
        let picked = tauri::async_runtime::spawn_blocking(move || {
            handle.dialog().file().blocking_pick_file()
        })
        .await
        .map_err(|e| FilePickerError {
            code: "PICK_FAILED".to_string(),
            message: format!("File picker task failed: {e}"),
        })?;

        match picked {
            Some(FilePath::Path(path)) => {
                let filename = path.file_name().map(|n| n.to_string_lossy().into_owned());
                let bytes = tokio::fs::read(&path).await.map_err(|e| FilePickerError {
                    code: "IO_ERROR".to_string(),
                    message: format!("Failed to read picked file: {e}"),
                })?;
                Ok(PickedFile { bytes, filename })
            }
            Some(_) => Err(FilePickerError {
                code: "INVALID_PATH".to_string(),
                message: "Picked path is not a readable filesystem path".to_string(),
            }),
            None => Err(FilePickerError {
                code: "CANCELLED".to_string(),
                message: "File picker cancelled".to_string(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// Extension trait
// ---------------------------------------------------------------------------

/// Extensions to access the file-picker handle from any [`Manager`]
/// (e.g. `AppHandle`).
pub trait FilePickerExt<R: Runtime> {
    /// Obtain the file-picker handle. Always present (registered on every
    /// target).
    fn file_picker(&self) -> &FilePicker<R>;
}

impl<R: Runtime, T: Manager<R>> FilePickerExt<R> for T {
    fn file_picker(&self) -> &FilePicker<R> {
        self.state::<FilePicker<R>>().inner()
    }
}

// ---------------------------------------------------------------------------
// Plugin initialization
// ---------------------------------------------------------------------------

/// Initializes the file-picker plugin.
///
/// On Android, registers the Kotlin `FilePickerPlugin` and manages the handle.
/// On other targets, manages a handle that drives `tauri-plugin-dialog`.
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("file-picker")
        .setup(|app, #[allow(unused_variables)] api| {
            #[cfg(target_os = "android")]
            {
                let handle = api.register_android_plugin(PLUGIN_IDENTIFIER, "FilePickerPlugin")?;
                app.manage(FilePicker(handle));
            }
            #[cfg(not(target_os = "android"))]
            {
                app.manage(FilePicker::<R>(app.clone()));
            }
            Ok(())
        })
        .build()
}
