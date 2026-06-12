# Fix: Auto-lock timer operates on wrong Store instance

Priority: P1 (security)
Discovered by: /plan-eng-review on 2026-06-12
Status: Open

## Problem

`reset_lock_timer` in `src-tauri/src/lib.rs:404-418` spawns an async task that creates a **new** `Store::new(config_dir)` and calls `lock()` on it. But `AppState` holds a **different** `Store` instance. Each `Store` has its own `cached_identity: RwLock<Option<Zeroizing<Vec<u8>>>>`.

The timer's `Store` starts with no cached identity, so `lock()` sets `None` to `None` — a no-op. The `AppState`'s cached identity is **never cleared** by the auto-lock timer.

The frontend receives the `identity-locked` event and navigates away, but the decrypted identity remains in memory until the process exits.

## Root cause

```rust
// src-tauri/src/lib.rs:404-418 — current (broken)
let handle = tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(300)).await;
    let store = Store::new(config_dir);  // ← NEW instance, not AppState's store
    store.lock();                         // ← locks wrong instance
    let _ = app_handle.emit("identity-locked", ());
});
```

## Fix

The timer needs to lock the **same** `Store` instance that `AppState` holds. Options:

### Option A: Share Store via Arc

```rust
struct AppState {
    store: Arc<Store>,  // wrap in Arc so the timer can hold a reference
    lock_timer: Mutex<Option<JoinHandle<()>>>,
}

// In reset_lock_timer:
let store = Arc::clone(&state.store);
let handle = tokio::spawn(async move {
    tokio::time::sleep(Duration::from_secs(300)).await;
    store.lock();  // ← locks the REAL store
    let _ = app_handle.emit("identity-locked", ());
});
```

### Option B: Send lock signal through a channel

Use a `tokio::sync::watch` or `mpsc` channel. Timer sends a "lock" message; the main task listener calls `state.store.lock()`.

### Recommendation

Option A — `Arc<Store>` is the simplest fix. `Store` is already `Send + Sync` (uses `std::sync::RwLock`). The `Arc` wrapper adds no runtime cost.

## Files to change

- `src-tauri/src/lib.rs` — `AppState.store` becomes `Arc<Store>`, `reset_lock_timer` clones the Arc

## Verification

1. Configure store, unlock with passphrase
2. Wait for auto-lock timer (5 min) or reduce `DEFAULT_LOCK_TIMEOUT_SECS` for testing
3. Verify `store.is_unlocked()` returns `false` after timer fires
4. Verify `identity-locked` event is emitted
5. Verify subsequent `get()` returns `IDENTITY_ENCRYPTED`
