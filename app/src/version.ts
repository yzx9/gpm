// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

/** The app version, resolved from the workspace package.json at build time
 *  (resolveJsonModule). Single source for every component that displays it —
 *  a per-component relative `package.json` import is fragile under file moves. */
import pkg from "../package.json";

export const version: string = pkg.version;
