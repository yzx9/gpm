// SPDX-FileCopyrightText: 2026 Zexin Yuan <gpm@yzx9.xyz>
//
// SPDX-License-Identifier: Apache-2.0

//! rustpass — a Rust library for age-encrypted gopass password stores.
//!
//! Provides read-only access to gopass-compatible password stores:
//! list entries, decrypt secrets, and sync (pull) from git remotes.
//!
//! # Quick start
//!
//! ```no_run
//! use rustpass::Store;
//! use std::path::PathBuf;
//!
//! let store = Store::new(PathBuf::from("/path/to/config"));
//! store.configure("https://example.com/repo.git", None, None, None, "AGE-SECRET-KEY-...")?;
//!
//! for entry in store.list()? {
//!     println!("{}", entry.name);
//! }
//!
//! let secret = store.get("cloud/aws/root")?;
//! println!("password: {}", secret.password());
//! # Ok::<(), rustpass::Error>(())
//! ```

#![warn(
    trivial_casts,
    trivial_numeric_casts,
    missing_copy_implementations,
    missing_debug_implementations,
    missing_docs,
    unsafe_code,
    unstable_features,
    unused_import_braces,
    unused_qualifications,
    clippy::dbg_macro,
    clippy::indexing_slicing,
    clippy::pedantic
)]

/// Configuration and identity persistence.
pub mod config;
/// Age decryption backend.
pub mod crypto;
/// Password store entry type.
pub mod entry;
/// Error types with safe (no-secret) messages.
pub mod error;
/// Git clone and pull operations.
pub mod git;
/// Decrypted secret type (gopass.Secret aligned).
pub mod secret;
/// SSH key generation and management.
pub mod ssh;
/// High-level store facade (gopass.Store aligned).
pub mod store;

// Re-export core types at crate root (gopass-aligned)
pub use config::{Config, RepoConfig};
pub use entry::Entry;
pub use error::{Error, ErrorCode};
pub use secret::Secret;
pub use store::{Store, SyncResult};
