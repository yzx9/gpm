# Add re-configuration flow

**Priority:** P2
**Status:** TODO
**Phase:** Post-MVP (v0.2)

## What

Allow users to change their repo URL, PAT, or age identity without re-running the full setup flow (which clears the local repo and re-clones). Provide a settings mechanism that updates config in-place.

## Why

Currently changing anything requires the full setup flow: enter new URL + PAT + identity → clone fresh. If a user rotates their PAT or switches to a different age identity, they must re-download the entire password store. This is a friction point for routine credential rotation.

## Context

### Current flow

```
SetupPage → configure() → clear_all() → save_identity() → clone_repo()
```

`clear_all()` deletes the local repo and all config. There's no way to update individual settings.

### Implementation options

1. **Add a Settings page** — New Vue page accessible from EntryListPage (gear icon). Sections for repo config and identity config. Changes save individually without clearing the repo.

2. **Modify SetupPage** — Detect if config already exists and show "Update" instead of "Set up". Allow partial updates (just identity, just PAT, etc.) without re-cloning.

3. **Keep SetupPage + add "Update identity" shortcut** — Minimal change. Add a button on EntryListPage that opens a dialog for identity update only.

### Recommended approach

Option 2 (modify SetupPage). Least new code. If config exists, pre-fill the form fields and show "Update configuration" button. Add a checkbox: "Re-clone repository (clears local data)" — unchecked by default. If only the identity changed, skip clone. If the repo URL changed, force re-clone.

### Key files

- `src/views/SetupPage.vue` — Add update mode, pre-fill fields, conditional clone
- `rustpass/src/config.rs` — Add `update_identity()`, `update_pat()` methods that don't clear repo
- `rustpass/src/store.rs` — Split `configure()` into granular update methods
- `src/views/EntryListPage.vue` — Add settings/gear button to access SetupPage in update mode

### Edge cases

- Changing repo URL: must re-clone (different repo entirely)
- Changing PAT only: no re-clone needed, just update credential
- Changing identity: no re-clone needed, but entries that were encrypted to the old identity's public key won't decrypt with the new one. Consider warning the user.
- Corrupted state: if update fails midway, existing config should remain valid

## Effort

~0.5-1 day (human) / ~20 min (CC)

## Depends on

None — independent of other plans.
