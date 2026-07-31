<!-- SPDX-License-Identifier: Apache-2.0 -->

# Contributing to gpm

## Commit Conventions

gpm uses [Conventional Commits](https://www.conventionalcommits.org/). Every
commit subject must match:

```
<type>(<scope>): <summary>
```

`<scope>` is **optional** — omit the parentheses entirely when none fit. A
`commit-msg` hook (`nix/hooks/check-commit-msg.sh`, wired via the flake's
git-hooks.nix) enforces all of this on every local commit.

### Types

`feat` · `fix` · `docs` · `style` · `refactor` · `perf` · `test` · `build` · `ci` · `chore` · `revert`

### Scope — a closed allowlist

The scope is **not free text**. It must be one of the values below, or omitted.
This is the whole point: a fixed, enumerable vocabulary stops the scope field
from drifting into synonyms and prose.

**Feature scopes** — one per product capability, sourced from
`docs/specs/*/prd.md` frontmatter (`scope: <token>`):

| Spec                     | scope      |
| ------------------------ | ---------- |
| 001 Entry Access         | `entries`  |
| 002 Secret Management    | `secrets`  |
| 003 age Encryption       | `age`      |
| 004 GPG Encryption       | `gpg`      |
| 005 Git Storage & Sync   | `git`      |
| 006 Identities & Trust   | `id`       |
| 007 App Lock & Auto-lock | `lock`     |
| 008 Android Autofill     | `autofill` |

**Code-area scopes** — when the change is cross-feature / internal /
infrastructural and maps to a part of the tree:

`rustpass` · `app` (`src-tauri/`) · `frontend` (`src/`) · `android` · `plugin` (`tauri-plugin-*`) · `ci` · `build` · `deps`

### Choosing a scope

1. **Product feature change** (often spans `rustpass` + `frontend`) → use the
   **feature scope**. This is the payoff of the feature axis: one scope covers
   the whole stack, so you never choose between `rustpass` and `frontend`.
2. **Cross-feature / internal / tooling change** with a clear home → use the
   **code-area scope**.
3. **No clear home** (a wide refactor, or a fix touching several areas equally)
   → **omit the scope**: `fix: refresh the entry list on app-unlock`.

Prefer splitting a cross-area change into focused commits
(`feat(rustpass): ...` then `feat(frontend): ...`) when each stands on its own.
When you can't split, pick the area that dominates; when nothing dominates,
omit.

### Rules

- Scope matches `[a-z0-9-]+` — lowercase, kebab-case. No spaces, slashes,
  capitals, sentences, or enum values in the scope.
- Never put the change description, ticket/req id, RFC/spec number, or phase in
  the scope — that belongs in the subject or body.
- Empty scope is always valid.
- To register a **new feature scope**, add `scope: <token>` to the spec's
  `prd.md` frontmatter — the hook reads it live, so the new token is allowed
  immediately with no second place to update.

### Examples

```
feat(lock): skip the biometric prompt after an idle re-lock
feat(rustpass): add the GPG/OpenPGP crypto backend
fix: refresh the entry list on app-unlock
docs(rfc): retire R055 — single-toggle verbose logging shipped
ci(flake): keep the pre-commit hook install out of CI shells
```
