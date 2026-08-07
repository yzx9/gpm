// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { App } from "vue";

import { writeLog } from "./api";

/**
 * Frontend diagnostics capture — the plumbing that feeds the backend log
 * (`writeLog`) from the WebView. This is app-bootstrap wiring, not API: the IPC
 * wrappers themselves live in `app/src/api/log.ts`.
 *
 * Two paths feed the same backend pipeline, so a bug report has a frontend
 * trace alongside the backend one:
 * - `installConsoleCapture` wraps every `console.*` method — the app's own logs
 *   AND Tauri's injected `console.error`/`warn` — forwarded at the mapped level.
 * - `installFrontendLogger` catches what `console.*` can't (Vue render errors,
 *   uncaught `window` errors, unhandled promise rejections).
 */

/** Stringify an unknown caught value for the log. Never serializes an arbitrary
 *  object's fields: the full diagnostics log now leaves the device via Export,
 *  so a rejected promise or Vue error carrying a secret field must not be dumped.
 *  Error messages still log; non-Error, non-string values log a type tag only. */
function formatErr(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  if (typeof e === "string") return e;
  // AppError ({code, message}) — the shape a backend `Err` serializes to over
  // IPC. It is a plain object (not an Error), so without this branch it falls
  // through to the ctor tag and loses both fields — regressing the normal
  // backend-error case when the catch is routed through the console shim.
  if (
    typeof (e as { code?: unknown })?.code === "string" &&
    typeof (e as { message?: unknown })?.message === "string"
  ) {
    return `${(e as { code: string }).code}: ${(e as { message: string }).message}`;
  }
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

/** Console method → backend log level. `console.log` maps to `info` (persists by
 *  default); `console.debug` is dropped server-side unless Verbose is on. */
const CONSOLE_LEVEL: Record<
  "debug" | "log" | "info" | "warn" | "error",
  string
> = {
  debug: "debug",
  log: "info",
  info: "info",
  warn: "warn",
  error: "error",
};

/**
 * Wrap every `console.*` method to forward the call into the backend log (via
 * `writeLog`), then call the original — so the app's own `console.*` AND Tauri's
 * injected `console.error`/`warn` (`ipc/protocol.rs`) both leave a persisted
 * trace. Each forward is fire-and-forget with a re-entrancy guard; it never
 * blocks rendering or recurses into itself.
 *
 * `c` defaults to the global `console`; tests inject a fake to assert both the
 * forward and that the original is still called. This has no `app` dependency,
 * so call it at the TOP of `main.ts` — before the route guards and i18n
 * bootstrap — so nothing prints before capture is armed. `installFrontendLogger
 * (app)` (the Vue/`window` handlers) runs later, once the app exists. */
export function installConsoleCapture(
  c: Pick<Console, "debug" | "log" | "info" | "warn" | "error"> = console,
): void {
  let inShim = false; // re-entrancy guard for the synchronous forward window
  for (const fn of ["debug", "log", "info", "warn", "error"] as const) {
    const orig = c[fn].bind(c);
    c[fn] = (...args: unknown[]) => {
      if (!inShim) {
        inShim = true;
        void writeLog(CONSOLE_LEVEL[fn], args.map(formatErr).join(" ")).catch(
          () => {},
        );
        inShim = false;
      }
      orig(...args);
    };
  }
}

/**
 * Install the frontend logging bridge: route uncaught errors into the backend
 * log so they leave a persisted trace. Each handler is fire-and-forget with a
 * swallowed rejection — logging must never break rendering or re-enter itself on
 * failure.
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
