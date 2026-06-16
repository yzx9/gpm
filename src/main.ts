// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";
import "./assets/main.css";

import type { AuthState } from "./types";

import SetupPage from "./pages/SetupPage.vue";
import EntryListPage from "./pages/EntryListPage.vue";
import EntryDetailPage from "./pages/EntryDetailPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import HistoryPage from "./pages/HistoryPage.vue";
import CreatePage from "./pages/CreatePage.vue";

const routes = [
  { path: "/", redirect: "/entries" },
  { path: "/setup", name: "setup", component: SetupPage },
  { path: "/entries", name: "entries", component: EntryListPage },
  { path: "/create", name: "create", component: CreatePage },
  {
    path: "/entry/:pathMatch(.*)",
    name: "entry",
    component: EntryDetailPage,
    props: true,
  },
  { path: "/settings", name: "settings", component: SettingsPage },
  { path: "/history", name: "history", component: HistoryPage },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Navigation guard: configured-only. The locked state is enforced by the global
// `UnlockModal` overlay (driven by `useLockState`), not by a route redirect, so
// the user re-authenticates in place instead of being navigated off their page.
router.beforeEach(async (to) => {
  // Allow access to setup page always
  if (to.name === "setup") return true;

  try {
    const auth = await invoke<AuthState>("get_auth_state");

    if (!auth.configured) return { name: "setup" };
  } catch {
    return { name: "setup" };
  }

  return true;
});

createApp(App).use(router).mount("#app");
