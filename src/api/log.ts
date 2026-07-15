// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import type { App } from "vue";

/**
 * Diagnostics logging IPC — mirrors `src-tauri/src/logging.rs` +
 * `app_config::{get,set}_log_level`. The in-app viewer (Settings → Logs) reads,
 * clears, and sets the level of the rotated log file via these.
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

/** The effective log level (`"error"|"warn"|"info"|"debug"`; default `"info"`). */
export async function getLogLevel(): Promise<string> {
  return invoke<string>("get_log_level");
}

/** Persist + apply a log level at runtime (`null` clears to the default). */
export async function setLogLevel(level: string | null): Promise<void> {
  await invoke("set_log_level", { level });
}

/** Write a frontend-emitted record into the backend log. */
export async function writeLog(level: string, message: string): Promise<void> {
  await invoke("write_log", { level, message });
}

/** Stringify an unknown caught value for the log (no secret reaches here). */
function formatErr(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    // `String()` can itself throw on a broken `toString()`/`Symbol.toPrimitive`.
    // Never let the error reporter throw — it would silently drop the log entry.
    try {
      return String(e);
    } catch {
      return "[unrepresentable]";
    }
  }
}

/**
 * Install the frontend logging bridge (RFC 0052, phase 2): route uncaught errors
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
