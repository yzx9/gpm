// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Registry lifecycle tests (R080 step 1, post-review): startup facade
//! seeding, unlock-time re-population, the schema-gated startup reconcile, and
//! the Emergency Reset wipe contract.
//!
//! These deliberately build DIVERGENT states (device facade at the config
//! root, repo facade rooted at `repositories/<id>/`) — the shape production
//! has post-m0010. The shared `make_unlocked_state` factory registers a
//! ptr-equal facade, which masks exactly the device-vs-repo bugs these tests
//! pin.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64};
use std::sync::{Arc, Mutex};

use rustpass::{LockMode, Store};

use crate::AppState;
use crate::app_config::{AppConfig, AppConfigStore};
use crate::migrations::APP_CONFIG_SCHEMA_VERSION;
use crate::registry::RepoId;

const ID: &str = "0123456789abcdef0123456789abcdef";

/// Build an `AppState` whose device facade is rooted at `dir`, with `cfg`
/// persisted as its `app.json` and the config cache loaded (display +
/// behavior halves).
async fn state_over(dir: &Path, cfg: AppConfig) -> AppState {
    let store = Arc::new(Store::new(dir.to_path_buf(), None));
    store
        .save_app_config(&serde_json::to_vec(&cfg).unwrap())
        .await
        .unwrap();
    let app_config = AppConfigStore::new(dir).await;
    app_config.set_store(Arc::clone(&store));
    app_config.reload_behavior().await.ok();
    AppState {
        store,
        master_key: None,
        registry: crate::registry::RepoRegistry::empty(),
        app_config: Arc::new(app_config),
        app_handle: None,
        lock_timer: crate::identity::IdleTimer::new(),
        pending_identity: Mutex::new(None),
        lock_mode: Mutex::new(LockMode::default()),
        clipboard_clear_secs: Mutex::new(rustpass::config::DEFAULT_CLIPBOARD_CLEAR_SECS),
        clipboard_clear_handle: Mutex::new(None),
        clipboard_clear_generation: Arc::new(AtomicU64::new(0)),
        app_lock_enabled: AtomicBool::new(false),
        app_locked: Arc::new(AtomicBool::new(false)),
        gate_idle_timer: crate::identity::IdleTimer::new(),
        last_activity_at: Mutex::new(std::time::Instant::now()),
        cached_entry: Arc::new(Mutex::new(None)),
        entry_cache_timer: crate::identity::IdleTimer::new(),
        identity_coupled: AtomicBool::new(false),
        seal_migrate_state: AtomicU8::new(0),
        backend_resolve_state: AtomicU8::new(0),
        active_cancel_slot: Arc::new(Mutex::new(None)),
        verbose_timer: Mutex::new(None),
        verbose_generation: Arc::new(AtomicU64::new(0)),
    }
}

/// Create a real (committed) git clone at `dir` with one `.age` entry, so
/// `Store::list` is functional against the resolved storage backend.
fn git_clone_with_entry(dir: &Path) {
    std::fs::create_dir_all(dir.join("servers")).unwrap();
    std::fs::write(dir.join("servers/prod.age"), b"secret\n").unwrap();
    for args in [
        vec!["init", "-q", "--initial-branch=main"],
        vec!["add", "-A"],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-qm",
            "init",
        ],
    ] {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(&args)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
    }
}

/// Seed one repo's post-relocate footprint — `repo.json` (plaintext, desktop
/// passthrough) + `identity` + a real clone under
/// `<root>/repositories/<id>/`. The `app.json` is the caller's business
/// (`state_over` persists it).
fn seed_relocated_repo_files(root: &Path, id: &str) {
    let subdir = crate::repositories_dir(root).join(id);
    let clone = subdir.join("repo");
    git_clone_with_entry(&clone);
    std::fs::write(
        subdir.join("repo.json"),
        format!(
            r#"{{"url":"https://x/repo.git","local_path":"{}"}}"#,
            clone.display()
        ),
    )
    .unwrap();
    std::fs::write(subdir.join("identity"), b"identity-bytes").unwrap();
}

/// Seed a repo's files at the config ROOT (the pre-relocate /
/// stranded-on-crash footprint).
fn seed_root_repo_files(root: &Path) {
    seed_root_clone_and_config(root);
    std::fs::write(root.join("identity"), b"identity-bytes").unwrap();
}

/// The mid-setup footprint: clone + `repo.json` at the config root, NO identity
/// (`clone_repo` completed, `complete_setup` never ran).
fn seed_root_clone_and_config(root: &Path) {
    let clone = root.join("repo");
    git_clone_with_entry(&clone);
    std::fs::write(
        root.join("repo.json"),
        format!(
            r#"{{"url":"https://x/repo.git","local_path":"{}"}}"#,
            clone.display()
        ),
    )
    .unwrap();
}

/// Populate the registry from the config cache the way `init_state` does.
fn populate_registry(state: &AppState) {
    let cfg = state.app_config.get();
    let ids = cfg
        .repositories
        .iter()
        .map(|s| RepoId::from(s.clone()))
        .collect::<Vec<_>>();
    let last_active = cfg.last_active.map(RepoId::from);
    state
        .registry
        .populate(ids, last_active, state.facade_builder());
}

/// C1+C2: registry facades built bare (`Store::new`) are backend-less and
/// default `autosync = true`. `seed_registry_facades` must resolve the storage
/// backend (a subsequent `list` works instead of `BACKEND_NOT_AVAILABLE`) and
/// seed the persisted autosync pref onto the facade.
#[tokio::test]
async fn seed_registry_facades_resolves_backends_and_seeds_autosync() {
    let dir = tempfile::tempdir().unwrap();
    seed_relocated_repo_files(dir.path(), ID);
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        autosync: false, // persisted OFF
        ..AppConfig::default()
    };
    let state = state_over(dir.path(), cfg).await;
    populate_registry(&state);

    let facade = state.registry.facade(&RepoId::from(ID)).expect("facade");
    // Preconditions pin the bug this test guards against.
    let before = facade.list(0, 10).await.unwrap_err();
    assert_eq!(
        before.code, "BACKEND_NOT_AVAILABLE",
        "bare facade must be unresolved"
    );
    assert!(facade.autosync(), "bare facade defaults autosync=true");

    crate::seed_registry_facades(&state).await;

    let page = facade.list(0, 10).await.expect("list works after seeding");
    assert_eq!(page.entries.len(), 1, "the seeded entry lists");
    assert!(
        !facade.autosync(),
        "persisted autosync=OFF is seeded onto the facade"
    );
}

/// C3/RT3: a deferred chain that completes at unlock rewrites the registry on
/// disk after `init_state` — `repopulate_registry_if_empty` must build + seed
/// the facades; when the registry is already populated it must be a no-op
/// (never swap live facades out from under the session).
#[tokio::test]
async fn repopulate_builds_when_empty_and_preserves_when_populated() {
    let dir = tempfile::tempdir().unwrap();
    seed_relocated_repo_files(dir.path(), ID);
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        autosync: false,
        ..AppConfig::default()
    };
    let state = state_over(dir.path(), cfg).await;

    // Empty ⇒ builds.
    assert!(state.registry.is_empty());
    crate::repopulate_registry_if_empty(&state).await;
    let facade = state
        .registry
        .facade(&RepoId::from(ID))
        .expect("repopulated facade");
    assert!(
        !Arc::ptr_eq(&facade, &state.store),
        "the registry facade diverges from the device facade"
    );
    assert!(!facade.autosync(), "seeded with the persisted pref");
    facade.list(0, 10).await.expect("backend resolved");

    // Populated ⇒ no-op (same facade Arc survives).
    crate::repopulate_registry_if_empty(&state).await;
    let after = state
        .registry
        .facade(&RepoId::from(ID))
        .expect("facade survives");
    assert!(
        Arc::ptr_eq(&facade, &after),
        "a populated registry is not rebuilt"
    );
}

/// C5: while the migration chain is still below current schema (the app-lock
/// deferral window), the reconcile must NOT relocate — the pending migrations
/// read the files via the config-root-rooted device facade.
#[tokio::test]
async fn startup_reconcile_skips_while_migrations_pending() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let cfg = AppConfig {
        schema_version: 9, // mid-migrate: m0010 still pending
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;

    crate::setup::startup_reconcile(&state).await.unwrap();

    assert!(
        root.join("repo.json").exists(),
        "mid-migrate: files must stay at the config root"
    );
    assert!(
        !crate::repositories_dir(root)
            .join(ID)
            .join("repo.json")
            .exists(),
        "mid-migrate: no relocation"
    );
}

/// P2-a/T2: an unregistered configured repo at the root (setup died before the
/// registry write) is adopted: files relocate under `repositories/<id>/`, the
/// id lands in `app.json`, and the registry resolves a facade that reads the
/// relocated repo.json. A re-run preserves the minted id.
#[tokio::test]
async fn startup_reconcile_adopts_unregistered_configured_repo() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;

    crate::setup::startup_reconcile(&state).await.unwrap();

    let id = state
        .app_config
        .get()
        .repositories
        .first()
        .cloned()
        .unwrap();
    let subdir = crate::repositories_dir(root).join(&id);
    for name in ["repo.json", "identity"] {
        assert!(subdir.join(name).exists(), "{name} relocated");
        assert!(!root.join(name).exists(), "{name} gone from the root");
    }
    assert!(subdir.join("repo").exists(), "clone relocated");
    assert_eq!(
        state.app_config.get().last_active.as_deref(),
        Some(id.as_str())
    );
    let facade = state
        .active_repo()
        .expect("registry populated by the adopt");
    assert!(!Arc::ptr_eq(&facade, &state.store), "divergent facade");
    assert!(
        facade.config().await.is_ok(),
        "the adopted facade reads the relocated repo.json"
    );
    // Re-run is a no-op that preserves the minted id (idempotency).
    let before = state.app_config.get().repositories.clone();
    crate::setup::register_first_repo(&state).await.unwrap();
    assert_eq!(state.app_config.get().repositories, before);
}

/// P2-a: the half-registered state — the id persisted to `app.json` but the
/// process died before the files moved. The reconcile must FINISH the
/// relocate for the registered id (the state both emptiness-gated recovery
/// paths used to miss).
#[tokio::test]
async fn startup_reconcile_finishes_half_registered_relocate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;
    populate_registry(&state); // registry non-empty, facade rooted at the subdir

    crate::setup::startup_reconcile(&state).await.unwrap();

    let subdir = crate::repositories_dir(root).join(ID);
    for name in ["repo.json", "identity"] {
        assert!(
            subdir.join(name).exists(),
            "{name} finished into the subdir"
        );
        assert!(!root.join(name).exists(), "{name} gone from the root");
    }
    assert!(
        subdir.join("repo").exists(),
        "clone finished into the subdir"
    );
}

/// T2 retry-safety: when `app.json` already carries an id from a prior
/// partial run (registry empty, files at the root), the adopt path REUSES it
/// rather than minting a fresh one (which would orphan the half-moved repo).
#[tokio::test]
async fn adopt_reuses_a_persisted_id_instead_of_minting() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        repositories: vec![ID.to_string()], // persisted by the prior partial run
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;

    crate::setup::startup_reconcile(&state).await.unwrap();

    assert_eq!(
        state.app_config.get().repositories,
        vec![ID.to_string()],
        "the persisted id is reused, not replaced"
    );
    assert!(
        crate::repositories_dir(root)
            .join(ID)
            .join("repo.json")
            .exists(),
        "the files land under the PERSISTED id's subdir"
    );
}

/// DM2 proxy: an IO error inside the relocate (here: `repositories` exists as
/// a FILE, so `create_dir_all` fails) must propagate as an error — never be
/// mistaken for "already moved" (which would strand files past the schema
/// bump).
#[tokio::test]
async fn relocate_io_error_propagates_instead_of_skipping() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(
        root.join("repo.json"),
        r#"{"url":"https://x/repo.git","local_path":"/p"}"#,
    )
    .unwrap();
    std::fs::write(root.join("repositories"), b"not a directory").unwrap();
    let store = Store::new(root.to_path_buf(), None);

    crate::relocate_repo_into_subdir(&store, &RepoId::from(ID.to_string()))
        .await
        .unwrap_err();

    assert!(
        root.join("repo.json").exists(),
        "the failing run leaves the files untouched"
    );
}

/// Round-3 C1 (R097 seam): the relocation's `local_path` rewrite must go
/// through the cross-process `ConfigLock` — a concurrent repo.json writer
/// holding the lock must see the relocate FAIL (`ConfigBusy`), never write
/// past it (the interleaved-writer lost-update class R097 exists to close).
#[tokio::test]
async fn relocate_repo_json_rewrite_takes_the_config_lock() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let store = Store::new(root.to_path_buf(), None);

    // A concurrent writer holds the config lock on the same dir.
    let _guard = rustpass::ConfigLock::acquire(root).await.expect("lock");

    let err = crate::relocate_repo_into_subdir(&store, &RepoId::from(ID.to_string()))
        .await
        .expect_err("the local_path rewrite must respect a held ConfigLock");
    assert_eq!(
        err.code, "CONFIG_BUSY",
        "locked out, not a silent write: {err}"
    );
}

/// Round-3 C2: a restart between `clone_repo` and `complete_setup` (clone +
/// repo.json at the root, no identity) must NOT be adopted — adopting would
/// relocate repo.json into the subdir, after which `complete_setup`'s identity
/// lands on the device root while `register_first_repo` no-ops on the
/// non-empty registry: a facade with no identity, configured=false forever,
/// /setup loop. The reconcile leaves the half-setup footprint alone;
/// `complete_setup`'s own register then relocates EVERYTHING.
#[tokio::test]
async fn adopt_skips_half_setup_repo_so_complete_setup_lands_together() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_clone_and_config(root); // mid-setup: NO identity
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;

    crate::setup::startup_reconcile(&state).await.unwrap();

    assert!(
        root.join("repo.json").exists(),
        "mid-setup: the reconcile must not relocate"
    );
    assert!(
        state.registry.is_empty(),
        "mid-setup: the reconcile must not register"
    );

    // `complete_setup`'s tail, in order: identity save on the device facade,
    // then `register_first_repo` (registry still empty ⇒ it runs). The identity
    // write is the on-disk footprint only — `save_identity` would reject the
    // placeholder content, and identity parsing is not what this test pins.
    std::fs::write(root.join("identity"), b"identity-bytes").unwrap();
    crate::setup::register_first_repo(&state).await.unwrap();

    let facade = crate::setup::active_or_device_facade(&state);
    assert!(
        facade.is_configured(),
        "post-adopt setup must report configured=true (else the router loops to /setup)"
    );
    assert!(!Arc::ptr_eq(&facade, &state.store), "relocated facade");
}

/// Round-3 P1: a persistent m0010 failure (here: `repositories` exists as a
/// FILE, so the relocate can never create the subdir) must not register a
/// facade over the empty subdir — `populate_registry_from_config` skips the
/// id, the device-facade fallback serves the session, and re-setup stays
/// possible (registry empty ⇒ `register_first_repo` runs).
#[tokio::test]
async fn populate_skips_unrelocated_id_so_device_fallback_serves() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    std::fs::write(root.join("repositories"), b"not a directory").unwrap();
    let cfg = AppConfig {
        schema_version: 9, // m0009 registered the id; m0010 fails forever
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;

    crate::populate_registry_from_config(&state).await;

    assert!(
        state.registry.is_empty(),
        "an id whose files are provably still at the root must not register"
    );
    let facade = crate::setup::active_or_device_facade(&state);
    assert!(
        Arc::ptr_eq(&facade, &state.store),
        "the device facade serves while the chain retries"
    );
    assert!(
        facade.config().await.is_ok(),
        "the device facade reads the root repo.json"
    );
}

/// Round-3 C3: the `app_unlock` discriminator — a chain that halted over a
/// configured repo (schema < current AND repo.json at the root) is the state
/// whose session would have a dead unlock surface; `app_unlock` fails loudly
/// on it instead.
#[tokio::test]
async fn chain_left_repo_at_root_discriminates_halted_chain() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_root_repo_files(root);
    let halted = state_over(
        root,
        AppConfig {
            schema_version: 9, // chain halted before m0010
            repositories: vec![ID.to_string()],
            ..AppConfig::default()
        },
    )
    .await;
    assert!(
        crate::applock::chain_left_repo_at_root(&halted).await,
        "halted chain over a configured repo ⇒ true"
    );

    // Completed chain (schema current): false, files or not.
    let done = state_over(
        root,
        AppConfig {
            schema_version: APP_CONFIG_SCHEMA_VERSION,
            ..AppConfig::default()
        },
    )
    .await;
    assert!(
        !crate::applock::chain_left_repo_at_root(&done).await,
        "schema at current ⇒ false"
    );

    // No repo at all (pre-setup): false — the unlock proceeds to /setup.
    let empty = tempfile::tempdir().unwrap();
    let no_repo = state_over(
        empty.path(),
        AppConfig {
            schema_version: 9,
            ..AppConfig::default()
        },
    )
    .await;
    assert!(
        !crate::applock::chain_left_repo_at_root(&no_repo).await,
        "no root repo.json ⇒ false (not the dead-surface state)"
    );
}

/// P2-b: Emergency Reset's wipe contract — registered repo dir, root-stranded
/// repo files, and orphaned `repositories/<id>/` dirs are ALL wiped; the
/// device prefs in `app.json` survive and the registry is cleared.
#[tokio::test]
async fn reset_wipes_registered_root_stranded_and_orphaned_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    seed_relocated_repo_files(root, ID); // registered repo, relocated
    // Root-stranded leftovers (the half-state a user resets out of).
    std::fs::write(root.join("identity"), b"stranded-identity").unwrap();
    std::fs::write(
        root.join("repo.json"),
        r#"{"url":"https://x/repo.git","local_path":"/nonexistent-clone"}"#,
    )
    .unwrap();
    // Orphaned subdir never registered in app.json.
    let orphan = crate::repositories_dir(root).join("ffffffffffffffffffffffffffffffff");
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(orphan.join("identity"), b"orphan-identity").unwrap();
    let cfg = AppConfig {
        schema_version: APP_CONFIG_SCHEMA_VERSION,
        repositories: vec![ID.to_string()],
        last_active: Some(ID.to_string()),
        locale: Some("zh-CN".to_string()), // a device pref that must survive
        ..AppConfig::default()
    };
    let state = state_over(root, cfg).await;
    populate_registry(&state);

    crate::config::reset_config_core(&state).await.unwrap();

    let subdir = crate::repositories_dir(root).join(ID);
    assert!(!subdir.exists(), "registered repo dir wiped");
    assert!(!orphan.exists(), "orphaned repo dir wiped");
    for name in ["identity", "repo.json"] {
        assert!(!root.join(name).exists(), "root-stranded {name} wiped");
    }
    assert!(state.registry.is_empty(), "in-memory registry cleared");
    // Prefs survive; registry fields cleared.
    let after = {
        let ac = AppConfigStore::new(root).await;
        ac.set_store(Arc::new(Store::new(root.to_path_buf(), None)));
        ac.reload_behavior().await.ok();
        ac.get()
    };
    assert_eq!(
        after.locale.as_deref(),
        Some("zh-CN"),
        "device prefs survive"
    );
    assert!(after.repositories.is_empty() && after.last_active.is_none());
}
