// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import type { AppError } from "@/api";
import { exportDiagnostics } from "@/api/log";
import { useDialog, useToast, type ZTier } from "@/composables";
import { ref } from "vue";
import { useI18n } from "vue-i18n";

/**
 * Shared diagnostics-export flow: confirm → SAF save → toast feedback.
 *
 * Extracted so Settings → Logs and the App Lock screen share one
 * implementation instead of two near-identical copies. The confirm and toast
 * are the app-wide surfaces from `useDialog`/`useToast`.
 *
 * When the caller sits above the default overlay tier — e.g. the opaque
 * `Z.gate` lock screen — pass `{ z: Z.gate }` so the confirm stacks above the
 * caller's own surface; otherwise the caller would paint over and hide its own
 * confirm dialog.
 *
 * The backend `export_diagnostics` is safe to run while the app is locked: it
 * omits repo_config.json/behavior.json and redacts credentials (see
 * diagnostics_export.rs). The SAF save picker is a native OS surface that
 * always paints above the WebView, and dismissing it surfaces as a `CANCELLED`
 * error treated here as a silent cancel.
 */
export interface DiagnosticsExportOptions {
  /** Stacking tier forwarded to the confirm dialog's `BaseModalShell`. Defaults
   *  to `Z.overlay` (via BaseModalShell) when omitted. Pass `Z.gate` when
   *  exporting from the lock screen. */
  z?: ZTier | number;
}

export function useDiagnosticsExport() {
  const { t } = useI18n();
  const { toast } = useToast();
  const { dialog } = useDialog();
  const exporting = ref(false);

  async function runExport(opts: DiagnosticsExportOptions = {}): Promise<void> {
    // Re-entry guard, armed BEFORE the confirm: the confirm + SAF picker are
    // both awaited, and a second tap while either is pending would otherwise
    // stack a second confirm. Arming here (not after confirm) also disables the
    // caller's link (`:loading="exporting"`) for the whole window, not just the
    // export, so the link can't be double-tapped while its own confirm is up.
    if (exporting.value) return;
    exporting.value = true;
    try {
      const confirmed = await dialog.confirm({
        message: t("log.exportConfirm"),
        confirmLabel: t("common.button.export"),
        z: opts.z,
      });
      if (!confirmed) return;
      await exportDiagnostics();
      toast.success(t("log.exported"));
    } catch (e) {
      const appError = e as AppError;
      // A dismissed SAF save dialog is a silent cancel, not an error.
      if (appError?.code === "CANCELLED") return;
      toast.danger(appError?.message || t("log.exportFailed"));
    } finally {
      exporting.value = false;
    }
  }

  return { exporting, runExport };
}
