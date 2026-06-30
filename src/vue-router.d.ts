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
  }
}
