// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { vi } from "vitest";

// Mock Tauri invoke — default no-op, tests override per-call
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  addPluginListener: vi.fn().mockResolvedValue(() => {}),
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
