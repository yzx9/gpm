// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import type { AppError } from "@/api";
import { exportRepository, type RepoReadmeEntry } from "@/api/repo";
import { useDialog, useToast, type ZTier } from "@/composables";
import { DEFAULT_LOCALE, SUPPORTED_LOCALES, i18n, loadBundle } from "@/i18n";
import { ref } from "vue";
import { useI18n } from "vue-i18n";

/**
 * Repository-export flow (R078): confirm → build one README per locale → SAF
 * save the `gpm-export.zip` archive → toast feedback.
 *
 * Mirrors {@link useDiagnosticsExport}, with one extra step: the README body is
 * localized in the WebView for **every** supported locale (the Rust backend does
 * no i18n) and passed in, so the archive carries one README file per locale
 * (`README.md`, `README.zh-cn.md`, …) regardless of the user's active language.
 *
 * The locale set is the single source of truth in `@/i18n` (`SUPPORTED_LOCALES`):
 * adding a locale there + shipping its `settings.repo.exportReadme` string is the
 * only change needed — neither this composable nor the backend names a locale.
 *
 * The backend `export_repository` is safe under App Lock: bundling packs git
 * objects without decrypting, so no identity is touched. The SAF save picker is a
 * native OS surface that always paints above the `WebView`; dismissing it surfaces
 * as a `CANCELLED` error treated here as a silent cancel.
 */
export interface RepoExportOptions {
  /** Stacking tier forwarded to the confirm dialog's `BaseModalShell`. Defaults
   *  to `Z.overlay` when omitted. */
  z?: ZTier | number;
}

/** The archive filename for `locale`'s README: the default locale gets the bare
 *  `README.md` (the one a finder / GitHub reads first); others get
 *  `README.<locale>.md`. */
function readmeFile(locale: string): string {
  return locale === DEFAULT_LOCALE
    ? "README.md"
    : `README.${locale.toLowerCase()}.md`;
}

/** Build one README entry per supported locale that has an `exportReadme` body,
 *  in `SUPPORTED_LOCALES` order. An empty body (a locale not yet carrying the
 *  README string) is skipped rather than emitting an empty file. */
async function readmeEntries(): Promise<RepoReadmeEntry[]> {
  const out: RepoReadmeEntry[] = [];
  for (const locale of SUPPORTED_LOCALES) {
    await loadBundle(locale, "settings");
    const msg = i18n.global.getLocaleMessage(locale) as {
      settings?: { repo?: { exportReadme?: string } };
    };
    const body = msg?.settings?.repo?.exportReadme;
    if (body) {
      out.push({ name: readmeFile(locale), body });
    }
  }
  return out;
}

export function useRepoExport() {
  const { t } = useI18n();
  const { toast } = useToast();
  const { dialog } = useDialog();
  const exporting = ref(false);

  async function runExport(opts: RepoExportOptions = {}): Promise<void> {
    // Re-entry guard, armed BEFORE the confirm (see useDiagnosticsExport): the
    // confirm + SAF picker are both awaited, and a second tap while either is
    // pending would stack a second confirm.
    if (exporting.value) return;
    exporting.value = true;
    try {
      const confirmed = await dialog.confirm({
        message: t("settings.repo.exportConfirm"),
        confirmLabel: t("common.button.export"),
        z: opts.z,
      });
      if (!confirmed) return;
      // Localize the README for every supported locale (the backend writes one
      // file each — README.md, README.zh-cn.md, …).
      await exportRepository(await readmeEntries());
      toast.success(t("settings.repo.exported"));
    } catch (e) {
      const appError = e as AppError;
      // A dismissed SAF save dialog is a silent cancel, not an error.
      if (appError?.code === "CANCELLED") return;
      toast.danger(appError?.message || t("settings.repo.exportFailed"));
    } finally {
      exporting.value = false;
    }
  }

  return { exporting, runExport };
}
