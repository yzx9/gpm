# Killable git transport — cancel during connection/auth

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

User-initiated cancel can abort only the _transfer_ phase of a clone or pull today. The cancel token is polled from inside libgit2's progress callbacks, which do not run during connection and authentication negotiation — DNS resolution, the TCP connect, the TLS handshake, and SSH key exchange / authentication. For a small store (the common gpm case) that pre-transfer phase dominates wall-time, so a cancel clicked "while still connecting" sets the token but the transport never checks it until data starts flowing or the network operation times out. This RFC proposes running git network transports as a killable subprocess (or an equally interruptible transport) so a cancel can terminate the handshake immediately, in any phase.

## Why

The clone screen now shows a "Cancelling…" state the moment the user clicks, so the click is no longer silent. But during the connection blind spot the clone grinds on until the transport resolves or its timeout elapses — potentially tens of seconds to minutes on a slow, misconfigured, or unreachable remote. That is a poor escape on exactly the screen (first-run setup) where the user is most likely to mistype a URL, point at a dead host, or wait out a flaky mobile connection. Honest feedback is the interim; true handshake cancellation is the resting-state fix, and the current token-based model structurally cannot deliver it.

## Context

The shipped cancellation design uses a single boolean cancel token that the in-process libgit2 transport polls from its transfer/sideband progress callbacks; flipping it makes an in-flight transfer abort. That model is bounded to phases where a callback fires. It cannot interrupt a blocking C call that is not polling, and the worker thread running that call cannot be killed — dropping the task handle leaves both the thread and the open remote connection running until the call returns. The planned push-phase cancellation inherits the same limit.

A subprocess transport sidesteps it: terminating the child process tears down the TCP/TLS/SSH session immediately, regardless of which phase it is in. The trust boundary is unchanged — gpm already trusts a subprocess it spawns around secrets (the age plugin subprocess for hardware-key recipients is the precedent), and gopass itself drives a system `git` over the store. Cancellation kills a process; it changes no committed state, and authenticity verification of any fetched commits is unaffected.

The cost is Android-first. Desktop can usually rely on a system `git` already on `PATH`, so the work there is process plumbing and parsing progress. Android has no system `git`, so true subprocess cancellation means bundling a git binary and its transport helpers into the APK — a non-trivial build and packaging effort plus an APK-size hit — which is the reason the project chose in-process libgit2 originally. A decision this RFC defers to its design phase: whether the cancellation win justifies that cost, or whether a partial answer (subprocess on desktop; retained in-process libgit2 with honest "may lag" feedback on Android) is the right resting state.

Threat-model impact: none beyond the existing trusted-subprocess boundary. Cancellation discards a partial fetch and tears down a network session; it writes nothing and bypasses no authenticity check — the same guarantee the token model already gives for transfer-phase cancels.

## Alternatives considered

- **Status quo + "Cancelling…" feedback (the interim this RFC follows).** Accepted for now; rejected as the resting state because the blind spot remains for the most common (small-store) case, exactly where users hit it.
- **A libgit2 connect-phase timeout.** Rejected — libgit2 exposes no connect or handshake timeout to the embedder (only low-speed-transfer timeouts via git config), so the pre-transfer window stays bounded only by the OS-level TCP timeout, which is long.
- **Drop the blocking task on cancel.** Rejected — the C call is not interruptible, and the orphaned thread keeps the remote session open and leaks resources until it returns; the cancel is illusory.
- **Subprocess git transport.** The recorded direction; deferred on the Android packaging scope, not on desirability.

## Effort

Large. Desktop is modest (system git usually present, plus process plumbing and progress parsing). Android dominates the cost: bundling git and its transport helpers, the APK-size hit, and reworking the transport layer that the backend crate currently owns in-process.

## Depends on / Supersedes

- Extends the shipped clone/pull cancellation design, and the push-phase cancellation tracked in `0032-cancellable-saves`; both inherit the same callback-polling limit this RFC addresses.
