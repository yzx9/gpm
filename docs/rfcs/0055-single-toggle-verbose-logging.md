# Single-Toggle Verbose Logging

**Priority:** P2
**Status:** Draft
**Phase:** Next

## What

Collapse the four-level diagnostics log-level selector (Errors / Warnings / Info / Debug) on the Settings → Logs screen into a single **verbose** toggle. The app runs at **Info** by default; the toggle turns on **Debug** for a bounded window, after which it reverts to Info automatically. The verbose flag is persisted, so a launch made with verbose on runs the entire session — including startup — at Debug. This revises the runtime level-control portion of 0052; the Logs viewer, file rotation, and plaintext-log threat model from 0052 stand unchanged.

## Why

The current selector exposes a concept that does not belong in a password manager's UI. Log levels are developer semantics; an end user's mental model is binary — "show me normal logs" or "something's wrong, capture everything for a bug report." Forcing a choice among four levels adds interaction for no benefit, since gpm's log volume is low and the levels are not meaningfully distinct to a user.

A second motivation surfaced while investigating a separate diagnostics leak: the logger had been configured with a Trace ceiling, which admitted pure third-party noise (notably the `jni` crate logging every JNI method lookup and call) during the startup window. That ceiling has since been lowered to Debug — gpm emits no trace-level diagnostics of its own, so Trace carried only dependency chatter. That change is the foundation for this one: Debug is now the natural "everything we've got" level, and Trace is structurally out of reach. There is no longer a reason to offer the user a path toward Trace, and little reason to distinguish Debug from the other sub-Info levels.

## Context

**Default and verbose level.** Info is the steady state. Verbose is Debug — the ceiling the logger is already capped at. Trace is deliberately not reachable: gpm has no trace-level diagnostics, and the only content Trace would add is third-party internal noise (JNI call tracing, TLS handshake internals, git transport trace) at high volume and zero diagnostic value for this app.

**Persistence — why verbose must survive a restart.** Verbose is persisted in the plaintext app config (same rationale as the display-language preference: it must be readable before unlock, it survives a config reset, and it is non-confidential — the local write attacker is out of scope per the threat model). Persistence is load-bearing here rather than incidental: a verbose mode that exists only in memory, toggled after launch, can never capture a startup failure — yet startup (identity load, key unsealing, config migration, backend resolution) is precisely when the hardest bugs happen. A user who hits a startup problem must be able to turn verbose on, relaunch, and have that launch's startup recorded at Debug. That requires the flag to be readable at startup, which requires it on disk.

**Time-boxed auto-revert.** Verbose turns itself off after a bounded window. The expiry is persisted alongside the flag (a deadline, not a session timer) so the window survives restart — letting a user relaunch several times within it to reproduce a flaky startup issue, while guaranteeing verbose cannot be left on indefinitely (which would grow the log and expand the metadata surface for no ongoing reason). The deadline is evaluated when the config is read at launch and on in-app activity, so an expired verbose reverts to Info without a restart. The exact window length is an implementation detail; on the order of ten-to-fifteen minutes fits the rotation budget and the "capture one repro" use case.

**What stays.** The Logs viewer, its mtime-ordered tail-truncated read, the rotated file under the app log directory, the clear action, and the frontend error bridge are all unchanged (0052 phases 1–2). Only the level-control mechanism and its UI change.

**Migration.** A user who previously pinned the level to Debug has that preference carried into the new verbose flag (one-time, via the existing app-config schema migration) so the upgrade is non-surprising; it then expires under the same time-box as any other verbose session. All other prior levels collapse to the Info default.

## Alternatives considered

**Keep the four-level selector.** Rejected: the levels are developer concepts, gpm's log volume does not make the distinction meaningful to a user, and the choice adds UI without adding power. A password manager's diagnostics either answer "what happened" (Info) or "capture everything for a report" (Debug); the in-between levels serve no user.

**In-memory verbose, not persisted (toggle only after launch).** Rejected: it cannot record startup, which is a primary debugging scenario. Persistence is the whole point for the startup case.

**Verbose = Trace.** Rejected: gpm emits nothing at Trace. The only Trace-level content is third-party internals (JNI, TLS, git transport) — high volume, zero signal for this app. The Debug ceiling already excludes it at every phase; reaching Trace would resurrect exactly the noise the recent ceiling change removed.

**Verbose stays on until manually turned off (no auto-revert).** Considered: simplest, and most flexible for chasing an intermittent bug across many attempts. Rejected in favor of a persisted-deadline auto-revert so verbose is bounded by default — protecting log size and the metadata surface — while still allowing multiple relaunches inside the window for flaky repros.

## Effort

~S/M (human) / ~M (CC)

## Depends on / Supersedes

Revises the runtime level-control portion of **0052** (In-app Diagnostics Logging). The viewer, rotation, plaintext-log threat model, and frontend error bridge from 0052 are unchanged. Builds on the recent Debug logger ceiling (no trace-level diagnostics exist in gpm).
