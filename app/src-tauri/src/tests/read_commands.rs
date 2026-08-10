// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! read-command cores — the decrypt-and-show glue that needs a live `AppState`
//! and a runtime (so it can't live in `rustpass`). Drives `show_password_core`
//! against the mock app.

use rustpass::LockMode;
use tauri::Manager;

use crate::AppState;
use crate::entry_cache;
use crate::entry_cache::EntryCacheReason;
use crate::read;
use crate::tests::{make_unlocked_state, mock_app, test_repo_id};

/// Under Immediate, `show_password_core` returns the secret AND soft-wipes the
/// identity afterward — the decoded secret lives in the returned
/// `SensitiveContent`, independent of the identity cache. A regression that
/// drops the wipe would leave the identity cached past the op; one that wipes
/// before the read resolves would lose the secret. The wipe must also fire on
/// the error path (covered by the `maybe_soft_wipe` tests), so the success path
/// is the remaining gap this pins down.
#[tokio::test]
async fn show_password_core_returns_secret_then_soft_wipes_under_immediate() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\nbody line")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = LockMode::Immediate;

    assert!(app_state.store.is_unlocked(), "precondition: unlocked");

    let content = read::show_password_core(&app_state, app.handle(), &app_state.store, "foo.age")
        .await
        .expect("show should succeed");
    // `password`/`notes` are `Zeroizing<String>` — deref to compare.
    assert_eq!(&*content.password, "hunter2");
    assert_eq!(&*content.notes, "body line");

    assert!(
        !app_state.store.is_unlocked(),
        "Immediate must soft-wipe the identity after show"
    );
}

/// `entry_probe` decrypts once and reports TOTP-presence + attachment metadata
/// for the detail view. A normal UTF-8 entry with no TOTP and no attachment
/// probes to `Some(EntryProbe { has_totp: false, attachment: None, edit_blocked: None })`.
#[tokio::test]
async fn entry_probe_returns_metadata_when_unlocked() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "foo.age".into(),
    )
    .await
    .expect("entry_probe should succeed on an unlocked store")
    .expect("a normal entry should probe to Some");
    assert!(!probe.has_totp, "no TOTP seed in the fixture");
    assert!(probe.attachment.is_none(), "no attachment in the fixture");
    assert!(probe.edit_blocked.is_none(), "a UTF-8 password is editable");
}

/// `entry_probe` must NEVER raise an unlock prompt. The fixture's identity is
/// passphrase-encrypted, so wiping the cache makes `Store::get` fail
/// `IDENTITY_ENCRYPTED`; the probe surfaces that as `Ok(None)` ("unknown")
/// before touching any lock timer. A regression that prompted would fire an
/// unlock modal off a passive probe.
#[tokio::test]
async fn entry_probe_never_prompts_when_identity_locked() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    app.state::<AppState>().store.lock(); // wipe the cached, passphrase-encrypted identity
    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "foo.age".into(),
    )
    .await
    .expect("a locked identity is Ok(None), not an error");
    assert!(
        probe.is_none(),
        "a locked identity must probe to None, never prompt"
    );
}

/// With no TOTP seed, `copy_totp` short-circuits BEFORE the clipboard write —
/// `copied == false`, no clear scheduled — so it's testable without the
/// clipboard plugin. Pins the no-seed branch + the `cleared_after_secs: 0`
/// contract.
#[tokio::test]
async fn copy_totp_skips_clipboard_when_no_totp_seed() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\n")]).await;
    let app = mock_app(state);
    let result = read::copy_totp(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "foo.age".into(),
        None,
    )
    .await
    .expect("copy_totp should succeed on a no-seed entry");
    assert!(!result.copied, "no TOTP seed ⇒ copied == false");
    assert_eq!(result.cleared_after_secs, 0);
    assert_eq!(result.entry_name, "foo");
}

/// R086: the entry-view cache. `show_password_core` warms the cache on the first
/// decrypt; a second `show` of the SAME entry HITS the cache and returns the
/// secret WITHOUT re-decrypting — so even under Immediate (where the first show
/// soft-wiped the identity) the second show succeeds instead of failing with
/// `IDENTITY_ENCRYPTED`. This is the core property: one unlock opens the view.
#[tokio::test]
async fn entry_cache_hit_serves_second_show_without_identity() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2\nbody line")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    *app_state.lock_mode.lock().unwrap() = LockMode::Immediate;

    // First show: decrypt + Immediate soft-wipe + warm the cache.
    let first = read::show_password_core(&app_state, app.handle(), &app_state.store, "foo.age")
        .await
        .expect("first show");
    assert_eq!(&*first.password, "hunter2");
    assert!(
        !app_state.store.is_unlocked(),
        "Immediate soft-wiped the identity after the first show"
    );
    assert!(
        app_state.cached_entry.lock().unwrap().is_some(),
        "the cache warmed on the first decrypt"
    );

    // Second show: identity is wiped, but the cache HITS → no re-auth, no decrypt.
    // (Without the cache this would reject with IDENTITY_ENCRYPTED.)
    let second = read::show_password_core(&app_state, app.handle(), &app_state.store, "foo.age")
        .await
        .expect("second show must hit the cache, not re-prompt");
    assert_eq!(&*second.password, "hunter2");
    // The hit reuses the cached oid for `version` (#11).
    assert_eq!(second.version, first.version);
    assert!(
        !app_state.store.is_unlocked(),
        "a cache hit touches no identity"
    );
}

/// R086: the cache is single-entry — viewing a different entry evicts the prior
/// one (a MISS repopulates). After showing `bar`, the cache holds `bar`, not `foo`.
#[tokio::test]
async fn entry_cache_evicts_on_entry_switch() {
    let (state, _guard) =
        make_unlocked_state(&[("foo.age", b"foo-pw\nbody"), ("bar.age", b"bar-pw\nbody")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();
    // Never keeps the identity cached across shows, so the second (different)
    // entry can MISS the cache and re-decrypt — the single-entry eviction path.
    *app_state.lock_mode.lock().unwrap() = LockMode::Never;

    read::show_password_core(&app_state, app.handle(), &app_state.store, "foo.age")
        .await
        .expect("show foo");
    assert_eq!(
        app_state
            .cached_entry
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.entry_path.as_str().to_owned()),
        Some("foo.age".to_string()),
        "cache holds foo"
    );

    read::show_password_core(&app_state, app.handle(), &app_state.store, "bar.age")
        .await
        .expect("show bar");
    assert_eq!(
        app_state
            .cached_entry
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.entry_path.as_str().to_owned()),
        Some("bar.age".to_string()),
        "single-entry cache evicted foo for bar"
    );
}

/// `soft_wipe_entry_cache` clears the cached entry (the wipe path the lock/leave
/// handlers share). After a show warms it, the wipe empties it.
#[tokio::test]
async fn soft_wipe_entry_cache_clears_the_cache() {
    let (state, _guard) = make_unlocked_state(&[("foo.age", b"hunter2")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    read::show_password_core(&app_state, app.handle(), &app_state.store, "foo.age")
        .await
        .expect("show");
    assert!(
        app_state.cached_entry.lock().unwrap().is_some(),
        "cache warmed"
    );

    entry_cache::soft_wipe_entry_cache(&app_state, app.handle(), EntryCacheReason::Leave);
    assert!(
        app_state.cached_entry.lock().unwrap().is_none(),
        "soft_wipe_entry_cache emptied the cache"
    );
}

// ---- legacy-YAML read-only branch (A004) ----

/// The IPC wire strings of `EditBlockReason` — an IPC drift here silently
/// disables the frontend hint (same drift class the `nonUtf8` pin guards
/// against), pinned by serializing through serde JSON.
#[test]
fn edit_block_reason_wire_strings() {
    let json = serde_json::to_string(&read::EditBlockReason::LegacyYaml).unwrap();
    assert_eq!(json, "\"legacyYaml\"");
    let json = serde_json::to_string(&read::EditBlockReason::NonUtf8).unwrap();
    assert_eq!(json, "\"nonUtf8\"");
}

/// A `---`-bearing secret shows with `edit_blocked == LegacyYaml` (and its
/// `k: v` lines are NOT surfaced as attributes — the block is opaque body).
#[tokio::test]
async fn show_password_core_marks_yaml_secret_edit_blocked() {
    let (state, _guard) = make_unlocked_state(&[("y.age", b"pw\n---\notp: bar")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    let content = read::show_password_core(&app_state, app.handle(), &app_state.store, "y.age")
        .await
        .expect("show");
    assert_eq!(&*content.password, "pw");
    // The YAML block stays one opaque body; no attribute rows.
    assert!(content.attributes.is_empty());
    assert_eq!(&*content.notes, "---\notp: bar");
    assert_eq!(
        content.edit_blocked,
        Some(read::EditBlockReason::LegacyYaml)
    );
}

/// The probe reports the same read-only verdict for the list/detail affordances.
#[tokio::test]
async fn entry_probe_marks_yaml_secret_edit_blocked() {
    let (state, _guard) = make_unlocked_state(&[("y.age", b"pw\n---\nk: v")]).await;
    let app = mock_app(state);

    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "y.age".into(),
    )
    .await
    .expect("probe")
    .expect("unlocked");
    assert_eq!(probe.edit_blocked, Some(read::EditBlockReason::LegacyYaml));
}

/// A non-UTF-8 YAML secret reports the stricter `NonUtf8` reason (both flags are
/// true; the data-loss reason wins — documented priority).
#[tokio::test]
async fn non_utf8_wins_over_legacy_yaml() {
    let (state, _guard) = make_unlocked_state(&[("y.age", b"\xff\xfe\n---\nk: v")]).await;
    let app = mock_app(state);

    let probe = read::entry_probe(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "y.age".into(),
    )
    .await
    .expect("probe")
    .expect("unlocked");
    assert_eq!(probe.edit_blocked, Some(read::EditBlockReason::NonUtf8));
}

/// A bare YAML doc (first line `---`) has no password: `show_password_core`
/// reports the empty password, and `copy_password` refuses the clipboard write
/// (`password_empty` byproduct) instead of a fake success over an empty
/// clipboard.
#[tokio::test]
async fn bare_yaml_doc_copy_reports_empty_password() {
    let (state, _guard) = make_unlocked_state(&[("y.age", b"---\nk: v")]).await;
    let app = mock_app(state);
    let app_state = app.state::<AppState>();

    let content = read::show_password_core(&app_state, app.handle(), &app_state.store, "y.age")
        .await
        .expect("show");
    assert_eq!(&*content.password, "");
    assert_eq!(
        content.edit_blocked,
        Some(read::EditBlockReason::LegacyYaml)
    );

    let result = read::copy_password(
        app.state::<AppState>(),
        app.handle().clone(),
        test_repo_id(),
        "y.age".into(),
        None,
    )
    .await
    .expect("copy");
    assert!(result.password_empty);
    assert!(!result.password_non_utf8);
    assert_eq!(result.cleared_after_secs, 0, "no clipboard clear scheduled");
}
