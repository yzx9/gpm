// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { vi } from "vitest";

// Mock Tauri invoke — default no-op, tests override per-call
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  addPluginListener: vi.fn().mockResolvedValue(() => {}),
}));

// Mock Tauri event listener — default resolves a no-op unlisten; tests grab the
// handler from `vi.mocked(listen).mock.calls[n][1]` to fire events.
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

// Mock vue-router
vi.mock("vue-router", () => ({
  createRouter: vi.fn(),
  createWebHashHistory: vi.fn(),
  useRouter: () => ({
    push: vi.fn(),
    replace: vi.fn(),
    back: vi.fn(),
  }),
  useRoute: () => ({
    params: {},
    query: {},
    name: "",
    path: "/",
    fullPath: "/",
  }),
}));

// jsdom lacks window.confirm
globalThis.confirm = vi.fn(() => true);

// jsdom lacks IntersectionObserver — provide a stub so components that wire up
// infinite scroll can mount. The explicit "Load more" button is the
// always-available path; the observer is only a progressive enhancement.
class IntersectionObserverStub {
  constructor(_cb: IntersectionObserverCallback) {}
  observe(): void {}
  unobserve(): void {}
  disconnect(): void {}
  takeRecords(): IntersectionObserverEntry[] {
    return [];
  }
}
globalThis.IntersectionObserver =
  IntersectionObserverStub as unknown as typeof IntersectionObserver;
