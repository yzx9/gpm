// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { App } from "vue";

/**
 * Diagnostics logging IPC — mirrors `src-tauri/src/logging.rs`. The in-app
 * viewer (Settings → Logs) reads and clears the rotated log file via these. The
 * runtime level (Info default, an optional time-boxed Verbose/Debug window) is
 * controlled via the app-config IPC, not here.
 *
 * The frontend logging bridge (`installFrontendLogger`) writes uncaught frontend
 * errors into the same backend pipeline through `write_log`, so a bug report has
 * a frontend trace alongside the backend one.
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

/** Stringify an unknown caught value for the log. Never serializes an arbitrary
 *  object's fields: the full diagnostics log now leaves the device via Export,
 *  so a rejected promise or Vue error carrying a secret field must not be dumped.
 *  Error messages still log; non-Error, non-string values log a type tag only. */
function formatErr(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  if (typeof e === "string") return e;
  const tag = tryCtorName(e);
  return tag ? `[${tag}]` : "[unrepresentable]";
}

/** Best-effort constructor-name tag for a caught value, never throwing — a broken
 *  `toString()`/`Symbol.toPrimitive` must not take down the error reporter. */
function tryCtorName(e: unknown): string | null {
  try {
    const name = (e as { constructor?: { name?: unknown } } | null)?.constructor
      ?.name;
    return typeof name === "string" && name.length > 0 ? name : null;
  } catch {
    return null;
  }
}

/**
 * Install the frontend logging bridge: route uncaught errors
 * into the backend log so they leave a persisted trace. Each handler is
 * fire-and-forget with a swallowed rejection — logging must never break
 * rendering or re-enter itself on failure.
 */
export function installFrontendLogger(app: App): void {
  const report = (source: string, e: unknown): void => {
    void writeLog("error", `${source}: ${formatErr(e)}`).catch(() => {});
  };
  // Vue render/watcher errors.
  app.config.errorHandler = (err: unknown, _vm: unknown, info: string) => {
    const detail =
      err instanceof Error ? `${err.message} (${info})` : formatErr(err);
    void writeLog("error", `vue: ${detail}`).catch(() => {});
  };
  // Uncaught runtime errors.
  window.addEventListener("error", (e) =>
    report("window", e.error ?? e.message),
  );
  // Unhandled promise rejections.
  window.addEventListener("unhandledrejection", (e) =>
    report("promise", e.reason),
  );
}
