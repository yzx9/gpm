// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Multi-repository registry (R080): an ordered in-memory index of independent
//! `Store` facades, one per repository ("vault"). Each facade is rooted at its
//! own per-repo directory; this module holds the constructed facades plus the
//! per-repository one-shot state that used to live as singletons on `AppState`.
//!
//! **Source of truth for the id list / order / last-active** is the device-scoped
//! `AppConfig` (`app.json` — `repositories` + `last_active`, sealed under the
//! auth-free master key). This struct is the in-memory index built from it: it
//! owns the `Arc<Store>` facades (which `AppConfig` cannot — it is plain data)
//! and resolves a `RepoId` to its facade in O(1).
//!
//! Step 1 introduces the structure with exactly one repository, behavior-
//! identical to today's single-repo app. The on-disk per-repo relocation
//! (`config_dir/repositories/<id>/`) lands with the `m0009` migration; until
//! then the facade is rooted at `config_dir` (the historical single-repo layout).

// R080 multi-repository is introduced incrementally: the `RepoEntry` one-shots
// and the wider `RepoRegistry` API land now and are consumed by the threading
// (Tasks 3–4), state-store removal (Task 5), relocate (Task 6), and worker
// fan-out (Task 7) steps. Allow unused items until those wire up.
#![allow(dead_code)]

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU8};
use std::sync::{Arc, Mutex, RwLock};

use rustpass::Store;

/// One-shot state machine values shared with the legacy `AppState` fields of the
/// same name (see `lib.rs`): `0 = Pending`, `1 = InFlight`, `2 = Done`.
const ONE_SHOT_PENDING: u8 = 0;

/// A stable, opaque repository identifier (a short string, e.g. random hex).
///
/// Generated when a repository is added and **never changes** — renaming a
/// repository is a display concern that touches only its display name, not its
/// identity, directory, or any reference. The user never sees this value; they
/// see a name derived from the remote URL.
///
/// `#[serde(transparent)]` so it crosses the Tauri IPC boundary as a plain
/// string (the frontend sends `repoId: "abc123"`, deserialized into `RepoId`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub(crate) struct RepoId(String);

impl RepoId {
    /// Wrap an already-validated id string.
    #[must_use]
    pub(crate) fn new(id: String) -> Self {
        Self(id)
    }

    /// The raw id string.
    #[must_use]
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// Generate a fresh opaque id: 16 random bytes from the OS CSPRNG,
    /// hex-encoded (32 chars, URL-segment-safe for future `/:repoId` routing).
    /// Used when a repository is first adopted into the registry (the `m0009`
    /// register migration, and the setup/add-repository path).
    #[allow(clippy::indexing_slicing)] // a 16-entry table indexed by a nibble (0–15)
    pub(crate) fn generate() -> Result<Self, rustpass::Error> {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut buf = [0u8; 16];
        rustpass::rng::fill_random(&mut buf)?;
        let mut id = String::with_capacity(32);
        for byte in buf {
            id.push(HEX[usize::from(byte >> 4)] as char);
            id.push(HEX[usize::from(byte & 0x0f)] as char);
        }
        Ok(Self(id))
    }
}

impl From<String> for RepoId {
    fn from(id: String) -> Self {
        Self(id)
    }
}

impl From<&str> for RepoId {
    fn from(id: &str) -> Self {
        Self(id.to_string())
    }
}

impl fmt::Display for RepoId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// One repository: its `Store` facade plus the per-repository one-shot state
/// (moved off `AppState` so it is correct by construction for any number of
/// repositories, not just one). Held behind `Arc` so command handlers can clone
/// a cheap handle to the entry/facade.
pub(crate) struct RepoEntry {
    /// This repository's stable identifier.
    pub(crate) id: RepoId,
    /// The self-contained store facade, rooted at this repository's directory.
    pub(crate) facade: Arc<Store>,
    /// One-shot state for the post-unlock storage-backend resolve (mirrors the
    /// legacy `AppState::backend_resolve_state`).
    pub(crate) backend_resolve_state: AtomicU8,
    /// One-shot state for the post-unlock crypto-backend resolve.
    pub(crate) crypto_resolve_state: AtomicU8,
    /// One-shot state for the post-unlock legacy identity-envelope migrate
    /// (`GPMATR1` → `GPMSEL1`).
    pub(crate) seal_migrate_state: AtomicU8,
    /// Cached `RepoConfig.unlock_identity_with_app`: when true the identity
    /// session follows the app-launch gate (mirrors `identity_coupled`).
    pub(crate) identity_coupled: AtomicBool,
    /// Cancel slot for this repository's in-flight clone/pull/push (mirrors
    /// `active_cancel_slot`).
    pub(crate) cancel_slot: rustpass::CancelSlot,
}

impl RepoEntry {
    /// Construct an entry around an existing facade; one-shots start pending.
    fn new(id: RepoId, facade: Arc<Store>) -> Self {
        Self {
            id,
            facade,
            backend_resolve_state: AtomicU8::new(ONE_SHOT_PENDING),
            crypto_resolve_state: AtomicU8::new(ONE_SHOT_PENDING),
            seal_migrate_state: AtomicU8::new(ONE_SHOT_PENDING),
            identity_coupled: AtomicBool::new(false),
            cancel_slot: Arc::new(Mutex::new(None)),
        }
    }
}

/// Ordered in-memory index of repositories, keyed by [`RepoId`].
///
/// The id list and order mirror `AppConfig.repositories`; `last_active` mirrors
/// `AppConfig.last_active`. Mutations that change the set (add/remove) must
/// update BOTH this index and `AppConfig` (persisted) — see the add/remove call
/// sites in setup / `reset_config`. Lookups (`facade`, `entry`) are read-only
/// and lock-free over a short critical section.
pub(crate) struct RepoRegistry {
    /// Ordered repositories (the order is the user's vault order).
    entries: RwLock<Vec<Arc<RepoEntry>>>,
    /// `O(1)` id → entry lookup, kept in sync with `entries`.
    by_id: RwLock<HashMap<RepoId, Arc<RepoEntry>>>,
    /// The active repository id (the vault the user is "in"). Mirrors
    /// `AppConfig.last_active`.
    last_active: RwLock<Option<RepoId>>,
}

impl RepoRegistry {
    /// An empty registry (no repositories configured — fresh install / pre-setup).
    #[must_use]
    pub(crate) fn empty() -> Self {
        Self {
            entries: RwLock::new(Vec::new()),
            by_id: RwLock::new(HashMap::new()),
            last_active: RwLock::new(None),
        }
    }

    /// Build the index from an ordered id list, constructing one facade per id
    /// via `facade_for(id)`. `last_active` is taken as-is (it should already name
    /// one of `ids`, else [`active_facade`](Self::active_facade) falls back to the
    /// first). Used by tests; startup uses [`empty`](Self::empty) +
    /// [`populate`](Self::populate) (the registry is filled after migrations run,
    /// once `AppState` already exists).
    pub(crate) fn from_ids<I, F>(ids: I, last_active: Option<RepoId>, facade_for: F) -> Self
    where
        I: IntoIterator<Item = RepoId>,
        F: Fn(&RepoId) -> Arc<Store>,
    {
        let registry = Self::empty();
        registry.populate(ids, last_active, facade_for);
        registry
    }

    /// (Re)populate an existing registry (e.g. the one on `AppState`, which starts
    /// empty and is filled after the migration chain assigns/persists the id
    /// list). Replaces all entries. `facade_for(id)` constructs one facade per id.
    pub(crate) fn populate<I, F>(&self, ids: I, last_active: Option<RepoId>, facade_for: F)
    where
        I: IntoIterator<Item = RepoId>,
        F: Fn(&RepoId) -> Arc<Store>,
    {
        let mut entries: Vec<Arc<RepoEntry>> = Vec::new();
        let mut by_id: HashMap<RepoId, Arc<RepoEntry>> = HashMap::new();
        for id in ids {
            let entry = Arc::new(RepoEntry::new(id.clone(), facade_for(&id)));
            by_id.insert(id.clone(), Arc::clone(&entry));
            entries.push(entry);
        }
        *self.entries.write().expect("registry entries poisoned") = entries;
        *self.by_id.write().expect("registry by_id poisoned") = by_id;
        *self
            .last_active
            .write()
            .expect("registry last_active poisoned") = last_active;
    }

    /// Look up a repository's facade by id (`None` if unknown — callers surface a
    /// clear not-found error, never a panic and never a secret leak).
    #[must_use]
    pub(crate) fn facade(&self, id: &RepoId) -> Option<Arc<Store>> {
        self.by_id
            .read()
            .expect("registry by_id poisoned")
            .get(id)
            .map(|e| Arc::clone(&e.facade))
    }

    /// Look up a full entry (facade + one-shots) by id.
    #[must_use]
    pub(crate) fn entry(&self, id: &RepoId) -> Option<Arc<RepoEntry>> {
        self.by_id
            .read()
            .expect("registry by_id poisoned")
            .get(id)
            .cloned()
    }

    /// The ordered id list (the user's vault order).
    #[must_use]
    pub(crate) fn list_ids(&self) -> Vec<RepoId> {
        self.entries
            .read()
            .expect("registry entries poisoned")
            .iter()
            .map(|e| e.id.clone())
            .collect()
    }

    /// The active repository id, if any.
    #[must_use]
    pub(crate) fn last_active(&self) -> Option<RepoId> {
        self.last_active
            .read()
            .expect("registry last_active poisoned")
            .clone()
    }

    /// The active repository's facade, falling back to the first repository if
    /// `last_active` is unset or names a missing id (defensive against a
    /// corrupt/partial upgrade — never panics). `None` only when the registry is
    /// empty (⇒ route to setup).
    ///
    /// No caller in Step 1 (commands resolve an explicit `repo_id` via
    /// [`facade`](Self::facade); the active-repo concept only matters once
    /// switching exists). The two-phase read (`last_active` then `by_id` in
    /// separate critical sections) is a latent TOCTOU — rework this before
    /// Step 2 switching lands.
    #[must_use]
    pub(crate) fn active_facade(&self) -> Option<Arc<Store>> {
        let last = self.last_active();
        let by_id = self.by_id.read().expect("registry by_id poisoned");
        if let Some(id) = last
            && let Some(entry) = by_id.get(&id)
        {
            return Some(Arc::clone(&entry.facade));
        }
        drop(by_id);
        // Fall back to the first repository (entries is the ordered source).
        self.entries
            .read()
            .expect("registry entries poisoned")
            .first()
            .map(|e| Arc::clone(&e.facade))
    }

    /// Set the active repository id. (Persistence of `last_active` to `app.json`
    /// is the caller's job — switching lands in Step 2.)
    pub(crate) fn set_last_active(&self, id: RepoId) {
        *self
            .last_active
            .write()
            .expect("registry last_active poisoned") = Some(id);
    }

    /// Number of registered repositories.
    #[must_use]
    pub(crate) fn len(&self) -> usize {
        self.entries.read().expect("registry entries poisoned").len()
    }

    /// Whether no repository is configured.
    #[must_use]
    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a registry over temp-dir-backed facades and exercise the accessors.
    /// The facades are real `Store`s (construction does no I/O), rooted at a
    /// per-test temp dir so nothing leaks across cases.
    #[test]
    fn from_ids_indexes_facades_and_resolves_active() {
        let dir = tempfile::tempdir().unwrap();
        let ids: Vec<RepoId> = vec!["alpha".into(), "beta".into()];
        let root = dir.path().to_path_buf();
        let registry = RepoRegistry::from_ids(ids.clone(), Some("beta".into()), |id| {
            // Distinct subdir per id so two facades never share a config dir.
            Arc::new(Store::new(root.join(id.as_str()), None))
        });

        // Both ids are indexed and resolvable.
        assert_eq!(registry.list_ids(), ids);
        assert!(registry.facade(&"alpha".into()).is_some());
        assert!(registry.facade(&"missing".into()).is_none());

        // last_active resolves; active_facade returns it.
        assert_eq!(registry.last_active(), Some("beta".into()));
        assert!(registry.active_facade().is_some());

        // Defensive fallback: a last_active naming a missing id ⇒ first repo.
        let registry2 = RepoRegistry::from_ids(ids, Some("ghost".into()), |id| {
            Arc::new(Store::new(root.join(id.as_str()), None))
        });
        assert!(registry2.active_facade().is_some());

        // Empty registry ⇒ no active facade.
        let empty = RepoRegistry::empty();
        assert!(empty.is_empty());
        assert!(empty.active_facade().is_none());
    }
}
