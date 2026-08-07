// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { installConsoleCapture } from "./log-capture";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

// A fake console: named vi.fn refs survive `installConsoleCapture` reassigning
// `fake.<fn>` to the shim (the original vi.fn is captured as `orig` and still
// reachable through these names), so we can assert BOTH the forward (invoke)
// AND that the original method was still called.
const origDebug = vi.fn();
const origLog = vi.fn();
const origInfo = vi.fn();
const origWarn = vi.fn();
const origError = vi.fn();
const fakeConsole = {
  debug: origDebug,
  log: origLog,
  info: origInfo,
  warn: origWarn,
  error: origError,
};

installConsoleCapture(fakeConsole);

describe("installConsoleCapture", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("forwards console.error to write_log at level error and still calls through", () => {
    fakeConsole.error("boom");
    expect(invoke).toHaveBeenCalledWith("write_log", {
      level: "error",
      message: "boom",
    });
    expect(origError).toHaveBeenCalledWith("boom");
  });

  it("maps console.log → info (D2), console.info → info, console.debug → debug, console.warn → warn", () => {
    fakeConsole.log("l");
    fakeConsole.info("i");
    fakeConsole.debug("d");
    fakeConsole.warn("w");
    expect(invoke).toHaveBeenNthCalledWith(1, "write_log", {
      level: "info",
      message: "l",
    });
    expect(invoke).toHaveBeenNthCalledWith(2, "write_log", {
      level: "info",
      message: "i",
    });
    expect(invoke).toHaveBeenNthCalledWith(3, "write_log", {
      level: "debug",
      message: "d",
    });
    expect(invoke).toHaveBeenNthCalledWith(4, "write_log", {
      level: "warn",
      message: "w",
    });
  });

  it("forwards exactly once per call (no recursion / no drop)", () => {
    fakeConsole.error("once");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("joins multiple args, each secret-safe-stringified", () => {
    // Error → "name: message"; plain object → "[Ctor]" (never dumps fields).
    fakeConsole.error("ctx", new TypeError("nope"), { secret: "hunter2" });
    expect(invoke).toHaveBeenCalledWith("write_log", {
      level: "error",
      message: "ctx TypeError: nope [Object]",
    });
    expect(origError).toHaveBeenCalledWith("ctx", expect.any(TypeError), {
      secret: "hunter2",
    });
  });

  // Regression for the original bug: a copy rejection had to surface the actual
  // error. Two rejection shapes that previously produced a useless log:
  //  - an AppError ({code,message}) — used to render "[Object]" (lost both)
  //  - a bare string (IPC-layer rejection) — used to render "(no message)"
  it("REGRESSION: renders an AppError rejection as CODE: msg (not [Object])", () => {
    fakeConsole.error("copy password failed", {
      code: "DECRYPT_FAILED",
      message: "bad header",
    });
    expect(invoke).toHaveBeenCalledWith("write_log", {
      level: "error",
      message: "copy password failed DECRYPT_FAILED: bad header",
    });
  });

  it("REGRESSION: renders a bare-string rejection verbatim (not '(no message)')", () => {
    fakeConsole.error("copy password failed", "ipc: channel closed");
    expect(invoke).toHaveBeenCalledWith("write_log", {
      level: "error",
      message: "copy password failed ipc: channel closed",
    });
  });
});
