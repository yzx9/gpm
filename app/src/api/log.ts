// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";

/**
 * Diagnostics logging IPC — mirrors `src-tauri/src/logging.rs`. The in-app
 * viewer (Settings → Logs) reads and clears the rotated log file via these, and
 * the runtime level (Info default, an optional time-boxed Verbose/Debug window)
 * is controlled via the app-config IPC, not here.
 *
 * This is the API layer only — thin invoke wrappers. The capture plumbing that
 * feeds `writeLog` (the `console.*` shim and the uncaught-error handlers) is
 * app-bootstrap wiring and lives in `app/src/log-capture.ts`.
 */

/** Read the diagnostics log (active + rotated, ordered, tail-truncated). */
export async function readLog(): Promise<string> {
  return invoke<string>("read_log");
}

/** Clear the log (rotated removed, active truncated in place). */
export async function clearLog(): Promise<void> {
  await invoke("clear_log");
}

/** Export a diagnostics bundle (zip) to a user-chosen location via SAF. */
export async function exportDiagnostics(): Promise<void> {
  await invoke("export_diagnostics");
}

/** Write a frontend-emitted record into the backend log. */
export async function writeLog(level: string, message: string): Promise<void> {
  await invoke("write_log", { level, message });
}
