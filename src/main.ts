// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp, nextTick } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { getAuthState } from "./api";
import App from "./App.vue";
import "./assets/main.css";

import {
  APP_LOCK_KEY,
  createAppLockStore,
  createLockState,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  LOCK_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
  TOAST_KEY,
} from "./composables";
import CreatePage from "./pages/CreatePage.vue";
import EntryDetailPage from "./pages/EntryDetailPage.vue";
import EntryListPage from "./pages/EntryListPage.vue";
import GeneratePasswordPage from "./pages/GeneratePasswordPage.vue";
import HistoryPage from "./pages/HistoryPage.vue";
import SettingsPage from "./pages/SettingsPage.vue";
import SetupPage from "./pages/SetupPage.vue";

// App-shell singletons — created once here (the composition root), provided
// app-wide, and held by direct ref where non-setup code needs them. The router
// guards below use `secureScreenState`/`toastState` directly because `inject`
// only resolves inside a component setup.
const lockState = createLockState();
const appLockStore = createAppLockStore();
const secureScreenState = createSecureScreen();
const securitySettingsState = createSecuritySettings();
const toastState = createToast();

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
      const auth = await getAuthState();
      if (!auth.configured) return { name: "setup" }; // /setup leg reconciles
    } catch {
      return { name: "setup" };
    }
  }

  // Cover the departing page too during the swap. `from.meta` is absent on the
  // initial navigation, so the first nav covers only the arriving route.
  const cover = !!(to.meta?.secure || from.meta?.secure);
  const secureOk = await secureScreenState.raiseSecureForRoute(cover);
  if (to.meta?.secure && !secureOk) {
    toastState.toast.danger("Couldn't secure screen — try again");
    return false;
  }
  return true;
});

router.afterEach(async (to) => {
  // The arriving page has mounted/painted; now drop the transition-time cover
  // and reconcile to the arriving route's own level.
  await nextTick();
  await secureScreenState.applySecureForRoute(!!to.meta?.secure);
});

const app = createApp(App);
app.use(router);
app.provide(LOCK_KEY, lockState);
app.provide(APP_LOCK_KEY, appLockStore);
app.provide(SECURE_SCREEN_KEY, secureScreenState);
app.provide(SECURITY_SETTINGS_KEY, securitySettingsState);
app.provide(TOAST_KEY, toastState);
app.mount("#app");
