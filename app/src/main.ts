// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import { createApp } from "vue";
import { createRouter, createWebHashHistory } from "vue-router";
import App from "./App.vue";
import "./style.css";

import {
  APP_LOCK_KEY,
  BACK_HANDLER_KEY,
  createAppLockStore,
  createBackHandlerRegistry,
  createDialog,
  createLockState,
  createNavDirection,
  createScrollLockController,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  DIALOG_KEY,
  LOCK_KEY,
  NAV_DIRECTION_KEY,
  SCROLL_LOCK_KEY,
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
import { installConsoleCapture, installFrontendLogger } from "./log-capture";
import { installRouteGuards } from "./router-guards";
import { reconcileThemeFromBackend } from "./theme";

// Arm console→backend capture FIRST, before any other module runs side effects
// (route guards, i18n bootstrap, app-shell singletons), so nothing can print to
// a console we aren't yet forwarding. No `app` dependency — the Vue/`window`
// handlers in `installFrontendLogger(app)` are wired later, once `app` exists.
installConsoleCapture();

// App-shell singletons — created once here (the composition root), provided
// app-wide, and held by direct ref where non-setup code needs them. The router
// guards below use `secureScreenState`/`toastState` directly because `inject`
// only resolves inside a component setup.
const lockState = createLockState();
const appLockStore = createAppLockStore();
const secureScreenState = createSecureScreen();
const securitySettingsState = createSecuritySettings();
const toastState = createToast();
const dialogState = createDialog();
const scrollLock = createScrollLockController();
const backHandlerRegistry = createBackHandlerRegistry();

// Screen-capture protection (FLAG_SECURE) is component-level (R031): each
// secret-bearing component acquires a claim while its secret is on screen (see
// `useSecureClaim`), so routes carry no `secure` flag and every navigation can
// animate — the secure↔capturable boundary no longer freezes the transition.
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
  },
  {
    path: "/create/preset/:presetId",
    name: "createPreset",
    component: () => import("./pages/CreatePresetPage.vue"),
  },
  {
    path: "/create/custom",
    name: "createCustom",
    component: () => import("./pages/CreateCustomPage.vue"),
  },
  {
    path: "/generate",
    name: "generate",
    component: () => import("./pages/GeneratePasswordPage.vue"),
  },
  {
    path: "/entry/:pathMatch(.*)",
    name: "entry",
    component: () => import("./pages/EntryDetailPage.vue"),
    props: true,
  },
  {
    path: "/revisions/:pathMatch(.*)",
    name: "revisions",
    component: () => import("./pages/RevisionsPage.vue"),
    props: true,
  },
  {
    path: "/edit/:pathMatch(.*)",
    name: "entryEdit",
    component: () => import("./pages/EntryEditPage.vue"),
    props: true,
  },
  {
    path: "/settings",
    name: "settings",
    component: () => import("./pages/SettingsPage.vue"),
    // `bundle` is redundant for the hub (name === "settings" already loads the
    // bundle) but is set on the hub + its sub-pages for uniformity. The sibling
    // `sshKey`/`addKey` routes intentionally keep their own namespaces (those
    // pages read `sshKey.*`/`addKey.*`, not `settings.*`).
    meta: { bundle: "settings" },
  },
  {
    path: "/settings/general",
    name: "settingsGeneral",
    component: () => import("./pages/SettingsGeneralPage.vue"),
    meta: { bundle: "settings" },
  },
  {
    path: "/settings/identity",
    name: "settingsIdentity",
    component: () => import("./pages/SettingsIdentityPage.vue"),
    meta: { bundle: "settings" },
  },
  {
    path: "/settings/repository",
    name: "settingsRepository",
    component: () => import("./pages/SettingsRepositoryPage.vue"),
    meta: { bundle: "settings" },
  },
  {
    path: "/settings/ssh-key",
    name: "sshKey",
    component: () => import("./pages/SshKeyPage.vue"),
  },
  {
    path: "/settings/pat",
    name: "pat",
    component: () => import("./pages/PatPage.vue"),
  },
  {
    path: "/settings/add-key",
    name: "addKey",
    component: () => import("./pages/AddKeyPage.vue"),
  },
  {
    path: "/history",
    name: "history",
    component: () => import("./pages/HistoryPage.vue"),
  },
  // About: overview, acknowledgements, and the auto-scanned license tree. Carries
  // no secret content, so it is NOT marked secure (capturable, like the entry
  // list / history). Reached via Settings (see SettingsPage) once that page
  // surfaces the entry; the route exists independently so it's testable now.
  {
    path: "/about",
    name: "about",
    component: () => import("./pages/AboutPage.vue"),
  },
  // Diagnostics log viewer. Standalone namespace like About — the log
  // is a self-contained viewer, not a settings category. NOT marked secure: the
  // log surfaces only entry names, which (like the entry list) carry no secret.
  {
    path: "/settings/log",
    name: "log",
    component: () => import("./pages/LogViewerPage.vue"),
  },
  // Security: plain-language summary of how gpm protects secrets. Carries no
  // secret content, so NOT marked secure (capturable, like About). Reached via
  // the Settings hub; the `security` locale namespace auto-loads by route name.
  {
    path: "/security",
    name: "security",
    component: () => import("./pages/SecurityPage.vue"),
  },
  // Permissions & data: what gpm accesses (notifications, biometrics, clipboard,
  // network, files), why, and a deep-link to system settings for the ones Android
  // suppresses after two denials. Carries no secret, so NOT marked secure
  // (capturable, like Security). The `permissions` locale namespace auto-loads
  // via meta.bundle.
  {
    path: "/settings/permissions",
    name: "settingsPermissions",
    component: () => import("./pages/SettingsPermissionsPage.vue"),
    meta: { bundle: "permissions" },
  },
];

const router = createRouter({
  history: createWebHashHistory(),
  routes,
});

// Configured-only access guard + per-route i18n bundle load. Screen-capture
// protection is component-level (R031), so the guard no longer touches
// FLAG_SECURE. Lives in `router-guards.ts` for testability.
installRouteGuards(router);

// Bootstrap. Wrapped async so the boot locale's `common` bundle can load before
// the first paint when the boot locale isn't the default (whose `common` is
// already inlined in `createI18n`) — that keeps nav/button strings in the right
// language on the first frame for, e.g., a Chinese-system user. After mount the
// backend reconcile corrects a pinned preference within one frame.
void (async () => {
  const app = createApp(App);
  app.use(router);
  app.use(i18n);
  // Frontend logging bridge: route uncaught frontend errors into the
  // backend log so a bug report has a persisted frontend trace. Fire-and-forget
  // with a recursion guard — it must never break rendering.
  installFrontendLogger(app);
  // Direction tracker for the <router-view> slide transition. Registered after
  // the auth guard. Screen-capture protection is component-level (R031), so
  // every navigation animates by direction — no secure-boundary freeze.
  const navDirection = createNavDirection(router);
  app.provide(LOCK_KEY, lockState);
  app.provide(APP_LOCK_KEY, appLockStore);
  app.provide(NAV_DIRECTION_KEY, navDirection);
  app.provide(SECURE_SCREEN_KEY, secureScreenState);
  app.provide(SECURITY_SETTINGS_KEY, securitySettingsState);
  app.provide(TOAST_KEY, toastState);
  app.provide(DIALOG_KEY, dialogState);
  app.provide(SCROLL_LOCK_KEY, scrollLock);
  app.provide(BACK_HANDLER_KEY, backHandlerRegistry);

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
  // Safety net: the Rust setup closure already baked the pinned theme into the
  // WebView's pre-paint init script (see theme_init_script in app_config.rs), so
  // frame 0 is correctly themed. This reconcile corrects the rare case where
  // pref.json was unreadable at setup — flipping the CSS-driven System default
  // to the pinned value within a frame.
  void reconcileThemeFromBackend();
})();
