<!--
Feature-level threat model for identities & recipients trust. Complements
docs/SECURITY.md. Living. (Multi-identity and recipients pinning are planned — see
prd.md §6 — so some defenses below are target state, not yet shipped.)
-->

# 006 — Identities & trust: threat model

## The recipients-injection threat

The store's recipients file lists every key secrets are encrypted to. An attacker who
can inject their own key into it (via a compromised remote, or local write — see the
system-wide assumption) gets silently encrypted-to on every future secret create, and
can then read those secrets. This is the core threat the planned defenses below address.

## recipients pinning (TOFU) — a file-level defense [planned]

Pinning stores a hash of the resolved recipients file locally and surfaces drift:
non-blocking on sync (Audit philosophy), but the secret-create write path
**hard-blocks** until the user reviews and acknowledges. This is a **file-level**
defense, independent of commit-signature verification (005) — it works even in
authenticity-Off mode. The pin lives in local `repo.json`, not in the repo.

## Overwrite safety [planned, with multi-identity]

A store that can hold entries not encrypted to the current identity needs an overwrite
gate: refuse to overwrite a remote entry whose ciphertext the current identity can't
decrypt, so a sync never silently clobbers a secret you can't read.

## Undecryptable entries

"Undecryptable" is an explicit, graceful state, not a crash: the entry's metadata is
listed, but its ciphertext never crosses to the WebView.

## Cross-references

- Commit-signature verification (the other defense layer): `005/security.md`.
- System-wide "no local write attacker" assumption: `docs/SECURITY.md`.
