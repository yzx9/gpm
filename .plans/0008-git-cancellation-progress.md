# Plan: Git Cancellation & Progress Reporting

## Context

After the Phase 1 async migration (complete), git clone and pull are the only operations that genuinely benefit from cancellation — they can take seconds to minutes depending on network and repo size. The frontend currently shows a fake progress spinner during clone and no feedback during pull. This plan adds real git2 progress callbacks and user-initiated cancellation for both clone and pull.

## Architecture Overview

```
Vue cancel button
  → invoke("cancel_git")
  → cancel_token.store(true)
  → git2 transfer_progress callback returns false
  → git2 aborts transfer → Error::Cancelled

git2 transfer_progress callback
  → std::sync::mpsc::Sender<GitProgress>
  → spawn_blocking drain task
  → app_handle.emit("git-progress", payload)
  → Vue listen("git-progress")
  → progress bar + text update
```

Key design: `rustpass` stays Tauri-free. The library uses `Arc<AtomicBool>` for cancellation and `std::sync::mpsc::Sender` for progress (synchronous, safe to call from git2's C callbacks on the blocking thread). The Tauri command layer creates the channel pair and bridges the receiver to Tauri events.

## Step 1: `rustpass/src/git.rs` — Types and callback changes

### New types

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Shared cancellation token. Set to `true` to abort an in-progress git operation.
pub type CancelToken = Arc<AtomicBool>;

/// Progress data reported by git2 during a transfer.
#[derive(Debug, Clone, serde::Serialize)]
pub struct GitProgress {
    pub total_objects: usize,
    pub received_objects: usize,
    pub indexed_objects: usize,
    pub received_bytes: usize,
    pub total_deltas: usize,
    pub indexed_deltas: usize,
    pub message: Option<String>,
}

/// Synchronous sender for git progress (used inside git2 C callbacks).
pub type ProgressSender = std::sync::mpsc::Sender<GitProgress>;
```

### Change `build_remote_callbacks` signature

```rust
fn build_remote_callbacks(
    auth: &GitAuth,
    cancel: Option<CancelToken>,
    progress: Option<ProgressSender>,
) -> RemoteCallbacks<'_>
```

After the existing credentials match, add two callbacks:

- **`transfer_progress`** — sends `GitProgress` with object/byte stats, returns `false` if `cancel` token is set
- **`sideband_progress`** — sends `GitProgress` with textual message (e.g. "Counting objects..."), always returns `true`

### Change `clone_repo` and `pull_repo` signatures

Append `cancel: Option<CancelToken>, progress: Option<ProgressSender>` to both. Pass them to `build_remote_callbacks`. On error, check if cancel token is set and map to `ErrorCode::Cancelled`.

Existing tests pass `None, None` — no test changes needed.

## Step 2: `rustpass/src/store.rs` — Thread cancel/progress through Store methods

Add `cancel: Option<git::CancelToken>, progress: Option<git::ProgressSender>` to:

- `clone_only` — forward into `spawn_blocking` closure
- `configure` — forward into `spawn_blocking` closure
- `sync` — forward into `spawn_blocking` closure

All other Store methods unchanged. Existing integration tests don't pass cancel/progress — they use the simpler signatures without trailing optional params. **However**, since Rust doesn't have default parameters, every existing call site that calls these three methods must be updated to pass `None, None`.

Files with call sites:

- `rustpass/tests/store_facade.rs` — 9 tests call `configure`, `clone_only`, `sync`
- `rustpass/tests/identity_encryption.rs` — 8 tests call `configure`, `clone_only`, `set_passphrase`, `sync`
- `rustpass/tests/storage_integration.rs` — may call `configure`
- `rustpass/tests/git_integration.rs` — calls `configure`, `sync`
- `src-tauri/src/lib.rs` — `clone_repo`, `setup`, `pull_repo` commands

## Step 3: `rustpass/src/lib.rs` — Re-export new types

```rust
pub use git::{CancelToken, GitProgress, ProgressSender};
```

## Step 4: `src-tauri/src/lib.rs` — Bridge layer

### AppState change

Add `active_cancel_token: Mutex<Option<CancelToken>>` to `AppState`.

### New `cancel_git` command

```rust
#[tauri::command]
fn cancel_git(state: tauri::State<'_, AppState>) -> Result<(), Error> {
    if let Some(token) = state.active_cancel_token.lock().ok().and_then(|t| t.take()) {
        token.store(true, Ordering::Relaxed);
    }
    Ok(())
}
```

### Progress event type

```rust
#[derive(Debug, Clone, Serialize)]
struct GitProgressEvent {
    total_objects: usize,
    received_objects: usize,
    received_bytes: usize,
    message: Option<String>,
}
```

(Only fields the frontend needs — omit indexed_objects, deltas, etc.)

### Change `clone_repo` and `pull_repo` commands

Both follow the same pattern:

1. Create `CancelToken` (`Arc::new(AtomicBool::new(false))`)
2. Store it in `state.active_cancel_token`
3. Create `std::sync::mpsc::channel::<GitProgress>()`
4. Spawn a `spawn_blocking` drain task that calls `app_handle.emit("git-progress", ...)` for each progress message
5. Pass `Some(token)`, `Some(tx)` into the Store method
6. In `finally`: drop sender (closes channel), await drain task, clear `active_cancel_token`

Register `cancel_git` in `invoke_handler`.

### Change `setup` command (calls `store.configure`)

Same pattern — this is the full-setup path that also does a clone.

## Step 5: `src/types.ts` — Add progress type

```typescript
export interface GitProgressEvent {
  total_objects: number;
  received_objects: number;
  received_bytes: number;
  message: string | null;
}
```

## Step 6: `src/pages/SetupPage.vue` — Real progress bar + cancel

- Remove fake progress: `progressSteps`, `progressStep`, `startProgress()`, `stopProgress()`, `progressTimer`
- Add reactive state: `progressText`, `progressPercent`, `receivedBytes`
- In `onClone()`: listen to `"git-progress"` events, update state
- Add `cancelClone()` → `invoke("cancel_git")`
- Template: progress bar (CSS width transition), text line, cancel button
- Handle `CANCELLED` error code in catch block

## Step 7: `src/pages/EntryListPage.vue` — Pull progress + cancel

- Add `pullProgressText`, `pullProgressPercent` refs
- In `pullRepo()`: listen to `"git-progress"`, update state
- Change Pull button to show "✕ Cancel" during pull, call `invoke("cancel_git")`
- Show thin progress bar below header during pull

## Files Modified

| File                          | Change                                                                                                                                                                                                                                  |
| ----------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `rustpass/src/git.rs`         | Add `CancelToken`, `GitProgress`, `ProgressSender` types; add `transfer_progress` + `sideband_progress` callbacks to `build_remote_callbacks`; add cancel/progress params to `clone_repo`, `pull_repo`; cancel → `ErrorCode::Cancelled` |
| `rustpass/src/store.rs`       | Add `cancel`/`progress` params to `clone_only`, `configure`, `sync`; forward into `spawn_blocking`                                                                                                                                      |
| `rustpass/src/lib.rs`         | Re-export `CancelToken`, `GitProgress`, `ProgressSender`                                                                                                                                                                                |
| `src-tauri/src/lib.rs`        | Add `GitProgressEvent`, `active_cancel_token` to `AppState`, `cancel_git` command, progress bridge in `clone_repo`/`setup`/`pull_repo` commands                                                                                         |
| `src/types.ts`                | Add `GitProgressEvent` interface                                                                                                                                                                                                        |
| `src/pages/SetupPage.vue`     | Replace fake progress with real progress bar + cancel button                                                                                                                                                                            |
| `src/pages/EntryListPage.vue` | Add pull progress bar + cancel                                                                                                                                                                                                          |

## Test updates (append `None, None` to call sites)

- `rustpass/tests/store_facade.rs` — 9 tests
- `rustpass/tests/identity_encryption.rs` — 8 tests
- `rustpass/tests/storage_integration.rs` — check if affected
- `rustpass/tests/git_integration.rs` — check if affected

New tests in `git.rs`:

- Cancel token → callback returns `false`
- Progress sender → receiver gets `GitProgress`

## Verification

1. `cargo test --workspace` — all tests pass
2. `cargo clippy --workspace -- -D warnings` — clean
3. `just dev` — clone a repo, verify progress bar shows real stats, click cancel mid-clone
4. Pull on entry list page, verify progress shows, cancel works
5. Verify `CANCELLED` error is silently handled (no crash)
