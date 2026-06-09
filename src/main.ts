// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { invoke } from "@tauri-apps/api/core";
import App from "./App.vue";
import "./assets/main.css";

import SetupPage from "./pages/SetupPage.vue";
import EntryListPage from "./pages/EntryListPage.vue";
import EntryDetailPage from "./pages/EntryDetailPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";

const routes = [
  { path: "/", redirect: "/setup" },
  { path: "/setup", name: "setup", component: SetupPage },
  { path: "/entries", name: "entries", component: EntryListPage },
  {
    path: "/entry/:pathMatch(.*)",
    name: "entry",
    component: EntryDetailPage,
    props: true,
  },
  { path: "/settings", name: "settings", component: SettingsPage },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Navigation guard: redirect to setup if not configured
router.beforeEach(async (to) => {
  if (to.name === "setup") return true;

  try {
    const configured = await invoke<boolean>("is_configured");
    if (!configured) return { name: "setup" };
  } catch {
    return { name: "setup" };
  }
  return true;
});

createApp(App).use(router).mount("#app");
