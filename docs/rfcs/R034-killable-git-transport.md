# Killable git transport — cancel during connection/auth

**Priority:** P3
**Status:** Declined
**Phase:** Dropped

## What

User-initiated cancel can abort only the _transfer_ phase of a clone or pull today. The cancel token is polled from inside libgit2's progress callbacks, which do not run during connection and authentication negotiation — DNS resolution, the TCP connect, the TLS handshake, and SSH key exchange / authentication. For a small store (the common gpm case) that pre-transfer phase dominates wall-time, so a cancel clicked "while still connecting" sets the token but the transport never checks it until data starts flowing or the network operation times out. This RFC proposes running git network transports as a killable subprocess (or an equally interruptible transport) so a cancel can terminate the handshake immediately, in any phase.

## Why

The clone screen now shows a "Cancelling…" state the moment the user clicks, so the click is no longer silent. But during the connection blind spot the clone grinds on until the transport resolves or its timeout elapses — potentially tens of seconds to minutes on a slow, misconfigured, or unreachable remote. That is a poor escape on exactly the screen (first-run setup) where the user is most likely to mistype a URL, point at a dead host, or wait out a flaky mobile connection. Honest feedback is the interim; true handshake cancellation is the resting-state fix, and the current token-based model structurally cannot deliver it.

## Context

The shipped cancellation design uses a single boolean cancel token that the in-process libgit2 transport polls from its transfer/sideband progress callbacks; flipping it makes an in-flight transfer abort. That model is bounded to phases where a callback fires. It cannot interrupt a blocking C call that is not polling, and the worker thread running that call cannot be killed — dropping the task handle leaves both the thread and the open remote connection running until the call returns. The planned push-phase cancellation inherits the same limit.

A subprocess transport sidesteps it: terminating the child process tears down the TCP/TLS/SSH session immediately, regardless of which phase it is in. The trust boundary is unchanged — gpm already trusts a subprocess it spawns around secrets (the age plugin subprocess for hardware-key recipients is the precedent), and gopass itself drives a system `git` over the store. Cancellation kills a process; it changes no committed state, and authenticity verification of any fetched commits is unaffected.

The cost is Android-first. Desktop can usually rely on a system `git` already on `PATH`, so the work there is process plumbing and parsing progress. Android has no system `git`, so true subprocess cancellation means bundling a git binary and its transport helpers into the APK — a non-trivial build and packaging effort plus an APK-size hit — which is the reason the project chose in-process libgit2 originally. A decision this RFC defers to its design phase: whether the cancellation win justifies that cost, or whether a partial answer (subprocess on desktop; retained in-process libgit2 with honest "may lag" feedback on Android) is the right resting state.

Threat-model impact: none beyond the existing trusted-subprocess boundary. Cancellation discards a partial fetch and tears down a network session; it writes nothing and bypasses no authenticity check — the same guarantee the token model already gives for transfer-phase cancels.

## Current state (interim shipped)

Two partial fixes have shipped, shrinking the blind spot without the subprocess:

- **Hardcoded connect/server timeouts.** The original "libgit2 exposes no connect timeout" premise (see _Alternatives considered_) was false for the vendored libgit2 1.9.6 — `git2 0.20.4` binds `GIT_OPT_SET_SERVER_CONNECT_TIMEOUT` (TCP connect + HTTPS TLS handshake) and `GIT_OPT_SET_SERVER_TIMEOUT` (post-connect: SSH key-exchange/auth and any stalled read). gpm sets both once at startup (20s connect / 60s server), so a clone against a dead host fails fast instead of grinding to the OS TCP timeout. `server_timeout` bounds the SSH handshake, which `server_connect_timeout` alone cannot reach; its trade-off is that it can also abort a legitimate slow transfer — acceptable because gpm stores are tiny.
- **Auth-phase + secondary-fetch cancel.** The cancel token is now honoured in the `credentials` callback (between handshake and the first transfer-progress tick — SSH userauth and HTTPS 401 retry become cancellable) and threaded through the secondary fetches that previously ignored it (divergence preview, keep-mine plan, adopt-remote, PAT verify).

Neither delivers the RFC's titular _instant_ cancel mid-handshake: the timeout is a bounded wait then an error (the user still waits the ceiling), and the `credentials` hook fires _after_ the TLS/SSH handshake. True mid-handshake termination would need a killable subprocess (system git, a libgit2 helper binary, or gix) or a custom libgit2 transport — libgit2's built-in cancel is checkpoint-only and the TLS handshake in `git_stream_connect` has no checkpoint to poll. **That full scope is declined:** the interim above bounds the worst case to ~20–60s, sufficient for gpm's tiny stores, and the subprocess / custom-transport cost is disproportionate. The RFC file is removed in a follow-up commit; this section is the decision record.

## Alternatives considered

- **Status quo + "Cancelling…" feedback.** The original interim; now augmented by the shipped timeouts and auth-phase cancel above, which bound the worst case to ~20–60s instead of minutes.
- **A libgit2 connect/server timeout.** Originally _rejected_ here on the claim that "libgit2 exposes no connect or handshake timeout to the embedder" — that premise is **false** for the vendored libgit2 1.9.6 (see _Current state_), and the timeout has shipped as the interim ceiling. It bounds the _hang_ but is a timeout, not an _instant cancel_.
- **Drop the blocking task on cancel.** Rejected — the C call is not interruptible, and the orphaned thread keeps the remote session open and leaks resources until it returns; the cancel is illusory.
- **Subprocess git transport.** The recorded direction for the resting state; deferred on the Android packaging scope, not on desirability.

## Effort

Large. Desktop is modest (system git usually present, plus process plumbing and progress parsing). Android dominates the cost: bundling git and its transport helpers, the APK-size hit, and reworking the transport layer that the backend crate currently owns in-process.

## Depends on / Supersedes

- Extends the shipped clone/pull/push cancellation design; all three inherit the same callback-polling limit this RFC addresses.
