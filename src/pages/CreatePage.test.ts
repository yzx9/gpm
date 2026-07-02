// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { describe, it, expect, vi, beforeEach } from "vitest";
import { mount, flushPromises, type DOMWrapper } from "@vue/test-utils";
import { invoke } from "@tauri-apps/api/core";
import CreatePage from "./CreatePage.vue";
import { mountWithApp } from "@/test/appTestUtils";

const { mockPush } = vi.hoisted(() => ({ mockPush: vi.fn() }));

vi.mock("@tauri-apps/api/core");

vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({ push: mockPush, replace: vi.fn(), back: vi.fn() }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "create",
    path: "/create",
    fullPath: "/create",
  }),
}));

const WEBSITE_PRESET = {
  id: "website",
  label: "Website login",
  prefix: "websites",
  name_from: ["url", "username"],
  fields: [
    {
      key: "url",
      label: "Website URL",
      required: true,
      type: "hostname",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
    {
      key: "username",
      label: "Username",
      required: true,
      type: "string",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
    {
      key: "password",
      label: "Password",
      required: true,
      type: "password",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
  ],
} as const;

const PIN_PRESET = {
  id: "pin",
  label: "PIN Code (numerical)",
  prefix: "pin",
  name_from: ["authority", "application"],
  fields: [
    {
      key: "authority",
      label: "Authority",
      required: true,
      type: "string",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
    {
      key: "application",
      label: "Entity",
      required: true,
      type: "string",
      charset: null,
      min: null,
      max: null,
      strict: false,
    },
    {
      key: "password",
      label: "PIN",
      required: true,
      type: "password",
      charset: "0123456789",
      min: 1,
      max: 64,
      strict: false,
    },
  ],
} as const;

/** Find the first button whose text contains `needle`. */
function findButton(wrapper: ReturnType<typeof mount>, needle: string) {
  // prettier-ignore
  return wrapper.findAll("button").find((b) => b.text().includes(needle)) as
    DOMWrapper<HTMLButtonElement> | undefined;
}

/** Wire `invoke` per command; returns a fresh mounted CreatePage. */
async function mountPage(
  handlers: Record<string, () => unknown>,
  presets: readonly object[] = [WEBSITE_PRESET],
) {
  vi.mocked(invoke).mockImplementation(((cmd: string) => {
    const h = handlers[cmd];
    if (h) return Promise.resolve(h());
    if (cmd === "list_create_presets") return Promise.resolve(presets);
    return Promise.resolve(undefined);
  }) as typeof invoke);
  const { wrapper } = mountWithApp(CreatePage);
  await flushPromises(); // loadPresets on mount
  return wrapper;
}

async function fillWebsiteForm(wrapper: ReturnType<typeof mount>) {
  await wrapper.findAll(".type-card")[0]!.trigger("click");
  await wrapper.find("#f-url").setValue("example.com");
  await wrapper.find("#f-username").setValue("alice");
  await wrapper.find("#f-password").setValue("hunter2");
}

describe("CreatePage", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("loads presets on mount", async () => {
    await mountPage({});
    expect(invoke).toHaveBeenCalledWith("list_create_presets");
  });

  it("creates a secret from a preset and navigates to the list", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({ kind: "written", commit: "abc1234" }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("create_from_preset_secret", {
      presetId: "website",
      fields: { url: "example.com", username: "alice", password: "hunter2" },
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("creates a custom secret via create_secret", async () => {
    const wrapper = await mountPage({
      create_secret: () => ({ kind: "written", commit: "c1" }),
    });

    await wrapper.findAll(".type-card")[1]!.trigger("click"); // Custom secret
    await wrapper.find("#c-name").setValue("servers/db1");
    await wrapper.find("#c-content").setValue("master-password");
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("create_secret", {
      name: "servers/db1",
      content: "master-password",
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("swallows AUTH_CANCELLED silently on submit when the auth overlay is dismissed", async () => {
    // unlocked:false → identity NOT cached → submit's runWithAuth parks on the
    // auth overlay (no singleton to wipe mid-test).
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "create_from_preset_secret")
        return Promise.resolve({ kind: "written", commit: "x" });
      if (cmd === "list_create_presets")
        return Promise.resolve([WEBSITE_PRESET]);
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper, lock } = mountWithApp(CreatePage, { unlocked: false });
    await flushPromises(); // loadPresets on mount

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises(); // parked awaiting auth

    lock.cancelAuth(); // user dismissed the overlay (back)
    await flushPromises();

    // No error UI — the catch swallowed AUTH_CANCELLED; create never ran.
    expect(wrapper.text()).not.toContain("Failed to create secret");
  });

  it("on a decryptable conflict, Keep mine resolves and navigates", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: true,
      }),
      resolve_write_conflict: () => ({ commit: "def5678" }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    expect(wrapper.text()).toContain("Remote copy exists");

    const keepMine = findButton(wrapper, "Keep mine");
    expect(keepMine).toBeTruthy();
    await keepMine!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
      choice: "keep_mine",
    });
    expect(mockPush).toHaveBeenCalledWith({ name: "entries" });
  });

  it("on an undecryptable conflict, force is gated behind explicit confirmation", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: false,
      }),
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    const force = findButton(wrapper, "Keep mine anyway");
    expect(force).toBeTruthy();
    expect((force!.element as HTMLButtonElement).disabled).toBe(true);

    await wrapper.find('input[type="checkbox"]').setValue(true);
    expect((force!.element as HTMLButtonElement).disabled).toBe(false);
  });

  it("cancel resolves with `cancel` and does not navigate", async () => {
    const wrapper = await mountPage({
      create_from_preset_secret: () => ({
        kind: "conflict",
        name: "websites/example.com/alice",
        remote_decryptable: true,
      }),
      resolve_write_conflict: () => null,
    });

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();

    const cancel = findButton(wrapper, "Cancel");
    expect(cancel).toBeTruthy();
    await cancel!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
      choice: "cancel",
    });
    expect(mockPush).not.toHaveBeenCalled();
  });

  it("cancel resolves without the cached identity (resolve dropped runWithAuth — regression)", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "create_from_preset_secret")
        return Promise.resolve({
          kind: "conflict",
          name: "websites/example.com/alice",
          remote_decryptable: true,
        });
      if (cmd === "resolve_write_conflict") return Promise.resolve(null);
      if (cmd === "list_create_presets")
        return Promise.resolve([WEBSITE_PRESET]);
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper, lock } = mountWithApp(CreatePage);
    await flushPromises();

    await fillWebsiteForm(wrapper);
    await wrapper.find("form").trigger("submit");
    await flushPromises();
    expect(wrapper.text()).toContain("Remote copy exists");

    // Wipe the cached identity AFTER the conflict is up. `resolve` no longer
    // wraps in runWithAuth (cancel/keep_remote need no identity), so this still
    // resolves — previously it would have parked on the auth overlay forever.
    lock.setLocked(true);

    const cancel = findButton(wrapper, "Cancel");
    expect(cancel).toBeTruthy();
    await cancel!.trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("resolve_write_conflict", {
      choice: "cancel",
    });
  });

  it("fills the password field with the generated value", async () => {
    const wrapper = await mountPage({ generate_password: () => "GENPW123" });
    await wrapper.findAll(".type-card")[0]!.trigger("click"); // Website
    await wrapper
      .find('button[aria-label="Generate password"]')
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("generate_password", {
      mode: "random",
      charset: null,
      minLen: null,
      maxLen: null,
      strict: false,
    });
    expect(
      (wrapper.find("#f-password").element as HTMLInputElement).value,
    ).toBe("GENPW123");
  });

  it("a PIN field has no mode selector and generates over its digit charset", async () => {
    const wrapper = await mountPage({ generate_password: () => "428917" }, [
      PIN_PRESET,
    ]);
    await wrapper.findAll(".type-card")[0]!.trigger("click"); // PIN

    // charset-locked field → no mode <select>.
    expect(wrapper.find('select[aria-label="Password style"]').exists()).toBe(
      false,
    );

    await wrapper
      .find('button[aria-label="Generate password"]')
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith("generate_password", {
      mode: "random",
      charset: "0123456789",
      minLen: 1,
      maxLen: 64,
      strict: false,
    });
    expect(
      (wrapper.find("#f-password").element as HTMLInputElement).value,
    ).toBe("428917");
  });

  it("the mode selector changes which generator runs", async () => {
    const wrapper = await mountPage({
      generate_password: () => "correct horse battery staple",
    });
    await wrapper.findAll(".type-card")[0]!.trigger("click");
    await wrapper.find('select[aria-label="Password style"]').setValue("xkcd");
    await wrapper
      .find('button[aria-label="Generate password"]')
      .trigger("click");
    await flushPromises();

    expect(invoke).toHaveBeenCalledWith(
      "generate_password",
      expect.objectContaining({ mode: "xkcd" }),
    );
  });

  it("a generate error shows a toast and leaves the field untouched", async () => {
    vi.mocked(invoke).mockImplementation(((cmd: string) => {
      if (cmd === "generate_password") {
        return Promise.reject({ code: "STORE_ERROR", message: "RNG down" });
      }
      if (cmd === "list_create_presets") {
        return Promise.resolve([WEBSITE_PRESET]);
      }
      return Promise.resolve(undefined);
    }) as typeof invoke);
    const { wrapper } = mountWithApp(CreatePage);
    await flushPromises();

    await wrapper.findAll(".type-card")[0]!.trigger("click");
    await wrapper.find("#f-password").setValue("keepme");
    await wrapper
      .find('button[aria-label="Generate password"]')
      .trigger("click");
    await flushPromises();

    expect(wrapper.text()).toContain("RNG down");
    expect(
      (wrapper.find("#f-password").element as HTMLInputElement).value,
    ).toBe("keepme");
  });

  it("disables Save while a password is generating", async () => {
    let resolveGen!: (v: string) => void;
    const genPromise = new Promise<string>((r) => {
      resolveGen = r;
    });
    const wrapper = await mountPage({ generate_password: () => genPromise });
    await fillWebsiteForm(wrapper); // makes canSubmit true
    const submit = () => wrapper.find('form button[type="submit"]');

    expect((submit().element as HTMLButtonElement).disabled).toBe(false);

    await wrapper
      .find('button[aria-label="Generate password"]')
      .trigger("click");
    await flushPromises();
    expect((submit().element as HTMLButtonElement).disabled).toBe(true);

    resolveGen("done");
    await flushPromises();
    expect((submit().element as HTMLButtonElement).disabled).toBe(false);
  });

  it("toggles a password field between masked and revealed", async () => {
    const wrapper = await mountPage({ generate_password: () => "s3cr3t" });
    await wrapper.findAll(".type-card")[0]!.trigger("click"); // Website
    const input = wrapper.find("#f-password");

    expect((input.element as HTMLInputElement).type).toBe("password");
    await wrapper.find('button[aria-label="Show"]').trigger("click");
    await flushPromises();
    expect((input.element as HTMLInputElement).type).toBe("text");
    expect(wrapper.find('button[aria-label="Hide"]').exists()).toBe(true);

    await wrapper.find('button[aria-label="Hide"]').trigger("click");
    await flushPromises();
    expect((input.element as HTMLInputElement).type).toBe("password");
  });
});
