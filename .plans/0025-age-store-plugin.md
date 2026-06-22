# Extract rustpass as a reusable age-store plugin

**Priority:** P3
**Status:** Draft
**Phase:** Future

## What

gpm's entire age-encrypted gopass-store engine — clone/sync, list/search, age decryption, secret write/create
with templates, at-rest AEAD, commit-signature authenticity, identity and recipient handling — lives in a
single pure-Rust crate that today is consumed only by this one app. This RFC proposes turning that engine
into a reusable Tauri plugin (`tauri-plugin-age-store`) so other Tauri apps can consume a vetted, age-only,
no-cloud password-store client — and, more importantly, defines _when_ that publication is worth doing and
what "plugin-ready" means in the meantime.

## Why

The engine is already structurally a library: it has no Tauri or platform dependencies, and all app coupling
lives in a thin command layer above it. So the cost of _becoming_ a plugin is low; the open question is only
whether and when to _publish_ one.

Two motivations, in tension:

1. **Infrastructure leverage + auditability.** The project's thesis is that trust is the product — a small,
   auditable, age-only, no-GPG, no-cloud store client. There is no other Tauri/Rust GUI story for the age +
   gopass intersection (the Android Password Store app is unmaintained and GPG-only; gopass itself is
   Go/CLI-only). Publishing the engine lets other apps reuse a vetted implementation instead of each
   rebuilding age + store-format + git-sync from scratch — which amplifies the audit story (more consumers,
   more eyes, stronger incentive to keep it correct).

2. **Premature-publication risk.** A published plugin is a public API with semver, stability, docs, and
   cross-platform-test obligations. The engine is still moving fast (write path, TOTP, edit/delete, recipient
   pinning are all open work). Freezing a public surface now would either throttle gpm's own velocity or
   produce a stream of breaking releases. This is exactly the "building for imaginary consumers" cost that
   caused the original design to defer extraction until adoption warranted it.

The contribution of this RFC is to replace that vague _"if adoption warrants it"_ with concrete readiness
criteria and a trigger, so the decision stops being perpetually hand-waved.

## Context

**Current shape.** The engine is a pure library crate depending only on an age crate, a git binding, an async
runtime, a serialization crate, an SSH-key crate, a directory walker, a fuzzy matcher, and an AEAD crate — no
Tauri types anywhere in its public surface. The app holds the engine behind a Tauri app-state wrapper and a
layer of Tauri commands; that command layer, not the engine, is where all Tauri coupling lives. The
extraction boundary therefore already exists in practice — there is nothing to decouple, only a packaging
decision to make.

**A new plugin shape for this repo.** Every local plugin in the repo today is a _native bridge_: a thin Rust
IPC layer in front of an Android Kotlin/Gradle module (safe-area, biometric-keystore, secure-keystore,
file-picker). A pure-Rust _logic_ plugin — no Kotlin, no Gradle module, consumed by other apps' Rust side —
would be a new pattern here, and a simpler one: it needs no mobile scaffolding at all. Worth calling out
because it lowers the packaging cost relative to the existing plugins, rather than raising it.

**What the plugin would expose (categories, not signatures).** The engine covers a cohesive set of
capabilities: a high-level store facade (list, search, decrypt, sync, write), age decryption, ff-only git
sync over HTTPS and SSH, gopass-compatible secret parsing and templates, at-rest AEAD for local private
files, commit-signature authenticity verification, single-identity age and SSH identity handling, recipient
discovery, SSH key generation, config persistence, and sanitized (no-secret) errors. That whole surface is
the candidate public API — which is also the size of the stability commitment.

**Threat-model impact.** Publishing the crypto-adjacent engine is a double-edged trust change. On one side,
external consumers and auditors strengthen the "auditable by design" thesis. On the other, a correctness bug
or a flawed error-sanitization path would affect every consumer, not just gpm — so the safe-error and
zeroize guarantees become a contract the plugin must not silently regress. Publication does not change gpm's
own threat model (the engine is already pure and already behind the command layer); it only extends the
audience.

## Recommended decision: plugin-ready now, published on a trigger

Rather than "extract now" or "never," stage it:

1. **Plugin-ready (near-term, low cost).** Explicitly separate the engine's _intended public surface_ from
   its internal modules, document the boundary, and confirm it builds as a standalone crate consumed via a
   path or registry dependency. Keep the right to change anything marked internal. This is mostly
   documentation and light API hygiene; the engine is already pure.

2. **Published plugin (trigger-based).** Turn it into a published `tauri-plugin-age-store` only when a
   concrete signal appears — a real second consumer asks for it, or gpm's feature set stabilizes (write
   path, edit/delete, TOTP, recipient pinning all landed) so freezing an API is not premature. At that point
   add the plugin manifest/permissions scaffolding, a changelog and semver policy, and a cross-platform test
   matrix.

This keeps forward motion (a named, documented boundary) without paying the stability tax before there is
anyone to be stable for, and it dissolves the original _"if adoption warrants it"_ into a checklist anyone
can evaluate.

## Alternatives considered

- **Extract and publish now.** Rejected as the default because there is no known consumer and the API is
  still in flux; the stability obligation would throttle gpm's velocity for an audience of zero. Remains the
  correct move the moment a real consumer appears.
- **Never extract; stay an app-private crate.** Rejected because the engine is already pure, the packaging
  cost is low, and the auditability/infrastructure argument is a genuine part of the project's thesis.
  "Never" throws away leverage that is cheap to keep optionable.
- **Extract as a plain library crate, not a Tauri plugin.** The engine already _is_ a plain library crate.
  The plugin wrapper only adds Tauri ergonomics (extension traits, command registration, permissions) so a
  Tauri consumer can wire it without re-deriving the command layer. Worth bundling with publication, but the
  library-vs-plugin distinction is not the hard part — timing is.

## Effort

Plugin-ready stage: ~0.5 day (human) / ~15 min (CC) — mostly API-boundary documentation and a
standalone-build check.

Published-plugin stage: ~1–2 days (human) / ~30–60 min (CC) — manifest/permissions scaffolding,
semver/changelog policy, cross-platform test matrix, and a publication path.

## Depends on / Supersedes

Captures a long-deferred roadmap item — package the age+gopass engine as a reusable plugin — that has been
on the list since the project's founding design. Natural to sequence after the in-flight write-path work
(0020 edit-secrets, 0021 delete-secrets, 0024 totp-2fa-codes) so the published surface reflects a
feature-complete engine.
