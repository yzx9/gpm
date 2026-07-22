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
//! # #[tokio::main]
//! # async fn main() -> Result<(), rustpass::Error> {
//! let store = Store::new(PathBuf::from("/path/to/config"), None);
//! store.configure("https://example.com/repo.git", None, None, None, "AGE-SECRET-KEY-...", None).await?;
//!
//! for entry in store.list().await? {
//!     println!("{}", entry.name);
//! }
//!
//! let secret = store.get("cloud/aws/root").await?;
//! println!("password: {}", secret.password());
//! # Ok(())
//! # }
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
/// Password generator (gopass `pkg/pwgen` analogue).
pub mod generator;
/// Identity type classification.
pub mod identity;
/// Recipient discovery and identity validation.
pub mod recipient;
/// Cryptographically-strong randomness primitives (OS CSPRNG + uniform index).
pub mod rng;
/// At-rest AEAD encryption for local private files (`repo.json`, `identity`).
pub mod seal;
/// Decrypted secret type (gopass.Secret aligned).
pub mod secret;
/// Git commit signature extraction + SSH-sig verification (repo authenticity).
pub mod signing;
/// SSH key generation and management.
pub mod ssh;
/// Storage / RCS backend abstraction (sync, write, commit-identity result
/// types; future home of the `StorageBackend` trait).
pub mod storage;
/// High-level store facade (gopass.Store aligned).
pub mod store;
/// gopass-compatible content templates and create presets.
pub mod template;
/// TOTP (RFC 6238) generation for entries storing a 2FA seed (gopass `pkg/otp`).
pub mod totp;

// Re-export core types at crate root (gopass-aligned)
pub use config::{Config, LockMode, RepoConfig};
pub use entry::Entry;
pub use error::{Error, ErrorCode};
pub use generator::{GenerateMode, GenerateOptions, generate_password};
pub use recipient::{IdentityInfo, KeyType, Recipient};
pub use secret::Secret;
pub use signing::{
    AuthenticityConfig, CommitSigInfo, CommitSigPage, CommitSigStatus, IgnoredIssue, TrustedGpgKey,
    TrustedKey, VerifyMode, fingerprint_of_public_key,
};
pub use storage::{CancelToken, GitProgress, ProgressSender, StoreBuilder};
pub use store::{
    CommitIdentity, DivergenceChoice, RankedPage, Store, SyncDivergence, SyncOutcome, SyncResult,
    WriteOutcome, WriteResult, clamp_lock_mode, normalize_clear_secs,
};
pub use totp::{Otp, extract, generate_at};

/// Upper bound on a trusted GPG/OpenPGP armored public key's size (paste OR
/// file import). Armored pubkeys are small; this rejects a mis-pasted/mis-picked
/// multi-MB blob before rpgp parses it. Enforced at the `Store::add_trusted_gpg_key`
/// chokepoint so both the paste and file-import paths share one guard.
pub const MAX_GPG_KEY_FILE_BYTES: usize = 64 * 1024;

/// Test-only serializer for identity-crypto (age-scrypt) round-trips in this
/// crate's unit-test binary.
///
/// Concurrent age-scrypt identity round-trips intermittently fail with
/// `WRONG_PASSPHRASE` on a correct passphrase — 0/100 single-threaded, up to
/// ~83% at `--test-threads=32`, with byte-identical on-disk input (so not an
/// IO race). The root cause is unconfirmed. The failure signature — correct
/// when alone, intermittent under concurrency — is the fingerprint of a data
/// race or undefined behavior, not a codegen miscompilation: deterministic
/// crypto miscompiled would fail deterministically on every call. The likely
/// host is latent UB in a dependency's hand-written SIMD code path (the sha2
/// x86_64 backend is the prime candidate), surfaced under aggressive
/// optimization.
///
/// As a test-only stopgap, a 1-permit serializer forces the provably-correct
/// single-threaded path: test bodies that round-trip an encrypted identity
/// acquire [`crypto_permit`] and hold it for the whole body. This guards tests
/// only — the shipped binary runs the same crypto under `opt-level = "z"` with
/// no serializer, so production safety rests on the app never running two
/// identity decrypts concurrently, which is an unverified invariant.
///
/// To localize the real cause, drop these permits and rerun the gated tests
/// under an opt-level=1 dependency build (`[profile.dev.package."*"]
/// opt-level = 1`) and under ThreadSanitizer (`cargo +nightly
/// -Zsanitizer=thread`): opt-level dependence implicates codegen, a TSan
/// report implicates a data race (likely in a dependency's SIMD backend). A
/// confirmed dependency bug is filed or pinned upstream; if production can
/// ever run two identity decrypts concurrently, the fix belongs at that call
/// site, not behind this test-only gate. Mirrors the per-binary serializer in
/// `rustpass/tests/common/mod.rs` (the failure is intra-binary, so the lib's
/// unit-test binary and each integration binary each get their own).
#[cfg(test)]
mod test_crypto_gate {
    use tokio::sync::{Semaphore, SemaphorePermit};

    static CRYPTO_SEM: Semaphore = Semaphore::const_new(1);

    /// Acquire the lib unit-test crypto serializer. Hold the returned permit for
    /// the whole test body (e.g. `let _crypto = test_crypto_gate::crypto_permit().await;`)
    /// so age-scrypt KDFs never run concurrently.
    pub(crate) async fn crypto_permit() -> SemaphorePermit<'static> {
        CRYPTO_SEM
            .acquire()
            .await
            .expect("crypto semaphore should never close")
    }
}
