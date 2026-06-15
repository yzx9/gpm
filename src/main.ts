// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import App from "./App.vue";
import "./assets/main.css";

import type { AuthState } from "./types";

import SetupPage from "./pages/SetupPage.vue";
import UnlockPage from "./pages/UnlockPage.vue";
import EntryListPage from "./pages/EntryListPage.vue";
import EntryDetailPage from "./pages/EntryDetailPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import HistoryPage from "./pages/HistoryPage.vue";

const routes = [
  { path: "/", redirect: "/setup" },
  { path: "/setup", name: "setup", component: SetupPage },
  { path: "/unlock", name: "unlock", component: UnlockPage },
  { path: "/entries", name: "entries", component: EntryListPage },
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

// Navigation guard: redirect based on auth state
router.beforeEach(async (to) => {
  // Allow access to setup page always
  if (to.name === "setup") return true;

  try {
    const auth = await invoke<AuthState>("get_auth_state");

    if (!auth.configured) return { name: "setup" };
    if (auth.encrypted && !auth.unlocked) return { name: "unlock" };
  } catch {
    return { name: "setup" };
  }

  return true;
});

// Listen for identity-locked event (timer expiry) and redirect
listen("identity-locked", () => {
  const currentRoute = router.currentRoute.value;
  // Don't redirect if already on unlock or setup
  if (currentRoute.name !== "unlock" && currentRoute.name !== "setup") {
    router.push({ name: "unlock" });
  }
});

createApp(App).use(router).mount("#app");
