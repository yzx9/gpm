// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp, nextTick } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import { getAuthState } from "./api";
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
//    explains why. With lazy routes the raise still precedes the paint: it runs
//    here in `beforeEach`, before the component chunk even resolves.
//  - `afterEach` SETTLES the flag to the arriving route's own level AFTER the
//    new page has painted (`nextTick`), so clearing the flag never happens while
//    a departing secret page is still on screen. Lazy routes extend the time
//    between raise and settle but not the ordering — the `secureGen` token still
//    drops any settle superseded by a newer navigation.
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
  // Load the arriving route's message bundle for the current locale, alongside
  // the (lazy) component chunk. Fire-and-forget — securing above is what gates a
  // secret page's paint, not this bundle load, and a late bundle just re-renders
  // with `fallbackLocale` covering the gap. Never throws (loadBundle swallows a
  // missing bundle), so it can't block or abort the navigation.
  if (typeof to.name === "string") {
    void loadBundle(currentLocale(), to.name);
  }
  return true;
});

// `afterEach` is fire-and-forget in vue-router, and it runs even for navigations
// that aborted or were cancelled (with `failure` set). Two guards keep the
// settle honest:
//  - `failure`: a nav that never confirmed never mounted its target page, so
//    settling to its route level would desync `currentRouteSecure` from the page
//    actually on screen (and re-raise FLAG for a route we never reached).
//  - the generation token: a newer navigation can confirm while an older one is
//    still awaiting `nextTick`; a stale settle could otherwise drop FLAG_SECURE
//    for a route we've already left — and the current route by then may be a
//    secret one. Dropping superseded settles means correctness no longer depends
//    on microtask/IPC ordering.
let secureGen = 0;
router.afterEach(async (to, _from, failure) => {
  // The arriving page has mounted/painted; now drop the transition-time cover
  // and reconcile to the arriving route's own level.
  const gen = ++secureGen;
  await nextTick();
  if (failure || gen !== secureGen) return; // aborted, or superseded by a newer nav
  await secureScreenState.applySecureForRoute(!!to.meta?.secure);
});

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
  if (boot !== DEFAULT_LOCALE) {
    // loadBundle already swallows a missing bundle; the `.catch` makes the
    // bootstrap robust against any future awaited call landing here — a
    // translation load must never prevent mount (and a blank first frame).
    await loadBundle(boot, "common").catch(() => {});
  }
  app.mount("#app");
  void reconcileLocaleFromBackend();
})();
