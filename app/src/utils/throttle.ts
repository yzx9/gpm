// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

/**
 * Leading-edge throttle: invoke `fn` on the first call, then drop calls until
 * `intervalMs` has elapsed; the next call after that invokes again. No trailing
 * call. `fn` runs at most once per `intervalMs`, measured from the last
 * invocation (relative, not wall-clock windows — so no burst at a window
 * boundary). For an async `fn`, wrap as `throttle(() => void asyncFn(), ms)`.
 *
 * The first call always fires: the internal `last` starts at `-Infinity`, so
 * `Date.now() - last` is `+Infinity` on the first call regardless of the clock
 * (production's ms-since-epoch, or a fake timer pinned to 0 in tests).
 */
export function throttle<A extends unknown[]>(
  fn: (...args: A) => void,
  intervalMs: number,
): (...args: A) => void {
  // -Infinity ⇒ the first call's `now - last` is +Infinity ⇒ never throttled.
  let last = -Infinity;
  return (...args: A) => {
    const now = Date.now();
    if (now - last < intervalMs) return;
    last = now;
    fn(...args);
  };
}
