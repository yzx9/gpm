// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

import { useAppLockState } from "./useAppLockState";
import { useLockState } from "./useLockState";

/**
 * Handle over the two lock signals — the identity hard lock (`useLockState`)
 * and the app-gate re-lock (`useAppLockState`).
 */
export function useLockSignals() {
  const identity = useLockState();
  const gate = useAppLockState();
  return {
    /**
     * Subscribe to both lock signals with one callback: the single definition
     * of "a lock happened, clear". Without the gate half, a gate re-lock
     * raises the mask but never fires the eager-secret wipers (issue #20).
     *
     * Each subscription auto-removes on scope dispose (both underlying
     * registries do their own `onScopeDispose`); the returned fn removes both
     * explicitly.
     *
     * @returns an unsubscribe function
     */
    onAnyLock(cb: () => void): () => void {
      const offIdentity = identity.onLock(cb);
      const offGate = gate.onAppLock(cb);
      return () => {
        offIdentity();
        offGate();
      };
    },
  };
}
