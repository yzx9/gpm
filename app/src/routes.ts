// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

// The app's route table. routes.test.ts pins the route→i18n-bundle wiring
// (the router guard resolves each route's namespace as `meta.bundle ?? name`).
//
// Screen-capture protection (FLAG_SECURE) is component-level (R031): each
// secret-bearing component acquires a claim while its secret is on screen (see
// `useSecureClaim`), so routes carry no `secure` flag and every navigation
// can animate.
//
// Route components are lazy so each page's JS chunk (and its message bundle,
// loaded by the `beforeEach` guard) loads on demand — keeping the initial
// payload small.
export const routes = [
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
  // The create-flow steps read `create.*` strings; without `bundle` a cold
  // deep-link past /create would look for a non-existent `createPreset` bundle.
  {
    path: "/create/preset/:presetId",
    name: "createPreset",
    component: () => import("./pages/CreatePresetPage.vue"),
    meta: { bundle: "create" },
  },
  // Cf. createPreset above — `create.*` strings, own route name.
  {
    path: "/create/custom",
    name: "createCustom",
    component: () => import("./pages/CreateCustomPage.vue"),
    meta: { bundle: "create" },
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
  // Reads `entry.*` strings, not `entryEdit.*` — same cold-deep-link rationale
  // as the create-flow steps above.
  {
    path: "/edit/:pathMatch(.*)",
    name: "entryEdit",
    component: () => import("./pages/EntryEditPage.vue"),
    props: true,
    meta: { bundle: "entry" },
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
  // About: the overview (what gpm is, design goals, version + update dialog).
  // Carries no secret content, so it is NOT marked secure (capturable, like
  // the entry list / history). Reached via Settings (see SettingsPage).
  {
    path: "/about",
    name: "about",
    component: () => import("./pages/AboutPage.vue"),
  },
  // Acknowledgements: the projects gpm builds on. Informational (like About),
  // not a settings category, and carries no secret — NOT marked secure. The
  // strings live in the `about` namespace, so meta.bundle points there (the
  // route name would otherwise imply a non-existent bundle and render raw
  // keys; cf. settingsPermissions → `permissions`).
  {
    path: "/settings/acknowledgements",
    name: "settingsAcknowledgements",
    component: () => import("./pages/AcknowledgementsPage.vue"),
    meta: { bundle: "about" },
  },
  // Licenses: the auto-scanned open-source license inventory. Same rationale
  // as acknowledgements above — informational, no secret, `about` bundle.
  {
    path: "/settings/licenses",
    name: "settingsLicenses",
    component: () => import("./pages/LicensesPage.vue"),
    meta: { bundle: "about" },
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
