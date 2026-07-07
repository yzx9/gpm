// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! The git storage backend — sole [`StorageBackend`] implementation.
//!
//! Split by concern across sibling modules:
//! - [`backend`] — [`GitStorage`] + the async `StorageBackend` impl (adapts the
//!   blocking RCS free functions via `spawn_blocking`).
//! - [`worktree`] — working-tree file ops + within-repo path guards
//!   (list/get/set/delete `.age` entries, recipients, templates).
//! - [`transport`] — Android HTTPS CA bundle, libgit2 callbacks, the shared
//!   fetch-into-temp primitive, and error mapping.
//! - [`commit`] — clone/init/remote_add/commit/push (the write surface).
//! - [`pull`] — sync read-in, authenticity-verified checkout, `adopt_remote`.
//! - [`divergence`] — divergence preview + keep-mine resolution.
//! - [`util`] — shared git helpers (`short_hash`, `gpm_signature`, …).
//!
//! `GitStorage` is stateless — auth/policy are passed per-op, not held at
//! construction (the real durable state is git's on-disk index, re-attached each
//! op via `Repository::discover`).

mod backend;
mod commit;
mod divergence;
mod pull;
#[cfg(test)]
mod test_support;
mod transport;
mod util;
mod worktree;

pub use backend::GitStorage;
pub use worktree::{list_entries, resolve_entry_path};

pub(crate) use worktree::passfile_rel;
