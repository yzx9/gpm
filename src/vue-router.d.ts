// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

import "vue-router";

declare module "vue-router" {
  interface RouteMeta {
    /**
     * Whether this route should set Android `FLAG_SECURE` (when the master
     * toggle is on). Sensitive pages — those that render decrypted/generated
     * secrets or credentials — set this; the entry list and history do not.
     */
    secure?: boolean;
    /**
     * i18n namespace (the `@/locales/<locale>/<ns>.json` bundle) the
     * `beforeEach` guard preloads for this route. Defaults to the route name.
     * Set when a route's strings live under a different namespace than its
     * name — e.g. the settings sub-pages (`/settings/general` etc.) all share
     * the `settings` bundle, so they set `bundle: "settings"` instead of each
     * needing its own `<routeName>.json` and a re-key of every `settings.*`
     * string.
     */
    bundle?: string;
  }
}
