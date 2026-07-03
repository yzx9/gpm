// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Barrel re-exporting the entire frontend ↔ Rust IPC surface.
 *
 * Pages, composables, and components import only from here — never from
 * `@tauri-apps/api/*` directly:
 *
 *     import { copyPassword, type SensitiveContent } from "@/api";
 *
 * Each domain module mirrors a `src-tauri/src/` command group and co-locates
 * the IPC types its commands produce/consume; cross-domain types live in
 * {@link ./common}.
 */

export * from "./appLock";
export * from "./auth";
export * from "./authenticity";
export * from "./biometric";
export * from "./common";
export * from "./config";
export * from "./identity";
export * from "./repo";
export * from "./secrets";
export * from "./setup";
export * from "./system";
