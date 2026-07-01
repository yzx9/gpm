// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp, nextTick } from "vue";
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
import GeneratePasswordPage from "./pages/GeneratePasswordPage.vue";
import { globalToast, useSecureScreen } from "./composables";

const { applySecureForRoute, raiseSecureForRoute } = useSecureScreen();

// `meta.secure` marks routes that render decrypted/generated secrets or
// credentials — the router guard sets Android FLAG_SECURE on these (when the
// user's master toggle is on). The entry list (names only) and history
// (commit signatures) carry no secret content and stay capturable.
const routes = [
  { path: "/", redirect: "/entries" },
  {
    path: "/setup",
    name: "setup",
    component: SetupPage,
    meta: { secure: true },
  },
  { path: "/entries", name: "entries", component: EntryListPage },
  {
    path: "/create",
    name: "create",
    component: CreatePage,
    meta: { secure: true },
  },
  {
    path: "/generate",
    name: "generate",
    component: GeneratePasswordPage,
    meta: { secure: true },
  },
  {
    path: "/entry/:pathMatch(.*)",
    name: "entry",
    component: EntryDetailPage,
    props: true,
    meta: { secure: true },
  },
  {
    path: "/settings",
    name: "settings",
    component: SettingsPage,
    meta: { secure: true },
  },
  { path: "/history", name: "history", component: HistoryPage },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Navigation guard: configured-only access + per-route screen-capture
// protection. The locked state is enforced by the global `UnlockModal` overlay
// (driven by `useLockState`), not by a route redirect, so the user
// re-authenticates in place instead of being navigated off their page.
//
// Screen-capture is split across the two hooks so a secret page is NEVER shown
// unprotected — not on arrival, and not while departing:
//  - `beforeEach` RAISES the flag to cover BOTH the route being left and the one
//    entered (awaited before the target page mounts/paints). A secret route we
//    failed to secure is never rendered — the nav aborts and a global toast
//    explains why.
//  - `afterEach` SETTLES the flag to the arriving route's own level AFTER the
//    new page has painted (`nextTick`), so clearing the flag never happens while
//    a departing secret page is still on screen.
router.beforeEach(async (to, from) => {
  if (to.name !== "setup") {
    try {
      const auth = await invoke<AuthState>("get_auth_state");
      if (!auth.configured) return { name: "setup" }; // /setup leg reconciles
    } catch {
      return { name: "setup" };
    }
  }

  // Cover the departing page too during the swap. `from.meta` is absent on the
  // initial navigation, so the first nav covers only the arriving route.
  const cover = !!(to.meta?.secure || from.meta?.secure);
  const secureOk = await raiseSecureForRoute(cover);
  if (to.meta?.secure && !secureOk) {
    globalToast("Couldn't secure screen — try again");
    return false;
  }
  return true;
});

router.afterEach(async (to) => {
  // The arriving page has mounted/painted; now drop the transition-time cover
  // and reconcile to the arriving route's own level.
  await nextTick();
  await applySecureForRoute(!!to.meta?.secure);
});

createApp(App).use(router).mount("#app");
