// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! Storage-backend registration (RFC 0049).
//!
//! The seam that lets the app layer supply rustpass a storage backend it
//! cannot construct itself — the motivating case being the Android cloud-
//! folder backend, whose implementation bridges to Kotlin and so cannot live
//! in pure-Rust rustpass.
//!
//! Two namespaces, kept disjoint by a reserved prefix:
//! - **Built-ins** (`git`, later `local`) live in rustpass and are dispatched
//!   natively by [`StorageRegistry::resolve`] — no registration needed.
//! - **Extensions** (`ext:<name>`) are registered by the app at startup via
//!   [`StoreBuilder::register_storage`] and dispatched by lookup.
//!
//! rustpass resolves a backend from a persisted `backend` type field (in sealed
//! `repo.json`, unreadable until app unlock) plus an opaque root token, without
//! learning what any extension backend is. The registry is populated at startup
//! (the app knows which backends this build offers) and frozen after
//! [`StoreBuilder::build`]; the backend itself is constructed lazily post-unlock
//! (see `Store::resolve_storage`).

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use crate::error::{Error, ErrorCode};
use crate::store::Store;

use super::{GitStorage, StorageBackend};

/// Reserved prefix for extension backend names. Extension backends registered
/// by the app carry it; built-in backends (`git`, …) never do. This keeps the
/// two namespaces from shadowing each other and lets a future built-in never
/// break an existing extension.
const EXT_PREFIX: &str = "ext:";

/// The built-in backend name for git. `None` in `RepoConfig.backend` is
/// equivalent (the default — a config written before the field existed).
const BUILTIN_GIT: &str = "git";

/// Factory that constructs an extension backend from its opaque root token.
/// The app supplies one at startup; rustpass calls it at resolve time, handing
/// the root token through untouched (a path for built-ins, a document-tree URI
/// for the cloud-folder backend). rustpass interprets the token only for its
/// own built-ins.
type ExtFactory = Arc<dyn Fn(&str) -> Result<Box<dyn StorageBackend>, Error> + Send + Sync>;

/// The backend registry: built-ins dispatched natively, extensions by lookup.
/// Held by [`Store`] as `Arc<StorageRegistry>` and consulted at resolve time
/// (once, post-unlock). Immutable after [`StoreBuilder::build`].
#[derive(Default)]
pub(crate) struct StorageRegistry {
    /// `ext:<name>` → factory. Built-ins are NOT here — they are dispatched
    /// natively in [`Self::resolve`].
    extensions: HashMap<String, ExtFactory>,
}

impl StorageRegistry {
    /// Resolve a persisted backend type + opaque root token to a constructed
    /// backend.
    ///
    /// - `None` or `"git"` → the git built-in (constructed natively; the root
    ///   token is a filesystem path, threaded per-call today and owned by the
    ///   backend once RFC 0046 reshapes the trait).
    /// - `"ext:<name>"` → the registered extension factory, handed `root`
    ///   opaquely.
    /// - Anything else → [`ErrorCode::BackendNotAvailable`] (an unregistered
    ///   extension, or a name without the `ext:` prefix that isn't a built-in).
    ///
    /// # Errors
    ///
    /// [`ErrorCode::BackendNotAvailable`] when the type is an unregistered
    /// `ext:` name or an unknown non-prefixed name; otherwise whatever the
    /// extension factory returns.
    pub(crate) fn resolve(
        &self,
        backend: Option<&str>,
        root: &str,
    ) -> Result<Box<dyn StorageBackend>, Error> {
        match backend {
            None | Some(BUILTIN_GIT) => {
                // GitStorage is a stateless unit struct today; the root token is
                // threaded per-call (the concrete-path keying RFC 0046 reworks).
                let _ = root;
                Ok(Box::new(GitStorage))
            }
            Some(name) if name.starts_with(EXT_PREFIX) => {
                let factory = self.extensions.get(name).ok_or_else(|| {
                    Error::new(
                        ErrorCode::BackendNotAvailable,
                        format!("storage backend {name:?} is not available in this build"),
                    )
                })?;
                factory(root)
            }
            Some(name) => Err(Error::new(
                ErrorCode::BackendNotAvailable,
                format!(
                    "unknown storage backend {name:?} (not a built-in, and missing the '{EXT_PREFIX}' extension prefix)"
                ),
            )),
        }
    }
}

/// Construction-time host for the backend registry (RFC 0049).
///
/// Hosts **only** the registry — the sole piece of construction state that
/// must be available before `Store` can parse its sealed config and that
/// rustpass cannot construct itself. Per-instance inputs (`config_dir`,
/// `master_key`) are [`build`](Self::build) parameters, not builder fields, so
/// one builder can produce many stores (tests; a future multi-store).
///
/// A future `register_crypto` (RFC 0050, blocked) is the namespaced sibling —
/// hence [`register_storage`](Self::register_storage), not `register_ext`.
///
/// ```no_run
/// use rustpass::StoreBuilder;
/// use std::path::PathBuf;
///
/// let mut builder = StoreBuilder::new();
/// // Extensions registered here on Android (0046 wires the SAF backend);
/// // desktop builds register none.
/// let store = builder.build(PathBuf::from("/config"), None);
/// ```
#[derive(Default)]
pub struct StoreBuilder {
    extensions: HashMap<String, ExtFactory>,
}

impl fmt::Debug for StoreBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // The factories are `dyn Fn` (not `Debug`); expose only the count.
        f.debug_struct("StoreBuilder")
            .field("extensions", &self.extensions.len())
            .finish_non_exhaustive()
    }
}

impl StoreBuilder {
    /// Start a builder with no extension backends. Built-ins (git) are always
    /// available (dispatched natively) and need no registration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an extension storage backend under `ext:<name>`.
    ///
    /// `name` MUST carry the `ext:` prefix (the namespace contract) and MUST
    /// not duplicate an existing registration. `factory` constructs the backend
    /// from its opaque root token at resolve time.
    ///
    /// # Errors
    ///
    /// [`ErrorCode::StoreError`] when `name` lacks the `ext:` prefix or is
    /// already registered.
    pub fn register_storage<F>(&mut self, name: &str, factory: F) -> Result<&mut Self, Error>
    where
        F: Fn(&str) -> Result<Box<dyn StorageBackend>, Error> + Send + Sync + 'static,
    {
        if !name.starts_with(EXT_PREFIX) {
            return Err(Error::new(
                ErrorCode::StoreError,
                format!("extension backend name must start with '{EXT_PREFIX}': got {name:?}"),
            ));
        }
        if self.extensions.contains_key(name) {
            return Err(Error::new(
                ErrorCode::StoreError,
                format!("extension backend {name:?} is already registered"),
            ));
        }
        self.extensions.insert(name.to_string(), Arc::new(factory));
        Ok(self)
    }

    /// Build a [`Store`] backed by this builder's registry. `&self` so the
    /// builder is reusable — the registry's factories are `Arc`-shared, so the
    /// clone is refcount bumps (negligible).
    ///
    /// The backend itself is NOT constructed here: it lives in sealed
    /// `repo.json`, unreadable until app unlock. `Store` resolves it lazily
    /// post-unlock (see `Store::resolve_storage`).
    #[must_use]
    pub fn build(&self, config_dir: PathBuf, master_key: Option<[u8; 32]>) -> Store {
        Store::with_registry(
            config_dir,
            master_key,
            Arc::new(StorageRegistry {
                extensions: self.extensions.clone(),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// `None` (the default — a config written before the `backend` field existed)
    /// resolves to the git built-in natively.
    #[test]
    fn builtins_resolve_git_for_none() {
        let r = StorageRegistry::default();
        r.resolve(None, "/tmp").expect("None resolves to git");
    }

    /// An explicit `"git"` resolves to the git built-in (same as `None`).
    #[test]
    fn builtins_resolve_git_for_explicit_git() {
        let r = StorageRegistry::default();
        r.resolve(Some("git"), "/tmp")
            .expect("\"git\" resolves to git");
    }

    /// `register_storage` rejects names without the `ext:` prefix — the namespace
    /// contract keeps built-in and extension names disjoint.
    #[test]
    fn register_storage_rejects_non_ext_prefix() {
        let mut b = StoreBuilder::new();
        assert!(
            b.register_storage("git", |_| Ok(Box::new(GitStorage)))
                .is_err(),
            "\"git\" is a built-in, not an extension"
        );
        assert!(
            b.register_storage("cloud-folder", |_| Ok(Box::new(GitStorage)))
                .is_err(),
            "extension names must carry the ext: prefix"
        );
    }

    /// `register_storage` rejects a duplicate `ext:` name.
    #[test]
    fn register_storage_rejects_duplicates() {
        let mut b = StoreBuilder::new();
        b.register_storage("ext:mock", |_| Ok(Box::new(GitStorage)))
            .expect("first registration succeeds");
        assert!(
            b.register_storage("ext:mock", |_| Ok(Box::new(GitStorage)))
                .is_err(),
            "duplicate registration is rejected"
        );
    }

    /// An unregistered `ext:` name → `BackendNotAvailable` (e.g. a cloud-folder
    /// config opened on a desktop build that didn't register the SAF backend).
    #[test]
    fn resolve_unregistered_ext_returns_backend_not_available() {
        let r = StorageRegistry::default();
        let err = r.resolve(Some("ext:nope"), "/tmp").err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    /// A non-prefixed, non-built-in name → `BackendNotAvailable` (not silently
    /// treated as git).
    #[test]
    fn resolve_unknown_non_prefixed_name_returns_backend_not_available() {
        let r = StorageRegistry::default();
        let err = r.resolve(Some("fossil"), "/tmp").err().unwrap();
        assert_eq!(err.code, "BACKEND_NOT_AVAILABLE");
    }

    /// Non-default-backend injection: an `ext:` resolve MUST invoke the
    /// registered factory (a git-only test would pass even if the registry were a
    /// silent no-op). This is the test the `injected-cross-boundary-cache-stale-
    /// on-migration` learning mandates.
    #[test]
    fn ext_dispatch_invokes_the_factory_not_the_native_path() {
        let mut b = StoreBuilder::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_closure = called.clone();
        b.register_storage("ext:mock", move |_root| {
            called_closure.store(true, Ordering::SeqCst);
            Ok(Box::new(GitStorage))
        })
        .unwrap();
        let r = StorageRegistry {
            extensions: b.extensions.clone(),
        };
        r.resolve(Some("ext:mock"), "/tmp/whatever").unwrap();
        assert!(
            called.load(Ordering::SeqCst),
            "ext: dispatch must invoke the registered factory"
        );
    }

    /// The native git path must NOT invoke an extension factory.
    #[test]
    fn native_git_does_not_invoke_ext_factory() {
        let mut b = StoreBuilder::new();
        let called = Arc::new(AtomicBool::new(false));
        let called_closure = called.clone();
        b.register_storage("ext:mock", move |_| {
            called_closure.store(true, Ordering::SeqCst);
            Ok(Box::new(GitStorage))
        })
        .unwrap();
        let r = StorageRegistry {
            extensions: b.extensions.clone(),
        };
        r.resolve(None, "/tmp").unwrap();
        assert!(
            !called.load(Ordering::SeqCst),
            "native git must not invoke the ext factory"
        );
    }

    /// `build` is `&self`, so one builder produces many stores (registry
    /// `Arc`-shared).
    #[test]
    fn builder_is_reusable() {
        let mut b = StoreBuilder::new();
        b.register_storage("ext:mock", |_| Ok(Box::new(GitStorage)))
            .unwrap();
        let _s1 = b.build(PathBuf::from("/tmp/a"), None);
        let _s2 = b.build(PathBuf::from("/tmp/b"), None);
        // Two stores constructed from one builder without panic.
    }
}
