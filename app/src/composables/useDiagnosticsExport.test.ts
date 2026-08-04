// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { exportDiagnostics } from "@/api/log";
import { mountWithApp } from "@/test/appTestUtils";
import { Z } from "@/zTiers";
import { flushPromises } from "@vue/test-utils";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { defineComponent } from "vue";
import { useDiagnosticsExport } from "./useDiagnosticsExport";

// The composable only consumes exportDiagnostics from @/api/log; mock just that.
vi.mock("@/api/log", () => ({
  exportDiagnostics: vi.fn(),
}));

// Host that drives the composable behind a click. mountWithApp provides
// DIALOG_KEY/TOAST_KEY (the composable's useDialog/useToast) plus the global
// test i18n its useI18n/t() resolve against.
const Host = defineComponent({
  setup() {
    const { exporting, runExport } = useDiagnosticsExport();
    const go = () => {
      void runExport({ z: Z.gate });
    };
    return { exporting, go };
  },
  template: `<button @click="go">go</button>`,
});

describe("useDiagnosticsExport", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    vi.mocked(exportDiagnostics).mockResolvedValue(undefined);
  });

  it("on confirm: exports, toasts success, and forwards z to the confirm dialog", async () => {
    const { wrapper, dialog, toast } = mountWithApp(Host);
    const successSpy = vi.spyOn(toast.toast, "success");
    // mountWithApp spies dialog.confirm to resolve true by default.

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(dialog.dialog.confirm).toHaveBeenCalledWith(
      expect.objectContaining({ z: Z.gate }),
    );
    expect(exportDiagnostics).toHaveBeenCalledTimes(1);
    expect(successSpy).toHaveBeenCalled();
  });

  it("on cancel: does not export", async () => {
    const { wrapper, dialog } = mountWithApp(Host);
    vi.mocked(dialog.dialog.confirm).mockResolvedValue(false);

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(exportDiagnostics).not.toHaveBeenCalled();
  });

  it("a dismissed SAF picker (CANCELLED) is a silent cancel — no danger toast", async () => {
    const { wrapper, toast } = mountWithApp(Host);
    const dangerSpy = vi.spyOn(toast.toast, "danger");
    vi.mocked(exportDiagnostics).mockRejectedValue({ code: "CANCELLED" });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(exportDiagnostics).toHaveBeenCalled();
    expect(dangerSpy).not.toHaveBeenCalled();
  });

  it("a real export error toasts danger with the error message", async () => {
    const { wrapper, toast } = mountWithApp(Host);
    const dangerSpy = vi.spyOn(toast.toast, "danger");
    vi.mocked(exportDiagnostics).mockRejectedValue({
      code: "IO",
      message: "disk full",
    });

    await wrapper.find("button").trigger("click");
    await flushPromises();

    expect(dangerSpy).toHaveBeenCalledWith("disk full");
  });

  it("guards against re-entry while an export is in flight", async () => {
    const { wrapper } = mountWithApp(Host);
    // Stall the export so the first run sits in its `await exportDiagnostics`
    // with exporting=true — the re-entry guard's actual window (after confirm).
    let resolveExport!: () => void;
    vi.mocked(exportDiagnostics).mockImplementation(
      () =>
        new Promise((res) => {
          resolveExport = () => res(undefined);
        }),
    );

    await wrapper.find("button").trigger("click"); // confirm(true) → exporting=true → await export
    await flushPromises();
    await wrapper.find("button").trigger("click"); // exporting is true → guard no-op
    await flushPromises();

    expect(exportDiagnostics).toHaveBeenCalledTimes(1);
    resolveExport();
    await flushPromises();
  });
});
