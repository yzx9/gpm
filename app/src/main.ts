// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

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
  createDraftsNotice,
  createLockState,
  createPlatform,
  createScrollLockController,
  createSecureScreen,
  createSecuritySettings,
  createToast,
  DIALOG_KEY,
  DRAFTS_NOTICE_KEY,
  LOCK_KEY,
  PLATFORM_KEY,
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
import { routes } from "./routes";
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
const draftsNotice = createDraftsNotice();
const secureScreenState = createSecureScreen();
const platformState = createPlatform();
const securitySettingsState = createSecuritySettings();
const toastState = createToast();
const dialogState = createDialog();
const scrollLock = createScrollLockController();
const backHandlerRegistry = createBackHandlerRegistry();

// The route table lives in ./routes; routes.test.ts pins its i18n-bundle
// wiring.

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
// language on the first frame. The pre-paint init script already bakes in the
// resolved (pinned-or-system) locale, so the post-mount reconcile below is a
// safety net, not the primary path.
void (async () => {
  const app = createApp(App);
  app.use(router);
  app.use(i18n);
  // Frontend logging bridge: route uncaught frontend errors into the
  // backend log so a bug report has a persisted frontend trace. Fire-and-forget
  // with a recursion guard — it must never break rendering.
  installFrontendLogger(app);
  app.provide(LOCK_KEY, lockState);
  app.provide(APP_LOCK_KEY, appLockStore);
  app.provide(DRAFTS_NOTICE_KEY, draftsNotice);
  app.provide(SECURE_SCREEN_KEY, secureScreenState);
  app.provide(PLATFORM_KEY, platformState);
  app.provide(SECURITY_SETTINGS_KEY, securitySettingsState);
  app.provide(TOAST_KEY, toastState);
  app.provide(DIALOG_KEY, dialogState);
  app.provide(SCROLL_LOCK_KEY, scrollLock);
  app.provide(BACK_HANDLER_KEY, backHandlerRegistry);

  const boot = currentLocale();
  // Mirror the boot locale to <html lang> for accessibility. The pre-paint init
  // script already sets this (so screen readers are correct from frame 0); this
  // is the JS-side authoritative set, idempotent with the inject. `setLocale`
  // mirrors it on every later switch, but the boot locale is never switched to,
  // so set it once here too.
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
  // Mount only after the initial route has resolved. Route components are lazy,
  // so mounting sooner leaves <router-view> empty until the first page's chunk
  // loads — a blank first frame. This also makes the matched component part of
  // the first render rather than an "enter after mount", so no transition fires
  // on the initial paint (the stacked router-view's START_LOCATION guard is now
  // a backstop).
  await router.isReady();
  app.mount("#app");
  // Safety net: the Rust setup closure already baked the resolved locale into
  // the WebView's pre-paint init script (see locale_init_script in
  // app_config.rs), so frame 0 renders the right language. This reconcile
  // corrects the rare case where pref.json was unreadable at setup.
  void reconcileLocaleFromBackend();
  // Safety net: the Rust setup closure already baked the pinned theme into the
  // WebView's pre-paint init script (see theme_init_script in app_config.rs), so
  // frame 0 is correctly themed. This reconcile corrects the rare case where
  // pref.json was unreadable at setup — flipping the CSS-driven System default
  // to the pinned value within a frame.
  void reconcileThemeFromBackend();
})();
