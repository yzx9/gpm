// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import "./style.css";

import {
  APP_LOCK_KEY,
  createAppLockStore,
  createLockState,
  createNavDirection,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  LOCK_KEY,
  NAV_DIRECTION_KEY,
  SECURE_SCREEN_KEY,
  SECURITY_SETTINGS_KEY,
  TOAST_KEY,
} from "./composables";
import {
  currentLocale,
  DEFAULT_LOCALE,
  i18n,
  loadBundle,
  reconcileLocaleFromBackend,
} from "./i18n";
import { installRouteGuards } from "./router-guards";

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
//
// Route components are lazy so each page's JS chunk (and its message bundle,
// loaded by the `beforeEach` guard) loads on demand — keeping the initial
// payload small.
const routes = [
  { path: "/", redirect: "/entries" },
  {
    path: "/setup",
    name: "setup",
    component: () => import("./pages/SetupPage.vue"),
    meta: { secure: true },
  },
  {
    path: "/entries",
    name: "entries",
    component: () => import("./pages/EntryListPage.vue"),
  },
  {
    path: "/create",
    name: "create",
    component: () => import("./pages/CreatePage.vue"),
    meta: { secure: true },
  },
  {
    path: "/generate",
    name: "generate",
    component: () => import("./pages/GeneratePasswordPage.vue"),
    meta: { secure: true },
  },
  {
    path: "/entry/:pathMatch(.*)",
    name: "entry",
    component: () => import("./pages/EntryDetailPage.vue"),
    props: true,
    meta: { secure: true },
  },
  {
    path: "/settings",
    name: "settings",
    component: () => import("./pages/SettingsPage.vue"),
    meta: { secure: true },
  },
  {
    path: "/history",
    name: "history",
    component: () => import("./pages/HistoryPage.vue"),
  },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Per-route screen-capture protection + configured-only access. The guards
// live in `router-guards.ts` (extracted so the raise-before-paint invariant is
// testable). Registered before the nav-direction afterEach (installed in the
// bootstrap below) so the secure settle runs first.
installRouteGuards(router, secureScreenState, toastState);

// Bootstrap. Wrapped async so the boot locale's `common` bundle can load before
// the first paint when the boot locale isn't the default (whose `common` is
// already inlined in `createI18n`) — that keeps nav/button strings in the right
// language on the first frame for, e.g., a Chinese-system user. After mount the
// backend reconcile corrects a pinned preference within one frame.
void (async () => {
  const app = createApp(App);
  app.use(router);
  app.use(i18n);
  // Direction tracker for the <router-view> slide transition. Registered after
  // the secure-screen guards so its afterEach runs alongside them. The
  // secure-boundary gate inside it keeps FLAG_SECURE safe (see useNavDirection).
  const navDirection = createNavDirection(router);
  app.provide(LOCK_KEY, lockState);
  app.provide(APP_LOCK_KEY, appLockStore);
  app.provide(NAV_DIRECTION_KEY, navDirection);
  app.provide(SECURE_SCREEN_KEY, secureScreenState);
  app.provide(SECURITY_SETTINGS_KEY, securitySettingsState);
  app.provide(TOAST_KEY, toastState);

  const boot = currentLocale();
  // Mirror the boot locale to <html lang> for accessibility and :lang() CSS.
  // `setLocale` does this on every switch, but the boot locale is never switched
  // to (the reconcile is a no-op when it already matches), so set it once here
  // or the first frame renders without a lang attribute.
  document.documentElement.lang = boot;
  if (boot !== DEFAULT_LOCALE) {
    // loadBundle already swallows a missing bundle; the `.catch` makes the
    // bootstrap robust against any future awaited call landing here — a
    // translation load must never prevent mount (and a blank first frame).
    await loadBundle(boot, "common").catch(() => {});
  }
  // Native-prompt text loads for every locale — only `common` is
  // inlined, so `native` always loads async. Awaited BEFORE mount so the
  // cold-start AppLockOverlay's unlock button can't fire before the prompt text
  // resolves: a fast tap would otherwise send untranslated/key strings to the
  // native BiometricPrompt. Like `common`, a failed load never blocks mount.
  await loadBundle(boot, "native").catch(() => {});
  app.mount("#app");
  void reconcileLocaleFromBackend();
})();
